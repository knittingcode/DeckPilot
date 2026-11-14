//! `deckpilot-plugin-host`: one plugin's sandboxed wasm host.
//!
//! Started by the supervisor with an empty environment, a fixed working
//! directory and stdin/stdout as the only channel. Arguments: none, or
//! `--no-os-sandbox` (development and tests only; reported in `ready`).
//! Exit codes: `deckpilot_plugin_abi::protocol::exit_code`.

use std::io::{self, BufReader, BufWriter};
use std::process::ExitCode;

use deckpilot_plugin_abi::protocol::exit_code;
use deckpilot_plugin_host::channel;
use deckpilot_plugin_host::engine::PluginEngine;
use deckpilot_plugin_host::sandbox::{self, Mode};

fn exit(code: i32) -> ExitCode {
    ExitCode::from(code as u8)
}

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mode = match args.as_slice() {
        [] => Mode::Enforce,
        [a] if a == "--no-os-sandbox" => Mode::Disabled,
        _ => return exit(exit_code::USAGE),
    };

    // Stage 1 while single-threaded, then the engine (which starts the
    // epoch ticker thread), then stage 2 for all threads. No plugin byte
    // has been read at this point.
    let pending = match sandbox::begin(mode) {
        Ok(p) => p,
        Err(_) => return exit(exit_code::SANDBOX_FAILED),
    };
    let engine = match PluginEngine::new() {
        Ok(e) => e,
        Err(_) => return exit(exit_code::LOAD_FAILED),
    };
    let report = match pending.finish() {
        Ok(r) => r,
        Err(_) => return exit(exit_code::SANDBOX_FAILED),
    };

    let reader = BufReader::new(io::stdin().lock());
    let writer = BufWriter::new(io::stdout().lock());
    let outcome = channel::run(&engine, report, reader, writer);
    // Exit without running destructors of the Store or the engine after a
    // fault; nothing of the guest is touched again.
    std::process::exit(outcome.code())
}
