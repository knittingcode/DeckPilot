//! Test guest for the plugin runtime. Each action is either a normal plugin
//! behaviour or a deliberate attack on the host.

use core::hint::black_box;

use deckpilot_plugin_guest::__rt::raw_host_call;
use deckpilot_plugin_guest::{
    ActionOutcome, CallResult, Context, HttpMethod, ParamValue, Params, export_plugin,
};

export_plugin!(handle);

static mut COUNTER: i64 = 0;

fn int(params: &Params, key: &str) -> i64 {
    match params.get(key) {
        Some(ParamValue::Integer(i)) => *i,
        _ => 0,
    }
}

fn string<'a>(params: &'a Params, key: &str) -> &'a str {
    match params.get(key) {
        Some(ParamValue::String(s)) => s,
        _ => "",
    }
}

fn result_data(r: &CallResult) -> Params {
    let mut data = Params::new();
    match r {
        CallResult::Ok(v) => {
            data.insert("ok".into(), ParamValue::Bool(true));
            data.insert("value".into(), ParamValue::String(v.to_string()));
        }
        CallResult::Error(e) => {
            data.insert("ok".into(), ParamValue::Bool(false));
            let code = serde_json::to_value(e.code).unwrap_or_default();
            data.insert(
                "code".into(),
                ParamValue::String(code.as_str().unwrap_or("").into()),
            );
        }
    }
    data
}

/// Sends raw request bytes through the import; returns the reply text.
fn raw_call(bytes: &[u8]) -> String {
    // SAFETY: a plain import call with a valid buffer.
    let packed =
        unsafe { raw_host_call(bytes.as_ptr() as usize as i32, bytes.len() as i32) } as u64;
    let (ptr, len) = ((packed >> 32) as usize, (packed & 0xffff_ffff) as usize);
    // SAFETY: the host wrote `len` bytes at `ptr` (from dp_alloc).
    let reply = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    String::from_utf8_lossy(reply).into_owned()
}

fn raw(reply: String) -> ActionOutcome {
    ActionOutcome::ok_with(Params::from([("reply".into(), ParamValue::String(reply))]))
}

#[allow(unconditional_recursion)]
fn recurse(n: u64) -> u64 {
    let buf = black_box([n; 32]);
    black_box(recurse(black_box(n + 1))).wrapping_add(buf[1])
}

fn handle(ctx: &mut Context, action: &str, params: &Params) -> ActionOutcome {
    match action {
        // --- normal behaviour ---
        "echo" => ActionOutcome::ok_with(params.clone()),
        "add" => ActionOutcome::ok_with(Params::from([(
            "sum".into(),
            ParamValue::Integer(int(params, "a").wrapping_add(int(params, "b"))),
        )])),
        "count" => {
            // SAFETY: wasm32 guests are single-threaded.
            let n = unsafe {
                COUNTER += 1;
                COUNTER
            };
            ActionOutcome::ok_with(Params::from([("n".into(), ParamValue::Integer(n))]))
        }
        "skip" => ActionOutcome::Skipped {
            message: Some("nothing to do".into()),
        },
        "fail" => ActionOutcome::failed("bad-input", "failed on purpose"),
        "fetch" => {
            let r = ctx.fetch(HttpMethod::Get, string(params, "url"));
            ActionOutcome::ok_with(result_data(&r))
        }
        "log_n" => {
            let n = int(params, "n");
            let (mut ok, mut err) = (0, 0);
            for i in 0..n {
                match ctx.log_info(&format!("line {i}")) {
                    CallResult::Ok(_) => ok += 1,
                    CallResult::Error(_) => err += 1,
                }
            }
            ActionOutcome::ok_with(Params::from([
                ("ok".into(), ParamValue::Integer(ok)),
                ("err".into(), ParamValue::Integer(err)),
            ]))
        }
        "std_probes" => {
            // Rust std on wasm32-unknown-unknown: all of these must fail.
            let flags = [
                ("fs_read", std::fs::read("/etc/passwd").is_ok()),
                (
                    "fs_write",
                    std::fs::write("/tmp/deckpilot-guest", b"x").is_ok(),
                ),
                ("read_dir", std::fs::read_dir("/").is_ok()),
                ("tcp", std::net::TcpStream::connect("127.0.0.1:80").is_ok()),
                (
                    "spawn",
                    std::process::Command::new("/bin/sh").output().is_ok(),
                ),
                ("env", std::env::var("HOME").is_ok()),
            ];
            ActionOutcome::ok_with(
                flags
                    .into_iter()
                    .map(|(k, v)| (k.into(), ParamValue::Bool(v)))
                    .collect(),
            )
        }
        // --- misbehaviour: resources ---
        "spin" => loop {
            black_box(());
        },
        "spin_with_calls" => loop {
            black_box(ctx.log_info("x"));
        },
        "memory_bomb" => {
            let mut chunks: Vec<Vec<u8>> = Vec::new();
            loop {
                chunks.push(black_box(vec![1u8; 1 << 20]));
            }
        }
        "grow_raw" => {
            while core::arch::wasm32::memory_grow(0, 16) != usize::MAX {}
            let pages = core::arch::wasm32::memory_size(0) as i64;
            ActionOutcome::ok_with(Params::from([("pages".into(), ParamValue::Integer(pages))]))
        }
        "recurse" => ActionOutcome::ok_with(Params::from([(
            "n".into(),
            ParamValue::Integer(recurse(0) as i64),
        )])),
        "sleep_in_host" => {
            // A host call that the test's executor answers slowly.
            let r = ctx.fetch(HttpMethod::Get, "https://slow.example.com/");
            ActionOutcome::ok_with(result_data(&r))
        }
        // --- misbehaviour: traps ---
        "panic" => panic!("guest panic"),
        "unreachable" => core::arch::wasm32::unreachable(),
        "oob" => {
            // SAFETY: deliberately invalid; wasm bounds checks trap.
            let v = unsafe { core::ptr::read_volatile(black_box(0xFFFF_FFF0usize) as *const i32) };
            ActionOutcome::ok_with(Params::from([("v".into(), ParamValue::Integer(v.into()))]))
        }
        // --- misbehaviour: guest ABI ---
        "host_call_bad_pointer" => {
            // SAFETY: deliberately invalid buffer; the host must refuse it.
            let _ = unsafe { raw_host_call(0x7FFF_FF00, 4096) };
            ActionOutcome::ok()
        }
        "host_call_huge_len" => {
            // SAFETY: deliberately invalid length.
            let _ = unsafe { raw_host_call(0, i32::MAX) };
            ActionOutcome::ok()
        }
        "host_call_negative" => {
            // SAFETY: deliberately invalid.
            let _ = unsafe { raw_host_call(-1, -1) };
            ActionOutcome::ok()
        }
        "host_call_oversized" => {
            let big = vec![b' '; 65 * 1024];
            raw(raw_call(&big))
        }
        "raw_request" => raw(raw_call(string(params, "json").as_bytes())),
        // --- misbehaviour: results ---
        "big_output" => ActionOutcome::Ok {
            message: None,
            data: (0..60)
                .map(|i| (format!("k{i}"), ParamValue::String("x".repeat(16 << 10))))
                .collect(),
        },
        "bad_code" => ActionOutcome::failed("Not A Code", "x"),
        _ => ActionOutcome::failed("unknown-action", "no such action"),
    }
}
