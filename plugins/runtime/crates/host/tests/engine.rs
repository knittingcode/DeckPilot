//! In-process tests of the wasm layer: what a module may import and export,
//! resource limits, traps, and hostile use of the guest ABI.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::protocol::GuestLimits;
use deckpilot_plugin_abi::{
    ActionOutcome, CallErrorCode, CallResult, CapabilityRequest, ParamValue, Params,
};
use deckpilot_plugin_host::engine::{
    ChannelError, Fault, HostCallHandler, LoadedPlugin, PluginEngine,
};
use deckpilot_plugin_test_fixtures::TEST_GUEST;
use serde_json::json;

#[derive(Default)]
struct Calls {
    requests: Vec<CapabilityRequest>,
    delay: Duration,
}

struct Recorder(Rc<RefCell<Calls>>);

impl HostCallHandler for Recorder {
    fn call(
        &mut self,
        _inv: u64,
        _call: u64,
        request: CapabilityRequest,
    ) -> Result<CallResult, ChannelError> {
        let delay = self.0.borrow().delay;
        std::thread::sleep(delay);
        self.0.borrow_mut().requests.push(request);
        Ok(CallResult::Ok(json!({"status": 200})))
    }
}

fn load_with(
    bytes: &[u8],
    limits: GuestLimits,
) -> (
    Result<LoadedPlugin, Fault>,
    Rc<RefCell<Calls>>,
    PluginEngine,
) {
    let engine = PluginEngine::new().unwrap();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let plugin = engine.load(bytes, &limits, Box::new(Recorder(calls.clone())));
    (plugin, calls, engine)
}

fn guest() -> (LoadedPlugin, Rc<RefCell<Calls>>, PluginEngine) {
    let (p, c, e) = load_with(TEST_GUEST, GuestLimits::default());
    (p.unwrap(), c, e)
}

fn params(v: serde_json::Value) -> Params {
    serde_json::from_value(v).unwrap()
}

fn data(outcome: &ActionOutcome) -> &Params {
    match outcome {
        ActionOutcome::Ok { data, .. } => data,
        other => panic!("expected ok, got {other:?}"),
    }
}

fn failed_code(outcome: &ActionOutcome) -> &str {
    match outcome {
        ActionOutcome::Failed { code, .. } => code,
        other => panic!("expected failed, got {other:?}"),
    }
}

/// A minimal ABI 1 module in WAT. `invoke` is the body of `dp_invoke`
/// (default: return `{"status":"ok"}` from a data segment).
fn wat(extra: &str, invoke: &str) -> Vec<u8> {
    let invoke = if invoke.is_empty() {
        "i64.const 8796093022223" // (2048 << 32) | 15
    } else {
        invoke
    };
    let text = format!(
        r#"(module
            {extra}
            (memory (export "memory") 1)
            (data (i32.const 2048) "{{\"status\":\"ok\"}}")
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 1024)
            (func (export "dp_invoke") (param i32 i32) (result i64) {invoke}))"#
    );
    wat::parse_str(&text).unwrap()
}

fn load_err(bytes: &[u8]) -> Fault {
    match load_with(bytes, GuestLimits::default()).0 {
        Ok(_) => panic!("module must be rejected"),
        Err(f) => f,
    }
}

#[test]
fn minimal_module_runs() {
    let (p, _, _e) = load_with(&wat("", ""), GuestLimits::default());
    let mut p = p.unwrap();
    assert_eq!(
        p.invoke(1, "a", &Params::new()).unwrap(),
        ActionOutcome::ok()
    );
}

#[test]
fn only_the_abi_import_is_linked() {
    for import in [
        r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#,
        r#"(import "wasi_snapshot_preview1" "path_open" (func (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))"#,
        r#"(import "wasi_snapshot_preview1" "sock_accept" (func (param i32 i32 i32) (result i32)))"#,
        r#"(import "wasi_snapshot_preview1" "environ_get" (func (param i32 i32) (result i32)))"#,
        r#"(import "wasi_snapshot_preview1" "clock_time_get" (func (param i32 i64 i32) (result i32)))"#,
        r#"(import "wasi_snapshot_preview1" "random_get" (func (param i32 i32) (result i32)))"#,
        r#"(import "wasi_snapshot_preview1" "proc_exit" (func (param i32)))"#,
        r#"(import "wasi:filesystem/types" "read" (func))"#,
        r#"(import "env" "system" (func (param i32) (result i32)))"#,
        r#"(import "deckpilot_v1" "get_database_key" (func (result i32)))"#,
        r#"(import "deckpilot_v1" "host_call" (func (param i32) (result i32)))"#,
        r#"(import "deckpilot_v1" "host_call" (func (param i32 i32) (result i32)))"#,
        r#"(import "deckpilot_v1" "host_call" (global i32))"#,
        r#"(import "env" "table" (table 1 funcref))"#,
    ] {
        match load_err(&wat(import, "")) {
            Fault::Load(m) => assert!(m.contains("forbidden import"), "{import}: {m}"),
            other => panic!("{import}: {other:?}"),
        }
    }
    let mem = wat::parse_str(
        r#"(module (import "env" "memory" (memory 1))
            (func (export "dp_abi_version") (result i32) i32.const 1))"#,
    )
    .unwrap();
    assert!(matches!(load_err(&mem), Fault::Load(_)));
}

#[test]
fn required_exports_are_checked() {
    let no_invoke = wat::parse_str(
        r#"(module (memory (export "memory") 1)
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 0))"#,
    )
    .unwrap();
    assert!(matches!(load_err(&no_invoke), Fault::Load(m) if m.contains("dp_invoke")));
    let no_memory = wat::parse_str(
        r#"(module (memory 1)
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 0)
            (func (export "dp_invoke") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    assert!(matches!(load_err(&no_memory), Fault::Load(_)));
    let wrong_type = wat::parse_str(
        r#"(module (memory (export "memory") 1)
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 0)
            (func (export "dp_invoke") (param i32) (result i32) i32.const 0))"#,
    )
    .unwrap();
    assert!(matches!(load_err(&wrong_type), Fault::Load(_)));
    let v2 = wat::parse_str(
        r#"(module (memory (export "memory") 1)
            (func (export "dp_abi_version") (result i32) i32.const 2)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 0)
            (func (export "dp_invoke") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    assert!(matches!(load_err(&v2), Fault::Load(m) if m.contains("version 2")));
}

#[test]
fn malformed_and_oversized_modules_are_rejected() {
    assert!(matches!(load_err(b"not wasm at all"), Fault::Load(_)));
    assert!(matches!(
        load_err(b"\0asm\x01\0\0\0\xff\xff"),
        Fault::Load(_)
    ));
    let mut big = wat("", "");
    big.resize(17 << 20, 0);
    assert!(matches!(load_err(&big), Fault::Load(m) if m.contains("too large")));
}

#[test]
fn structural_limits() {
    // initial memory above the 64 MiB cap (1,100 pages = 68.75 MiB)
    let huge_mem = wat::parse_str(
        r#"(module (memory (export "memory") 1100)
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 0)
            (func (export "dp_invoke") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    assert!(matches!(load_err(&huge_mem), Fault::Load(_)));
    // huge table
    assert!(matches!(
        load_err(&wat("(table 1000000 funcref)", "")),
        Fault::Load(_)
    ));
    // more than one table
    assert!(matches!(
        load_err(&wat("(table 1 funcref) (table 1 funcref)", "")),
        Fault::Load(_)
    ));
    // more than one memory (multi-memory disabled)
    assert!(matches!(load_err(&wat("(memory 1)", "")), Fault::Load(_)));
    // shared memory (threads disabled)
    let shared = wat::parse_str(
        r#"(module (memory (export "memory") 1 1 shared)
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 0)
            (func (export "dp_invoke") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    assert!(matches!(load_err(&shared), Fault::Load(_)));
}

#[test]
fn start_function_runs_under_the_deadline() {
    let looping = wat("(func $s (loop br 0)) (start $s)", "");
    let t = Instant::now();
    assert!(matches!(load_err(&looping), Fault::Deadline));
    assert!(t.elapsed() < Duration::from_secs(3));
}

#[test]
fn memory_growth_stops_at_the_cap() {
    let grow = wat(
        "",
        "(block (loop (br_if 1 (i32.eq (memory.grow (i32.const 1)) (i32.const -1))) (br 0))) i64.const 8796093022223",
    );
    let (p, _, _e) = load_with(&grow, GuestLimits::default());
    let mut p = p.unwrap();
    assert_eq!(
        p.invoke(1, "a", &Params::new()).unwrap(),
        ActionOutcome::ok()
    );
    assert!(p.memory_size() <= 64 << 20, "{}", p.memory_size());
    assert!(p.memory_size() >= 63 << 20);
}

#[test]
fn infinite_loop_hits_the_epoch_deadline() {
    let (p, _, _e) = load_with(&wat("", "(loop br 0) i64.const 0"), GuestLimits::default());
    let mut p = p.unwrap();
    let t = Instant::now();
    assert!(matches!(
        p.invoke(1, "a", &Params::new()),
        Err(Fault::Deadline)
    ));
    let took = t.elapsed();
    assert!(
        took >= Duration::from_millis(240) && took < Duration::from_secs(2),
        "{took:?}"
    );
}

#[test]
fn infinite_loop_runs_out_of_fuel() {
    let limits = GuestLimits {
        deadline_ms: 10_000,
        fuel_per_invocation: 10_000_000,
        ..GuestLimits::default()
    };
    let (p, _, _e) = load_with(&wat("", "(loop br 0) i64.const 0"), limits);
    let mut p = p.unwrap();
    assert!(matches!(
        p.invoke(1, "a", &Params::new()),
        Err(Fault::OutOfFuel)
    ));
}

#[test]
fn fuel_is_per_invocation() {
    let limits = GuestLimits {
        fuel_per_invocation: 5_000_000,
        ..GuestLimits::default()
    };
    let (p, _, _e) = load_with(TEST_GUEST, limits);
    let mut p = p.unwrap();
    for i in 1..=50 {
        assert!(matches!(
            p.invoke(i, "add", &params(json!({"a": 1, "b": 2}))),
            Ok(ActionOutcome::Ok { .. })
        ));
    }
}

#[test]
fn deep_recursion_traps() {
    let (p, _, _e) = load_with(
        &wat("(func $r (call $r))", "call $r i64.const 0"),
        GuestLimits::default(),
    );
    let mut p = p.unwrap();
    assert!(
        matches!(p.invoke(1, "a", &Params::new()), Err(Fault::Trap(m)) if m.contains("StackOverflow"))
    );
}

#[test]
fn hostile_outputs_become_invalid_result() {
    for body in [
        "i64.const -1",              // ptr 0xffffffff, len 0xffffffff
        "i64.const 281470681743360", // ptr 0xffff, len 0 -> fine bounds, empty JSON
        "i64.const 8796093022208",   // ptr 2048, len 0
        "i64.const 8796093022222",   // truncated JSON
        "i64.const 4294967295",      // ptr 0, len 4 GiB
        "i64.const 196609",          // ptr 0, len > MAX_GUEST_OUTPUT
    ] {
        let (p, _, _e) = load_with(&wat("", body), GuestLimits::default());
        let mut p = p.unwrap();
        let o = p.invoke(1, "a", &Params::new()).unwrap();
        assert_eq!(failed_code(&o), "invalid-result", "{body}");
    }
}

#[test]
fn hostile_alloc_is_a_guest_abi_violation() {
    let zero = wat::parse_str(
        r#"(module (memory (export "memory") 1)
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 0)
            (func (export "dp_invoke") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    let (p, _, _e) = load_with(&zero, GuestLimits::default());
    assert!(matches!(
        p.unwrap().invoke(1, "a", &Params::new()),
        Err(Fault::GuestAbi(_))
    ));
    let oob = wat::parse_str(
        r#"(module (memory (export "memory") 1)
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param i32) (result i32) i32.const 65530)
            (func (export "dp_invoke") (param i32 i32) (result i64) i64.const 0))"#,
    )
    .unwrap();
    let (p, _, _e) = load_with(&oob, GuestLimits::default());
    assert!(matches!(
        p.unwrap().invoke(1, "a", &Params::new()),
        Err(Fault::GuestAbi(_))
    ));
}

#[test]
fn host_call_flooding_is_bounded() {
    // A WAT guest that calls host_call with a valid request forever.
    let req = r#"{\"op\":\"log\",\"args\":{\"level\":\"info\",\"message\":\"x\"}}"#;
    let len = req.replace("\\\"", "\"").len();
    let flood = wat(
        &format!(
            r#"(import "deckpilot_v1" "host_call" (func $hc (param i32 i32) (result i64)))
               (data (i32.const 4096) "{req}")"#
        ),
        &format!("(loop (drop (call $hc (i32.const 4096) (i32.const {len}))) (br 0)) i64.const 0"),
    );
    let (p, calls, _e) = load_with(&flood, GuestLimits::default());
    let mut p = p.unwrap();
    assert!(
        matches!(p.invoke(1, "a", &Params::new()), Err(Fault::GuestAbi(m)) if m.contains("too many"))
    );
    assert_eq!(calls.borrow().requests.len(), 256);
}

#[test]
fn guest_does_normal_work() {
    let (mut p, calls, _e) = guest();
    let o = p
        .invoke(1, "add", &params(json!({"a": 40, "b": 2})))
        .unwrap();
    assert_eq!(data(&o)["sum"], ParamValue::Integer(42));
    let echo = params(json!({"s": "ü", "l": [1, 2.5, true], "n": -3}));
    assert_eq!(data(&p.invoke(2, "echo", &echo).unwrap()), &echo);
    // one instance for the life of the plugin: state persists
    for i in 1..=3 {
        let o = p.invoke(10 + i, "count", &Params::new()).unwrap();
        assert_eq!(data(&o)["n"], ParamValue::Integer(i as i64));
    }
    assert!(matches!(
        p.invoke(20, "skip", &Params::new()).unwrap(),
        ActionOutcome::Skipped { .. }
    ));
    assert_eq!(
        failed_code(&p.invoke(21, "fail", &Params::new()).unwrap()),
        "bad-input"
    );
    let o = p
        .invoke(
            22,
            "fetch",
            &params(json!({"url": "https://api.example.com/"})),
        )
        .unwrap();
    assert_eq!(data(&o)["ok"], ParamValue::Bool(true));
    assert_eq!(calls.borrow().requests.len(), 1);
    assert_eq!(calls.borrow().requests[0].op(), "http.fetch");
}

#[test]
fn rust_std_has_no_ambient_authority_in_the_guest() {
    let (mut p, calls, _e) = guest();
    let o = p.invoke(1, "std_probes", &Params::new());
    // Some std functions trap on wasm32-unknown-unknown instead of failing;
    // either way nothing reaches the host.
    if let Ok(o) = o {
        for (k, v) in data(&o) {
            assert_eq!(
                v,
                &ParamValue::Bool(false),
                "{k} succeeded inside the guest"
            );
        }
    }
    assert!(calls.borrow().requests.is_empty());
}

#[test]
fn guest_traps_are_faults() {
    for action in ["panic", "unreachable", "oob", "memory_bomb", "recurse"] {
        let (mut p, _, _e) = guest();
        match p.invoke(1, action, &Params::new()) {
            Err(Fault::Trap(_)) => {}
            other => panic!("{action}: {other:?}"),
        }
    }
    let (mut p, _, _e) = guest();
    assert!(matches!(
        p.invoke(1, "spin", &Params::new()),
        Err(Fault::Deadline)
    ));
    let (mut p, _, _e) = guest();
    let o = p.invoke(1, "grow_raw", &Params::new()).unwrap();
    assert!(matches!(data(&o)["pages"], ParamValue::Integer(n) if n <= 1024));
}

#[test]
fn hostile_host_calls_are_guest_abi_violations() {
    for action in [
        "host_call_bad_pointer",
        "host_call_huge_len",
        "host_call_negative",
        "host_call_oversized",
        "spin_with_calls",
    ] {
        let (mut p, calls, _e) = guest();
        match p.invoke(1, action, &Params::new()) {
            Err(Fault::GuestAbi(_)) => {}
            other => panic!("{action}: {other:?}"),
        }
        if action != "spin_with_calls" {
            assert!(calls.borrow().requests.is_empty(), "{action}");
        }
    }
}

#[test]
fn malformed_requests_get_errors_and_never_reach_the_handler() {
    let (mut p, calls, _e) = guest();
    for (json, code) in [
        (
            r#"{"op":"fs.read","args":{"path":"/etc/passwd"}}"#,
            CallErrorCode::UnsupportedOp,
        ),
        (
            r#"{"op":"process.exec","args":{"cmd":"sh"}}"#,
            CallErrorCode::UnsupportedOp,
        ),
        (
            r#"{"op":"keystore.getDatabaseKey","args":{}}"#,
            CallErrorCode::UnsupportedOp,
        ),
        (
            r#"{"op":"http.fetch","args":{"method":"GET","url":"https://a.example.com/","plugin":"other.plugin"}}"#,
            CallErrorCode::BadRequest,
        ),
        (
            r#"{"op":"log","args":{"level":"info","message":"m"},"pluginId":"x"}"#,
            CallErrorCode::BadRequest,
        ),
        ("not json", CallErrorCode::BadRequest),
        ("", CallErrorCode::BadRequest),
        (
            r#"{"op":"log","args":{"level":"loud","message":"m"}}"#,
            CallErrorCode::BadRequest,
        ),
    ] {
        let o = p
            .invoke(1, "raw_request", &params(json!({"json": json})))
            .unwrap();
        let ParamValue::String(reply) = &data(&o)["reply"] else {
            panic!()
        };
        let r: CallResult = serde_json::from_str(reply).unwrap();
        match r {
            CallResult::Error(e) => assert_eq!(e.code, code, "{json}"),
            other => panic!("{json}: {other:?}"),
        }
    }
    let long = format!(
        r#"{{"op":"log","args":{{"level":"info","message":"{}"}}}}"#,
        "x".repeat(3000)
    );
    let o = p
        .invoke(2, "raw_request", &params(json!({"json": long})))
        .unwrap();
    assert!(matches!(&data(&o)["reply"], ParamValue::String(s) if s.contains("bad-request")));
    assert!(calls.borrow().requests.is_empty());
}

#[test]
fn oversized_or_invalid_results_are_reported() {
    let (mut p, _, _e) = guest();
    assert_eq!(
        failed_code(&p.invoke(1, "big_output", &Params::new()).unwrap()),
        "invalid-result"
    );
    assert_eq!(
        failed_code(&p.invoke(2, "bad_code", &Params::new()).unwrap()),
        "invalid-result"
    );
    // and the plugin keeps working afterwards (no trap happened)
    assert!(matches!(
        p.invoke(3, "add", &Params::new()).unwrap(),
        ActionOutcome::Ok { .. }
    ));
}

#[test]
fn invalid_params_are_refused_before_entering_the_guest() {
    let (mut p, _, _e) = guest();
    assert_eq!(
        failed_code(&p.invoke(1, "bad name!", &Params::new()).unwrap()),
        "invalid-params"
    );
    let deep = params(json!({"a": [[[[1]]]]}));
    assert_eq!(
        failed_code(&p.invoke(2, "echo", &deep).unwrap()),
        "invalid-params"
    );
}

#[test]
fn time_in_the_host_does_not_count_against_the_deadline() {
    let (mut p, calls, _e) = guest();
    calls.borrow_mut().delay = Duration::from_millis(400);
    let o = p
        .invoke(
            1,
            "fetch",
            &params(json!({"url": "https://api.example.com/"})),
        )
        .unwrap();
    assert_eq!(data(&o)["ok"], ParamValue::Bool(true));
    // several slow calls in a row (deadline reset before each re-entry)
    calls.borrow_mut().delay = Duration::from_millis(150);
    let o = p.invoke(2, "log_n", &params(json!({"n": 4}))).unwrap();
    assert_eq!(data(&o)["ok"], ParamValue::Integer(4));
}
