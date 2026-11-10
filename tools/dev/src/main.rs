//! `deckpilot-dev`: starts, inspects and stops a DeckPilot development
//! session. Used by the Makefile and `pnpm dev`; not part of the product.
//!
//! A session consists of the background agent (with simulated devices next
//! to any connected hardware) and the configuration app in Tauri
//! development mode with Vite HMR. Both use an isolated data directory in
//! `.dev/data`, so the user's real configuration is never touched.

mod logs;
mod package;
mod process;
mod release;
mod seed;
mod session;
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
  up [--foreground]      Start the agent and the app with live reload
     [--devices <list>]  Simulated devices: mk2,xl,plus (default) or none
  down                   Stop the session started by `up`
  restart                down + up
  status                 Show the session and the agent state
  logs [-f] [-n <lines>] [agent|ui|supervisor]
                         Show session logs (follow with -f)
  agent                  Run the agent in the foreground (dev data)
  ui                     Run the app with HMR in the foreground (dev data)
  seed [--reset]         Fill the dev database with demonstration data
  clean [--data]         Remove logs and session files (and dev data)
  check-versions [--tag <tag>]
                         Verify that all version numbers match
  package [-- <tauri build args>]
                         Build installers including the agent
  verify-release <dir>   Check a release's latest.json and artifact signatures
                         (public key from DECKPILOT_UPDATER_PUBKEY)";

/// The configuration app in development mode.
const UI_ARGS: &[&str] = &["--filter", "@deckpilot/desktop", "tauri", "dev"];
const DEFAULT_DEVICES: &str = "mk2,xl,plus";

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
fn supervise(paths: &DevPaths, devices: &str, foreground: bool) -> Result<(), String> {
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

    let mut ui_command = Command::new(program("pnpm"));
    ui_command.args(UI_ARGS);
    dev_env(&mut ui_command, paths);
    let mut ui = spawn_logged(&mut ui_command, "ui", paths, foreground)?;
    if let Some(owned) = process::record("ui", ui.id()) {
        session.processes.push(owned);
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
            println!("the configuration app exited ({status}); the agent keeps running");
            ui_running = false;
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

fn up(paths: &DevPaths, devices: &str, foreground: bool) -> Result<(), String> {
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
    if seed::seed(paths).map_err(|e| e.to_string())? {
        println!("seeded the development database with demonstration data");
    }
    build_agent(paths)?;

    if foreground {
        return supervise(paths, devices, true);
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    command.args(["supervise", "--devices", devices]);
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
    seed::seed(paths).map_err(|e| e.to_string())?;
    let mut command = Command::new("cargo");
    command
        .args(["run", "-p", "deckpilot-agent", "--"])
        .args(agent_args(devices));
    dev_env(&mut command, paths);
    let status = command.status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("agent exited with {status}"))
    }
}

fn foreground_ui(paths: &DevPaths) -> Result<(), String> {
    paths.ensure_isolated()?;
    let mut command = Command::new(program("pnpm"));
    command.args(UI_ARGS);
    dev_env(&mut command, paths);
    let status = command.status().map_err(|e| e.to_string())?;
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
    let paths = DevPaths::new();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let devices = option(args, "--devices").unwrap_or_else(|| DEFAULT_DEVICES.to_owned());
    match args.first().map(String::as_str) {
        Some("doctor") => doctor(),
        Some("up") => up(&paths, &devices, flag("--foreground")),
        Some("supervise") => supervise(&paths, &devices, false),
        Some("down") => down(&paths),
        Some("restart") => {
            let devices = Session::load(&paths).map_or(devices, |s| s.devices);
            down(&paths)?;
            up(&paths, &devices, false)
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
        Some("seed") => {
            if flag("--reset") {
                if Session::load(&paths).is_some_and(|s| s.supervisor_alive()) {
                    return Err("stop the session before resetting its data".into());
                }
                let _ = std::fs::remove_dir_all(&paths.data);
            }
            let seeded = seed::seed(&paths).map_err(|e| e.to_string())?;
            println!(
                "{}",
                if seeded {
                    "seeded demonstration data"
                } else {
                    "already seeded"
                }
            );
            Ok(())
        }
        Some("clean") => clean(&paths, flag("--data")),
        Some("check-versions") => {
            let version = versions::check(&paths.root, option(args, "--tag").as_deref())?;
            println!("all versions are {version}");
            Ok(())
        }
        Some("package") => {
            let extra: Vec<String> = args
                .iter()
                .skip_while(|a| *a != "--")
                .skip(1)
                .cloned()
                .collect();
            package::package(&paths.root, &extra)
        }
        Some("verify-release") => {
            let dir = args
                .get(1)
                .ok_or("expected the directory with the release files")?;
            let key = std::env::var("DECKPILOT_UPDATER_PUBKEY")
                .map_err(|_| "DECKPILOT_UPDATER_PUBKEY is not set".to_owned())?;
            let version = versions::check(&paths.root, None)?;
            let verified = release::verify(std::path::Path::new(dir), &key, &version)?;
            for line in verified {
                println!("verified {line}");
            }
            Ok(())
        }
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
