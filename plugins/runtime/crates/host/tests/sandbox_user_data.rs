//! The user's own data, not only a temporary directory, is out of reach
//! under the host's OS sandbox (ported from the retired macOS spike,
//! `sandboxed_host_cannot_read_write_connect_or_exec`, which probed `$HOME`,
//! a dotfile and the list of users). `sandbox.rs` probes a fresh temporary
//! directory; this file probes a directory inside `$HOME` and the home
//! directory itself, the places a plugin that escaped wasm would look for
//! secrets. Every probe has a control outside the sandbox that must
//! succeed. Windows has the same check in `sandbox.rs`
//! (`app_container_denies_user_files_network_and_exec`, user profile).

#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::process::{Command, Stdio};

use serde_json::Value;

const PROBE: &str = env!("CARGO_BIN_EXE_deckpilot-sandbox-probe");

/// `(kind, thread, ok, detail)` for every probe on every thread.
fn run(sandboxed: bool, probes: &[(&str, String)]) -> (Value, Vec<(String, String, bool, String)>) {
    let mut c = Command::new(PROBE);
    if !sandboxed {
        c.arg("--no-os-sandbox");
    }
    for (k, t) in probes {
        c.arg(k).arg(t);
    }
    let out = c.stdin(Stdio::null()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
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

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME is set"))
}

#[test]
fn sandbox_denies_the_users_home() {
    // A private directory inside $HOME, removed when the test ends.
    let dir = tempfile::Builder::new()
        .prefix(".deckpilot-sandbox-test-")
        .tempdir_in(home())
        .unwrap();
    std::fs::write(dir.path().join("credentials"), b"user secret").unwrap();
    let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    let users = if cfg!(target_os = "macos") {
        "/Users"
    } else {
        "/home"
    };
    let mut probes = vec![
        ("read_file", path("credentials")),
        ("list_dir", dir.path().to_string_lossy().into_owned()),
        ("list_dir", home().to_string_lossy().into_owned()),
    ];
    if std::path::Path::new(users).is_dir() {
        probes.push(("list_dir", users.to_owned()));
    }

    // Control: the same probes outside the sandbox succeed, plus a write.
    let mut control = probes.clone();
    control.push(("write_file", path("control.txt")));
    let (report, results) = run(false, &control);
    assert_eq!(report["enforcement"], "none");
    assert_eq!(results.len(), control.len() * 2, "{results:?}");
    for (kind, thread, ok, detail) in &results {
        assert!(ok, "control {kind} on {thread} failed: {detail}");
    }
    assert!(dir.path().join("control.txt").exists());

    // Sandboxed: every read, listing and write is denied on both threads.
    let mut sandboxed = probes.clone();
    sandboxed.push(("write_file", path("dropped.txt")));
    let (report, results) = run(true, &sandboxed);
    assert_ne!(report["enforcement"], "none", "{report}");
    assert_eq!(results.len(), sandboxed.len() * 2, "{results:?}");
    for (kind, thread, ok, detail) in &results {
        assert!(!ok, "sandboxed {kind} on {thread} succeeded: {detail}");
    }
    assert!(!dir.path().join("dropped.txt").exists());
}
