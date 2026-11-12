//! `deckpilot-dev`: starts, inspects and stops a DeckPilot development
//! session. Used by the Makefile and `pnpm dev`; not part of the product.
//!
//! A session consists of the background agent (with simulated devices next
//! to any connected hardware) and the configuration app in Tauri
//! development mode with Vite HMR. Both use an isolated data directory in
//! `.dev/data`, so the user's real configuration is never touched.

mod hardware;
mod logs;
mod package;
mod prepare;
mod process;
mod release;
mod seed;
mod session;
mod validation;
mod versions;

use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use deckpilot_ipc::{AgentClient, ClientError, ClientKind, Request, Response};

use crate::process::OwnedProcess;
use crate::session::{DevPaths, Session};

const USAGE: &str = "\
DeckPilot development environment

Usage: deckpilot-dev <command> [options]

Commands:
  doctor                 Check development dependencies
  up [--foreground] [--locked]
                         Start the agent and the app with live reload
                         (--locked: lock the configuration after start)
     [--devices <list>]  Simulated devices: mk2,xl,plus (default) or none
  down                   Stop the session started by `up`
  restart                down + up
  status                 Show the session and the agent state
  logs [-f] [-n <lines>] [agent|ui|supervisor]
                         Show session logs (follow with -f)
  agent                  Run the agent in the foreground (dev data)
  ui                     Run the app with HMR in the foreground (dev data)
  open-ui                Start the app of the running session again (with its
                         Vite server) unless it runs; the tray's
                         \"Open DeckPilot\" does this in a session
  show                   Ask the dev agent to show the app, as the tray does
  seed [--reset] [--password] [--encrypt-legacy]
                         Fill the dev database with demonstration data. A new
                         database gets a system-managed key from the agent
                         (Keychain service \"DeckPilot Development\");
                         --password: protect it with the dev password instead
                         --encrypt-legacy: encrypt an unencrypted database
                         from an earlier version with the dev password
  window <id>            Ask the dev agent to open a native window, e.g.
                         security.unlock or workflows.execution-summary
  press <serial> <key>   Press a key of a simulated device
  clean [--data]         Remove logs and session files (and dev data)
  set-version <version>  Set the application version everywhere it is declared
  check-versions [--tag <tag>] [--root <dir>]
                         Verify that all version numbers match (and that
                         <tag> is the release tag of that version)
  package [-- <tauri build args>]
                         Build installers including the agent
  verify-release <dir>   Check a release's latest.json and artifact signatures
                         (public key from DECKPILOT_UPDATER_PUBKEY)
  check-updater-key      Check that DECKPILOT_UPDATER_PUBKEY is a valid public key
  release-notes          Print the CHANGELOG.md section of the current version
  prepare-release <version> [--local]
                         On a new branch from origin/main: set the version,
                         move the CHANGELOG \"Unreleased\" notes into its
                         section, validate, commit, push and open a pull
                         request (--local: commit only). Never creates a tag
  release-tag <tag>      Classify a release tag (vX.Y.Z, -alpha.N, -beta.N, -rc.N):
                         version, channel, manifest; fails for any other tag
  hardware-test [--mock] [--dir <data-dir>] [--force]
                         Guided test of a physical Stream Deck with the real
                         agent; --mock checks the procedure on a simulated MK.2
  validation seed|populate|unlock|status|inventory <data-dir>
                         Update-test helpers on an isolated data directory
                         (see docs/release-validation.md)
  verify-signature <file> <file.sig>
                         Verify one updater signature against
                         DECKPILOT_UPDATER_PUBKEY";

/// The configuration app in development mode.
const UI_ARGS: &[&str] = &["--filter", "@deckpilot/desktop", "tauri", "dev"];
const DEFAULT_DEVICES: &str = "mk2,xl,plus";

/// The agent starts this program to open the app (`launcher.rs`).
const DESKTOP_BIN_ENV: &str = "DECKPILOT_DESKTOP_BIN";
/// Set by the supervisor on its agent: the development directory whose
/// session the agent belongs to. `deckpilot-dev` started by that agent
/// (without a command, or with `--window <id>` like the app) asks that
/// session's supervisor to start the app instead of printing its usage.
const UI_LAUNCHER_ENV: &str = "DECKPILOT_DEV_UI_LAUNCHER";
/// An extra Tauri configuration for the app of a session (the integration
/// preview: its own identifier, window title and `devUrl`), with
/// `DECKPILOT_VITE_PORT` for its Vite server.
const TAURI_CONFIG_ENV: &str = "DECKPILOT_TAURI_CONFIG";

/// Executable name for tools installed as `.cmd` shims on Windows.
pub fn program(name: &str) -> String {
    if cfg!(windows) && name == "pnpm" {
        format!("{name}.cmd")
    } else {
        name.to_owned()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn dev_env(command: &mut Command, paths: &DevPaths) {
    command
        .current_dir(&paths.root)
        .env(deckpilot_ipc::paths::DATA_DIR_ENV, &paths.data)
        // The session manages the agent; the app must not start another one.
        .env("DECKPILOT_NO_AGENT_AUTOSTART", "1")
        .env("DECKPILOT_DEV", "1");
}

/// The configuration app in development mode (`tauri dev`, which starts
/// Vite first).
fn ui_command(paths: &DevPaths) -> Command {
    ui_command_with(paths, std::env::var_os(TAURI_CONFIG_ENV))
}

fn ui_command_with(paths: &DevPaths, config: Option<std::ffi::OsString>) -> Command {
    let mut command = Command::new(program("pnpm"));
    command.args(UI_ARGS);
    if let Some(config) = config.filter(|v| !v.is_empty()) {
        command.arg("--config").arg(config);
    }
    dev_env(&mut command, paths);
    command
}

/// Lets the session's agent open the real app: instead of the debug app
/// next to it (which shows nothing once `tauri dev` and its Vite server have
/// ended), the agent starts this program, which asks this supervisor to
/// start `tauri dev` again. Absolute paths, so the working directory of
/// the agent does not matter.
fn set_ui_launcher(command: &mut Command, paths: &DevPaths) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    command
        .env(DESKTOP_BIN_ENV, exe)
        .env(UI_LAUNCHER_ENV, paths.dev_dir());
    Ok(())
}

fn agent_args(devices: &str) -> Vec<String> {
    let mut args = vec!["--log-level".to_owned(), "debug".to_owned()];
    if devices != "none" {
        args.push("--mock-devices".into());
        args.push(devices.into());
    }
    args
}

// ---------------------------------------------------------------- doctor

fn version_of(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn doctor() -> Result<(), String> {
    let mut missing = Vec::new();
    for (name, program, args) in [
        ("Rust (cargo)", "cargo".to_owned(), vec!["--version"]),
        ("Node.js", "node".to_owned(), vec!["--version"]),
        ("pnpm", program("pnpm"), vec!["--version"]),
    ] {
        if let Some(version) = version_of(&program, &args) {
            println!("  ok  {name}: {version}");
        } else {
            println!("  !!  {name}: not found");
            missing.push(name);
        }
    }
    if let Some(node) = version_of("node", &["--version"]) {
        let major: u32 = node
            .trim_start_matches('v')
            .split('.')
            .next()
            .and_then(|m| m.parse().ok())
            .unwrap_or(0);
        if major < 22 {
            missing.push("Node.js 22 or newer");
        }
    }
    if cfg!(target_os = "linux") {
        for library in ["webkit2gtk-4.1", "libudev", "ayatana-appindicator3-0.1"] {
            let found = Command::new("pkg-config")
                .args(["--exists", library])
                .status()
                .is_ok_and(|s| s.success());
            println!("  {}  {library}", if found { "ok" } else { "!!" });
            if !found {
                missing.push(library);
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing: {} (see docs/development.md)",
            missing.join(", ")
        ))
    }
}

// ---------------------------------------------------------------- session

fn spawn_logged(
    command: &mut Command,
    role: &'static str,
    paths: &DevPaths,
    echo: bool,
) -> Result<Child, String> {
    let file = logs::open_append(&logs::log_path(&paths.logs, role)).map_err(|e| e.to_string())?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {role}: {e}"))?;
    if let Some(stdout) = child.stdout.take() {
        logs::pipe(stdout, role, file.clone(), echo);
    }
    if let Some(stderr) = child.stderr.take() {
        logs::pipe(stderr, role, file, echo);
    }
    Ok(child)
}

async fn agent_status(paths: &DevPaths) -> Result<deckpilot_ipc::AgentStatus, ClientError> {
    let mut client = AgentClient::connect(&paths.agent_paths(), ClientKind::Other).await?;
    match client.request(Request::GetStatus).await? {
        Response::Status { status } => Ok(*status),
        other => Err(ClientError::Protocol(format!("{other:?}"))),
    }
}

fn wait_for_agent(paths: &DevPaths, agent: &mut Child, timeout: Duration) -> Result<(), String> {
    let runtime = runtime();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if runtime.block_on(agent_status(paths)).is_ok() {
            return Ok(());
        }
        if let Ok(Some(status)) = agent.try_wait() {
            return Err(format!(
                "the agent exited ({status}); see {}",
                logs::log_path(&paths.logs, "agent").display()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("the agent did not become ready in time".into())
}

/// `seed [--reset] [--password] [--encrypt-legacy]`.
fn seed_command(
    paths: &DevPaths,
    reset: bool,
    password: bool,
    encrypt_legacy: bool,
) -> Result<(), String> {
    if reset {
        if Session::load(paths).is_some_and(|s| s.supervisor_alive()) {
            return Err("stop the session before resetting its data".into());
        }
        let _ = std::fs::remove_dir_all(&paths.data);
    }
    if encrypt_legacy {
        if Session::load(paths).is_some_and(|s| s.supervisor_alive()) {
            return Err("stop the session before encrypting its data".into());
        }
        seed::encrypt_legacy(paths).map_err(|e| e.to_string())?;
        println!("encrypted the development database with the development password");
    }
    let mut seeded = seed::seed(paths, password).map_err(|e| e.to_string())?;
    // A system-managed database is seeded with the key of the running agent.
    if !seeded {
        seeded = seed_through_agent(paths)?;
    }
    let missing = !paths.data.join("deckpilot.db").exists();
    println!(
        "{}",
        if seeded {
            "seeded demonstration data"
        } else if missing {
            "no database yet: `make up` creates one with a system-managed key and seeds it"
        } else {
            "already seeded (or the agent is not running)"
        }
    );
    Ok(())
}

/// Seeds a system-managed development database with the key of the running
/// development agent and lets it reload. Nothing happens if the agent is
/// not running or locked.
fn seed_through_agent(paths: &DevPaths) -> Result<bool, String> {
    let key = runtime().block_on(async {
        let mut client = AgentClient::connect(&paths.agent_paths(), ClientKind::Other).await?;
        client.request(Request::GetDatabaseKey).await
    });
    let Ok(Response::DatabaseKey { key }) = key else {
        return Ok(false);
    };
    let key = deckpilot_storage::DatabaseKey::from_hex(key.expose()).map_err(|e| e.to_string())?;
    let seeded = seed::seed_with_key(paths, key).map_err(|e| e.to_string())?;
    if seeded {
        let _ = dev_request(paths, Request::ReloadConfiguration);
    }
    Ok(seeded)
}

/// `window` and `press`: requests to the running development agent.
fn agent_command(name: &str, args: &[String], paths: &DevPaths) -> Result<(), String> {
    match name {
        "window" => {
            let id = args.first().ok_or("expected a window identifier")?;
            dev_request(paths, Request::RequestWindow { window: id.clone() })
        }
        "press" => {
            let (Some(serial), Some(key)) = (args.first(), args.get(1)) else {
                return Err("expected a simulated device serial and a key index".into());
            };
            let index: u16 = key.parse().map_err(|_| "the key index must be a number")?;
            let status = runtime()
                .block_on(agent_status(paths))
                .map_err(|e| e.to_string())?;
            let device = status
                .devices
                .iter()
                .find(|d| &d.id.serial == serial)
                .ok_or_else(|| format!("no simulated device {serial}"))?
                .id
                .clone();
            dev_request(
                paths,
                Request::SimulateInput {
                    device,
                    control: deckpilot_device::ControlId::Key { index },
                    input: deckpilot_device::ControlInput::Press,
                },
            )
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// Sends one request to the development agent.
fn dev_request(paths: &DevPaths, request: Request) -> Result<(), String> {
    runtime()
        .block_on(async {
            let mut client = AgentClient::connect(&paths.agent_paths(), ClientKind::Other).await?;
            client.request(request).await
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Prepares the started development agent. A system-managed configuration
/// is open already (and is seeded on the first start); a password-protected
/// one is unlocked with the development password, so that `make up` behaves
/// like an unlocked session. `--locked` (`make up LOCKED=1`) locks it to try
/// the unlock screens.
fn unlock_dev_agent(paths: &DevPaths, locked: bool) {
    let mut status = runtime().block_on(agent_status(paths)).ok();
    if status.as_ref().is_some_and(|s| s.waiting_for_key_store) {
        // A rebuilt agent has a new code signature: macOS asks once per
        // build whether it may use its Keychain item.
        println!(
            "the agent is waiting for the credential store; on macOS, answer the Keychain dialog \
             for the rebuilt agent (\"Always Allow\"), or use `deckpilot-dev seed --reset --password` \
             to develop without the Keychain"
        );
        let deadline = Instant::now() + Duration::from_secs(120);
        while status.as_ref().is_some_and(|s| s.waiting_for_key_store) && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(500));
            status = runtime().block_on(agent_status(paths)).ok();
        }
    }
    if status
        .as_ref()
        .is_some_and(|s| s.protection == Some(deckpilot_ipc::ProtectionMode::SystemManaged))
    {
        match seed_through_agent(paths) {
            Ok(true) => println!("seeded the development database with demonstration data"),
            Ok(false) => {}
            Err(error) => println!("could not seed the development database: {error}"),
        }
        if let Some(problem) = status.and_then(|s| s.security_problem) {
            println!("agent locked: {problem}");
        } else if locked {
            let _ = dev_request(paths, Request::Lock);
            println!("agent locked; unlock it in the app (system authentication)");
        } else {
            println!("configuration opened with the system-managed development key");
        }
        return;
    }
    if locked {
        println!(
            "agent left locked; the development password is `{}`",
            seed::DEV_PASSWORD
        );
        return;
    }
    let Some(key) = seed::dev_key(paths) else {
        println!("agent locked: the app asks for a password");
        return;
    };
    let unlocked = runtime().block_on(async {
        let mut client = AgentClient::connect(&paths.agent_paths(), ClientKind::Other).await?;
        client
            .request(Request::Unlock {
                key: deckpilot_ipc::KeyMaterial::new(key.to_hex()),
            })
            .await
    });
    match unlocked {
        Ok(_) => println!("agent unlocked with the development password"),
        Err(error) => println!("could not unlock the agent: {error}"),
    }
}

fn stop_agent_gracefully(paths: &DevPaths, owned: Option<&OwnedProcess>) {
    let runtime = runtime();
    let requested = runtime
        .block_on(async {
            let mut client = AgentClient::connect(&paths.agent_paths(), ClientKind::Other).await?;
            client.request(Request::Shutdown).await
        })
        .is_ok();
    let Some(owned) = owned else { return };
    let deadline = Instant::now() + Duration::from_secs(10);
    while requested && Instant::now() < deadline && process::is_alive(owned) {
        std::thread::sleep(Duration::from_millis(100));
    }
    // Only if the agent did not stop on request (and is still ours).
    process::terminate_tree(owned, Duration::from_secs(3));
}

/// Runs the session in this process until a stop is requested.
fn supervise(
    paths: &DevPaths,
    devices: &str,
    foreground: bool,
    locked: bool,
) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst)).map_err(|e| e.to_string())?;

    let supervisor = process::record("supervisor", std::process::id())
        .ok_or("cannot inspect the supervisor process")?;
    let mut session = Session {
        id: uuid::Uuid::new_v4().to_string(),
        started_at: now(),
        supervisor,
        processes: Vec::new(),
        devices: devices.to_owned(),
    };
    let _ = std::fs::remove_file(&paths.stop_request);
    session.save(paths).map_err(|e| e.to_string())?;
    println!("session {} started", session.id);

    let mut agent_command = Command::new(paths.agent_binary());
    agent_command.args(agent_args(devices));
    dev_env(&mut agent_command, paths);
    set_ui_launcher(&mut agent_command, paths)?;
    let mut agent = spawn_logged(&mut agent_command, "agent", paths, foreground)?;
    if let Some(owned) = process::record("agent", agent.id()) {
        session.processes.push(owned);
        session.save(paths).map_err(|e| e.to_string())?;
    }
    if let Err(error) = wait_for_agent(paths, &mut agent, Duration::from_secs(30)) {
        stop_agent_gracefully(paths, session.process("agent"));
        Session::remove(paths);
        return Err(error);
    }
    println!("agent ready (data: {})", paths.data.display());
    unlock_dev_agent(paths, locked);

    let mut ui = spawn_logged(&mut ui_command(paths), "ui", paths, foreground)?;
    if let Some(owned) = process::record("ui", ui.id()) {
        session.replace_process(owned);
        session.save(paths).map_err(|e| e.to_string())?;
    }
    println!(
        "configuration app starting with live reload (logs: {})",
        paths.logs.display()
    );

    let (mut agent_running, mut ui_running) = (true, true);
    while !stop.load(Ordering::SeqCst) && !paths.stop_request.exists() {
        if agent_running && let Ok(Some(status)) = agent.try_wait() {
            println!("the agent exited ({status}); run `make restart` to start it again");
            agent_running = false;
        }
        if ui_running && let Ok(Some(status)) = ui.try_wait() {
            println!(
                "the configuration app exited ({status}); the agent keeps running \
                 (the tray's Open DeckPilot or `deckpilot-dev open-ui` starts it again)"
            );
            ui_running = false;
        }
        if take_ui_request(paths, &session.id) {
            if ui_running {
                println!("the configuration app was requested; it is running already");
            } else {
                println!("starting the configuration app again (requested)");
                match spawn_logged(&mut ui_command(paths), "ui", paths, foreground) {
                    Ok(child) => {
                        if let Some(owned) = process::record("ui", child.id()) {
                            session.replace_process(owned);
                            let _ = session.save(paths);
                        }
                        ui = child;
                        ui_running = true;
                    }
                    Err(error) => println!("{error}"),
                }
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    println!("stopping session {}", session.id);
    if let Some(owned) = session.process("ui") {
        process::terminate_tree(owned, Duration::from_secs(10));
    }
    stop_agent_gracefully(paths, session.process("agent"));
    let _ = agent.wait();
    let _ = ui.wait();
    Session::remove(paths);
    println!("session stopped");
    Ok(())
}

/// Whether `open-ui` asked the session `id` for the app. Consumes the
/// request; one left by an earlier session is dropped.
fn take_ui_request(paths: &DevPaths, id: &str) -> bool {
    let Ok(requested) = std::fs::read_to_string(&paths.ui_request) else {
        return false;
    };
    let _ = std::fs::remove_file(&paths.ui_request);
    requested.trim() == id
}

#[derive(Debug, PartialEq, Eq)]
enum UiRequest {
    /// The supervisor will start the app.
    Requested,
    /// The app (`tauri dev`) of the session is running or starting.
    Running,
}

/// `open-ui`: has the supervisor of the session in `paths` start the app
/// again unless it runs. Only that session is touched: its own data
/// directory, its own app configuration and port.
fn request_ui(paths: &DevPaths) -> Result<UiRequest, String> {
    let session = Session::load(paths)
        .filter(Session::supervisor_alive)
        .ok_or_else(|| {
            format!(
                "no development session is running for {}; start one with `make up`",
                paths.data.display()
            )
        })?;
    if session.process("ui").is_some_and(process::is_alive) {
        return Ok(UiRequest::Running);
    }
    std::fs::write(&paths.ui_request, session.id.as_bytes()).map_err(|e| e.to_string())?;
    Ok(UiRequest::Requested)
}

fn open_ui(paths: &DevPaths) -> Result<(), String> {
    match request_ui(paths)? {
        UiRequest::Requested => {
            println!("the session starts the configuration app (`make logs` shows its progress)");
        }
        UiRequest::Running => {
            println!("the configuration app is running; asking it to show its window");
            dev_request(paths, Request::ShowWindow)?;
        }
    }
    Ok(())
}

/// Started by the session's agent in place of the app (see
/// [`set_ui_launcher`]). Its output goes nowhere, so the outcome is noted
/// in the session's app log.
fn open_ui_for_agent(dev_dir: &std::path::Path) -> Result<(), String> {
    let paths = DevPaths::in_dev_dir(DevPaths::new().root, dev_dir);
    let outcome = launcher_for(
        &paths,
        std::env::var_os(deckpilot_ipc::paths::DATA_DIR_ENV).as_deref(),
    )
    .and_then(|()| request_ui(&paths));
    let note = match &outcome {
        Ok(UiRequest::Requested) => "app requested by the agent; the supervisor starts it".into(),
        Ok(UiRequest::Running) => "app requested by the agent; it is starting already".into(),
        Err(error) => format!("app requested by the agent: {error}"),
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs::log_path(&paths.logs, "ui"))
    {
        use std::io::Write;
        let _ = writeln!(
            file,
            "{} [launcher] {note}",
            logs::timestamp(SystemTime::now())
        );
    }
    outcome.map(|_| ())
}

/// The agent passes its own data directory: a launcher variable that leaked
/// into another environment must not open a different session.
fn launcher_for(paths: &DevPaths, agent_data: Option<&std::ffi::OsStr>) -> Result<(), String> {
    match agent_data.map(std::path::Path::new) {
        Some(data) if data != paths.data => Err(format!(
            "this launcher belongs to {}, not to {}",
            paths.data.display(),
            data.display()
        )),
        _ => Ok(()),
    }
}

/// Removes metadata of a session whose supervisor is gone, stopping any
/// processes that are verifiably still ours.
fn clean_stale(paths: &DevPaths, session: &Session) {
    println!("cleaning up stale session {}", session.id);
    if let Some(owned) = session.process("ui") {
        process::terminate_tree(owned, Duration::from_secs(5));
    }
    stop_agent_gracefully(paths, session.process("agent"));
    Session::remove(paths);
}

fn build_agent(paths: &DevPaths) -> Result<(), String> {
    println!("building the agent…");
    let status = Command::new("cargo")
        .current_dir(&paths.root)
        .args(["build", "-p", "deckpilot-agent"])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("building the agent failed".into())
    }
}

fn up(paths: &DevPaths, devices: &str, foreground: bool, locked: bool) -> Result<(), String> {
    paths.ensure_isolated()?;
    paths.create().map_err(|e| e.to_string())?;
    if let Some(session) = Session::load(paths) {
        if session.supervisor_alive() {
            println!(
                "a development session is already running (session {})",
                session.id
            );
            status(paths);
            return Ok(());
        }
        clean_stale(paths, &session);
    }
    doctor()?;
    if seed::seed(paths, false).map_err(|e| e.to_string())? {
        println!("seeded the development database with demonstration data");
    }
    build_agent(paths)?;

    if foreground {
        return supervise(paths, devices, true, locked);
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    command.args(["supervise", "--devices", devices]);
    if locked {
        command.arg("--locked");
    }
    command.current_dir(&paths.root);
    let log = std::fs::File::create(logs::log_path(&paths.logs, "supervisor"))
        .map_err(|e| e.to_string())?;
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    detach(&mut command);
    let mut child = command.spawn().map_err(|e| e.to_string())?;

    // Wait until the supervisor reports both processes.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(session) = Session::load(paths)
            && session.process("ui").is_some()
        {
            println!("development session {} is running", session.id);
            println!(
                "the app window opens once its first build finishes; `make logs` shows progress"
            );
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            let log = std::fs::read_to_string(logs::log_path(&paths.logs, "supervisor"))
                .unwrap_or_default();
            return Err(format!("the session failed to start ({status}):\n{log}"));
        }
        if Instant::now() > deadline {
            return Err("timed out waiting for the session to start".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn detach(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
}

fn down(paths: &DevPaths) -> Result<(), String> {
    let Some(session) = Session::load(paths) else {
        println!("no development session is running");
        return Ok(());
    };
    if !session.supervisor_alive() {
        clean_stale(paths, &session);
        return Ok(());
    }
    std::fs::write(&paths.stop_request, session.id.as_bytes()).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while session.supervisor_alive() {
        if Instant::now() > deadline {
            println!("the supervisor did not stop in time; stopping its processes directly");
            clean_stale(paths, &session);
            process::terminate_tree(&session.supervisor, Duration::from_secs(3));
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Session::remove(paths);
    println!(
        "development session {} stopped; data kept in {}",
        session.id,
        paths.data.display()
    );
    Ok(())
}

fn status(paths: &DevPaths) {
    println!("data directory: {}", paths.data.display());
    match Session::load(paths) {
        None => println!("session: not running"),
        Some(session) => {
            let state = |alive: bool| if alive { "running" } else { "not running" };
            println!(
                "session: {} (supervisor {}, pid {})",
                session.id,
                state(session.supervisor_alive()),
                session.supervisor.pid
            );
            for owned in &session.processes {
                println!(
                    "  {:<10} pid {:<7} {}",
                    owned.role,
                    owned.pid,
                    state(process::is_alive(owned))
                );
            }
        }
    }
    match runtime().block_on(agent_status(paths)) {
        Ok(status) => {
            println!(
                "agent: {} (protocol {}, pid {})",
                status.version, status.protocol, status.pid
            );
            if let Some(profile) = &status.active_profile {
                let project = status
                    .active_project
                    .as_ref()
                    .map_or("-", |p| p.key.as_str());
                println!("  profile {} · project {project}", profile.name);
            }
            for device in &status.devices {
                println!(
                    "  {} {} [{}] {:?}",
                    device.model_name, device.id.serial, device.source, device.state
                );
            }
        }
        Err(ClientError::NotRunning) => println!("agent: not running"),
        Err(error) => println!("agent: {error}"),
    }
}

fn foreground_agent(paths: &DevPaths, devices: &str) -> Result<(), String> {
    paths.ensure_isolated()?;
    paths.create().map_err(|e| e.to_string())?;
    seed::seed(paths, false).map_err(|e| e.to_string())?;
    let mut command = Command::new("cargo");
    command
        .args(["run", "-p", "deckpilot-agent", "--"])
        .args(agent_args(devices));
    dev_env(&mut command, paths);
    // Unlock the agent once it is listening, as `make up` does.
    let watcher = paths.clone();
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            if runtime().block_on(agent_status(&watcher)).is_ok() {
                unlock_dev_agent(&watcher, false);
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    let status = command.status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("agent exited with {status}"))
    }
}

fn foreground_ui(paths: &DevPaths) -> Result<(), String> {
    paths.ensure_isolated()?;
    let status = ui_command(paths).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("app exited with {status}"))
    }
}

fn clean(paths: &DevPaths, data: bool) -> Result<(), String> {
    if Session::load(paths).is_some_and(|s| s.supervisor_alive()) {
        return Err("a development session is running; run `make down` first".into());
    }
    Session::remove(paths);
    let _ = std::fs::remove_dir_all(&paths.logs);
    let _ = std::fs::remove_dir_all(paths.root.join("apps/desktop/src-tauri/binaries"));
    if data {
        let _ = std::fs::remove_dir_all(&paths.data);
        let _ = std::fs::remove_dir_all(&paths.workspace);
        println!("removed development data");
    }
    println!("removed logs and session files");
    Ok(())
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn run(args: &[String]) -> Result<(), String> {
    if let Some(dev_dir) = std::env::var_os(UI_LAUNCHER_ENV).filter(|v| !v.is_empty())
        && args.first().is_none_or(|arg| arg == "--window")
    {
        return open_ui_for_agent(std::path::Path::new(&dev_dir));
    }
    let paths = DevPaths::new();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let devices = option(args, "--devices").unwrap_or_else(|| DEFAULT_DEVICES.to_owned());
    match args.first().map(String::as_str) {
        Some("doctor") => doctor(),
        Some("up") => up(&paths, &devices, flag("--foreground"), flag("--locked")),
        Some("supervise") => supervise(&paths, &devices, false, flag("--locked")),
        Some("down") => down(&paths),
        Some("restart") => {
            let devices = Session::load(&paths).map_or(devices, |s| s.devices);
            down(&paths)?;
            up(&paths, &devices, false, flag("--locked"))
        }
        Some("status") => {
            status(&paths);
            Ok(())
        }
        Some("logs") => {
            let lines = option(args, "-n")
                .and_then(|n| n.parse().ok())
                .unwrap_or(50);
            let roles: Vec<&str> = ["agent", "ui", "supervisor"]
                .into_iter()
                .filter(|role| args.iter().any(|a| a == role))
                .collect();
            let roles = if roles.is_empty() {
                vec!["agent", "ui"]
            } else {
                roles
            };
            let files: Vec<_> = roles
                .iter()
                .map(|r| logs::log_path(&paths.logs, r))
                .collect();
            let stop = Arc::new(AtomicBool::new(false));
            let flag_stop = stop.clone();
            let _ = ctrlc::set_handler(move || flag_stop.store(true, Ordering::SeqCst));
            logs::show(&files, lines, flag("-f") || flag("--follow"), &stop);
            Ok(())
        }
        Some("agent") => foreground_agent(&paths, &devices),
        Some("ui") => foreground_ui(&paths),
        Some("open-ui") => open_ui(&paths),
        Some("show") => dev_request(&paths, Request::ShowWindow),
        Some("seed") => seed_command(
            &paths,
            flag("--reset"),
            flag("--password"),
            flag("--encrypt-legacy"),
        ),
        Some("clean") => clean(&paths, flag("--data")),
        Some(command @ ("window" | "press")) => agent_command(command, &args[1..], &paths),
        Some("set-version") => {
            let version = args.get(1).ok_or("usage: set-version <version>")?;
            versions::set(&paths.root, version)?;
            println!(
                "version set to {version} (Cargo.toml, Cargo.lock, package.json, tauri.conf.json)"
            );
            Ok(())
        }
        Some("check-versions") => {
            // --root: another checkout (the release gate's tests); read only.
            let root = option(args, "--root").map_or_else(|| paths.root.clone(), Into::into);
            let version = versions::check(&root, option(args, "--tag").as_deref())?;
            println!("all versions are {version}");
            Ok(())
        }
        Some("prepare-release") => prepare::command(&paths.root, &args[1..]),
        Some("validation") => validation::command(&args[1..]),
        Some("hardware-test") => hardware::command(&paths.root, &args[1..]),
        Some("package") => {
            let extra: Vec<String> = args
                .iter()
                .skip_while(|a| *a != "--")
                .skip(1)
                .cloned()
                .collect();
            package::package(&paths.root, &extra)
        }
        Some(
            command @ ("verify-release" | "check-updater-key" | "verify-signature"
            | "release-notes" | "release-tag"),
        ) => release::command(command, &args[1..], &paths.root),
        Some("help" | "--help" | "-h") | None => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::PathBuf;

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "deckpilot-dev-{tag}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn session_in(paths: &DevPaths) -> Session {
        paths.create().unwrap();
        let session = Session {
            id: uuid::Uuid::new_v4().to_string(),
            started_at: now(),
            // This test process stands in for a live supervisor.
            supervisor: process::record("supervisor", std::process::id()).unwrap(),
            processes: Vec::new(),
            devices: "mk2".into(),
        };
        session.save(paths).unwrap();
        session
    }

    fn env_of<'a>(command: &'a Command, name: &str) -> Option<&'a OsStr> {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value)
    }

    #[test]
    fn opening_the_app_asks_only_the_session_of_its_own_directory() {
        let (a, b) = (TempDir::new("a"), TempDir::new("b"));
        let root = DevPaths::new().root;
        let (a, b) = (
            DevPaths::in_dev_dir(root.clone(), &a.0),
            DevPaths::in_dev_dir(root, &b.0),
        );
        assert_ne!(a.data, b.data);
        let session = session_in(&a);

        assert_eq!(request_ui(&a), Ok(UiRequest::Requested));
        assert!(!b.ui_request.exists(), "another session is not asked");
        let error = request_ui(&b).unwrap_err();
        assert!(error.contains(&b.data.display().to_string()), "{error}");
        assert!(!b.ui_request.exists());

        // Only the supervisor of that session takes the request, once.
        assert!(take_ui_request(&a, &session.id));
        assert!(!take_ui_request(&a, &session.id));
        // A request left for an earlier session is dropped.
        std::fs::write(&a.ui_request, "earlier-session").unwrap();
        assert!(!take_ui_request(&a, &session.id));
        assert!(!a.ui_request.exists());
    }

    #[test]
    fn a_running_app_is_not_started_again() {
        let dir = TempDir::new("running");
        let paths = DevPaths::in_dev_dir(DevPaths::new().root, &dir.0);
        let mut session = session_in(&paths);
        session.replace_process(process::record("ui", std::process::id()).unwrap());
        session.save(&paths).unwrap();
        assert_eq!(request_ui(&paths), Ok(UiRequest::Running));
        assert!(!paths.ui_request.exists());

        // An app that exited (window closed) is requested again.
        let gone = OwnedProcess {
            start_time: session.supervisor.start_time + 1,
            ..session.process("ui").unwrap().clone()
        };
        session.replace_process(gone);
        assert_eq!(
            session.processes.iter().filter(|p| p.role == "ui").count(),
            1
        );
        session.save(&paths).unwrap();
        assert_eq!(request_ui(&paths), Ok(UiRequest::Requested));
        assert!(paths.ui_request.exists());

        // Without a live supervisor nothing can start the app.
        session.supervisor.start_time += 1;
        session.save(&paths).unwrap();
        assert!(request_ui(&paths).is_err());
    }

    #[test]
    fn the_agent_of_a_session_opens_the_app_through_that_session() {
        let dir = TempDir::new("launcher");
        let paths = DevPaths::in_dev_dir(DevPaths::new().root, &dir.0);
        let mut agent = Command::new(paths.agent_binary());
        dev_env(&mut agent, &paths);
        set_ui_launcher(&mut agent, &paths).unwrap();
        let launcher = PathBuf::from(env_of(&agent, DESKTOP_BIN_ENV).unwrap());
        assert!(launcher.is_absolute());
        assert_eq!(launcher, std::env::current_exe().unwrap());
        assert_eq!(
            env_of(&agent, UI_LAUNCHER_ENV),
            Some(dir.0.as_os_str()),
            "the session directory, not the working directory"
        );
        assert_eq!(
            env_of(&agent, deckpilot_ipc::paths::DATA_DIR_ENV),
            Some(paths.data.as_os_str())
        );
        // The launcher checks it was started by that session's agent.
        assert!(launcher_for(&paths, Some(paths.data.as_os_str())).is_ok());
        assert!(launcher_for(&paths, None).is_ok());
        assert!(launcher_for(&paths, Some(OsStr::new("/elsewhere/data"))).is_err());
        assert_eq!(paths.dev_dir(), dir.0.as_path());
    }

    #[test]
    fn the_app_uses_the_session_data_and_an_optional_extra_configuration() {
        let dir = TempDir::new("ui");
        let paths = DevPaths::in_dev_dir(DevPaths::new().root, &dir.0);
        let args = |command: &Command| -> Vec<String> {
            command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        let plain = ui_command_with(&paths, None);
        assert_eq!(args(&plain), UI_ARGS);
        assert_eq!(plain.get_current_dir(), Some(paths.root.as_path()));
        assert_eq!(
            env_of(&plain, deckpilot_ipc::paths::DATA_DIR_ENV),
            Some(paths.data.as_os_str())
        );
        assert_eq!(
            env_of(&plain, "DECKPILOT_NO_AGENT_AUTOSTART"),
            Some(OsStr::new("1"))
        );
        // The app never gets the agent's launcher.
        assert_eq!(env_of(&plain, UI_LAUNCHER_ENV), None);
        let preview = ui_command_with(&paths, Some("/p/tauri.preview.conf.json".into()));
        assert_eq!(
            args(&preview)[UI_ARGS.len()..],
            ["--config", "/p/tauri.preview.conf.json"]
        );
        assert_eq!(args(&ui_command_with(&paths, Some("".into()))), UI_ARGS);
    }
}
