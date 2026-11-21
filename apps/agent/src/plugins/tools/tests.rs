//! The denial matrix of `tool.run` (X3 in
//! `docs/marketplace/module-migration.md` §7) and its runs: every
//! argument check without running anything, then, with fake `git` and
//! `docker` executables that print their argv, environment and working
//! directory, exactly what the agent runs, in which environment, within
//! which limits, under which grants and run origins, and what it audits.

use deckpilot_plugin_abi::privileged::{Tool, ToolRun};
use deckpilot_plugin_manifest::ToolDeclaration;
use deckpilot_plugin_manifest::tools::{
    DOCKER_DENIED_OPTIONS, DOCKER_DENIED_SUBCOMMANDS, GIT_DENIED_OPTIONS, policy_of,
};
use serde_json::json;

use super::*;

const HANDLE: &str = "0123456789abcdef0123456789abcdef";

/// `unix`, an absolute Unix path, as an absolute path of this platform
/// (on Windows under `C:`, with `\` for `/`).
fn absolute(unix: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!("C:{}", unix.replace('/', "\\")))
    } else {
        PathBuf::from(unix)
    }
}

/// `entries` joined as a `PATH` of this platform (`;` on Windows), each
/// absolute one (`/…`) made [`absolute`].
fn path_of(entries: &[&str]) -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    entries
        .iter()
        .map(|e| {
            if e.starts_with('/') {
                absolute(e).to_string_lossy().into_owned()
            } else {
                (*e).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(separator)
}

/// Everything the host's table allows, declared, with both sub-grants.
fn everything(tool: Tool) -> ToolDeclaration {
    let executable = executable_of(tool);
    serde_json::from_value(json!({
        "executable": executable.as_str(),
        "subcommands": policy_of(executable).iter().map(|p| json!({
            "name": p.subcommand,
            "options": p.options,
        })).collect::<Vec<_>>(),
        "network": tool == Tool::Git,
        "modify": tool == Tool::Docker,
    }))
    .unwrap()
}

fn run(tool: Tool, args: &[&str]) -> ToolRun {
    ToolRun {
        tool,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        cwd: Some(HANDLE.into()),
        timeout_ms: None,
    }
}

fn checked(tool: Tool, args: &[&str]) -> Result<Checked, Denial> {
    check(&run(tool, args), Some(&everything(tool)))
}

#[track_caller]
fn refused(tool: Tool, args: &[&str], code: &str) {
    match checked(tool, args) {
        Ok(checked) => panic!("{tool:?} {args:?} passed: {checked:?}"),
        Err(denial) => assert_eq!(denial.code, code, "{tool:?} {args:?}: {denial:?}"),
    }
}

#[track_caller]
fn passes(tool: Tool, args: &[&str]) -> Checked {
    checked(tool, args).unwrap_or_else(|d| panic!("{tool:?} {args:?} refused: {d:?}"))
}

// ---------------------------------------------------------------------
// The table

#[test]
fn the_host_table_is_exactly_the_manifest_crates_policy() {
    for tool in [Tool::Git, Tool::Docker] {
        let policy = policy_of(executable_of(tool));
        let specs = specs(tool);
        assert_eq!(policy.len(), specs.len(), "{tool:?}");
        for spec in specs {
            let allowed = policy
                .iter()
                .find(|p| p.subcommand == spec.subcommand)
                .unwrap_or_else(|| panic!("{} is not in the policy", spec.subcommand));
            let mut ours: Vec<&str> = spec.options.iter().map(|o| o.name()).collect();
            let mut theirs = allowed.options.to_vec();
            ours.sort_unstable();
            theirs.sort_unstable();
            assert_eq!(ours, theirs, "{tool:?} {}", spec.subcommand);
            for required in spec.required {
                assert!(ours.contains(required), "{}", spec.subcommand);
            }
            assert!(spec.min <= spec.max && (spec.max == 0) == spec.slots.is_empty());
        }
    }
}

#[test]
fn what_the_built_ins_run_passes() {
    for args in [
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ][..],
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files",
            "no",
        ],
        &[
            "log",
            "-1",
            "--no-show-signature",
            "--format=format:%H%x00%h%x00%an%x00%aI%x00%s",
        ],
        &["config", "--get", "branch.main.remote"],
        &["config", "--get", "branch.feature/x.merge"],
        &["remote"],
        &["rev-parse", "--verify", "-q", "origin/main"],
        &["ls-remote", "--", "origin", "refs/heads/main"],
        &["ls-remote", "origin"],
    ] {
        passes(Tool::Git, args);
    }
    for args in [
        &["context", "show"][..],
        &["version", "--format", "{{json .}}"],
        &["compose", "version", "--short"],
        &[
            "ps",
            "--no-trunc",
            "--format",
            "{{json .}}",
            "--all",
            "--filter",
            "name=web",
        ],
        &["compose", "ps", "--all", "--format", "json"],
        &[
            "compose",
            "logs",
            "--no-color",
            "--tail",
            "100",
            "--since=10m",
            "--",
            "api",
        ],
        &[
            "logs",
            "--tail=100",
            "--since",
            "2026-11-20T10:00:00",
            "--",
            "web-1",
        ],
        &[
            "compose",
            "up",
            "--detach",
            "--remove-orphans",
            "--",
            "api",
            "db",
        ],
        &["compose", "down", "--remove-orphans"],
        &["compose", "restart", "api"],
        &["start", "--", "web"],
        &["stop", "web"],
        &["restart", "web"],
    ] {
        passes(Tool::Docker, args);
    }
}

// ---------------------------------------------------------------------
// git: the denial matrix

#[test]
fn every_denied_git_option_is_refused_in_every_spelling() {
    for denied in GIT_DENIED_OPTIONS {
        let joined = format!("{denied}=x");
        let attached = format!("{denied}x");
        let mut forms: Vec<Vec<&str>> = vec![
            vec!["status", denied],
            vec!["status", &joined],
            vec!["status", denied, "x"],
            vec![denied, "x", "status"],
            vec![&joined, "status"],
            vec!["ls-remote", denied, "origin"],
        ];
        if !denied.starts_with("--") {
            forms.push(vec!["status", &attached]);
        }
        for args in forms {
            refused(Tool::Git, &args, codes::OPTION_DENIED);
        }
    }
    for args in [
        &["-cfoo=bar", "status"][..],
        &["status", "-cprotocol.ext.allow=always"],
        &["status", "-zc", "core.pager=sh"],
        &["status", "-zC", "/"],
        &["rev-parse", "--verify", "-qo", "file"],
        &["ls-remote", "-u", "evil", "origin"],
        &["ls-remote", "--upload-pack=touch /tmp/pwned", "origin"],
        &["status", "--config-env=core.pager=EVIL"],
        &["status", "--git-dir=/etc"],
        &["status", "--work-tree", "/"],
        &["log", "-1", "--output=/tmp/x"],
        &["log", "-1", "--output", "/tmp/x"],
        &["log", "-1", "--ext-diff"],
        &["log", "-1", "--textconv"],
    ] {
        refused(Tool::Git, args, codes::OPTION_DENIED);
    }
}

#[test]
fn core_and_alias_keys_are_refused_by_every_route() {
    refused(
        Tool::Git,
        &["log", "-1", "--format=core.x"],
        codes::OPTION_DENIED,
    );
    refused(
        Tool::Git,
        &["log", "-1", "--format=%Halias.x"],
        codes::OPTION_DENIED,
    );
    for key in [
        "core.sshCommand",
        "alias.st",
        "credential.helper",
        "remote.origin.url",
        "http.https://x.extraHeader",
        "url.https://x.insteadOf",
        "branch.main.pushRemote",
        "branch.-x.remote",
        "branch..remote",
    ] {
        refused(
            Tool::Git,
            &["config", "--get", key],
            codes::INVALID_ARGUMENT,
        );
    }
    // A key and a value would write the configuration.
    refused(
        Tool::Git,
        &["config", "--get", "branch.main.remote", "evil"],
        codes::INVALID_ARGUMENT,
    );
    refused(
        Tool::Git,
        &["config", "branch.main.remote"],
        codes::INVALID_ARGUMENT,
    );
    refused(Tool::Git, &["config", "--get-all", "x"], codes::NOT_ALLOWED);
    refused(
        Tool::Git,
        &["config", "--global", "--get", "x"],
        codes::NOT_ALLOWED,
    );
}

#[test]
fn only_builtin_git_subcommands_of_the_table_run() {
    for name in [
        "st",
        "fetch",
        "pull",
        "push",
        "clone",
        "commit",
        "checkout",
        "diff",
        "show",
        "submodule",
        "filter-branch",
        "upload-pack",
        "daemon",
        "difftool",
        "help",
        "lfs",
    ] {
        refused(Tool::Git, &[name], codes::NOT_ALLOWED);
    }
    refused(Tool::Git, &["--version"], codes::SUBCOMMAND_DENIED);
    refused(Tool::Git, &["--exec-path"], codes::OPTION_DENIED);
    refused(Tool::Git, &["remote", "-v"], codes::NOT_ALLOWED);
    refused(
        Tool::Git,
        &["remote", "get-url", "origin"],
        codes::INVALID_ARGUMENT,
    );
    refused(Tool::Git, &["status", "--porcelain=v1"], codes::NOT_ALLOWED);
    refused(
        Tool::Git,
        &["status", "--untracked-files=yes"],
        codes::INVALID_ARGUMENT,
    );
    refused(
        Tool::Git,
        &["status", "--untracked-files"],
        codes::INVALID_ARGUMENT,
    );
    refused(Tool::Git, &["log", "--format=%H"], codes::INVALID_ARGUMENT);
    refused(
        Tool::Git,
        &["log", "-1", "--format=format:%(trailers)"],
        codes::INVALID_ARGUMENT,
    );
    refused(Tool::Git, &["log", "-1", "-1"], codes::INVALID_ARGUMENT);
    refused(
        Tool::Git,
        &["rev-parse", "--verify", "-qz", "x"],
        codes::NOT_ALLOWED,
    );
}

#[test]
fn refs_and_remotes_never_start_with_a_dash_or_name_a_url_or_path() {
    for value in ["-x", "--upload-pack=x", "--output=/tmp/x", "-"] {
        refused(
            Tool::Git,
            &["rev-parse", "--verify", "--", value],
            codes::INVALID_ARGUMENT,
        );
        refused(
            Tool::Git,
            &["ls-remote", "--", value],
            codes::INVALID_ARGUMENT,
        );
        refused(
            Tool::Git,
            &["ls-remote", "--", "origin", value],
            codes::INVALID_ARGUMENT,
        );
    }
    for remote in [
        "https://evil.example/x",
        "ext::sh -c touch% /tmp/pwned",
        "file:///etc",
        "git@evil.example:x",
        "../outside",
        "/tmp/repo",
        "..",
        ".",
        "origin/../x",
    ] {
        refused(
            Tool::Git,
            &["ls-remote", "--", remote],
            codes::INVALID_ARGUMENT,
        );
    }
    for reference in [
        "a..b", "a b", "@{u}", "x~1", "x^", "x:y", "/x", "x/", "x.lock", ".x", "@",
    ] {
        refused(
            Tool::Git,
            &["rev-parse", "--verify", reference],
            codes::INVALID_ARGUMENT,
        );
    }
}

#[test]
fn path_arguments_are_never_accepted() {
    for path in [
        "../outside",
        "/etc/passwd",
        "link-to-outside",
        "sub/file.txt",
    ] {
        refused(Tool::Git, &["status", "--", path], codes::INVALID_ARGUMENT);
        refused(Tool::Git, &["status", path], codes::INVALID_ARGUMENT);
        refused(
            Tool::Git,
            &["log", "-1", "--", path],
            codes::INVALID_ARGUMENT,
        );
        refused(Tool::Git, &["remote", path], codes::INVALID_ARGUMENT);
    }
}

#[test]
fn the_double_dash_ends_options_and_the_host_places_its_own() {
    // After `--` an option is a positional, which its slot refuses.
    refused(
        Tool::Git,
        &["ls-remote", "--", "-c", "x"],
        codes::INVALID_ARGUMENT,
    );
    let checked = passes(Tool::Git, &["ls-remote", "origin", "refs/heads/main"]);
    assert_eq!(checked.positionals, ["origin", "refs/heads/main"]);
    // A denied option before `--` is found even when `--` follows.
    refused(
        Tool::Git,
        &["ls-remote", "--upload-pack=x", "--", "origin"],
        codes::OPTION_DENIED,
    );
}

#[test]
fn a_manifest_only_narrows_the_table() {
    let narrow: ToolDeclaration = serde_json::from_value(json!({
        "executable": "git",
        "subcommands": [{"name": "status", "options": ["-z"]}, {"name": "ls-remote"}],
    }))
    .unwrap();
    let check = |args: &[&str]| check(&run(Tool::Git, args), Some(&narrow));
    assert!(check(&["status", "-z"]).is_ok());
    assert_eq!(
        check(&["status", "--branch"]).unwrap_err().code,
        codes::NOT_DECLARED
    );
    assert_eq!(check(&["remote"]).unwrap_err().code, codes::NOT_DECLARED);
    // ls-remote without the declared network sub-grant.
    assert_eq!(
        check(&["ls-remote", "origin"]).unwrap_err().code,
        codes::NOT_DECLARED
    );
    // No declaration of the tool at all.
    assert_eq!(
        super::check(&run(Tool::Git, &["status"]), None)
            .unwrap_err()
            .code,
        codes::NOT_DECLARED
    );
    // A declaration can never widen: an option outside the table stays
    // refused even if a manifest names it.
    let wide: ToolDeclaration = serde_json::from_value(json!({
        "executable": "git",
        "subcommands": [{"name": "status", "options": ["--ignored", "-z"]}],
    }))
    .unwrap();
    assert_eq!(
        super::check(&run(Tool::Git, &["status", "--ignored"]), Some(&wide))
            .unwrap_err()
            .code,
        codes::NOT_ALLOWED
    );
    refused(Tool::Git, &["status", "-z", "-z"], codes::INVALID_ARGUMENT);
}

// ---------------------------------------------------------------------
// docker: the denial matrix

#[test]
fn every_denied_docker_subcommand_is_refused() {
    for denied in DOCKER_DENIED_SUBCOMMANDS {
        let mut args: Vec<&str> = denied.split(' ').collect();
        args.push("x");
        refused(Tool::Docker, &args, codes::SUBCOMMAND_DENIED);
    }
    for args in [
        &["run", "-it", "--rm", "alpine", "sh"][..],
        &["exec", "web", "sh"],
        &["cp", "web:/etc/shadow", "."],
        &["build", "."],
        &["login", "-u", "x"],
        &["inspect", "web"],
        &["compose", "config"],
    ] {
        let code = checked(Tool::Docker, args).unwrap_err().code;
        assert!(
            [codes::SUBCOMMAND_DENIED, codes::OPTION_DENIED].contains(&code),
            "{args:?}: {code}"
        );
    }
    for name in [
        "kill", "rm", "rmi", "update", "commit", "save", "load", "events", "top",
    ] {
        assert!(checked(Tool::Docker, &[name, "web"]).is_err(), "{name}");
    }
}

#[test]
fn every_denied_docker_option_is_refused_in_every_spelling() {
    for denied in DOCKER_DENIED_OPTIONS {
        let joined = format!("{denied}=x");
        for args in [
            vec!["ps", denied],
            vec!["ps", &joined],
            vec!["ps", denied, "x"],
            vec![denied, "x", "ps"],
            vec!["compose", denied, "x", "ps"],
            vec!["compose", "up", "--detach", denied],
            vec!["compose", "down", &joined],
            vec!["logs", denied, "x", "--", "web"],
        ] {
            refused(Tool::Docker, &args, codes::OPTION_DENIED);
        }
    }
    for args in [
        &["ps", "-av"][..],
        &["-H", "tcp://evil:2375", "ps"],
        &["--host=ssh://evil", "ps"],
        &["--context", "evil", "ps"],
        &["compose", "-f", "/etc/evil.yaml", "ps"],
        &["compose", "--file=/x", "up", "--detach"],
        &["compose", "down", "-v"],
        &["compose", "down", "--volumes"],
        &["compose", "down", "--rmi=all"],
        &["compose", "up", "--detach", "--build"],
        &[
            "compose",
            "up",
            "--detach",
            "--force-recreate",
            "--renew-anon-volumes",
        ],
        &["compose", "--project-directory", "/", "ps"],
        &["--tlsverify", "ps"],
        &["--config", "/tmp", "ps"],
    ] {
        refused(Tool::Docker, args, codes::OPTION_DENIED);
    }
}

#[test]
fn docker_values_fill_their_slots_only() {
    for args in [
        &["logs", "--tail", "0", "--", "web"][..],
        &["logs", "--tail", "1001", "--", "web"],
        &["logs", "--tail=+5", "--", "web"],
        &["logs", "--tail", "05", "--", "web"],
        &["logs", "--since", "-1h", "--", "web"],
        &["logs", "--", "--privileged"],
        &["logs", "--", "$(id)"],
        &["logs", "--", "a;rm -rf /"],
        &["logs", "--", "../x"],
        &["logs", "--", "web", "db"],
        &["logs"],
        &["ps", "--format", "{{.Env}}"],
        &["ps", "--format", "{{json .Config.Env}}"],
        &["ps", "--filter", "label=x"],
        &["ps", "--filter", "name=a b"],
        &["version", "--format", "table"],
        &["compose", "up"],
        &["compose", "down", "web"],
        &["start"],
    ] {
        refused(Tool::Docker, args, codes::INVALID_ARGUMENT);
    }
    for args in [
        &["ps", "--quiet"][..],
        &["compose", "ps", "--services"],
        &["compose", "logs", "--follow"],
        &["compose", "up", "--detach", "--wait"],
        &["context", "use", "evil"],
        &["context", "ls"],
        &["compose"],
    ] {
        refused(Tool::Docker, args, codes::NOT_ALLOWED);
    }
}

#[test]
fn modify_subcommands_need_the_declared_sub_grant() {
    let read: ToolDeclaration = serde_json::from_value(json!({
        "executable": "docker",
        "subcommands": [{"name": "compose up", "options": ["--detach"]}, {"name": "ps"}],
    }))
    .unwrap();
    let denial = check(
        &run(Tool::Docker, &["compose", "up", "--detach"]),
        Some(&read),
    )
    .unwrap_err();
    assert_eq!(denial.code, codes::NOT_DECLARED);
    assert!(check(&run(Tool::Docker, &["ps"]), Some(&read)).is_ok());
}

#[test]
fn every_value_check_refuses_a_leading_dash_and_control_characters() {
    for slot in [
        Slot::RefName,
        Slot::BranchKey,
        Slot::RemoteName,
        Slot::LogFormat,
        Slot::Untracked,
        Slot::JsonTemplate,
        Slot::ComposeFormat,
        Slot::NameFilter,
        Slot::Tail,
        Slot::Since,
        Slot::Name,
    ] {
        for value in ["", "-", "-x", "--x=y", "a\nb", "a\0b"] {
            assert!(!valid(slot, value), "{slot:?} {value:?}");
        }
    }
}

#[test]
fn userinfo_is_removed_from_every_url() {
    assert_eq!(
        redact_userinfo(
            "fatal: unable to access 'https://bob:ghp_secret@github.com/a/b.git/': 403"
        ),
        "fatal: unable to access 'https://github.com/a/b.git/': 403"
    );
    assert_eq!(
        redact_userinfo("ssh://git@host/x and http://a:b@c"),
        "ssh://host/x and http://c"
    );
    assert_eq!(redact_userinfo("no url"), "no url");
    assert_eq!(redact_userinfo("https://host/x@y"), "https://host/x@y");
}

#[test]
fn runs_per_plugin_are_bounded_and_one_modifies_at_a_time() {
    let plugin = "deckpilot.slots-test";
    let never = || false;
    let give_up = || true;
    let slots: Vec<RunSlot> = (0..MAX_CONCURRENT)
        .map(|_| RunSlot::acquire(plugin, false, &never).unwrap())
        .collect();
    assert!(RunSlot::acquire(plugin, false, &give_up).is_none());
    drop(slots);
    let modifying = RunSlot::acquire(plugin, true, &never).unwrap();
    assert!(RunSlot::acquire(plugin, true, &give_up).is_none());
    let reading = RunSlot::acquire(plugin, false, &give_up);
    assert!(reading.is_some());
    drop(modifying);
    assert!(RunSlot::acquire(plugin, true, &give_up).is_some());
    // Another plugin is not held up.
    assert!(RunSlot::acquire("deckpilot.slots-other", true, &give_up).is_some());
}

#[test]
fn the_executable_must_be_the_tools_own() {
    let host = ToolHost::for_test()
        .with_environment(&[("PATH", "")])
        .with_executable(Tool::Git, "git");
    assert_eq!(
        host.executable(Tool::Git).unwrap_err().code,
        codes::INVALID_EXECUTABLE
    );
    let host = ToolHost::for_test().with_environment(&[]).with_executable(
        Tool::Docker,
        absolute("/nonexistent/bin").join(executable_name(Tool::Docker)),
    );
    assert_eq!(
        host.executable(Tool::Docker).unwrap_err().code,
        codes::NOT_FOUND
    );
    // Without a setting the agent's PATH is searched (and, on macOS,
    // Homebrew's and Docker Desktop's folders), never a relative entry.
    let relative = path_of(&[".", "bin"]);
    let host = ToolHost::for_test().with_environment(&[("PATH", &relative)]);
    assert!(
        host.search_path(Tool::Docker)
            .iter()
            .all(|d| d.is_absolute())
    );
}

#[test]
fn each_class_has_its_time_limit_which_the_plugin_may_only_shorten() {
    let host = ToolHost::for_test().with_timeouts(1, 99_999);
    let limit = |tool, args: &[&str], requested| host.timeout(&passes(tool, args), requested);
    assert_eq!(limit(Tool::Git, &["status"], None), GIT_READ_TIMEOUT);
    assert_eq!(
        limit(Tool::Git, &["ls-remote", "origin"], None),
        Duration::from_secs(5)
    );
    assert_eq!(limit(Tool::Docker, &["ps"], None), DOCKER_READ_TIMEOUT);
    assert_eq!(
        limit(Tool::Docker, &["compose", "up", "--detach"], None),
        Duration::from_secs(1800)
    );
    assert_eq!(
        limit(Tool::Git, &["status"], Some(500)),
        Duration::from_millis(500)
    );
    assert_eq!(
        limit(Tool::Git, &["status"], Some(600_000)),
        GIT_READ_TIMEOUT
    );
    let defaults = ToolHost::for_test();
    assert_eq!(
        defaults.timeout(&passes(Tool::Git, &["ls-remote", "origin"]), None),
        Duration::from_secs(20)
    );
    assert_eq!(
        defaults.timeout(&passes(Tool::Docker, &["stop", "web"]), None),
        Duration::from_secs(300)
    );
}

/// L1: a bare word names a `pretty.*` format of the repository's own
/// configuration (which may use `%G?` and run `gpg`), so a format must be
/// `format:` or `tformat:`, of known placeholders, never `%G…`.
#[test]
fn log_formats_must_be_explicit_and_never_check_signatures() {
    for format in [
        "short2",
        "%H",
        "oneline",
        "fuller",
        "format:%G?",
        "tformat:%GS",
        "format:%(describe)",
    ] {
        refused(
            Tool::Git,
            &["log", "-1", &format!("--format={format}")],
            codes::INVALID_ARGUMENT,
        );
    }
    passes(Tool::Git, &["log", "-1", "--format=tformat:%h %s"]);
    assert!(git::HARDENING.contains(&"log.showSignature=false"));
}

/// M3: the compose project is the binding's, never the plugin's.
#[test]
fn a_plugin_can_never_name_the_compose_project() {
    for args in [
        &["compose", "ps", "--project-name", "other"][..],
        &["compose", "ps", "--project-name=other"],
        &["compose", "-p", "other", "down"],
        &["compose", "down", "-p", "other"],
        &["compose", "down", "-pother"],
        &["compose", "up", "--detach", "-dp", "other"],
    ] {
        refused(Tool::Docker, args, codes::OPTION_DENIED);
    }
    assert_eq!(docker::normalized_project("My App.v2"), "myappv2");
    assert_eq!(docker::normalized_project("_-x_y"), "x_y");
    assert_eq!(docker::normalized_project("..."), "");
}

/// M2: a folder handle's compose file is one inside the folder, never one
/// above it or one a link or `.env` points to outside.
#[test]
fn the_compose_file_of_a_folder_is_inside_it() {
    let tree = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tree.path()).unwrap();
    std::fs::write(
        root.join("compose.yaml"),
        "services: {outside: {image: alpine}}\n",
    )
    .unwrap();
    for dir in ["bound", "b2", "linked", "named", "Odd Name"] {
        std::fs::create_dir(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("bound/docker-compose.yml"), "services: {}\n").unwrap();
    std::fs::write(root.join("bound/compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(root.join("b2/.env"), "COMPOSE_FILE=../compose.yaml\n").unwrap();
    std::fs::write(
        root.join("named/compose.yml"),
        "name: shop # pinned\nservices: {}\n",
    )
    .unwrap();
    std::fs::write(root.join("Odd Name/compose.yaml"), "name: ${X}\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("compose.yaml"), root.join("linked/compose.yaml"))
        .unwrap();

    let project = |dir: &str| docker::compose_project(&Target::Directory(root.join(dir)));
    let bound = project("bound").unwrap();
    assert_eq!(bound.file, root.join("bound/compose.yaml"));
    assert_eq!(bound.dir, root.join("bound"));
    assert_eq!(bound.name, "bound");
    // No file in the folder: refused, whatever its `.env` says.
    assert_eq!(project("b2").unwrap_err().code, codes::INVALID_CWD);
    #[cfg(unix)]
    assert_eq!(project("linked").unwrap_err().code, codes::INVALID_CWD);
    assert_eq!(project("named").unwrap().name, "shop");
    // An interpolated name is not taken; the folder's name is.
    assert_eq!(project("Odd Name").unwrap().name, "oddname");
    let file = docker::compose_project(&Target::File(root.join("named/compose.yml"))).unwrap();
    assert_eq!(
        (file.dir, file.name),
        (root.join("named"), "shop".to_owned())
    );
}

/// I2: credentials in URLs are removed however they are written.
#[test]
fn credentials_in_queries_and_passwords_with_slashes_are_redacted() {
    for (text, expected) in [
        ("https://u:pa/ss@host/x", "https://host/x"),
        ("https://user:p@ss@host/", "https://host/"),
        (
            "x https://me@corp.com:p@s/s@git.host/r.git y",
            "x https://git.host/r.git y",
        ),
        (
            "fatal: 'https://oauth2:abc/def@gitlab.com/g/p.git'",
            "fatal: 'https://gitlab.com/g/p.git'",
        ),
        (
            "https://host/api?private_token=glpat-1&x=1",
            "https://host/api?private_token=[redacted]&x=1",
        ),
        (
            "GET /x?access_token=ghp_1 HTTP",
            "GET /x?access_token=[redacted] HTTP",
        ),
        ("u?a=1&Token=s3cr3t#frag", "u?a=1&Token=[redacted]#frag"),
        (
            "https://h/?password=p'w",
            "https://h/?password=[redacted]'w",
        ),
        ("https://host/x@y", "https://host/x@y"),
        ("tokens=1 notoken=2", "tokens=1 notoken=2"),
    ] {
        assert_eq!(redact_userinfo(text), expected, "{text}");
    }
}

/// L3: only absolute `PATH` entries reach a run, and the executable's own
/// folder.
#[test]
fn only_absolute_path_entries_reach_the_child() {
    let agent_path = path_of(&["/usr/bin", "", ".", "relative/bin", "/bin"]);
    let host = ToolHost::for_test().with_environment(&[("PATH", &agent_path)]);
    let env = host.environment(
        &[],
        &absolute("/opt/tools").join(executable_name(Tool::Git)),
    );
    let path = env
        .iter()
        .find(|(n, _)| n == "PATH")
        .map(|(_, v)| v.clone());
    assert_eq!(
        path,
        Some(OsString::from(path_of(&["/usr/bin", "/bin", "/opt/tools"])))
    );
    assert_eq!(
        host.path_entries(),
        [absolute("/usr/bin"), absolute("/bin")]
    );
}

/// I1: the user's settings of the built-in modules are the host's.
#[test]
fn the_users_module_settings_are_the_hosts() {
    let git_setting = absolute("/opt/git/bin").join(executable_name(Tool::Git));
    let git = deckpilot_core::ActionParameters::new()
        .with(
            "gitExecutable",
            deckpilot_core::ParameterValue::Text(git_setting.to_string_lossy().into_owned()),
        )
        .with(
            "networkTimeoutSeconds",
            deckpilot_core::ParameterValue::Integer(60),
        );
    let docker = deckpilot_core::ActionParameters::new()
        .with(
            "dockerExecutable",
            deckpilot_core::ParameterValue::Text(" docker ".into()),
        )
        .with(
            "context",
            deckpilot_core::ParameterValue::Text("colima".into()),
        )
        .with(
            "timeoutSeconds",
            deckpilot_core::ParameterValue::Integer(5000),
        );
    let settings = ToolSettings::from_modules(Some(&git), Some(&docker));
    assert_eq!(
        settings,
        ToolSettings {
            git: Some(git_setting.clone()),
            docker: None,
            docker_context: Some("colima".into()),
            network_seconds: Some(60),
            modify_seconds: Some(5000),
        }
    );
    let host = ToolHost::for_test()
        .with_environment(&[])
        .with_settings(&settings);
    assert_eq!(host.docker_context.as_deref(), Some("colima"));
    assert_eq!(
        host.timeout(&passes(Tool::Git, &["ls-remote", "origin"]), None),
        Duration::from_secs(60)
    );
    assert_eq!(
        host.timeout(&passes(Tool::Docker, &["stop", "web"]), None),
        Duration::from_secs(1800)
    );
    // The setting is the executable: not found there is not found.
    assert_eq!(
        host.executable(Tool::Git).unwrap_err().code,
        codes::NOT_FOUND
    );
    assert_eq!(
        ToolSettings::from_modules(None, None),
        ToolSettings::default()
    );
}

#[cfg(unix)]
mod runs;
