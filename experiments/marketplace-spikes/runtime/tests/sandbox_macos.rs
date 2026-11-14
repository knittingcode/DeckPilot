//! Spike 4: OS sandbox of the plugin host process. macOS only
//! (`sandbox-exec` + deny-default Seatbelt profile). The probes run in the
//! host process's own native code, outside wasm, i.e. they model a plugin
//! that has already escaped the wasm layer.
#![cfg(target_os = "macos")]

use std::net::TcpListener;
use std::time::Duration;

use runtime_spike::broker::{Broker, Grant};
use runtime_spike::sandbox::SandboxProfile;
use runtime_spike::supervisor::{PluginProcess, SpawnConfig};
use serde_json::json;

const HOST: &str = env!("CARGO_BIN_EXE_plugin-host");
const T: Duration = Duration::from_secs(10);

struct Env {
    scratch: tempfile::TempDir,
    outside: tempfile::TempDir,
    listener: TcpListener,
    /// Stands in for the agent's local IPC endpoint.
    agent_socket: std::os::unix::net::UnixListener,
    agent_socket_path: std::path::PathBuf,
}

fn env() -> Env {
    let scratch = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "SECRET").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let agent_socket_path = outside.path().join("agent.sock");
    let agent_socket = std::os::unix::net::UnixListener::bind(&agent_socket_path).unwrap();
    Env {
        scratch,
        outside,
        listener,
        agent_socket,
        agent_socket_path,
    }
}

fn spawn(e: &Env, sandboxed: bool) -> PluginProcess {
    let mut cfg = SpawnConfig::new(HOST, "a");
    if sandboxed {
        cfg.sandbox =
            Some(SandboxProfile::macos_deny_default(HOST.as_ref(), e.scratch.path()).unwrap());
    }
    PluginProcess::spawn(cfg).unwrap()
}

fn probes(e: &Env) -> Vec<(&'static str, String)> {
    let home = std::env::var("HOME").unwrap();
    vec![
        (
            "read_file",
            e.outside.path().join("secret.txt").display().to_string(),
        ),
        ("read_file", "/etc/passwd".into()),
        ("read_file", format!("{home}/.zshrc")),
        ("list_dir", home.clone()),
        ("list_dir", "/Users".into()),
        (
            "write_file",
            e.outside.path().join("dropped.txt").display().to_string(),
        ),
        (
            "write_file",
            format!("{home}/deckpilot-spike-should-not-exist"),
        ),
        ("connect", e.listener.local_addr().unwrap().to_string()),
        ("connect", "1.1.1.1:443".into()),
        ("connect_unix", e.agent_socket_path.display().to_string()),
        ("exec", "/bin/echo".into()),
    ]
}

#[test]
fn control_unsandboxed_host_has_ambient_authority() {
    // Same host, same probes, no OS sandbox: everything local succeeds.
    // This is why "separate process" is crash isolation only.
    let e = env();
    let mut p = spawn(&e, false);
    let secret = e.outside.path().join("secret.txt").display().to_string();
    assert!(p.probe("read_file", &secret).unwrap().0);
    assert!(
        p.probe("list_dir", &std::env::var("HOME").unwrap())
            .unwrap()
            .0
    );
    assert!(
        p.probe(
            "write_file",
            &e.outside.path().join("w.txt").display().to_string()
        )
        .unwrap()
        .0
    );
    assert!(
        p.probe("connect", &e.listener.local_addr().unwrap().to_string())
            .unwrap()
            .0
    );
    assert!(
        p.probe("connect_unix", &e.agent_socket_path.display().to_string())
            .unwrap()
            .0
    );
    assert!(p.probe("exec", "/bin/echo").unwrap().0);
    let _ = &e.agent_socket;
}

#[test]
fn sandboxed_host_starts_and_runs_wasm() {
    let e = env();
    let mut p = spawn(&e, true);
    p.ping(1, T).unwrap();
    // Cranelift-compiled code still runs under the profile.
    assert_eq!(
        p.invoke(&Broker::new(), "add", None, T)
            .unwrap()
            .result
            .unwrap(),
        5
    );
    assert_eq!(
        p.invoke(&Broker::new(), "spin", None, T)
            .unwrap()
            .result
            .unwrap_err(),
        "trap: Interrupt"
    );
}

#[test]
fn sandboxed_host_cannot_read_write_connect_or_exec() {
    let e = env();
    let mut p = spawn(&e, true);
    for (kind, target) in probes(&e) {
        let (ok, detail) = p.probe(kind, &target).unwrap();
        eprintln!("sandboxed {kind} {target}: ok={ok} {detail}");
        assert!(!ok, "{kind} {target} should be blocked: {detail}");
    }
    assert!(!e.outside.path().join("dropped.txt").exists());
    // The process is still alive after the denials.
    p.ping(2, T).unwrap();
}

#[test]
fn sandboxed_host_can_use_its_scratch_dir_only() {
    let e = env();
    let mut p = spawn(&e, true);
    let f = e.scratch.path().join("ok.txt").display().to_string();
    assert!(p.probe("write_file", &f).unwrap().0);
    assert!(p.probe("read_file", &f).unwrap().0);
}

#[test]
fn broker_still_works_for_sandboxed_host() {
    // The file the plugin may read is read by the supervisor (unsandboxed)
    // and sent over the channel; the host itself cannot open it.
    let e = env();
    let mut b = Broker::new();
    b.grant("a", Grant::fs_read(e.outside.path()).unwrap());
    let mut p = spawn(&e, true);
    let secret = e.outside.path().join("secret.txt").display().to_string();
    assert!(!p.probe("read_file", &secret).unwrap().0);
    let req = json!({"op":"fs.read","args":{"path":"secret.txt"}}).to_string();
    let out = p.invoke(&b, "relay", Some(&req), T).unwrap();
    assert_eq!(out.result.unwrap()["ok"]["text"], "SECRET");
}
