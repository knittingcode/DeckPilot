//! The supervisor against scripted, hostile hosts. The host process is
//! assumed compromised (design §4.4, TB3), so every frame it sends is
//! hostile. This test binary re-executes itself as `fake-host <script>`
//! (hence `harness = false`: stdout must carry only frames).

use std::io::{self, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::frame::{read_frame, write_frame};
use deckpilot_plugin_abi::protocol::{
    Enforcement, FatalReason, SandboxReport, ToHost, ToSupervisor,
};
use deckpilot_plugin_abi::{ActionOutcome, CallResult, CapabilityRequest, ParamValue, Params};
use deckpilot_plugin_broker::{
    Allowed, ApprovedPermissions, Broker, ManualClock, MemoryAudit, PluginId, RateLimits,
};
use deckpilot_plugin_supervisor::{
    CapabilityExecutor, HostCommand, HostConfig, HostError, PluginHost,
};
use serde_json::json;

// ---------------------------------------------------------------- fake host

fn out_frame(msg: &ToSupervisor) {
    let mut out = io::stdout().lock();
    write_frame(&mut out, msg, 1 << 20).unwrap();
}

fn out_raw(body: &[u8]) {
    let mut out = io::stdout().lock();
    out.write_all(&(body.len() as u32).to_be_bytes()).unwrap();
    out.write_all(body).unwrap();
    out.flush().unwrap();
}

fn recv() -> Option<ToHost> {
    read_frame(&mut io::stdin().lock(), 32 << 20).ok().flatten()
}

fn ready(enforcement: Enforcement, protocol: u32) {
    out_frame(&ToSupervisor::Ready {
        protocol,
        host_version: "fake".into(),
        engine: "fake".into(),
        sandbox: SandboxReport {
            mechanism: "fake".into(),
            enforcement,
            notes: vec![],
        },
    });
}

fn handshake() {
    ready(Enforcement::Full, 1);
    match recv() {
        Some(ToHost::Load { .. }) => out_frame(&ToSupervisor::Loaded {
            abi_version: 1,
            descriptor: None,
        }),
        _ => std::process::exit(99),
    }
}

fn next_invoke() -> u64 {
    loop {
        match recv() {
            Some(ToHost::Invoke { invocation_id, .. }) => return invocation_id,
            Some(ToHost::Shutdown {}) | None => std::process::exit(0),
            Some(_) => {}
        }
    }
}

fn result(invocation_id: u64, outcome: ActionOutcome) {
    out_frame(&ToSupervisor::Result {
        invocation_id,
        outcome,
    });
}

fn call(invocation_id: u64, call_id: u64, request: CapabilityRequest) -> CallResult {
    out_frame(&ToSupervisor::Call {
        invocation_id,
        call_id,
        request,
    });
    match recv() {
        Some(ToHost::Reply { result, .. }) => result,
        _ => std::process::exit(98),
    }
}

fn log_request() -> CapabilityRequest {
    serde_json::from_value(json!({"op": "log", "args": {"level": "info", "message": "m"}})).unwrap()
}

fn sleep_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn fake(script: &str) {
    match script {
        "no-ready" => sleep_forever(),
        "garbage-ready" => out_raw(b"hello"),
        "huge-frame" => {
            io::stdout()
                .lock()
                .write_all(&0x7fff_ffffu32.to_be_bytes())
                .unwrap();
            io::stdout().lock().flush().unwrap();
            sleep_forever()
        }
        "unsandboxed" => ready(Enforcement::None, 1),
        "protocol-2" => ready(Enforcement::Full, 2),
        "result-before-ready" => result(1, ActionOutcome::ok()),
        "exit-after-ready" => {
            ready(Enforcement::Full, 1);
            let _ = recv();
            std::process::exit(7)
        }
        "fatal-on-load" => {
            ready(Enforcement::Full, 1);
            let _ = recv();
            out_frame(&ToSupervisor::Fatal {
                reason: FatalReason::LoadFailed,
                detail: "nope".into(),
            });
            std::process::exit(4)
        }
        _ => {
            handshake();
            let id = next_invoke();
            match script {
                "ok" => {
                    result(id, ActionOutcome::ok());
                    let id = next_invoke();
                    result(id, ActionOutcome::ok());
                    sleep_forever()
                }
                "call-wrong-invocation" => {
                    call(id + 1, 1, log_request());
                }
                "call-out-of-order" => {
                    call(id, 5, log_request());
                }
                "call-plugin-field" => {
                    out_raw(
                        format!(
                            r#"{{"type":"call","invocationId":{id},"callId":1,"plugin":"acme.weather","request":{{"op":"log","args":{{"level":"info","message":"m"}}}}}}"#
                        )
                        .as_bytes(),
                    );
                }
                "call-unknown-op" => {
                    out_raw(
                        format!(
                            r#"{{"type":"call","invocationId":{id},"callId":1,"request":{{"op":"fs.read","args":{{"path":"/etc/passwd"}}}}}}"#
                        )
                        .as_bytes(),
                    );
                }
                "call-flood" => {
                    for c in 1..=300 {
                        call(id, c, log_request());
                    }
                }
                "result-wrong-id" => result(id + 7, ActionOutcome::ok()),
                "invalid-outcome" => result(id, ActionOutcome::failed("Not A Code", "x")),
                "exit-42" => std::process::exit(42),
                "ignore-shutdown" => {
                    result(id, ActionOutcome::ok());
                    // Ignore shutdown and EOF alike.
                    let mut sink = Vec::new();
                    let _ = io::stdin().lock().read_to_end(&mut sink);
                    sleep_forever()
                }
                "denied-fetch" => {
                    let req = serde_json::from_value(json!({
                        "op": "http.fetch",
                        "args": {"method": "GET", "url": "https://evil.net/"}
                    }))
                    .unwrap();
                    let r = call(id, 1, req);
                    let code = match r {
                        CallResult::Error(e) => serde_json::to_value(e.code).unwrap(),
                        CallResult::Ok(_) => json!("ok"),
                    };
                    let data = Params::from([(
                        "code".into(),
                        ParamValue::String(code.as_str().unwrap_or("").into()),
                    )]);
                    result(id, ActionOutcome::ok_with(data));
                    let _ = next_invoke();
                }
                other => panic!("unknown script {other}"),
            }
            sleep_forever()
        }
    }
    sleep_forever()
}

// -------------------------------------------------------------------- tests

#[derive(Default)]
struct Executor {
    executed: usize,
}

impl CapabilityExecutor for Executor {
    fn execute(&mut self, _plugin: &PluginId, _allowed: &Allowed) -> CallResult {
        self.executed += 1;
        CallResult::Ok(json!(null))
    }
}

fn config(script: &str) -> HostConfig {
    let me = std::env::current_exe().unwrap();
    let mut cfg = HostConfig::new(
        HostCommand::new(me).arg("fake-host").arg(script),
        Arc::from(&b"\0asm"[..]),
    );
    cfg.handshake_timeout = Duration::from_millis(1500);
    cfg.load_timeout = Duration::from_millis(1500);
    cfg.shutdown_grace = Duration::from_millis(300);
    cfg
}

fn broker() -> Broker {
    let b = Broker::new(
        Arc::new(MemoryAudit::new(100)),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    b.set_grants(
        &plugin(),
        &ApprovedPermissions::from_json(
            r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]}]"#,
        )
        .unwrap(),
    )
    .unwrap();
    b
}

fn plugin() -> PluginId {
    PluginId::new("acme.weather").unwrap()
}

fn spawn(script: &str) -> Result<PluginHost, HostError> {
    PluginHost::spawn(&config(script), broker().session(plugin()))
}

fn invoke(script: &str) -> (Result<ActionOutcome, HostError>, Executor, PluginHost) {
    let mut h = spawn(script).unwrap();
    let mut ex = Executor::default();
    let r = h.invoke("a", &Params::new(), Duration::from_secs(5), &mut ex);
    (r, ex, h)
}

fn handshake_failures() {
    let t = Instant::now();
    assert!(matches!(spawn("no-ready"), Err(HostError::Handshake(_))));
    assert!(t.elapsed() < Duration::from_secs(4));
    assert!(matches!(
        spawn("garbage-ready"),
        Err(HostError::Handshake(_))
    ));
    assert!(matches!(spawn("huge-frame"), Err(HostError::Handshake(_))));
    assert!(matches!(spawn("protocol-2"), Err(HostError::Handshake(_))));
    assert!(matches!(
        spawn("result-before-ready"),
        Err(HostError::Handshake(_))
    ));
    assert!(matches!(
        spawn("unsandboxed"),
        Err(HostError::SandboxMissing(_))
    ));
    match spawn("exit-after-ready") {
        Err(HostError::LoadFailed { status, .. }) => {
            assert_eq!(status.and_then(|s| s.code()), Some(7))
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        spawn("fatal-on-load"),
        Err(HostError::LoadFailed { .. })
    ));
}

fn protocol_violations_kill_the_host() {
    for script in [
        "call-wrong-invocation",
        "call-out-of-order",
        "call-plugin-field",
        "call-unknown-op",
        "result-wrong-id",
    ] {
        let (r, ex, h) = invoke(script);
        assert!(
            matches!(r, Err(HostError::Protocol { .. })),
            "{script}: {r:?}"
        );
        assert_eq!(ex.executed, 0, "{script}");
        assert!(!h.is_running(), "{script}");
    }
}

fn call_flood_is_cut_off() {
    let (r, ex, _h) = invoke("call-flood");
    assert!(matches!(r, Err(HostError::Protocol { .. })), "{r:?}");
    // 256 calls were served (the broker throttled most of them), then the
    // host was killed; nothing beyond the cap was executed.
    assert!(ex.executed <= 256);
}

fn invalid_outcomes_are_replaced() {
    let (r, _, h) = invoke("invalid-outcome");
    match r.unwrap() {
        ActionOutcome::Failed { code, .. } => assert_eq!(code, "invalid-result"),
        other => panic!("{other:?}"),
    }
    assert!(h.is_running());
}

fn crash_reports_exit_status() {
    let (r, _, _h) = invoke("exit-42");
    match r {
        Err(HostError::Crashed { status, .. }) => {
            assert_eq!(status.and_then(|s| s.code()), Some(42))
        }
        other => panic!("{other:?}"),
    }
}

fn shutdown_kills_a_host_that_ignores_it() {
    let (r, _, h) = invoke("ignore-shutdown");
    assert!(r.is_ok());
    let t = Instant::now();
    let _ = h.shutdown(Duration::from_millis(300));
    assert!(t.elapsed() < Duration::from_secs(3));
}

fn denied_requests_are_never_executed() {
    let (r, ex, _h) = invoke("denied-fetch");
    let o = r.unwrap();
    let ActionOutcome::Ok { data, .. } = o else {
        panic!()
    };
    assert_eq!(data["code"], ParamValue::String("not-permitted".into()));
    assert_eq!(ex.executed, 0);
}

fn normal_host_works() {
    let (r, _, mut h) = invoke("ok");
    assert_eq!(r.unwrap(), ActionOutcome::ok());
    let r = h.invoke(
        "b",
        &Params::new(),
        Duration::from_secs(5),
        &mut Executor::default(),
    );
    assert_eq!(r.unwrap(), ActionOutcome::ok());
    // a hung host (the fake sleeps now) is killed on timeout
    let t = Instant::now();
    let r = h.invoke(
        "c",
        &Params::new(),
        Duration::from_millis(300),
        &mut Executor::default(),
    );
    assert!(matches!(r, Err(HostError::Timeout { .. })), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(2));
    assert!(!h.is_running());
}

fn invalid_params_are_not_sent() {
    let (_, _, mut h) = invoke("ok");
    let deep: Params = serde_json::from_value(json!({"a": [[[[1]]]]})).unwrap();
    let r = h.invoke("b", &deep, Duration::from_secs(5), &mut Executor::default());
    assert!(matches!(r, Err(HostError::InvalidParams)));
    assert!(h.is_running());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("fake-host") {
        fake(&args[2]);
        return;
    }
    let filter = args.iter().skip(1).find(|a| !a.starts_with('-')).cloned();
    let tests: &[(&str, fn())] = &[
        ("handshake_failures", handshake_failures),
        (
            "protocol_violations_kill_the_host",
            protocol_violations_kill_the_host,
        ),
        ("call_flood_is_cut_off", call_flood_is_cut_off),
        (
            "invalid_outcomes_are_replaced",
            invalid_outcomes_are_replaced,
        ),
        ("crash_reports_exit_status", crash_reports_exit_status),
        (
            "shutdown_kills_a_host_that_ignores_it",
            shutdown_kills_a_host_that_ignores_it,
        ),
        (
            "denied_requests_are_never_executed",
            denied_requests_are_never_executed,
        ),
        ("normal_host_works", normal_host_works),
        ("invalid_params_are_not_sent", invalid_params_are_not_sent),
    ];
    let mut failed = 0;
    let mut ran = 0;
    for (name, test) in tests {
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        ran += 1;
        let ok = catch_unwind(AssertUnwindSafe(test)).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        failed += usize::from(!ok);
    }
    println!(
        "\ntest result: {}. {} passed; {failed} failed",
        if failed == 0 { "ok" } else { "FAILED" },
        ran - failed
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
