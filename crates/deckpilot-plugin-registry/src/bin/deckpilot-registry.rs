//! `deckpilot-registry`: offline publishing tooling for the static plugin
//! registry (feature `publish`). See the crate README.

use std::io::Write as _;
use std::process::ExitCode;

use deckpilot_plugin_registry::publish::{Io, run};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = |name: &str| std::env::var(name).ok();
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let mut io = Io {
        env: &env,
        stdin: &mut stdin,
        out: &mut stdout,
    };
    match run(&args, &mut io) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "error: {error}");
            ExitCode::FAILURE
        }
    }
}
