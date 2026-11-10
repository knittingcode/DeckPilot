//! `deckpilot-agent` command-line entry point.

use std::process::ExitCode;

use deckpilot_agent::autostart::Autostart;
use deckpilot_agent::{Agent, InstanceLock, LockError, default_options};
use deckpilot_device::{DeviceModel, models};
use deckpilot_ipc::AgentPaths;
use tracing::{error, info};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const USAGE: &str = "\
DeckPilot background agent

Usage:
  deckpilot-agent [--headless] [--mock-devices [<models>]] [--log-level <level>]
  deckpilot-agent autostart <enable|disable|status>

Options:
  --headless          Run without a tray icon (servers, CI, debugging)
  --mock-devices [<models>]
                      Add simulated devices next to USB devices. <models> is a
                      comma-separated list of mk2, xl and plus (default: mk2)
  --log-level <level> error, warn, info (default), debug or trace
  --version           Print the version
  --help              Print this help

Environment:
  DECKPILOT_DATA_DIR    Use a different data directory
  DECKPILOT_DESKTOP_BIN Path of the configuration app executable";

struct Args {
    headless: bool,
    mock_models: Vec<DeviceModel>,
    level: LevelFilter,
}

fn mock_models(list: &str) -> Result<Vec<DeviceModel>, String> {
    list.split(',')
        .map(|name| match name.trim() {
            "mk2" => Ok(models::stream_deck_mk2()),
            "xl" => Ok(models::stream_deck_xl()),
            "plus" => Ok(models::stream_deck_plus()),
            other => Err(format!(
                "unknown mock device model `{other}` (use mk2, xl or plus)"
            )),
        })
        .collect()
}

fn parse_args() -> Result<Result<Args, String>, ExitCode> {
    let mut args = std::env::args().skip(1).peekable();
    let mut parsed = Args {
        headless: false,
        mock_models: Vec::new(),
        level: LevelFilter::INFO,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => parsed.headless = true,
            "--mock-devices" => {
                let list = match args.peek() {
                    Some(next) if !next.starts_with("--") => args.next().unwrap_or_default(),
                    _ => "mk2".to_owned(),
                };
                match mock_models(&list) {
                    Ok(models) => parsed.mock_models = models,
                    Err(message) => return Ok(Err(message)),
                }
            }
            "--log-level" => {
                let value = args.next().unwrap_or_default();
                match value.parse() {
                    Ok(level) => parsed.level = level,
                    Err(_) => return Ok(Err(format!("invalid log level `{value}`"))),
                }
            }
            "--version" => {
                println!("deckpilot-agent {}", deckpilot_agent::service::VERSION);
                return Err(ExitCode::SUCCESS);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Err(ExitCode::SUCCESS);
            }
            "autostart" => return Err(autostart_command(args.next().as_deref())),
            other => return Ok(Err(format!("unknown argument `{other}`"))),
        }
    }
    Ok(Ok(parsed))
}

fn autostart_command(action: Option<&str>) -> ExitCode {
    let autostart = match Autostart::for_current_exe() {
        Ok(autostart) => autostart,
        Err(error) => {
            eprintln!("autostart is unavailable: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = match action {
        Some("enable") => autostart.set_enabled(true).map(|()| "enabled".to_owned()),
        Some("disable") => autostart.set_enabled(false).map(|()| "disabled".to_owned()),
        Some("status") => autostart
            .is_enabled()
            .map(|on| if on { "enabled" } else { "disabled" }.to_owned()),
        _ => Err("expected enable, disable or status".into()),
    };
    match result {
        Ok(message) => {
            println!("autostart {message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Logs to stderr and to daily-rotated files in the data directory.
fn init_logging(
    paths: &AgentPaths,
    level: LevelFilter,
) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("agent")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&paths.log_dir);
    let (file_layer, guard) = match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer);
            (Some(layer), Some(guard))
        }
        Err(error) => {
            eprintln!("file logging disabled: {error}");
            (None, None)
        }
    };
    tracing_subscriber::registry()
        .with(level)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
                .with_writer(std::io::stderr),
        )
        .with(file_layer)
        .init();
    guard
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Ok(args)) => args,
        Ok(Err(message)) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
        Err(code) => return code,
    };
    let paths = match AgentPaths::resolve().and_then(|paths| paths.create_dirs().map(|()| paths)) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("cannot prepare the data directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let _guard = init_logging(&paths, args.level);

    let lock = match InstanceLock::acquire(&paths.lock_file) {
        Ok(lock) => lock,
        Err(LockError::AlreadyRunning) => {
            info!("the agent is already running");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            error!(%error, "cannot start");
            return ExitCode::FAILURE;
        }
    };
    let options = default_options(paths, &args.mock_models);

    if args.headless {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                error!(%error, "cannot start the async runtime");
                return ExitCode::FAILURE;
            }
        };
        let result = runtime.block_on(async {
            let agent = Agent::start(options, lock)?;
            agent.run(deckpilot_agent::service::shutdown_signal()).await
        });
        if let Err(error) = result {
            error!(%error, "agent stopped with an error");
            return ExitCode::FAILURE;
        }
    } else {
        deckpilot_agent::tray::run(options, lock);
    }
    ExitCode::SUCCESS
}
