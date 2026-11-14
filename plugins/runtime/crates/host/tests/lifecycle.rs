//! The optional ABI 1 exports `dp_describe`, `dp_init` and `dp_shutdown`:
//! absent is fine (original ABI 1 modules), present ones run under the
//! invocation limits, and host calls inside them never reach the handler.

use std::cell::RefCell;
use std::rc::Rc;

use deckpilot_plugin_abi::protocol::GuestLimits;
use deckpilot_plugin_abi::{ActionOutcome, CallResult, CapabilityRequest, Params};
use deckpilot_plugin_host::engine::{
    ChannelError, Fault, HostCallHandler, LoadedPlugin, PluginEngine,
};
use serde_json::json;

struct Recorder(Rc<RefCell<Vec<CapabilityRequest>>>);

impl HostCallHandler for Recorder {
    fn call(
        &mut self,
        _inv: u64,
        _call: u64,
        request: CapabilityRequest,
    ) -> Result<CallResult, ChannelError> {
        self.0.borrow_mut().push(request);
        Ok(CallResult::Ok(json!(null)))
    }
}

type Loaded = (
    Result<LoadedPlugin, Fault>,
    Rc<RefCell<Vec<CapabilityRequest>>>,
    PluginEngine,
);

fn load(bytes: &[u8]) -> Loaded {
    let engine = PluginEngine::new().unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let p = engine.load(
        bytes,
        &GuestLimits::default(),
        Box::new(Recorder(calls.clone())),
    );
    (p, calls, engine)
}

fn load_err(bytes: &[u8]) -> Fault {
    match load(bytes).0 {
        Ok(p) => panic!("module must be rejected: {p:?}"),
        Err(f) => f,
    }
}

const DESCRIPTOR: &str = r#"{"sdk":"wat/1","apiLevel":"1.0","actions":[{"id":"greet","params":[{"key":"name","required":true,"kind":{"type":"text"}}]}]}"#;
const LOG: &str = r#"{"op":"log","args":{"level":"info","message":"from a hook"}}"#;

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// An ABI 1 module with `extra` exports. Data: the descriptor at 4096, a
/// log request at 8192, `{"status":"ok"}` at 2048.
fn module(extra: &str) -> Vec<u8> {
    module_with(extra, "")
}

/// Like [`module`], with `invoke` run at the start of `dp_invoke`.
fn module_with(extra: &str, invoke: &str) -> Vec<u8> {
    let text = format!(
        r#"(module
            (import "deckpilot_v1" "host_call" (func $host_call (param i32 i32) (result i64)))
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 16384))
            (data (i32.const 2048) "{{\"status\":\"ok\"}}")
            (data (i32.const 4096) "{descriptor}")
            (data (i32.const 8192) "{log}")
            (func (export "dp_abi_version") (result i32) i32.const 1)
            (func (export "dp_alloc") (param $n i32) (result i32)
                (local $p i32)
                global.get $bump
                local.set $p
                global.get $bump
                local.get $n
                i32.add
                global.set $bump
                local.get $p)
            (func (export "dp_invoke") (param i32 i32) (result i64)
                {invoke}
                i64.const 8796093022223)
            (func $log
                i32.const 8192
                i32.const {log_len}
                call $host_call
                drop)
            {extra})"#,
        descriptor = escape(DESCRIPTOR),
        log = escape(LOG),
        log_len = LOG.len(),
    );
    wat::parse_str(&text).unwrap()
}

fn describe_returning(ptr: u64, len: u64) -> String {
    format!(
        r#"(func (export "dp_describe") (result i64) i64.const {})"#,
        (ptr << 32) | len
    )
}

#[test]
fn modules_without_optional_exports_load_unchanged() {
    let (p, calls, _e) = load(&module(""));
    let mut p = p.unwrap();
    assert!(p.descriptor().is_none());
    assert_eq!(
        p.invoke(1, "a", &Params::new()).unwrap(),
        ActionOutcome::ok()
    );
    p.shutdown().unwrap();
    assert!(calls.borrow().is_empty());
}

#[test]
fn descriptor_is_read_and_validated() {
    let (p, _, _e) = load(&module(&describe_returning(4096, DESCRIPTOR.len() as u64)));
    let p = p.unwrap();
    let d = p.descriptor().unwrap();
    assert_eq!(d.sdk, "wat/1");
    assert_eq!(d.actions[0].id, "greet");

    for (extra, what) in [
        // Truncated JSON.
        (describe_returning(4096, 10), "does not parse"),
        // Not a descriptor (the action outcome).
        (describe_returning(2048, 15), "does not parse"),
        (describe_returning(65_000, 4096), "out of bounds"),
        (describe_returning(0, 64 * 1024 + 1), "too large"),
    ] {
        match load_err(&module(&extra)) {
            Fault::Load(m) => assert!(m.contains(what), "{extra}: {m}"),
            other => panic!("{extra}: {other:?}"),
        }
    }
}

#[test]
fn init_runs_once_and_can_refuse_the_load() {
    let (p, calls, _e) = load(&module(
        r#"(func (export "dp_init") (result i32) call $log i32.const 0)"#,
    ));
    p.unwrap();
    // The host call inside the hook was answered locally.
    assert!(calls.borrow().is_empty());

    match load_err(&module(
        r#"(func (export "dp_init") (result i32) i32.const 7)"#,
    )) {
        Fault::Load(m) => assert_eq!(m, "dp_init returned 7"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        load_err(&module(
            r#"(func (export "dp_init") (result i32) unreachable)"#
        )),
        Fault::Trap(_)
    ));
    assert!(matches!(
        load_err(&module(
            r#"(func (export "dp_init") (result i32) (loop $l br $l) i32.const 0)"#
        )),
        Fault::Deadline | Fault::OutOfFuel
    ));
}

#[test]
fn shutdown_runs_under_the_limits() {
    let (p, calls, _e) = load(&module(r#"(func (export "dp_shutdown") call $log)"#));
    let mut p = p.unwrap();
    p.shutdown().unwrap();
    // Only once.
    p.shutdown().unwrap();
    assert!(calls.borrow().is_empty());

    let (p, _, _e) = load(&module(r#"(func (export "dp_shutdown") unreachable)"#));
    assert!(matches!(p.unwrap().shutdown(), Err(Fault::Trap(_))));
    let (p, _, _e) = load(&module(r#"(func (export "dp_shutdown") (loop $l br $l))"#));
    assert!(matches!(
        p.unwrap().shutdown(),
        Err(Fault::Deadline | Fault::OutOfFuel)
    ));
}

#[test]
fn host_calls_inside_invocations_still_reach_the_handler() {
    let (p, calls, _e) = load(&module_with(
        r#"(func (export "dp_init") (result i32) call $log i32.const 0)
           (func (export "dp_shutdown") call $log)"#,
        "call $log",
    ));
    let mut p = p.unwrap();
    assert!(calls.borrow().is_empty());
    // The lifecycle flag is off again after dp_init.
    p.invoke(1, "a", &Params::new()).unwrap();
    assert_eq!(calls.borrow().len(), 1);
    p.shutdown().unwrap();
    assert_eq!(calls.borrow().len(), 1);
    p.invoke(2, "a", &Params::new()).unwrap();
    assert_eq!(calls.borrow().len(), 2);
}

#[test]
fn optional_exports_must_have_the_right_type() {
    for extra in [
        r#"(func (export "dp_init") (result i64) i64.const 0)"#,
        r#"(func (export "dp_init") (param i32) (result i32) i32.const 0)"#,
        r#"(func (export "dp_describe") (result i32) i32.const 0)"#,
        r#"(func (export "dp_shutdown") (result i32) i32.const 0)"#,
        r#"(global (export "dp_describe") i64 (i64.const 0))"#,
    ] {
        match load_err(&module(extra)) {
            Fault::Load(m) => assert!(m.contains("export dp_"), "{extra}: {m}"),
            other => panic!("{extra}: {other:?}"),
        }
    }
}
