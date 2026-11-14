//! The real host binary under the real supervisor: handshake, OS sandbox
//! report, broker relay, crash and hang containment, restart policy, and
//! malformed frames sent straight to the host.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::frame::{read_frame, write_frame};
use deckpilot_plugin_abi::protocol::{
    Enforcement, FatalReason, GuestLimits, ToHost, ToSupervisor, exit_code,
};
use deckpilot_plugin_abi::{ActionOutcome, CallResult, ParamValue, Params};
use deckpilot_plugin_broker::{
    Allowed, ApprovedPermissions, Broker, ManualClock, MemoryAudit, PluginId, RateLimits,
};
use deckpilot_plugin_supervisor::{
    CapabilityExecutor, HostCommand, HostConfig, HostError, PluginHost, RestartPolicy, Supervisor,
    SupervisorError, SupervisorState,
};
use deckpilot_plugin_test_fixtures::TEST_GUEST;
use serde_json::json;

const HOST: &str = env!("CARGO_BIN_EXE_deckpilot-plugin-host");
const PROBE: &str = env!("CARGO_BIN_EXE_deckpilot-sandbox-probe");
const T: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Executor {
    executed: Vec<Allowed>,
    delay: Duration,
}

impl CapabilityExecutor for Executor {
    fn execute(&mut self, _plugin: &PluginId, allowed: &Allowed) -> CallResult {
        std::thread::sleep(self.delay);
        self.executed.push(allowed.clone());
        CallResult::Ok(json!({"status": 200}))
    }
}

fn plugin() -> PluginId {
    PluginId::new("acme.weather").unwrap()
}

fn broker(perms: &str) -> Broker {
    let b = Broker::new(
        Arc::new(MemoryAudit::new(1000)),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    b.set_grants(&plugin(), &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    b
}

fn config() -> HostConfig {
    HostConfig::new(HostCommand::new(HOST), Arc::from(TEST_GUEST))
}

fn spawn(cfg: &HostConfig, b: &Broker) -> PluginHost {
    PluginHost::spawn(cfg, b.session(plugin())).unwrap()
}

fn params(v: serde_json::Value) -> Params {
    serde_json::from_value(v).unwrap()
}

fn data(o: &ActionOutcome) -> &Params {
    match o {
        ActionOutcome::Ok { data, .. } => data,
        other => panic!("expected ok, got {other:?}"),
    }
}

const API: &str = r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]}]"#;

#[test]
fn host_starts_sandboxed_and_serves_invocations() {
    let b = broker(API);
    let mut h = spawn(&config(), &b);
    let s = h.sandbox().clone();
    assert_ne!(s.enforcement, Enforcement::None, "{s:?}");
    let expected = if cfg!(target_os = "macos") {
        "macos-seatbelt"
    } else if cfg!(target_os = "linux") {
        "linux-landlock-seccomp"
    } else {
        "windows-job-lowil"
    };
    assert_eq!(s.mechanism, expected);
    let mut ex = Executor::default();
    let o = h
        .invoke("add", &params(json!({"a": 2, "b": 3})), T, &mut ex)
        .unwrap();
    assert_eq!(data(&o)["sum"], ParamValue::Integer(5));
    for i in 1..=3 {
        let o = h.invoke("count", &Params::new(), T, &mut ex).unwrap();
        assert_eq!(data(&o)["n"], ParamValue::Integer(i));
    }
    assert_eq!(h.shutdown(T).map(|s| s.code()), Some(Some(0)));
}

#[test]
fn capability_calls_go_through_the_broker() {
    let b = broker(API);
    let mut h = spawn(&config(), &b);
    let mut ex = Executor::default();
    let o = h
        .invoke(
            "fetch",
            &params(json!({"url": "https://API.example.com/x?q=1"})),
            T,
            &mut ex,
        )
        .unwrap();
    assert_eq!(data(&o)["ok"], ParamValue::Bool(true));
    let Allowed::Fetch(f) = &ex.executed[0] else {
        panic!()
    };
    assert_eq!(f.url.as_str(), "https://api.example.com/x?q=1");

    for url in [
        "https://api.example.com@evil.net/",
        "https://evil.net/",
        "http://api.example.com/",
        "https://127.0.0.1/",
    ] {
        let o = h
            .invoke("fetch", &params(json!({"url": url})), T, &mut ex)
            .unwrap();
        assert_eq!(data(&o)["ok"], ParamValue::Bool(false), "{url}");
        assert_eq!(
            data(&o)["code"],
            ParamValue::String("not-permitted".into()),
            "{url}"
        );
    }
    assert_eq!(
        ex.executed.len(),
        1,
        "denied requests must never be executed"
    );

    // revocation applies to the next call, same process
    b.remove_plugin(&plugin());
    let o = h
        .invoke(
            "fetch",
            &params(json!({"url": "https://api.example.com/"})),
            T,
            &mut ex,
        )
        .unwrap();
    assert_eq!(data(&o)["ok"], ParamValue::Bool(false));
    assert_eq!(ex.executed.len(), 1);
}

#[test]
fn identity_comes_from_the_process_not_the_plugin() {
    // The broker knows two plugins; this process belongs to `evil.thief`,
    // which has no network grant. Its guest cannot borrow acme.weather's.
    let b = broker(API);
    let thief = PluginId::new("evil.thief").unwrap();
    b.set_grants(&thief, &ApprovedPermissions::default())
        .unwrap();
    let mut h = PluginHost::spawn(&config(), b.session(thief)).unwrap();
    let mut ex = Executor::default();
    let o = h
        .invoke(
            "fetch",
            &params(json!({"url": "https://api.example.com/"})),
            T,
            &mut ex,
        )
        .unwrap();
    assert_eq!(data(&o)["ok"], ParamValue::Bool(false));
    let spoof = r#"{"op":"http.fetch","args":{"method":"GET","url":"https://api.example.com/","plugin":"acme.weather"}}"#;
    let o = h
        .invoke("raw_request", &params(json!({"json": spoof})), T, &mut ex)
        .unwrap();
    assert!(matches!(&data(&o)["reply"], ParamValue::String(s) if s.contains("bad-request")));
    assert!(ex.executed.is_empty());
}

fn expect_crash(action: &str, reason: FatalReason, limits: Option<GuestLimits>) {
    let b = broker("[]");
    let mut cfg = config();
    if let Some(l) = limits {
        cfg.limits = l;
    }
    let mut h = spawn(&cfg, &b);
    let mut ex = Executor::default();
    let t = Instant::now();
    match h.invoke(action, &Params::new(), T, &mut ex) {
        Err(HostError::Crashed {
            fatal: Some((r, _)),
            status: Some(status),
        }) => {
            assert_eq!(r, reason, "{action}");
            assert_eq!(
                status.code(),
                Some(reason.exit_code()),
                "{action}: {status:?}"
            );
        }
        other => panic!("{action}: {other:?}"),
    }
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "{action} took {:?}",
        t.elapsed()
    );
    assert!(!h.is_running());
    assert!(matches!(
        h.invoke("add", &Params::new(), T, &mut ex),
        Err(HostError::NotRunning)
    ));
}

#[test]
fn host_exits_after_any_trap() {
    for action in ["panic", "unreachable", "oob", "memory_bomb", "recurse"] {
        expect_crash(action, FatalReason::Trap, None);
    }
    expect_crash("spin", FatalReason::Deadline, None);
    expect_crash(
        "spin",
        FatalReason::OutOfFuel,
        Some(GuestLimits {
            deadline_ms: 10_000,
            fuel_per_invocation: 50_000_000,
            ..GuestLimits::default()
        }),
    );
    for action in [
        "host_call_bad_pointer",
        "host_call_huge_len",
        "host_call_oversized",
        "spin_with_calls",
    ] {
        expect_crash(action, FatalReason::GuestAbiViolation, None);
    }
}

#[test]
fn hung_host_is_killed_on_timeout() {
    // Deadline and fuel high: only the supervisor's wall clock stops it.
    let b = broker("[]");
    let mut cfg = config();
    cfg.limits = GuestLimits {
        deadline_ms: 10_000,
        fuel_per_invocation: 50_000_000_000,
        ..GuestLimits::default()
    };
    let mut h = spawn(&cfg, &b);
    let t = Instant::now();
    let r = h.invoke(
        "spin",
        &Params::new(),
        Duration::from_millis(500),
        &mut Executor::default(),
    );
    assert!(matches!(r, Err(HostError::Timeout { .. })), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert!(!h.is_running());
}

#[test]
fn slow_capability_calls_do_not_trip_the_deadline() {
    let b = broker(API);
    let mut h = spawn(&config(), &b);
    let mut ex = Executor {
        delay: Duration::from_millis(400),
        ..Executor::default()
    };
    let o = h
        .invoke(
            "fetch",
            &params(json!({"url": "https://api.example.com/"})),
            T,
            &mut ex,
        )
        .unwrap();
    assert_eq!(data(&o)["ok"], ParamValue::Bool(true));
}

#[test]
fn flooding_capability_calls_is_throttled_then_fatal() {
    let b = broker("[]");
    let mut h = spawn(&config(), &b);
    let mut ex = Executor::default();
    // 120 log calls: the broker's log bucket (50/s) throttles the rest.
    let o = h
        .invoke("log_n", &params(json!({"n": 120})), T, &mut ex)
        .unwrap();
    let (ok, err) = (&data(&o)["ok"], &data(&o)["err"]);
    assert_eq!(ok, &ParamValue::Integer(50));
    assert_eq!(err, &ParamValue::Integer(70));
    // Beyond the per-invocation cap the host itself stops the guest.
    let r = h.invoke("log_n", &params(json!({"n": 1000})), T, &mut ex);
    assert!(
        matches!(
            r,
            Err(HostError::Crashed {
                fatal: Some((FatalReason::GuestAbiViolation, _)),
                ..
            })
        ),
        "{r:?}"
    );
}

#[test]
fn invalid_modules_fail_to_load() {
    let b = broker("[]");
    let wasi = wat::parse_str(
        r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1))"#,
    )
    .unwrap();
    let mut cfg = config();
    cfg.module = Arc::from(wasi);
    match PluginHost::spawn(&cfg, b.session(plugin())) {
        Err(HostError::LoadFailed { status, detail }) => {
            assert_eq!(
                status.and_then(|s| s.code()),
                Some(exit_code::LOAD_FAILED),
                "{detail}"
            );
            assert!(detail.contains("forbidden import"), "{detail}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn unsandboxed_host_is_refused_when_a_sandbox_is_required() {
    let b = broker("[]");
    let mut cfg = config();
    cfg.command = HostCommand::new(HOST).arg("--no-os-sandbox");
    match PluginHost::spawn(&cfg, b.session(plugin())) {
        Err(HostError::SandboxMissing(r)) => assert_eq!(r.enforcement, Enforcement::None),
        other => panic!("{other:?}"),
    }
    cfg.require_sandbox = false;
    let mut h = PluginHost::spawn(&cfg, b.session(plugin())).unwrap();
    assert!(
        h.invoke("add", &Params::new(), T, &mut Executor::default())
            .is_ok()
    );
}

#[test]
fn supervisor_restarts_with_backoff_and_breaks_crash_loops() {
    let b = broker("[]");
    let clock = ManualClock::default();
    let mut sup = Supervisor::new(
        config(),
        b,
        plugin(),
        RestartPolicy::default(),
        Arc::new(clock.clone()),
    );
    let mut ex = Executor::default();
    assert!(sup.invoke("add", &Params::new(), T, &mut ex).is_ok());
    assert_eq!(sup.state(), SupervisorState::Running);

    let mut backoffs = vec![];
    for i in 0..5 {
        let r = sup.invoke("panic", &Params::new(), T, &mut ex);
        assert!(
            matches!(r, Err(SupervisorError::Host(HostError::Crashed { .. }))),
            "{i}: {r:?}"
        );
        assert_eq!(
            sup.last_exit().and_then(|s| s.code()),
            Some(exit_code::TRAP)
        );
        if i == 4 {
            break;
        }
        let SupervisorState::BackingOff { retry_after } = sup.state() else {
            panic!("{:?}", sup.state())
        };
        backoffs.push(retry_after.as_secs());
        assert!(matches!(
            sup.invoke("add", &Params::new(), T, &mut ex),
            Err(SupervisorError::BackingOff { .. })
        ));
        clock.advance(retry_after);
    }
    assert_eq!(backoffs, vec![1, 2, 4, 8]);
    assert_eq!(sup.state(), SupervisorState::Disabled);
    assert!(matches!(
        sup.invoke("add", &Params::new(), T, &mut ex),
        Err(SupervisorError::Disabled)
    ));
    sup.reenable();
    assert!(sup.invoke("add", &Params::new(), T, &mut ex).is_ok());
    assert_eq!(sup.shutdown().and_then(|s| s.code()), Some(0));
}

#[test]
fn crashes_outside_the_window_are_forgotten() {
    let clock = ManualClock::default();
    let mut sup = Supervisor::new(
        config(),
        broker("[]"),
        plugin(),
        RestartPolicy {
            max_crashes_in_window: 2,
            ..RestartPolicy::default()
        },
        Arc::new(clock.clone()),
    );
    let mut ex = Executor::default();
    assert!(sup.invoke("panic", &Params::new(), T, &mut ex).is_err());
    clock.advance(Duration::from_secs(601));
    assert!(sup.invoke("panic", &Params::new(), T, &mut ex).is_err());
    assert_ne!(sup.state(), SupervisorState::Disabled);
}

#[test]
fn spawned_host_gets_no_environment_and_no_extra_descriptors() {
    // The probe binary is spawned exactly like the host (same command
    // builder), without the sandbox so that it can inspect itself.
    let mut c = HostCommand::new(PROBE)
        .arg("--no-os-sandbox")
        .arg("env_count")
        .arg("-");
    if cfg!(unix) {
        c = c.arg("open_fds").arg("-");
    }
    let out = c.command().stdin(Stdio::null()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for l in &lines[1..] {
        assert_eq!(l["detail"], "0", "{l}");
    }
}

// --- malformed frames sent straight to the host process ---

fn raw_host() -> std::process::Child {
    HostCommand::new(HOST).command().spawn().unwrap()
}

fn read_ready(child: &mut std::process::Child) {
    let out = child.stdout.as_mut().unwrap();
    let m: Option<ToSupervisor> = read_frame(out, 1 << 20).unwrap();
    assert!(matches!(m, Some(ToSupervisor::Ready { .. })));
}

fn exit_code_after(bytes: &[u8], load_first: bool) -> Option<i32> {
    let mut child = raw_host();
    read_ready(&mut child);
    let stdin = child.stdin.as_mut().unwrap();
    if load_first {
        write_frame(
            stdin,
            &ToHost::Load {
                abi_version: 1,
                module: TEST_GUEST.to_vec(),
                limits: GuestLimits::default(),
            },
            32 << 20,
        )
        .unwrap();
        let m: Option<ToSupervisor> = read_frame(child.stdout.as_mut().unwrap(), 1 << 20).unwrap();
        assert!(matches!(m, Some(ToSupervisor::Loaded { .. })), "{m:?}");
    }
    let stdin = child.stdin.as_mut().unwrap();
    let _ = stdin.write_all(bytes);
    let _ = stdin.flush();
    drop(child.stdin.take());
    let mut rest = Vec::new();
    let _ = child.stdout.as_mut().unwrap().read_to_end(&mut rest);
    child.wait().unwrap().code()
}

fn frame(body: &[u8]) -> Vec<u8> {
    let mut v = (body.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(body);
    v
}

#[test]
fn malformed_frames_end_the_host() {
    let pv = Some(exit_code::PROTOCOL_VIOLATION);
    assert_eq!(exit_code_after(&frame(b"garbage"), false), pv);
    assert_eq!(exit_code_after(&u32::MAX.to_be_bytes(), false), pv);
    assert_eq!(
        exit_code_after(
            &frame(br#"{"type":"invoke","invocationId":1,"action":"add"}"#),
            false
        ),
        pv
    );
    assert_eq!(
        exit_code_after(&frame(br#"{"type":"getDatabaseKey"}"#), true),
        pv
    );
    assert_eq!(exit_code_after(&frame(&[0xff; 64]), true), pv);
    // a second load
    let load = serde_json::to_vec(&ToHost::Load {
        abi_version: 1,
        module: vec![0],
        limits: GuestLimits::default(),
    })
    .unwrap();
    assert_eq!(exit_code_after(&frame(&load), true), pv);
    // a reply nobody asked for
    let reply = br#"{"type":"reply","invocationId":1,"callId":1,"result":{"ok":1}}"#;
    assert_eq!(exit_code_after(&frame(reply), true), pv);
    // non-increasing invocation ids
    let inv = br#"{"type":"invoke","invocationId":0,"action":"add"}"#;
    assert_eq!(exit_code_after(&frame(inv), true), pv);
    // after load, frames above 2 MiB are refused before allocation
    assert_eq!(exit_code_after(&(3u32 << 20).to_be_bytes(), true), pv);
    // unsupported ABI version
    let v2 = serde_json::to_vec(&ToHost::Load {
        abi_version: 2,
        module: TEST_GUEST.to_vec(),
        limits: GuestLimits::default(),
    })
    .unwrap();
    assert_eq!(
        exit_code_after(&frame(&v2), false),
        Some(exit_code::LOAD_FAILED)
    );
    // truncated frame, then EOF
    let mut t = frame(br#"{"type":"shutdown"}"#);
    t.truncate(8);
    assert_eq!(exit_code_after(&t, true), Some(exit_code::IO));
    // clean EOF and shutdown
    assert_eq!(exit_code_after(b"", true), Some(0));
    assert_eq!(
        exit_code_after(&frame(br#"{"type":"shutdown"}"#), true),
        Some(0)
    );
}

#[test]
fn unknown_arguments_are_refused() {
    let status = Command::new(HOST)
        .arg("--allow-network")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(exit_code::USAGE));
}
