//! `tool.run` through the broker and the executor, with fake `git` and
//! `docker` executables (shell scripts in a temporary folder) that print
//! their argv, environment and working directory: exactly what runs, in
//! which environment, within which limits and grants, and what is audited.
//! Never the system's docker; never a network.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_core::{MachineResourceBinding, ResourceKey, ResourceKind as Core};
use deckpilot_plugin_abi::capability::{RunCause, RunContext, RunTrigger};
use deckpilot_plugin_abi::privileged::{ResourceKind, Tool, ToolOutput};
use deckpilot_plugin_abi::{CallErrorCode, CallResult, CapabilityRequest};
use deckpilot_plugin_broker::{ApprovedPermissions, Broker, Decision, DenyReason, PluginId};
use deckpilot_plugin_manifest::Manifest;
use deckpilot_plugin_supervisor::{AbortSignal, CallInterrupt, CapabilityExecutor};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::super::{MAX_LOG_OUTPUT, MAX_STDERR, MAX_STDOUT, ToolHost};
use crate::plugins::executor::resources::{InvocationResources, ParamResource};
use crate::plugins::executor::{
    DataAccess, Executor, Opener, OwnVariables, PrivilegedAudit, PrivilegedEntry, Transport,
};
use crate::plugins::variables::PublishedVariables;

const EXAMPLE: &str = include_str!(
    "../../../../../../crates/deckpilot-plugin-manifest/tests/fixtures/example-manifest.json"
);

/// Prints `ARG=` per argument, `ENV=` per variable, then `CWD=`.
const ECHO: &str = r#"for a in "$@"; do printf 'ARG=%s\n' "$a"; done
env | LC_ALL=C sort | sed 's/^/ENV=/'
printf 'CWD=%s\n' "$(pwd -P)"
"#;

/// How a fake `git` answers `rev-parse --show-toplevel --absolute-git-dir`
/// (its working directory for both).
const TOPLEVEL: &str = "case \" $* \" in *\" --show-toplevel \"*) pwd -P; pwd -P; exit 0;; esac\n";

/// Variables the shell running a fake sets itself.
const SHELL_OWN: &[&str] = &["PWD", "SHLVL", "_", "OLDPWD"];

/// The agent's environment in these tests: secrets next to what the tools
/// may keep.
const AGENT_ENV: &[(&str, &str)] = &[
    ("PATH", "/usr/bin::.:relative/bin:/bin"),
    ("HOME", "/home/someone"),
    ("LANG", "de_DE.UTF-8"),
    ("LC_ALL", "de_DE.UTF-8"),
    ("AWS_SECRET_ACCESS_KEY", "aws-secret"),
    ("GITHUB_TOKEN", "ghp_secret"),
    (
        "GIT_CONFIG_PARAMETERS",
        "'core.sshCommand'='touch /tmp/pwned'",
    ),
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_SSH_COMMAND", "touch /tmp/pwned"),
    ("GIT_ASKPASS", "/tmp/askpass"),
    ("GIT_DIR", "/etc"),
    ("GIT_EXEC_PATH", "/usr/libexec/git-core"),
    ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
    ("SSH_ASKPASS", "/tmp/askpass"),
    ("GCM_INTERACTIVE", "always"),
    ("GIT_CEILING_DIRECTORIES", "/"),
    ("DOCKER_HOST", "unix:///var/run/docker.sock"),
    ("DOCKER_CONFIG", "/nonexistent/docker-config"),
    ("COMPOSE_FILE", "/tmp/evil.yaml"),
    ("LD_PRELOAD", "/tmp/evil.so"),
    ("DYLD_INSERT_LIBRARIES", "/tmp/evil.dylib"),
];

/// A temporary tree: `repo/` (with a link to `outside/`), `outside/`,
/// `compose.yaml`, and `bin/` for the fakes.
struct Tree {
    dir: tempfile::TempDir,
}

impl Tree {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["repo", "outside", "bin"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(dir.path().join("compose.yaml"), "services: {}\n").unwrap();
        std::os::unix::fs::symlink(dir.path().join("outside"), dir.path().join("repo/link"))
            .unwrap();
        Self { dir }
    }

    fn path(&self, relative: &str) -> PathBuf {
        std::fs::canonicalize(self.dir.path())
            .unwrap()
            .join(relative)
    }

    /// A fake `name` in `bin/` running `body`. A fake `git` answers the
    /// host's `rev-parse --show-toplevel --absolute-git-dir` with its
    /// working directory.
    fn fake(&self, name: &str, body: &str) -> PathBuf {
        let path = self.path("bin").join(name);
        let toplevel = if name == "git" { TOPLEVEL } else { "" };
        write_script(&path, &format!("#!/bin/sh\n{toplevel}{body}"));
        path
    }
}

/// Writes the executable script `path` without this process ever holding
/// it open for writing. A child process that another test starts at that
/// moment shares every descriptor of this process until it runs its
/// program, and starting a script while a writable descriptor of it is
/// open anywhere fails ("text file busy" on Linux). Here only a helper
/// shell writes the file, and it has exited when this returns.
fn write_script(path: &Path, script: &str) {
    use std::io::Write as _;
    let mut writer = std::process::Command::new("/bin/sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    writer
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    assert!(
        writer.wait().unwrap().success(),
        "{} not written",
        path.display()
    );
}

fn binding(key: &str, kind: Core, value: &Path) -> MachineResourceBinding {
    MachineResourceBinding {
        key: ResourceKey::new(key).unwrap(),
        kind,
        value: value.display().to_string(),
    }
}

fn manifest(tools: &Value) -> Manifest {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["permissions"] = json!([{
        "kind": "tools",
        "tools": tools,
        "justification": "test",
    }]);
    Manifest::from_json(&value.to_string()).unwrap()
}

fn declared_everything() -> Value {
    json!([
        super::everything(Tool::Git),
        super::everything(Tool::Docker),
    ])
}

struct Fixture {
    broker: Broker,
    plugin: PluginId,
    records: Arc<Mutex<Vec<PrivilegedEntry>>>,
    manifest: Arc<Manifest>,
    host: ToolHost,
}

impl Fixture {
    fn new(grants: &str, host: ToolHost) -> Self {
        // Its own plugin id, so that runs of tests in parallel never wait
        // for each other's slots.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let broker = Broker::new(
            Arc::new(deckpilot_plugin_broker::MemoryAudit::new(100)),
            Arc::new(deckpilot_plugin_broker::SystemClock::default()),
            deckpilot_plugin_broker::RateLimits::default(),
        );
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let plugin = PluginId::new(&format!("deckpilot.git-test{n}")).unwrap();
        broker
            .set_grants(&plugin, &ApprovedPermissions::from_json(grants).unwrap())
            .unwrap();
        Self {
            broker,
            plugin,
            records: Arc::default(),
            manifest: Arc::new(manifest(&declared_everything())),
            host,
        }
    }

    /// An executor of one invocation in `run` whose action's `repository`
    /// (a directory), `compose` (a file) and `engine` (a Docker
    /// environment) parameters are bound to `bindings`; the handles by
    /// parameter.
    fn invocation(
        &self,
        run: RunContext,
        bindings: &[(&str, MachineResourceBinding)],
    ) -> (Executor, BTreeMap<String, String>) {
        let (events, _) = broadcast::channel(4);
        let sink = self.records.clone();
        let audit: PrivilegedAudit = Arc::new(move |entry: &PrivilegedEntry| {
            sink.lock().unwrap().push(entry.clone());
        });
        let declared = BTreeMap::from([
            ("repository".to_owned(), vec![ResourceKind::Directory]),
            (
                "compose".to_owned(),
                vec![ResourceKind::File, ResourceKind::Directory],
            ),
            ("engine".to_owned(), vec![ResourceKind::DockerEnvironment]),
        ]);
        let resolved = InvocationResources {
            params: bindings
                .iter()
                .map(|(key, binding)| {
                    (
                        (*key).to_owned(),
                        ParamResource {
                            via_role: false,
                            binding: Ok(binding.clone()),
                        },
                    )
                })
                .collect(),
            ..InvocationResources::default()
        };
        Executor::new(
            self.broker.clone(),
            "Git".into(),
            events,
            Transport::Network,
            OwnVariables {
                store: PublishedVariables::default(),
                namespace: "plugins.deckpilot.git".into(),
                generation: 0,
                declared: Vec::new(),
            },
            Opener::System,
        )
        .with_data(DataAccess {
            data: Arc::new(Mutex::new(None)),
            manifest: self.manifest.clone(),
        })
        .with_audit(audit)
        .with_tools(self.host.clone())
        .with_run(run)
        .with_resources(self.plugin.as_str(), "1.0.0", declared, resolved)
    }

    fn call(
        &self,
        executor: &mut Executor,
        request: &Value,
        interrupt: &CallInterrupt,
    ) -> CallResult {
        let request: CapabilityRequest = serde_json::from_value(request.clone()).unwrap();
        let session = self.broker.session(self.plugin.clone());
        match session.authorize_in(&request, executor.run_context()) {
            Decision::Allow(allowed) => {
                executor.execute_interruptible(&self.plugin, &allowed, interrupt)
            }
            Decision::Deny(reason) => {
                CallResult::error(reason.call_error_code(), reason.to_string())
            }
        }
    }

    fn records(&self) -> Vec<PrivilegedEntry> {
        self.records.lock().unwrap().clone()
    }
}

fn key() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

fn later() -> CallInterrupt {
    CallInterrupt::new(None, Instant::now() + Duration::from_secs(60))
}

fn tool_run(tool: &str, args: &[&str], cwd: &str) -> Value {
    json!({"op": "tool.run", "args": {"tool": tool, "args": args, "cwd": cwd}})
}

#[track_caller]
fn output(result: &CallResult) -> ToolOutput {
    match result {
        CallResult::Ok(value) => serde_json::from_value(value.clone()).unwrap(),
        CallResult::Error(e) => panic!("refused: {e:?}"),
    }
}

/// The ABI code and the reason code of a refusal.
#[track_caller]
fn refusal(result: &CallResult) -> (CallErrorCode, String) {
    match result {
        CallResult::Error(e) => (
            e.code,
            e.message.split(':').next().unwrap_or_default().to_owned(),
        ),
        CallResult::Ok(value) => panic!("carried out: {value}"),
    }
}

/// What the fake printed: its arguments, its environment (without what
/// the shell sets itself) and its working directory.
struct Echo {
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: String,
}

fn echo(stdout: &str) -> Echo {
    let mut echo = Echo {
        args: Vec::new(),
        env: BTreeMap::new(),
        cwd: String::new(),
    };
    for line in stdout.lines() {
        if let Some(arg) = line.strip_prefix("ARG=") {
            echo.args.push(arg.to_owned());
        } else if let Some((name, value)) =
            line.strip_prefix("ENV=").and_then(|v| v.split_once('='))
        {
            if !SHELL_OWN.contains(&name) {
                echo.env.insert(name.to_owned(), value.to_owned());
            }
        } else if let Some(cwd) = line.strip_prefix("CWD=") {
            cwd.clone_into(&mut echo.cwd);
        }
    }
    echo
}

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn git_fixture(tree: &Tree, body: &str, grants: &str) -> Fixture {
    let git = tree.fake("git", body);
    Fixture::new(
        grants,
        ToolHost::for_test()
            .with_environment(AGENT_ENV)
            .with_executable(Tool::Git, git),
    )
}

const GIT: &str = r#"[{"kind":"tool","tool":"git"}]"#;
const GIT_NETWORK: &str = r#"[{"kind":"tool","tool":"git","network":true}]"#;

#[test]
fn git_runs_exactly_the_canonical_argv_in_the_allowlisted_environment() {
    let tree = Tree::new();
    let fixture = git_fixture(&tree, ECHO, GIT);
    let repo = tree.path("repo");
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, &repo))],
    );
    let result = fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files",
                "all",
            ],
            &handles["repository"],
        ),
        &later(),
    );
    let out = output(&result);
    assert_eq!(out.exit, Some(0));
    let echo = echo(&out.stdout);
    let repo_text = repo.display().to_string();
    assert_eq!(
        echo.args,
        [
            "-C",
            repo_text.as_str(),
            "--no-pager",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "log.showSignature=false",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "protocol.ssh.allow=always",
            "-c",
            "protocol.git.allow=always",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.file.allow=never",
            "-c",
            "credential.interactive=never",
            "-c",
            "core.askPass=",
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ]
    );
    // The whole environment: no secret, no git override, no SSH agent
    // without the network sub-grant, no askpass, no loader variables, the
    // locale C, absolute PATH entries and the executable's folder (L3),
    // discovery that stops at the binding (M1), no prompts (L4).
    let bin = format!("/usr/bin:/bin:{}", tree.path("bin").display());
    let ceiling = tree.path("").display().to_string();
    assert_eq!(
        echo.env,
        env(&[
            ("PATH", bin.as_str()),
            ("HOME", "/home/someone"),
            ("LC_ALL", "C"),
            ("GIT_EXEC_PATH", "/usr/libexec/git-core"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_OPTIONAL_LOCKS", "0"),
            ("GIT_PROTOCOL_FROM_USER", "0"),
            ("GCM_INTERACTIVE", "never"),
            ("GIT_ASKPASS", ""),
            ("GIT_CEILING_DIRECTORIES", ceiling.trim_end_matches('/')),
        ])
    );
    assert_eq!(echo.cwd, repo_text);
}

#[test]
fn shell_metacharacters_are_passed_literally_and_run_nothing() {
    let tree = Tree::new();
    let fixture = git_fixture(&tree, ECHO, GIT);
    let repo = tree.path("repo");
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, &repo))],
    );
    let format =
        "--format=format:%H $(touch pwned1); `touch pwned2` | touch pwned3 && touch pwned4 > x *";
    let out = output(&fixture.call(
        &mut executor,
        &tool_run("git", &["log", "-1", format], &handles["repository"]),
        &later(),
    ));
    let echo = echo(&out.stdout);
    assert_eq!(echo.args.last().map(String::as_str), Some(format));
    for name in ["pwned1", "pwned2", "pwned3", "pwned4", "x"] {
        assert!(!repo.join(name).exists(), "{name}");
        assert!(!tree.path("bin").join(name).exists(), "{name}");
    }
}

#[test]
fn ls_remote_needs_the_network_grant_a_configured_remote_and_gets_the_ssh_agent() {
    let tree = Tree::new();
    let body = format!(
        "case \" $* \" in *\" remote \"*) printf 'origin\\nupstream\\n'; exit 0;; esac\n{ECHO}"
    );
    let repo = tree.path("repo");
    let bindings = [("repository", binding("app", Core::Directory, &repo))];
    let request = |remote: &str, handle: &str| {
        tool_run(
            "git",
            &["ls-remote", "--", remote, "refs/heads/main"],
            handle,
        )
    };

    // Without `tools:git:network` the broker refuses it.
    let fixture = git_fixture(&tree, &body, GIT);
    let (mut executor, handles) = fixture.invocation(key(), &bindings);
    let result = fixture.call(
        &mut executor,
        &request("origin", &handles["repository"]),
        &later(),
    );
    assert_eq!(refusal(&result).0, CallErrorCode::NotPermitted);

    let fixture = git_fixture(&tree, &body, GIT_NETWORK);
    let (mut executor, handles) = fixture.invocation(key(), &bindings);
    let out = output(&fixture.call(
        &mut executor,
        &request("origin", &handles["repository"]),
        &later(),
    ));
    let echo = echo(&out.stdout);
    let at = echo.args.iter().position(|a| a == "ls-remote").unwrap();
    assert_eq!(
        echo.args[at..],
        ["ls-remote", "--", "origin", "refs/heads/main"]
    );
    assert_eq!(
        echo.env.get("SSH_AUTH_SOCK").map(String::as_str),
        Some("/tmp/agent.sock")
    );
    assert!(!echo.env.contains_key("GIT_SSH_COMMAND"));

    // A remote `git remote` does not list is never contacted.
    let result = fixture.call(
        &mut executor,
        &request("evil", &handles["repository"]),
        &later(),
    );
    assert_eq!(
        refusal(&result),
        (CallErrorCode::Failed, "tool-unknown-remote".into())
    );
}

#[test]
fn the_working_directory_is_only_a_live_directory_handle_of_this_invocation() {
    let tree = Tree::new();
    let fixture = git_fixture(&tree, ECHO, GIT);
    let repo = tree.path("repo");
    let bindings = [
        ("repository", binding("app", Core::Directory, &repo)),
        (
            "compose",
            binding("stack", Core::Other, &tree.path("compose.yaml")),
        ),
    ];
    let (mut executor, handles) = fixture.invocation(key(), &bindings);
    // A handle that was never issued, and one of another invocation.
    let (_, other) = fixture.invocation(key(), &bindings);
    for handle in [
        "0123456789abcdef0123456789abcdef",
        other["repository"].as_str(),
    ] {
        let result = fixture.call(
            &mut executor,
            &tool_run("git", &["status"], handle),
            &later(),
        );
        assert_eq!(
            refusal(&result),
            (CallErrorCode::NotPermitted, "tool-invalid-cwd".into()),
            "{handle}"
        );
    }
    // A file is no directory for git.
    let result = fixture.call(
        &mut executor,
        &tool_run("git", &["status"], &handles["compose"]),
        &later(),
    );
    assert_eq!(refusal(&result).1, "tool-invalid-cwd");
    // A path through a link out of the repository is no argument.
    for path in [
        "link",
        "link/x",
        "../outside",
        tree.path("outside").to_str().unwrap(),
    ] {
        let result = fixture.call(
            &mut executor,
            &tool_run("git", &["status", "--", path], &handles["repository"]),
            &later(),
        );
        assert_eq!(
            refusal(&result),
            (CallErrorCode::BadRequest, "tool-invalid-argument".into())
        );
    }
}

#[test]
fn only_the_tools_own_executable_runs() {
    let tree = Tree::new();
    let repo = tree.path("repo");
    let bindings = [("repository", binding("app", Core::Directory, &repo))];
    let sh_named_git = tree.path("bin").join("git");
    std::os::unix::fs::symlink("/bin/sh", &sh_named_git).unwrap();
    let not_git = tree.fake("not-git", ECHO);
    for executable in [PathBuf::from("/bin/sh"), sh_named_git, not_git] {
        let fixture = Fixture::new(
            GIT,
            ToolHost::for_test()
                .with_environment(AGENT_ENV)
                .with_executable(Tool::Git, &executable),
        );
        let (mut executor, handles) = fixture.invocation(key(), &bindings);
        let result = fixture.call(
            &mut executor,
            &tool_run("git", &["status"], &handles["repository"]),
            &later(),
        );
        assert_eq!(
            refusal(&result).1,
            "tool-executable-invalid",
            "{}",
            executable.display()
        );
    }
    // Without a setting: the first `git` on the agent's PATH.
    let other = tempfile::tempdir().unwrap();
    let tree_bin = tree.path("bin");
    std::fs::remove_file(tree_bin.join("git")).unwrap();
    tree.fake("git", ECHO);
    let path = format!(
        "{}:{}:/usr/bin:/bin",
        other.path().display(),
        tree_bin.display()
    );
    let fixture = Fixture::new(
        GIT,
        ToolHost::for_test().with_environment(&[("PATH", &path)]),
    );
    let (mut executor, handles) = fixture.invocation(key(), &bindings);
    let out = output(&fixture.call(
        &mut executor,
        &tool_run("git", &["remote"], &handles["repository"]),
        &later(),
    ));
    assert_eq!(
        echo(&out.stdout).args.last().map(String::as_str),
        Some("remote")
    );
}

#[test]
fn a_run_stops_at_its_time_limit_and_when_its_invocation_is_aborted() {
    let tree = Tree::new();
    let fixture = git_fixture(&tree, "trap '' TERM\nsleep 30\n", GIT);
    let repo = tree.path("repo");
    let bindings = [("repository", binding("app", Core::Directory, &repo))];

    let (mut executor, handles) = fixture.invocation(key(), &bindings);
    let mut request = tool_run("git", &["status"], &handles["repository"]);
    request["args"]["timeoutMs"] = json!(200);
    let begun = Instant::now();
    let result = fixture.call(&mut executor, &request, &later());
    assert_eq!(
        refusal(&result),
        (CallErrorCode::Failed, "timed-out".into())
    );
    assert!(begun.elapsed() < Duration::from_secs(10));

    // The invocation's own deadline wins over the tool's limit.
    let (mut executor, handles) = fixture.invocation(key(), &bindings);
    let soon = CallInterrupt::new(None, Instant::now() + Duration::from_millis(200));
    let result = fixture.call(
        &mut executor,
        &tool_run("git", &["status"], &handles["repository"]),
        &soon,
    );
    assert_eq!(refusal(&result).1, "timed-out");

    let (mut executor, handles) = fixture.invocation(key(), &bindings);
    let abort = AbortSignal::new();
    let interrupt = CallInterrupt::new(
        Some(abort.clone()),
        Instant::now() + Duration::from_secs(60),
    );
    let aborter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        abort.abort();
    });
    let begun = Instant::now();
    let result = fixture.call(
        &mut executor,
        &tool_run("git", &["status"], &handles["repository"]),
        &interrupt,
    );
    aborter.join().unwrap();
    assert_eq!(
        refusal(&result),
        (CallErrorCode::Failed, "cancelled".into())
    );
    assert!(begun.elapsed() < Duration::from_secs(10));
}

#[test]
fn revoking_the_grant_stops_a_running_process() {
    let tree = Tree::new();
    let fixture = git_fixture(&tree, "sleep 30\n", GIT);
    let repo = tree.path("repo");
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, &repo))],
    );
    let broker = fixture.broker.clone();
    let plugin = fixture.plugin.clone();
    let revoker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        broker
            .set_grants(&plugin, &ApprovedPermissions::from_json("[]").unwrap())
            .unwrap();
    });
    let begun = Instant::now();
    let result = fixture.call(
        &mut executor,
        &tool_run("git", &["status"], &handles["repository"]),
        &later(),
    );
    revoker.join().unwrap();
    assert_eq!(refusal(&result).1, "revoked");
    assert!(begun.elapsed() < Duration::from_secs(10));
}

#[test]
fn output_is_capped_flagged_and_redacted() {
    let tree = Tree::new();
    let body = "printf 'https://bob:ghp_secret@github.com/a.git\\n'\n\
                head -c 1000000 /dev/zero | tr '\\0' 'o'\n\
                head -c 100000 /dev/zero | tr '\\0' 'e' >&2\n";
    let fixture = git_fixture(&tree, body, GIT);
    let repo = tree.path("repo");
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, &repo))],
    );
    let out = output(&fixture.call(
        &mut executor,
        &tool_run("git", &["status"], &handles["repository"]),
        &later(),
    ));
    assert!(out.truncated);
    assert!(
        out.stdout.starts_with("https://github.com/a.git\n"),
        "{}",
        &out.stdout[..40]
    );
    assert!(!out.stdout.contains("ghp_secret"));
    assert!(out.stdout.len() <= MAX_STDOUT && out.stdout.len() > MAX_STDOUT - 64);
    assert_eq!(out.stderr.len(), MAX_STDERR);
    let reply = serde_json::to_vec(&out).unwrap();
    assert!(reply.len() < deckpilot_plugin_abi::limits::MAX_HOST_CALL_REPLY);
}

fn docker_fixture(tree: &Tree, grants: &str, host: ToolHost) -> Fixture {
    let docker = tree.fake("docker", ECHO);
    Fixture::new(grants, host.with_executable(Tool::Docker, docker))
}

const DOCKER: &str = r#"[{"kind":"tool","tool":"docker"}]"#;
const DOCKER_MODIFY: &str = r#"[{"kind":"tool","tool":"docker","modify":true}]"#;

#[test]
fn docker_compose_gets_the_bound_file_and_the_host_held_context() {
    let tree = Tree::new();
    let fixture = docker_fixture(
        &tree,
        DOCKER_MODIFY,
        ToolHost::for_test()
            .with_environment(AGENT_ENV)
            .with_docker_context("colima"),
    );
    let file = tree.path("compose.yaml");
    let (mut executor, handles) =
        fixture.invocation(key(), &[("compose", binding("stack", Core::Other, &file))]);
    let out = output(&fixture.call(
        &mut executor,
        &tool_run(
            "docker",
            &["compose", "up", "--detach", "api"],
            &handles["compose"],
        ),
        &later(),
    ));
    let echo = echo(&out.stdout);
    let dir = tree.path("");
    let dir = dir.to_str().unwrap().trim_end_matches('/');
    let file_text = file.display().to_string();
    // The project name is the binding's: compose's default for its folder.
    let name = format!(
        "--project-name={}",
        super::super::docker::normalized_project(
            &tree.path("").file_name().unwrap().to_string_lossy()
        )
    );
    assert_eq!(
        echo.args,
        [
            "--context",
            "colima",
            "compose",
            "--project-directory",
            dir,
            "-f",
            file_text.as_str(),
            name.as_str(),
            "up",
            "--detach",
            "--",
            "api",
        ]
    );
    let bin = format!("/usr/bin:/bin:{}", tree.path("bin").display());
    assert_eq!(
        echo.env,
        env(&[
            ("PATH", bin.as_str()),
            ("HOME", "/home/someone"),
            ("LC_ALL", "C"),
            ("DOCKER_HOST", "unix:///var/run/docker.sock"),
            ("DOCKER_CONFIG", "/nonexistent/docker-config"),
        ])
    );
    assert_eq!(echo.cwd, dir);
}

#[test]
fn docker_outside_compose_may_use_a_docker_environment_and_its_ssh_agent() {
    let tree = Tree::new();
    let host = ToolHost::for_test().with_environment(&[
        ("PATH", "/usr/bin:/bin"),
        ("DOCKER_HOST", "ssh://me@build-box"),
        ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
    ]);
    let fixture = docker_fixture(&tree, DOCKER, host);
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[
            (
                "engine",
                binding("box", Core::DockerEnvironment, Path::new("build-box")),
            ),
            (
                "compose",
                binding("stack", Core::Other, &tree.path("compose.yaml")),
            ),
        ],
    );
    let out = output(&fixture.call(
        &mut executor,
        &tool_run(
            "docker",
            &["ps", "--format", "{{json .}}"],
            &handles["engine"],
        ),
        &later(),
    ));
    let echo_ = echo(&out.stdout);
    assert_eq!(
        echo_.args,
        ["--context", "build-box", "ps", "--format={{json .}}"]
    );
    // A named context without an ssh endpoint on record: no SSH agent.
    assert!(!echo_.env.contains_key("SSH_AUTH_SOCK"));
    // The default context with an ssh:// DOCKER_HOST: the SSH agent.
    let out = output(&fixture.call(
        &mut executor,
        &tool_run("docker", &["context", "show"], &handles["compose"]),
        &later(),
    ));
    let echo_ = echo(&out.stdout);
    assert_eq!(echo_.args, ["context", "show"]);
    assert_eq!(
        echo_.env.get("SSH_AUTH_SOCK").map(String::as_str),
        Some("/tmp/agent.sock")
    );
    // A compose command never runs against a Docker environment handle.
    let result = fixture.call(
        &mut executor,
        &tool_run("docker", &["compose", "ps"], &handles["engine"]),
        &later(),
    );
    assert_eq!(refusal(&result).1, "tool-invalid-cwd");
}

#[test]
fn docker_logs_are_capped_at_64_kib() {
    let tree = Tree::new();
    let docker = tree.fake("docker", "head -c 500000 /dev/zero | tr '\\0' 'l'\n");
    let fixture = Fixture::new(
        DOCKER,
        ToolHost::for_test()
            .with_environment(AGENT_ENV)
            .with_executable(Tool::Docker, docker),
    );
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[(
            "compose",
            binding("stack", Core::Other, &tree.path("compose.yaml")),
        )],
    );
    let out = output(&fixture.call(
        &mut executor,
        &tool_run(
            "docker",
            &["logs", "--tail", "100", "--", "web"],
            &handles["compose"],
        ),
        &later(),
    ));
    assert_eq!(out.stdout.len(), MAX_LOG_OUTPUT);
    assert!(out.truncated);
}

#[test]
fn modifying_is_off_by_default_and_only_in_runs_the_user_started() {
    let tree = Tree::new();
    let file = tree.path("compose.yaml");
    let bindings = [("compose", binding("stack", Core::Other, &file))];
    let up = |handle: &str| tool_run("docker", &["compose", "up", "--detach"], handle);

    // Read grant only: every modifying subcommand is refused.
    let fixture = docker_fixture(
        &tree,
        DOCKER,
        ToolHost::for_test().with_environment(AGENT_ENV),
    );
    for args in [
        &["compose", "up", "--detach"][..],
        &["compose", "down"],
        &["compose", "restart"],
        &["start", "web"],
        &["stop", "web"],
        &["restart", "web"],
    ] {
        let (mut executor, handles) = fixture.invocation(key(), &bindings);
        let result = fixture.call(
            &mut executor,
            &tool_run("docker", args, &handles["compose"]),
            &later(),
        );
        assert_eq!(refusal(&result).0, CallErrorCode::NotPermitted, "{args:?}");
    }

    let fixture = docker_fixture(
        &tree,
        DOCKER_MODIFY,
        ToolHost::for_test().with_environment(AGENT_ENV),
    );
    for run in [
        RunContext::workflow(RunCause::Schedule, None),
        RunContext::workflow(RunCause::Event, None),
        RunContext::workflow(RunCause::Startup, None),
        RunContext::unknown(),
    ] {
        let (mut executor, handles) = fixture.invocation(run.clone(), &bindings);
        let result = fixture.call(&mut executor, &up(&handles["compose"]), &later());
        assert_eq!(refusal(&result).0, CallErrorCode::NotPermitted, "{run:?}");
    }
    for run in [
        key(),
        RunContext::workflow(RunCause::User, None),
        RunContext::workflow(RunCause::WorkSession, None),
    ] {
        let (mut executor, handles) = fixture.invocation(run.clone(), &bindings);
        let out = output(&fixture.call(&mut executor, &up(&handles["compose"]), &later()));
        assert_eq!(out.exit, Some(0), "{run:?}");
    }
}

#[test]
fn every_run_is_audited_by_names_never_by_values() {
    let tree = Tree::new();
    let fixture = git_fixture(&tree, ECHO, GIT);
    let repo = tree.path("repo");
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, &repo))],
    );
    output(&fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["log", "-1", "--format=format:%H secret-format"],
            &handles["repository"],
        ),
        &later(),
    ));
    // An option before the subcommand: the broker refuses it (and audits
    // it in its own log) before the executor sees it.
    let result = fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["-c", "core.sshCommand=evil", "status"],
            &handles["repository"],
        ),
        &later(),
    );
    assert_eq!(refusal(&result).0, CallErrorCode::NotPermitted);
    let result = fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["status", "-c", "core.sshCommand=evil"],
            &handles["repository"],
        ),
        &later(),
    );
    assert_eq!(refusal(&result).1, "tool-option-denied");
    let _ = fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["rev-parse", "--verify", "secret-ref~1"],
            &handles["repository"],
        ),
        &later(),
    );
    let records = fixture.records();
    assert_eq!(records.len(), 3, "{records:?}");
    let ran = &records[0];
    assert_eq!(ran.record.op, "tool.run");
    assert_eq!(ran.record.grant, "tools:git");
    assert_eq!(ran.record.outcome, None);
    assert_eq!(ran.origin, "key");
    assert_eq!(ran.version, "1.0.0");
    let details: BTreeMap<&str, &str> = ran
        .record
        .details
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .collect();
    assert_eq!(details["tool"], "git");
    assert_eq!(details["subcommand"], "log");
    assert_eq!(details["options"], "-1,--format");
    assert_eq!(details["label"], "app");
    assert_eq!(details["exit"], "0");
    assert!(details["stdoutBytes"].parse::<usize>().unwrap() > 0);
    assert_eq!(details["stderrBytes"], "0");
    assert_eq!(records[1].record.outcome, Some("tool-option-denied"));
    assert_eq!(records[2].record.outcome, Some("tool-invalid-argument"));
    let text = format!("{records:?}");
    for (index, secret) in [
        "secret-format",
        "secret-ref",
        "core.sshCommand",
        "evil",
        repo.to_str().unwrap(),
    ]
    .iter()
    .enumerate()
    {
        // Never print the value or the records holding it.
        assert!(
            !text.contains(*secret),
            "forbidden value #{index} reached the audit records"
        );
    }
}

#[test]
fn tool_run_is_refused_in_a_state_call() {
    let fixture = Fixture::new(GIT, ToolHost::for_test());
    let session = fixture.broker.session(fixture.plugin.clone());
    let request: CapabilityRequest =
        serde_json::from_value(tool_run("git", &["status"], super::HANDLE)).unwrap();
    assert!(matches!(
        session.authorize_state(&request),
        Decision::Deny(DenyReason::NotInStateCall)
    ));
}

// ---------------------------------------------------------------------
// The security review's findings, each as its attack

/// The system's real `git`, for the tests that need git's own behaviour
/// (in temporary repositories only); `None` skips them.
fn real_git() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("git"))
        .find(|p| p.is_file())
        .and_then(|p| std::fs::canonicalize(p).ok())
        .filter(|p| p.file_name().is_some_and(|n| n == "git"))
}

/// Runs the real `git` for a test's setup, with no user configuration.
fn setup_git(git: &Path, home: &Path, dir: &Path, args: &[&str]) {
    let status = std::process::Command::new(git)
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    // The step is not printed: its arguments may hold the planted values.
    assert!(status.success(), "a git setup step failed");
}

fn real_git_fixture(git: &Path, home: &Path, grants: &str) -> Fixture {
    let home = home.display().to_string();
    Fixture::new(
        grants,
        ToolHost::for_test()
            .with_environment(&[("PATH", "/usr/bin:/bin"), ("HOME", &home)])
            .with_executable(Tool::Git, git),
    )
}

/// M1: a bound folder that is no repository but lies inside one (a
/// dotfiles repository in the home folder) never shows that repository.
#[test]
fn git_never_finds_a_repository_above_the_binding() {
    let Some(git) = real_git() else { return };
    let tree = Tree::new();
    let home = tree.path("home");
    std::fs::create_dir_all(home.join("Projects/client-x")).unwrap();
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    std::fs::write(home.join("Documents/acme-nda.pdf"), "secret").unwrap();
    setup_git(&git, &home, &home, &["init", "-q"]);
    let bound = home.join("Projects/client-x");
    let fixture = real_git_fixture(&git, &home, GIT);
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("client", Core::Directory, &bound))],
    );
    let result = fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
            &handles["repository"],
        ),
        &later(),
    );
    assert_eq!(
        refusal(&result),
        (CallErrorCode::Failed, "not-a-repository".into())
    );

    // A repository that is the binding works.
    setup_git(&git, &home, &bound, &["init", "-q"]);
    std::fs::write(bound.join("a.txt"), "a").unwrap();
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("client", Core::Directory, &bound))],
    );
    let out = output(&fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
            &handles["repository"],
        ),
        &later(),
    ));
    assert_eq!(out.exit, Some(0));
    assert!(
        out.stdout.contains("a.txt") && !out.stdout.contains("acme-nda"),
        "{}",
        out.stdout
    );
}

/// M1, second line: whatever git reports, a work tree outside the binding
/// is refused before the run.
#[test]
fn a_work_tree_outside_the_binding_is_refused() {
    let tree = Tree::new();
    let fake = format!(
        "case \" $* \" in *\" --show-toplevel \"*) echo '{0}'; echo '{0}'; exit 0;; esac\n{ECHO}",
        tree.path("").display()
    );
    let git = tree.path("bin").join("git");
    write_script(&git, &format!("#!/bin/sh\n{fake}"));
    let fixture = Fixture::new(
        GIT,
        ToolHost::for_test()
            .with_environment(AGENT_ENV)
            .with_executable(Tool::Git, &git),
    );
    let repo = tree.path("repo");
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, &repo))],
    );
    let result = fixture.call(
        &mut executor,
        &tool_run("git", &["status"], &handles["repository"]),
        &later(),
    );
    assert_eq!(
        refusal(&result),
        (CallErrorCode::NotPermitted, "tool-outside-binding".into())
    );
}

/// A commit with a signature, written without signing it.
fn signed_commit(git: &Path, home: &Path, repo: &Path) {
    setup_git(git, home, repo, &["init", "-q"]);
    std::fs::write(repo.join("f"), "x").unwrap();
    setup_git(git, home, repo, &["add", "f"]);
    let tree = std::process::Command::new(git)
        .arg("-C")
        .arg(repo)
        .arg("write-tree")
        .env("HOME", home)
        .output()
        .unwrap();
    let tree = String::from_utf8(tree.stdout).unwrap();
    let commit = format!(
        "tree {}\nauthor a <a@b> 1 +0000\ncommitter a <a@b> 1 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n AAAA\n -----END PGP SIGNATURE-----\n\nmsg\n",
        tree.trim()
    );
    let file = repo.join(".git/c.txt");
    std::fs::write(&file, commit).unwrap();
    let hash = std::process::Command::new(git)
        .arg("-C")
        .arg(repo)
        .args(["hash-object", "-t", "commit", "-w"])
        .arg(&file)
        .env("HOME", home)
        .output()
        .unwrap();
    let hash = String::from_utf8(hash.stdout).unwrap();
    setup_git(
        git,
        home,
        repo,
        &["update-ref", "refs/heads/main", hash.trim()],
    );
    setup_git(
        git,
        home,
        repo,
        &["symbolic-ref", "HEAD", "refs/heads/main"],
    );
}

/// L1: a hostile repository's `log.showSignature` and `gpg.program` never
/// run on `log`.
#[test]
fn a_hostile_repository_cannot_run_gpg_on_log() {
    let Some(git) = real_git() else { return };
    let tree = Tree::new();
    let home = tree.path("home");
    std::fs::create_dir_all(&home).unwrap();
    let repo = tree.path("repo");
    signed_commit(&git, &home, &repo);
    let pwned = tree.path("pwned");
    let evil = repo.join(".git/evil.sh");
    write_script(
        &evil,
        &format!("#!/bin/sh\necho gpg >> '{}'\nexit 1\n", pwned.display()),
    );
    setup_git(&git, &home, &repo, &["config", "log.showSignature", "true"]);
    setup_git(
        &git,
        &home,
        &repo,
        &["config", "gpg.program", evil.to_str().unwrap()],
    );
    setup_git(&git, &home, &repo, &["config", "pretty.short2", "%H %G?"]);
    let fixture = real_git_fixture(&git, &home, GIT);
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, &repo))],
    );
    let out = output(&fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["log", "-1", "--format=format:%H"],
            &handles["repository"],
        ),
        &later(),
    ));
    assert_eq!(out.exit, Some(0), "{}", out.stderr);
    let result = fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["log", "-1", "--no-show-signature", "--format=short2"],
            &handles["repository"],
        ),
        &later(),
    );
    assert_eq!(refusal(&result).1, "tool-invalid-argument");
    assert!(!pwned.exists(), "gpg ran");
}

/// L2: a remote helper transport (`testx::…` runs `git-remote-testx`) is
/// never used, even for a configured remote.
#[test]
fn remote_helper_transports_never_run() {
    let Some(git) = real_git() else { return };
    let tree = Tree::new();
    let home = tree.path("home");
    std::fs::create_dir_all(&home).unwrap();
    let repo = tree.path("repo");
    setup_git(&git, &home, &repo, &["init", "-q"]);
    setup_git(
        &git,
        &home,
        &repo,
        &["remote", "add", "helper", "testx::whatever"],
    );
    let pwned = tree.path("pwned");
    let helper = tree.fake(
        "git-remote-testx",
        &format!("echo helper >> '{}'\nexit 1\n", pwned.display()),
    );
    let path = format!("/usr/bin:/bin:{}", helper.parent().unwrap().display());
    let home_text = home.display().to_string();
    let fixture = Fixture::new(
        GIT_NETWORK,
        ToolHost::for_test()
            .with_environment(&[("PATH", &path), ("HOME", &home_text)])
            .with_executable(Tool::Git, &git),
    );
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, &repo))],
    );
    let out = output(&fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["ls-remote", "--", "helper"],
            &handles["repository"],
        ),
        &later(),
    ));
    assert_ne!(out.exit, Some(0));
    assert!(!pwned.exists(), "the remote helper ran: {}", out.stderr);
}

/// M2: compose on a folder handle uses the folder's own compose file,
/// never the one above it that a `.env` in the folder names.
#[test]
fn compose_on_a_folder_runs_the_folders_own_file_only() {
    let tree = Tree::new();
    let fixture = docker_fixture(
        &tree,
        DOCKER,
        ToolHost::for_test().with_environment(AGENT_ENV),
    );
    let root = tree.path("dk");
    for dir in ["bound", "b2"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(
        root.join("compose.yaml"),
        "services: {outside: {image: alpine, privileged: true}}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("bound/compose.yaml"),
        "services: {inside: {image: alpine}}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("bound/.env"),
        "COMPOSE_FILE=../compose.yaml\nCOMPOSE_PROJECT_NAME=other\n",
    )
    .unwrap();
    std::fs::write(root.join("b2/.env"), "COMPOSE_FILE=../compose.yaml\n").unwrap();
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[
            (
                "compose",
                binding("bound", Core::Directory, &root.join("bound")),
            ),
            (
                "repository",
                binding("b2", Core::Directory, &root.join("b2")),
            ),
        ],
    );
    let out = output(&fixture.call(
        &mut executor,
        &tool_run(
            "docker",
            &["compose", "ps", "--format", "json"],
            &handles["compose"],
        ),
        &later(),
    ));
    let args = echo(&out.stdout).args;
    let file = root.join("bound/compose.yaml").display().to_string();
    let at = args.iter().position(|a| a == "-f").unwrap();
    assert_eq!(args[at + 1], file);
    assert!(
        args.contains(&"--project-name=bound".to_owned()),
        "{args:?}"
    );
    let result = fixture.call(
        &mut executor,
        &tool_run("docker", &["compose", "ps"], &handles["repository"]),
        &later(),
    );
    assert_eq!(refusal(&result).1, "tool-invalid-cwd");
}

/// The outcome of `git log` in the folder `bound` through the executor.
fn log_in(git: &Path, home: &Path, bound: &Path) -> CallResult {
    let fixture = real_git_fixture(git, home, GIT);
    let (mut executor, handles) = fixture.invocation(
        key(),
        &[("repository", binding("app", Core::Directory, bound))],
    );
    fixture.call(
        &mut executor,
        &tool_run(
            "git",
            &["log", "-1", "--format=format:%s"],
            &handles["repository"],
        ),
        &later(),
    )
}

/// A repository with one commit whose subject is `subject`.
fn repository(git: &Path, home: &Path, dir: &Path, subject: &str) {
    std::fs::create_dir_all(dir).unwrap();
    setup_git(git, home, dir, &["init", "-q"]);
    std::fs::write(dir.join("f"), subject).unwrap();
    setup_git(git, home, dir, &["add", "f"]);
    setup_git(git, home, dir, &["commit", "-qm", subject]);
}

/// The second review's residual: a `.git` file or link in the binding
/// that leads to another repository never lets git read it; a genuine
/// linked worktree works; a submodule is refused (not supported yet).
#[test]
fn a_git_directory_outside_the_binding_is_refused_except_for_a_linked_worktree() {
    let Some(git) = real_git() else { return };
    let tree = Tree::new();
    let home = tree.path("home");
    std::fs::create_dir_all(&home).unwrap();
    let secret = tree.path("secret");
    repository(&git, &home, &secret, "SECRET-SUBJECT");
    let outside = (
        CallErrorCode::NotPermitted,
        "tool-outside-binding".to_owned(),
    );

    // A `gitdir:` file pointing to another repository.
    let file = tree.path("bind-file");
    std::fs::create_dir_all(&file).unwrap();
    std::fs::write(file.join(".git"), "gitdir: ../secret/.git\n").unwrap();
    let result = log_in(&git, &home, &file);
    assert_eq!(refusal(&result), outside);

    // A `.git` link to another repository's.
    let link = tree.path("bind-link");
    std::fs::create_dir_all(&link).unwrap();
    std::os::unix::fs::symlink(secret.join(".git"), link.join(".git")).unwrap();
    assert_eq!(refusal(&log_in(&git, &home, &link)), outside);

    // `core.worktree` pointing outside, and a bare repository.
    let moved = tree.path("bind-moved");
    std::fs::create_dir_all(&moved).unwrap();
    setup_git(&git, &home, &moved, &["init", "-q"]);
    setup_git(
        &git,
        &home,
        &moved,
        &["config", "core.worktree", secret.to_str().unwrap()],
    );
    assert!(matches!(&log_in(&git, &home, &moved), CallResult::Error(_)));
    let bare = tree.path("bare");
    std::fs::create_dir_all(&bare).unwrap();
    setup_git(&git, &home, &bare, &["init", "-q", "--bare"]);
    assert_eq!(refusal(&log_in(&git, &home, &bare)).1, "not-a-repository");

    // A normal repository.
    let normal = tree.path("normal");
    repository(&git, &home, &normal, "normal-subject");
    assert_eq!(
        output(&log_in(&git, &home, &normal)).stdout,
        "normal-subject"
    );

    // A linked worktree of it, outside the main repository's folder.
    let linked = tree.path("linked");
    setup_git(
        &git,
        &home,
        &normal,
        &["worktree", "add", "-q", linked.to_str().unwrap()],
    );
    assert_eq!(
        output(&log_in(&git, &home, &linked)).stdout,
        "normal-subject"
    );
    // Another work tree claiming that worktree's git directory: refused.
    let thief = tree.path("thief");
    std::fs::create_dir_all(&thief).unwrap();
    let worktree_dir = std::fs::read_to_string(linked.join(".git")).unwrap();
    std::fs::write(thief.join(".git"), worktree_dir).unwrap();
    assert_eq!(refusal(&log_in(&git, &home, &thief)), outside);

    // A submodule's work tree (git directory in the superproject's
    // `.git/modules`): not supported as a binding yet.
    let superproject = tree.path("super");
    repository(&git, &home, &superproject, "super");
    let sub = superproject.join("sub");
    let modules = superproject.join(".git/modules/sub");
    std::fs::create_dir_all(superproject.join(".git/modules")).unwrap();
    setup_git(
        &git,
        &home,
        &superproject,
        &[
            "init",
            "-q",
            &format!("--separate-git-dir={}", modules.display()),
            sub.to_str().unwrap(),
        ],
    );
    repository(&git, &home, &sub, "sub-subject");
    assert_eq!(refusal(&log_in(&git, &home, &sub)), outside);
}
