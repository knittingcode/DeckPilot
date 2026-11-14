//! OS sandbox probes. `deckpilot-sandbox-probe` applies exactly the host's
//! sandbox (same code, same two stages, a thread created in between like
//! the epoch ticker) and then tries file, network and process operations
//! from native code on both threads, i.e. as a plugin that escaped wasm.
//! The same probes without the sandbox are the control: they must succeed,
//! so a failure under the sandbox is the sandbox's doing.
//!
//! On Windows the isolation comes from how the supervisor launches the
//! host (AppContainer, Job Object), so the sandboxed probe is started with
//! `HostCommand::spawn`, exactly like the host; started directly it only
//! gets the host's fallback (own job, Low integrity).

use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

const PROBE: &str = env!("CARGO_BIN_EXE_deckpilot-sandbox-probe");

struct Fixture {
    dir: tempfile::TempDir,
    _listener: TcpListener,
    addr: String,
    #[cfg(unix)]
    _unix: std::os::unix::net::UnixListener,
    #[cfg(unix)]
    unix_path: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.txt"), b"user data").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    #[cfg(unix)]
    let (unix, unix_path) = {
        // Short path: sun_path is limited to ~104 bytes.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("dp-probe-{}-{n}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        (
            std::os::unix::net::UnixListener::bind(&p).unwrap(),
            p.to_string_lossy().into_owned(),
        )
    };
    Fixture {
        dir,
        _listener: listener,
        addr,
        #[cfg(unix)]
        _unix: unix,
        #[cfg(unix)]
        unix_path,
    }
}

fn exec_target() -> &'static str {
    if cfg!(windows) {
        "C:\\Windows\\System32\\whoami.exe"
    } else {
        "/bin/echo"
    }
}

/// Runs the probe; returns the sandbox report and `(kind, thread) -> ok`.
fn run(sandboxed: bool, probes: &[(&str, String)]) -> (Value, Vec<(String, String, bool, String)>) {
    run_with(if sandboxed { &[] } else { &["--no-os-sandbox"] }, probes)
}

fn run_with(
    flags: &[&str],
    probes: &[(&str, String)],
) -> (Value, Vec<(String, String, bool, String)>) {
    let mut c = Command::new(PROBE);
    c.args(flags);
    for (k, t) in probes {
        c.arg(k).arg(t);
    }
    let out = c.stdin(Stdio::null()).output().unwrap();
    parse(&String::from_utf8_lossy(&out.stdout))
}

type Results = Vec<(String, String, bool, String)>;

fn parse(text: &str) -> (Value, Results) {
    let mut lines = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    let report = lines.next().expect("probe printed nothing");
    assert!(report.get("error").is_none(), "sandbox failed: {report}");
    let results = lines
        .map(|l| {
            (
                l["kind"].as_str().unwrap().to_owned(),
                l["thread"].as_str().unwrap().to_owned(),
                l["ok"].as_bool().unwrap(),
                l["detail"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    (report["sandbox"].clone(), results)
}

fn probes(f: &Fixture, write_name: &str) -> Vec<(&'static str, String)> {
    let p = |x: &Path| x.to_string_lossy().into_owned();
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut v = vec![
        ("read_file", p(&f.dir.path().join("secret.txt"))),
        ("write_file", p(&f.dir.path().join(write_name))),
        ("list_dir", p(f.dir.path())),
        ("connect_tcp", f.addr.clone()),
        ("exec", exec_target().to_owned()),
    ];
    #[cfg(unix)]
    {
        v.push(("read_file", "/etc/passwd".to_owned()));
        v.push(("connect_unix", f.unix_path.clone()));
    }
    v
}

#[test]
fn control_without_sandbox_can_do_everything() {
    let f = fixture();
    let (report, results) = run(false, &probes(&f, "control.txt"));
    assert_eq!(report["enforcement"], "none");
    for (kind, thread, ok, detail) in &results {
        assert!(ok, "control {kind} on {thread} failed: {detail}");
    }
    assert!(f.dir.path().join("control.txt").exists());
}

fn assert_denied(results: &[(String, String, bool, String)], kinds: &[&str]) {
    for (kind, thread, ok, detail) in results {
        if kinds.contains(&kind.as_str()) {
            assert!(!ok, "sandboxed {kind} on {thread} succeeded: {detail}");
        }
    }
    for k in kinds {
        assert!(
            results.iter().filter(|r| r.0 == *k).count() >= 2,
            "{k} was not probed on both threads"
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn sandbox_denies_files_network_and_exec() {
    let f = fixture();
    let (report, results) = run(true, &probes(&f, "sandboxed.txt"));
    let mechanism = if cfg!(target_os = "macos") {
        "macos-seatbelt"
    } else {
        "linux-landlock-seccomp"
    };
    assert_eq!(report["mechanism"], mechanism, "{report}");
    assert_ne!(report["enforcement"], "none");
    assert_denied(
        &results,
        &[
            "read_file",
            "write_file",
            "list_dir",
            "connect_tcp",
            "connect_unix",
            "exec",
        ],
    );
    assert!(!f.dir.path().join("sandboxed.txt").exists());
}

/// The probe launched like the host by the supervisor: in a fresh
/// AppContainer without capabilities inside the supervisor's Job Object.
#[cfg(windows)]
fn run_like_the_host(probes: &[(&str, String)]) -> (Value, Vec<Probed>) {
    use std::io::Read;
    let mut c = deckpilot_plugin_supervisor::HostCommand::new(PROBE);
    for (k, t) in probes {
        c = c.arg(k).arg(t);
    }
    let mut child = c.spawn().expect("spawn in AppContainer");
    drop(child.stdin.take());
    let mut text = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let status = child.wait().unwrap();
    eprintln!("probe exit status: {status:?}\n{text}");
    let mut lines = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    let report = lines.next().expect("probe printed nothing");
    assert!(report.get("error").is_none(), "sandbox failed: {report}");
    let results = lines
        .map(|l| Probed {
            kind: l["kind"].as_str().unwrap().to_owned(),
            target: l["target"].as_str().unwrap().to_owned(),
            _thread: l["thread"].as_str().unwrap().to_owned(),
            ok: l["ok"].as_bool().unwrap(),
            detail: l["detail"].as_str().unwrap().to_owned(),
        })
        .collect();
    (report["sandbox"].clone(), results)
}

#[cfg(windows)]
#[derive(Debug)]
struct Probed {
    kind: String,
    target: String,
    /// Shown in assertion messages (Debug).
    _thread: String,
    ok: bool,
    detail: String,
}

/// Started directly (not by the supervisor), the probe gets the host's
/// fallback: its own job and a Low integrity token. That denies writes to
/// the user's files and exec, not reads or network (reported as partial).
#[cfg(windows)]
#[test]
fn fallback_without_app_container_denies_writes_and_exec() {
    let f = fixture();
    let (report, results) = run(true, &probes(&f, "sandboxed.txt"));
    assert_eq!(report["mechanism"], "windows-job-lowil", "{report}");
    assert_eq!(report["enforcement"], "partial", "{report}");
    assert_denied(&results, &["write_file", "exec"]);
    assert!(!f.dir.path().join("sandboxed.txt").exists());
}

/// The production launch: user files in the temp directory and in the
/// user profile (read, write, list), TCP to loopback and process creation
/// are denied, on both threads; a system file stays readable and the
/// pipes work (this output arrived over them), so the denials are the
/// AppContainer's, not a broken process.
#[cfg(windows)]
#[test]
fn app_container_denies_user_files_network_and_exec() {
    let f = fixture();
    let home = std::path::PathBuf::from(std::env::var_os("USERPROFILE").expect("USERPROFILE"));
    let in_home = tempfile::Builder::new()
        .prefix("dp-probe-")
        .tempdir_in(&home)
        .unwrap();
    std::fs::write(in_home.path().join("secret.txt"), b"user data").unwrap();
    let system_root =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap_or("C:\\Windows".into()));
    let system_file = system_root
        .join("System32")
        .join("kernel32.dll")
        .to_string_lossy()
        .into_owned();
    let p = |x: &Path| x.to_string_lossy().into_owned();
    let home_probes = |write_name: &str| {
        vec![
            ("read_file", p(&in_home.path().join("secret.txt"))),
            ("write_file", p(&in_home.path().join(write_name))),
            ("list_dir", p(in_home.path())),
            ("list_dir", p(&home)),
        ]
    };

    // Control: outside the AppContainer every probe succeeds.
    let mut control = probes(&f, "control.txt");
    control.extend(home_probes("control.txt"));
    let (_, results) = run(false, &control);
    for (kind, thread, ok, detail) in &results {
        assert!(ok, "control {kind} on {thread} failed: {detail}");
    }

    let mut sandboxed = probes(&f, "sandboxed.txt");
    sandboxed.extend(home_probes("sandboxed.txt"));
    sandboxed.push(("read_file", system_file.clone()));
    sandboxed.push(("env_count", "-".to_owned()));
    let (report, results) = run_like_the_host(&sandboxed);
    assert_eq!(report["mechanism"], "windows-appcontainer-job", "{report}");
    assert_eq!(report["enforcement"], "full", "{report}");
    assert_eq!(results.len(), sandboxed.len() * 2, "{results:?}");
    for r in &results {
        let allowed = r.kind == "env_count" || (r.kind == "read_file" && r.target == system_file);
        if allowed {
            assert!(r.ok, "{r:?} should work");
        } else {
            assert!(!r.ok, "sandboxed probe succeeded: {r:?}");
        }
        if r.kind == "connect_tcp" {
            // 10106: Winsock could not load its providers (environment),
            // which would make the denial meaningless.
            assert!(!r.detail.contains("10106"), "{r:?}");
        }
    }
    for kind in ["read_file", "write_file", "list_dir", "connect_tcp", "exec"] {
        let denied = results.iter().filter(|r| r.kind == kind && !r.ok).count();
        assert!(denied >= 2, "{kind} was not denied on both threads");
    }
    assert!(!f.dir.path().join("sandboxed.txt").exists());
    assert!(!in_home.path().join("sandboxed.txt").exists());
}

/// Landlock on its own (no seccomp): file access is denied by Landlock
/// (EACCES), independently of the seccomp layer (EPERM).
#[cfg(target_os = "linux")]
#[test]
fn landlock_alone_denies_file_access() {
    let f = fixture();
    let (report, results) = run_with(&["--stage1-only"], &probes(&f, "landlock.txt"));
    let notes = report["notes"].to_string();
    if notes.contains("NotEnforced") {
        eprintln!("SKIPPED: Landlock is not available on this kernel ({notes})");
        return;
    }
    for (kind, thread, ok, detail) in &results {
        if ["read_file", "write_file", "list_dir", "exec"].contains(&kind.as_str()) {
            assert!(!ok, "{kind} on {thread} succeeded under Landlock: {detail}");
            assert!(
                detail.contains("os error 13"),
                "{kind}: not a Landlock denial: {detail}"
            );
        }
    }
    assert!(!f.dir.path().join("landlock.txt").exists());
}
