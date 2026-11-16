//! Feasibility probe for the plugin host under the macOS App Sandbox and the
//! hardened runtime (docs/distribution/capability-matrix.md, sections 2.6
//! and 2.7).
//!
//!   probe parent <child executable> <label>   report, spawn the child, relay its report
//!   probe child <label>                       report, JIT, sandbox_init, JIT again
//!
//! The child does what the plugin host does at start-up
//! (plugins/runtime/crates/host): it builds a Cranelift wasmtime engine,
//! compiles and runs a module (JIT), then confines itself with
//! `sandbox_init` and the host's deny-default profile, and compiles and
//! runs a module again. run.sh signs the child with different entitlements
//! and starts it directly or from a sandboxed parent.

use std::ffi::{CStr, CString, c_char, c_int};
use std::process::{Command, ExitCode};

use wasmtime::{Config, Engine, Instance, Module, OptLevel, Store};

unsafe extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// The plugin host's profile (plugins/runtime/crates/host/src/sandbox/macos.rs).
const PROFILE: &str = "(version 1)\n(deny default)\n";

/// `(module (func (export "add") (param i32 i32) (result i32)
///   local.get 0 local.get 1 i32.add))`, encoded by hand so the probe needs
/// no text-format parser.
const ADD_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic, version
    0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f, // type: (i32 i32) -> i32
    0x03, 0x02, 0x01, 0x00, // function 0 has type 0
    0x07, 0x07, 0x01, 0x03, b'a', b'd', b'd', 0x00, 0x00, // export "add"
    0x0a, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b, // code
];

fn sandbox_lines(role: &str) {
    let container = std::env::var("APP_SANDBOX_CONTAINER_ID").ok();
    println!(
        "{role}.app_sandbox_container_id = {}",
        container.as_deref().unwrap_or("(not set)")
    );
    // The real ~/Library/Preferences is outside every container (a TCC-free
    // control that the App Sandbox is enforced).
    // SAFETY: getpwuid returns static storage or null; read at once.
    let home = unsafe {
        let entry = libc::getpwuid(libc::getuid());
        (!entry.is_null() && !(*entry).pw_dir.is_null()).then(|| {
            CStr::from_ptr((*entry).pw_dir)
                .to_string_lossy()
                .into_owned()
        })
    };
    if let Some(home) = home {
        let result = match std::fs::read_dir(format!("{home}/Library/Preferences")) {
            Ok(_) => "readable".to_owned(),
            Err(e) => format!("denied ({e})"),
        };
        println!("{role}.read_dir(~/Library/Preferences) = {result}");
    }
}

fn engine() -> Result<Engine, String> {
    let mut config = Config::new();
    // The host's settings that affect code generation and memory
    // (plugins/runtime/crates/host/src/engine.rs).
    config
        .consume_fuel(true)
        .memory_init_cow(false)
        .cranelift_opt_level(OptLevel::Speed);
    Engine::new(&config).map_err(|e| format!("{e:#}"))
}

fn jit(engine: &Engine) -> String {
    let run = || -> Result<i32, String> {
        let module = Module::new(engine, ADD_WASM).map_err(|e| format!("compile: {e:#}"))?;
        let mut store = Store::new(engine, ());
        store.set_fuel(10_000).map_err(|e| format!("fuel: {e:#}"))?;
        let instance =
            Instance::new(&mut store, &module, &[]).map_err(|e| format!("instantiate: {e:#}"))?;
        let add = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "add")
            .map_err(|e| format!("export: {e:#}"))?;
        add.call(&mut store, (40, 2))
            .map_err(|e| format!("call: {e:#}"))
    };
    match run() {
        Ok(42) => "ok (add(40, 2) = 42)".into(),
        Ok(other) => format!("wrong result {other}"),
        Err(e) => format!("failed ({e})"),
    }
}

fn apply_sandbox() -> Result<(), String> {
    let profile = CString::new(PROFILE).expect("profile has no NUL");
    let mut err: *mut c_char = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated profile and out-pointer.
    let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut err) };
    if rc == 0 {
        return Ok(());
    }
    if err.is_null() {
        return Err(format!("rc {rc}, no message"));
    }
    // SAFETY: sandbox_init returned a C string; freed right after.
    let message = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { sandbox_free_error(err) };
    Err(format!("rc {rc}: {message}"))
}

fn child(label: &str) -> ExitCode {
    println!("child.label = {label}");
    println!("child.pid = {}", std::process::id());
    sandbox_lines("child");
    let engine = match engine() {
        Ok(engine) => engine,
        Err(e) => {
            println!("child.engine = failed ({e})");
            println!("end");
            return ExitCode::FAILURE;
        }
    };
    println!("child.engine = ok");
    println!("child.jit_before_sandbox_init = {}", jit(&engine));
    match apply_sandbox() {
        Ok(()) => {
            println!("child.sandbox_init(deny default) = ok");
            let probe = match std::fs::read_dir("/") {
                Ok(_) => "readable".into(),
                Err(e) => format!("denied ({e})"),
            };
            println!("child.after_sandbox_init.read_dir(/) = {probe}");
            println!("child.jit_after_sandbox_init = {}", jit(&engine));
        }
        Err(e) => println!("child.sandbox_init(deny default) = failed ({e})"),
    }
    println!("end");
    ExitCode::SUCCESS
}

fn parent(child_path: &str, label: &str) -> ExitCode {
    println!("parent.label = {label}");
    sandbox_lines("parent");
    match Command::new(child_path).args(["child", label]).output() {
        Ok(output) => {
            println!("parent.spawn = ok, child exit status {}", output.status);
            print!("{}", String::from_utf8_lossy(&output.stdout));
            let stderr = String::from_utf8_lossy(&output.stderr);
            for line in stderr.lines().take(20) {
                println!("child.stderr: {line}");
            }
        }
        Err(e) => println!("parent.spawn = failed ({e})"),
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("child") => child(args.get(2).map_or("unlabelled", String::as_str)),
        Some("parent") if args.len() >= 4 => parent(&args[2], &args[3]),
        _ => {
            eprintln!("usage: probe parent <child> <label> | probe child <label>");
            ExitCode::from(2)
        }
    }
}
