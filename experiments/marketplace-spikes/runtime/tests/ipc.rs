//! Spike 2: supervisor <-> plugin host process over length-prefixed JSON.
//! Platform independent (stdin/stdout pipes). IPC is not a sandbox: this
//! file proves crash isolation and supervision only.

use std::io::Cursor;
use std::time::{Duration, Instant};

use runtime_spike::broker::Broker;
use runtime_spike::protocol::{ToHost, ToSupervisor, read_frame, write_frame};
use runtime_spike::supervisor::{PluginProcess, SpawnConfig, SupError};

const HOST: &str = env!("CARGO_BIN_EXE_plugin-host");
const T: Duration = Duration::from_secs(10);

fn spawn() -> PluginProcess {
    PluginProcess::spawn(SpawnConfig::new(HOST, "test.plugin")).unwrap()
}

#[test]
fn ready_handshake() {
    let p = spawn();
    assert!(matches!(p.ready, ToSupervisor::Ready { .. }));
}

#[test]
fn ping_1000_round_trips() {
    let mut p = spawn();
    let mut samples = Vec::with_capacity(1000);
    for seq in 0..1000 {
        let t = Instant::now();
        p.ping(seq, T).unwrap();
        samples.push(t.elapsed());
    }
    samples.sort();
    eprintln!(
        "ping x1000 (debug build): p50={:?} p99={:?} max={:?}",
        samples[500], samples[990], samples[999]
    );
}

#[test]
fn guest_runs_in_host_process() {
    let mut p = spawn();
    let out = p.invoke(&Broker::new(), "add", None, T).unwrap();
    assert_eq!(out.result.unwrap(), 5);
}

#[test]
fn host_environment_is_empty() {
    let mut p = spawn();
    let (ok, detail) = p.probe("env", "").unwrap();
    assert!(ok);
    assert_eq!(detail, "0 vars");
}

#[test]
fn guest_trap_is_reported_and_host_keeps_serving() {
    let mut p = spawn();
    let b = Broker::new();
    for entry in ["trap_panic", "trap_unreachable", "memory_bomb"] {
        let out = p.invoke(&b, entry, None, T).unwrap();
        let err = out.result.unwrap_err();
        assert!(err.starts_with("trap:"), "{entry}: {err}");
    }
    p.ping(1, T).unwrap();
    assert_eq!(p.invoke(&b, "add", None, T).unwrap().result.unwrap(), 5);
}

#[test]
fn guest_infinite_loop_is_interrupted_inside_host() {
    let mut p = spawn();
    let t = Instant::now();
    let out = p.invoke(&Broker::new(), "spin", None, T).unwrap();
    assert_eq!(out.result.unwrap_err(), "trap: Interrupt");
    assert!(t.elapsed() < Duration::from_secs(3));
    p.ping(2, T).unwrap();
}

#[test]
fn supervisor_survives_host_crash_and_respawns() {
    let mut p = spawn();
    let pid = p.pid();
    p.send(&ToHost::Crash).unwrap();
    match p.recv(T) {
        Err(SupError::Closed(status)) => {
            let status = status.expect("exit status");
            assert!(!status.success());
            eprintln!("host {pid} crashed: {status:?}");
        }
        other => panic!("expected Closed, got {other:?}"),
    }
    // Sending to the dead child is an error, not a supervisor crash.
    assert!(p.ping(3, Duration::from_secs(1)).is_err());
    let mut q = spawn();
    q.ping(4, T).unwrap();
}

#[test]
fn supervisor_kills_hung_host_on_timeout() {
    let mut p = spawn();
    p.send(&ToHost::Hang).unwrap();
    let t = Instant::now();
    let r = p.recv(Duration::from_millis(500));
    assert!(matches!(r, Err(SupError::Timeout)), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(2));
    // The child was killed: waiting returns at once with a non-success status.
    let status = p.wait().unwrap();
    assert!(!status.success());
    let mut q = spawn();
    q.ping(5, T).unwrap();
}

#[test]
fn invoke_deadline_kills_host_that_never_answers() {
    let mut p = spawn();
    p.send(&ToHost::Hang).unwrap();
    let r = p.invoke(&Broker::new(), "add", None, Duration::from_millis(300));
    assert!(matches!(r, Err(SupError::Timeout)), "{r:?}");
}

#[test]
fn host_exits_on_malformed_frame() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(HOST)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&[0, 0, 0, 5]).unwrap();
    stdin.write_all(b"nope!").unwrap();
    drop(stdin);
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(3));
}

#[test]
fn frame_codec_rejects_oversized_and_garbage() {
    let mut buf = Vec::new();
    write_frame(&mut buf, &ToHost::Ping { seq: 7 }).unwrap();
    let msg: ToHost = read_frame(&mut Cursor::new(&buf), 1024).unwrap().unwrap();
    assert!(matches!(msg, ToHost::Ping { seq: 7 }));
    // Larger than the limit: rejected before allocating the body.
    let huge = [0xFFu8, 0xFF, 0xFF, 0xFF];
    assert!(read_frame::<_, ToHost>(&mut Cursor::new(&huge), 1024).is_err());
    // Not JSON / unknown message type.
    let mut bad = Vec::new();
    bad.extend_from_slice(&5u32.to_be_bytes());
    bad.extend_from_slice(b"{\"a\":");
    assert!(read_frame::<_, ToHost>(&mut Cursor::new(&bad), 1024).is_err());
    let mut unknown = Vec::new();
    write_frame(&mut unknown, &serde_json::json!({"type":"getDatabaseKey"})).unwrap();
    assert!(read_frame::<_, ToHost>(&mut Cursor::new(&unknown), 1024).is_err());
    // Truncated frame.
    assert!(read_frame::<_, ToHost>(&mut Cursor::new(&buf[..buf.len() - 1]), 1024).is_err());
    // Clean EOF.
    assert!(
        read_frame::<_, ToHost>(&mut Cursor::new(&[] as &[u8]), 1024)
            .unwrap()
            .is_none()
    );
}
