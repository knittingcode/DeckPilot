//! Spike 1: wasm isolation of the guest, in process (no IPC, no OS sandbox).
//! Platform independent.

use std::time::{Duration, Instant};

use runtime_spike::engine::{EpochTicker, GUEST_WASM, Limits, Runtime, Trap, engine, trap_of};
use wasmtime::Module;

fn noop() -> runtime_spike::engine::Handler {
    Box::new(|_| b"{}".to_vec())
}

fn rt() -> Runtime {
    Runtime::new(GUEST_WASM).unwrap()
}

fn no_epoch() -> Limits {
    Limits {
        epoch_ticks: None,
        ..Limits::default()
    }
}

#[test]
fn guest_imports_only_host_call() {
    let rt = rt();
    let imports: Vec<(String, String)> = rt
        .module
        .imports()
        .map(|i| (i.module().to_owned(), i.name().to_owned()))
        .collect();
    assert_eq!(
        imports,
        vec![("deckpilot".to_owned(), "host_call".to_owned())]
    );
}

#[test]
fn guest_std_has_no_ambient_authority() {
    // Each probe either returns 0 (std reported an error) or traps (std
    // panicked: not implemented). Neither reaches the host's resources.
    let rt = rt();
    for probe in [
        "probe_fs_read",
        "probe_fs_write",
        "probe_read_dir",
        "probe_env",
        "probe_args",
        "probe_tcp",
        "probe_clock",
        "probe_spawn",
    ] {
        let mut p = rt.instantiate(&no_epoch(), noop()).unwrap();
        match p.call::<(), u32>(probe, ()) {
            Ok(v) => {
                assert_eq!(v, 0, "{probe} reached ambient authority");
                eprintln!("{probe}: std returned an error");
            }
            Err(e) => {
                assert!(trap_of(&e).is_some(), "{probe}: {e:#}");
                eprintln!("{probe}: trapped ({:?})", trap_of(&e));
            }
        }
    }
}

#[test]
fn modules_importing_wasi_do_not_instantiate() {
    // The host links no WASI, so a module asking for files, sockets,
    // environment, clocks, randomness or exit cannot even start.
    let wasi = [
        ("fd_write", "(param i32 i32 i32 i32) (result i32)"),
        (
            "path_open",
            "(param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)",
        ),
        ("sock_accept", "(param i32 i32 i32) (result i32)"),
        ("environ_get", "(param i32 i32) (result i32)"),
        ("clock_time_get", "(param i32 i64 i32) (result i32)"),
        ("random_get", "(param i32 i32) (result i32)"),
        ("proc_exit", "(param i32)"),
    ];
    let engine = engine().unwrap();
    for (name, sig) in wasi {
        let wat = format!(
            r#"(module (import "wasi_snapshot_preview1" "{name}" (func {sig})) (memory (export "memory") 1))"#
        );
        let module = Module::new(&engine, &wat).unwrap();
        let rt = Runtime::with_module(engine.clone(), module).unwrap();
        let err = rt
            .instantiate(&no_epoch(), noop())
            .err()
            .expect("must not instantiate");
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown import"), "{name}: {msg}");
    }
}

#[test]
fn fuel_stops_infinite_loop() {
    let limits = Limits {
        fuel: Some(50_000_000),
        epoch_ticks: None,
        ..Limits::default()
    };
    let mut p = rt().instantiate(&limits, noop()).unwrap();
    let t = Instant::now();
    let err = p.call::<(), i32>("spin", ()).unwrap_err();
    assert_eq!(trap_of(&err), Some(Trap::OutOfFuel));
    eprintln!("fuel: 50M units exhausted after {:?}", t.elapsed());
}

#[test]
fn epoch_deadline_stops_infinite_loop() {
    let rt = rt();
    let _ticker = EpochTicker::start(&rt.engine);
    let mut p = rt.instantiate(&Limits::default(), noop()).unwrap(); // 25 ticks of 10 ms
    let t = Instant::now();
    let err = p.call::<(), i32>("spin", ()).unwrap_err();
    let took = t.elapsed();
    assert_eq!(trap_of(&err), Some(Trap::Interrupt));
    assert!(
        took >= Duration::from_millis(200) && took < Duration::from_secs(3),
        "{took:?}"
    );
    eprintln!("epoch: spin interrupted after {took:?}");
}

#[test]
fn epoch_deadline_resets_while_host_works() {
    // 400 ms spent in the host (e.g. waiting on the broker) is longer than
    // the 250 ms deadline but must not count against the guest.
    let rt = rt();
    let _ticker = EpochTicker::start(&rt.engine);
    let handler = Box::new(|_: &[u8]| {
        std::thread::sleep(Duration::from_millis(400));
        br#"{"ok":1}"#.to_vec()
    });
    let mut p = rt.instantiate(&Limits::default(), handler).unwrap();
    let reply = p.relay(br#"{"op":"x"}"#).unwrap();
    assert_eq!(reply, br#"{"ok":1}"#);
}

#[test]
fn memory_bomb_stopped_by_store_limits() {
    let mut p = rt().instantiate(&no_epoch(), noop()).unwrap();
    let err = p.call::<(), u32>("memory_bomb", ()).unwrap_err();
    assert!(trap_of(&err).is_some(), "{err:#}");
    let size = p.memory_bytes();
    assert!(size <= 64 << 20, "memory grew to {size}");
    eprintln!(
        "memory bomb: trapped ({:?}) at {} MiB",
        trap_of(&err),
        size >> 20
    );
}

#[test]
fn raw_memory_grow_is_capped() {
    let mut p = rt().instantiate(&no_epoch(), noop()).unwrap();
    let pages: u32 = p.call("grow_raw", ()).unwrap();
    let bytes = pages as usize * 65536;
    assert!(
        bytes <= 64 << 20 && bytes > (64 << 20) - 16 * 65536,
        "{bytes}"
    );
}

#[test]
fn declared_memory_above_limit_does_not_instantiate() {
    let engine = engine().unwrap();
    let module = Module::new(&engine, r#"(module (memory (export "memory") 2000))"#).unwrap();
    let rt = Runtime::with_module(engine, module).unwrap();
    assert!(rt.instantiate(&no_epoch(), noop()).is_err());
}

#[test]
fn traps_do_not_crash_the_host() {
    let rt = rt();
    type Case = Box<dyn Fn(&mut runtime_spike::engine::Plugin) -> wasmtime::Result<()>>;
    let cases: [(&str, Case); 5] = [
        (
            "trap_panic",
            Box::new(|p| p.call::<(), ()>("trap_panic", ())),
        ),
        (
            "trap_unreachable",
            Box::new(|p| p.call::<(), ()>("trap_unreachable", ())),
        ),
        (
            "trap_oob",
            Box::new(|p| p.call::<(), i32>("trap_oob", ()).map(|_| ())),
        ),
        (
            "trap_stack_overflow",
            Box::new(|p| p.call::<u32, u32>("trap_stack_overflow", 0).map(|_| ())),
        ),
        (
            "memory_bomb",
            Box::new(|p| p.call::<(), u32>("memory_bomb", ()).map(|_| ())),
        ),
    ];
    for (name, f) in cases {
        let mut p = rt.instantiate(&no_epoch(), noop()).unwrap();
        let err = f(&mut p).unwrap_err();
        let trap = trap_of(&err);
        assert!(trap.is_some(), "{name}: not a trap: {err:#}");
        eprintln!("{name}: {trap:?}");
        // The Store is discarded; a fresh one works in the same process.
        drop(p);
        let mut q = rt.instantiate(&no_epoch(), noop()).unwrap();
        assert_eq!(q.call::<(i32, i32), i32>("add", (2, 40)).unwrap(), 42);
    }
}

#[test]
fn host_call_rejects_out_of_bounds_pointers() {
    // A hostile guest passes a pointer outside its memory. The host must
    // return an error (which surfaces as a failed call), not read host memory.
    let wat = r#"(module
        (import "deckpilot" "host_call" (func $hc (param i32 i32) (result i64)))
        (memory (export "memory") 1)
        (func (export "guest_alloc") (param i32) (result i32) i32.const 0)
        (func (export "evil") (result i64) (call $hc (i32.const 0xFFFFFFF0) (i32.const 64)))
        (func (export "evil_len") (result i64) (call $hc (i32.const 0) (i32.const 0x7FFFFFFF))))"#;
    let engine = engine().unwrap();
    let module = Module::new(&engine, wat).unwrap();
    let rt = Runtime::with_module(engine, module).unwrap();
    let called = std::rc::Rc::new(std::cell::Cell::new(false));
    let c = called.clone();
    let mut p = rt
        .instantiate(
            &no_epoch(),
            Box::new(move |_| {
                c.set(true);
                b"{}".to_vec()
            }),
        )
        .unwrap();
    assert!(p.call::<(), i64>("evil", ()).is_err());
    let mut p2 = rt.instantiate(&no_epoch(), noop()).unwrap();
    assert!(p2.call::<(), i64>("evil_len", ()).is_err());
    assert!(
        !called.get(),
        "handler must not see an out-of-bounds request"
    );
}
