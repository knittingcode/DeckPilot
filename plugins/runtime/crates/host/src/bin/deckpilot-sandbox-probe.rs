//! Test-only probe (not shipped): applies exactly the host's sandbox
//! (`deckpilot_plugin_host::sandbox`, same two stages, with a thread started
//! in between like the host's epoch ticker), then attempts operations from
//! native code, i.e. it models a plugin that has escaped the wasm layer.
//!
//! Usage: `deckpilot-sandbox-probe [--no-os-sandbox] [--stage1-only] (<kind> <target>)...`
//! Kinds: `read_file`, `write_file`, `list_dir`, `connect_tcp` (`ip:port`),
//! `connect_unix` (Unix only), `exec`, `env_count`, `open_fds` (Unix only).
//! Each probe runs on the main thread and on the helper thread. Output: one
//! JSON line with the sandbox report, then one per probe and thread:
//! `{"kind","target","thread","ok","detail"}`.

use std::io::{self, Write};
use std::sync::mpsc;
use std::time::Duration;

use deckpilot_plugin_host::sandbox::{self, Mode};
use serde_json::json;

fn probe(kind: &str, target: &str) -> (bool, String) {
    let r: io::Result<String> = match kind {
        "read_file" => std::fs::read(target).map(|b| format!("read {} bytes", b.len())),
        "write_file" => std::fs::write(target, b"probe").map(|_| "written".into()),
        "list_dir" => std::fs::read_dir(target).map(|d| format!("{} entries", d.count())),
        "connect_tcp" => target
            .parse()
            .map_err(io::Error::other)
            .and_then(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(3)))
            .map(|_| "connected".into()),
        "exec" => std::process::Command::new(target)
            .output()
            .map(|o| format!("exit {}", o.status)),
        #[cfg(unix)]
        "connect_unix" => {
            std::os::unix::net::UnixStream::connect(target).map(|_| "connected".into())
        }
        "env_count" => Ok(format!("{}", std::env::vars_os().count())),
        #[cfg(unix)]
        "open_fds" => Ok(open_fds().to_string()),
        _ => Err(io::Error::other("unknown probe")),
    };
    match r {
        Ok(d) => (true, d),
        Err(e) => (false, format!("{:?}: {e}", e.kind())),
    }
}

/// Descriptors other than 0, 1 and 2 that are open. Meaningful only
/// without the sandbox (under seccomp `fcntl` itself is denied).
#[cfg(unix)]
fn open_fds() -> usize {
    (3..1024)
        // SAFETY: F_GETFD on any integer is harmless.
        .filter(|fd| unsafe { libc::fcntl(*fd, libc::F_GETFD) } != -1)
        .count()
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mode = if args.first().is_some_and(|a| a == "--no-os-sandbox") {
        args.remove(0);
        Mode::Disabled
    } else {
        Mode::Enforce
    };
    // `--stage1-only`: apply only the first stage (Linux: Landlock and
    // process attributes, no seccomp), to test that layer on its own.
    let stage1_only = args.first().is_some_and(|a| a == "--stage1-only");
    if stage1_only {
        args.remove(0);
    }
    let probes: Vec<(String, String)> = args
        .chunks(2)
        .map(|c| (c[0].clone(), c.get(1).cloned().unwrap_or_default()))
        .collect();

    let pending = match sandbox::begin(mode) {
        Ok(p) => p,
        Err(e) => {
            println!("{}", json!({"error": e.to_string()}));
            std::process::exit(5);
        }
    };
    // Like the host's epoch ticker: a thread created between the stages.
    let (to_helper, helper_rx) = mpsc::channel::<(String, String)>();
    let (helper_tx, from_helper) = mpsc::channel::<(bool, String)>();
    std::thread::spawn(move || {
        for (k, t) in helper_rx {
            let _ = helper_tx.send(probe(&k, &t));
        }
    });
    let report = if stage1_only {
        json!({ "stage1Only": true, "notes": pending.notes() })
    } else {
        match pending.finish() {
            Ok(r) => serde_json::to_value(r).unwrap_or_default(),
            Err(e) => {
                println!("{}", json!({"error": e.to_string()}));
                std::process::exit(5);
            }
        }
    };
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{}", json!({ "sandbox": report }));
    for (kind, target) in probes {
        let (ok, detail) = probe(&kind, &target);
        let _ = writeln!(
            out,
            "{}",
            json!({"kind": kind, "target": target, "thread": "main", "ok": ok, "detail": detail})
        );
        let _ = to_helper.send((kind.clone(), target.clone()));
        let (ok, detail) = from_helper
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or((false, "helper thread did not answer".into()));
        let _ = writeln!(
            out,
            "{}",
            json!({"kind": kind, "target": target, "thread": "helper", "ok": ok, "detail": detail})
        );
    }
    let _ = out.flush();
}
