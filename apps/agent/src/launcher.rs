//! Starts the configuration application.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Overrides the configuration app executable.
pub const DESKTOP_BIN_ENV: &str = "DECKPILOT_DESKTOP_BIN";

const CANDIDATES: &[&str] = &["deckpilot-desktop", "DeckPilot"];

/// Locates the configuration app: `DECKPILOT_DESKTOP_BIN`, or an executable
/// next to the agent (development builds and installed bundles).
///
/// A development session (`deckpilot-dev up`) sets `DECKPILOT_DESKTOP_BIN`
/// to its own launcher, which starts the app together with its Vite server;
/// the debug app next to the agent would show nothing without that server.
pub fn desktop_executable() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    resolve_desktop(std::env::var_os(DESKTOP_BIN_ENV).map(PathBuf::from), &dir)
}

fn resolve_desktop(override_path: Option<PathBuf>, agent_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = override_path.filter(|path| !path.as_os_str().is_empty()) {
        return path.is_file().then_some(path);
    }
    CANDIDATES
        .iter()
        .map(|name| agent_dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
        .find(|path| path.is_file())
}

/// The app started last, while it may still be starting up.
struct Started {
    child: Child,
    at: Instant,
}

static STARTED: Mutex<Option<Started>> = Mutex::new(None);

/// How long a started app counts as starting: until it connects, the agent
/// cannot ask it to show its window, so further requests in this time
/// would start a second app.
const STARTING: Duration = Duration::from_secs(20);

/// Whether the app started earlier is still starting (running, but not yet
/// connected). Reaps it once it has exited.
fn still_starting(started: &mut Option<Started>, now: Instant) -> bool {
    let Some(previous) = started else {
        return false;
    };
    let running = matches!(previous.child.try_wait(), Ok(None));
    if running && now.duration_since(previous.at) < STARTING {
        return true;
    }
    if !running {
        *started = None;
    }
    false
}

/// Starts the configuration app detached from the agent, so neither process
/// terminates the other. A request while the app started by the previous
/// one is still starting up starts no second app.
pub fn open_desktop() -> Result<(), String> {
    let mut started = STARTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if still_starting(&mut started, Instant::now()) {
        return Ok(());
    }
    let path = desktop_program()?;
    let child = spawn_detached_child(Command::new(&path))
        .map_err(|e| format!("cannot start {}: {e}", path.display()))?;
    *started = Some(Started {
        child,
        at: Instant::now(),
    });
    Ok(())
}

/// What starts the configuration app: inside an AppImage the AppImage file
/// itself (its main program is the app), so that the app gets a mount of
/// its own instead of depending on this agent's; otherwise the executable
/// next to the agent.
fn desktop_program() -> Result<PathBuf, String> {
    program_for_desktop(
        deckpilot_ipc::paths::appimage(),
        desktop_executable().ok_or("the DeckPilot configuration app was not found"),
    )
}

fn program_for_desktop(
    appimage: Option<PathBuf>,
    next_to_agent: Result<PathBuf, &str>,
) -> Result<PathBuf, String> {
    match appimage {
        Some(file) => Ok(file),
        None => next_to_agent.map_err(str::to_owned),
    }
}

/// Starts the configuration app showing only the window `id` (for example
/// the unlock prompt) when no app is running. The process ends when the
/// window closes. `id` was validated by the caller and is passed as a
/// separate argument, never through a shell.
pub fn open_window(id: &str) -> Result<(), String> {
    let path = desktop_program()?;
    let mut command = Command::new(&path);
    command.args(["--window", id]);
    spawn_detached(command).map_err(|e| format!("cannot start {}: {e}", path.display()))
}

pub fn spawn_detached(command: Command) -> std::io::Result<()> {
    spawn_detached_child(command).map(|_| ())
}

fn spawn_detached_child(mut command: Command) -> std::io::Result<Child> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A new process group keeps terminal signals for the parent away.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command.spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_is_started_through_the_appimage_when_the_agent_runs_from_one() {
        let appimage = PathBuf::from("/home/u/Apps/DeckPilot.AppImage");
        assert_eq!(
            program_for_desktop(
                Some(appimage.clone()),
                Ok(PathBuf::from("/tmp/.mount_X/usr/bin/deckpilot-desktop")),
            ),
            Ok(appimage)
        );
        assert_eq!(
            program_for_desktop(None, Ok(PathBuf::from("/usr/bin/deckpilot-desktop"))),
            Ok(PathBuf::from("/usr/bin/deckpilot-desktop"))
        );
        assert!(program_for_desktop(None, Err("not found")).is_err());
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "deckpilot-launcher-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&path, b"").unwrap();
        path
    }

    #[test]
    fn the_app_is_found_next_to_the_agent_in_builds_and_installed_bundles() {
        // target/debug (development build) and Linux packages: the cargo
        // binary name; macOS `.app` (Contents/MacOS) and Windows installs:
        // the product name.
        for name in CANDIDATES {
            let dir = TempDir::new();
            executable(dir.path(), "deckpilot-agent");
            let app = executable(dir.path(), name);
            assert_eq!(resolve_desktop(None, dir.path()), Some(app));
        }
        // Both present: the build name wins, as before.
        let dir = TempDir::new();
        let build = executable(dir.path(), "deckpilot-desktop");
        executable(dir.path(), "DeckPilot");
        assert_eq!(resolve_desktop(None, dir.path()), Some(build));
        // Nothing next to the agent: not found (a directory does not count).
        let empty = TempDir::new();
        std::fs::create_dir(empty.path().join("DeckPilot")).unwrap();
        assert_eq!(resolve_desktop(None, empty.path()), None);
    }

    #[test]
    fn the_override_replaces_the_search_and_must_exist() {
        let dir = TempDir::new();
        let app = executable(dir.path(), "deckpilot-desktop");
        let other = TempDir::new();
        let launcher = executable(other.path(), "deckpilot-dev");
        // A development session's launcher, regardless of what lies next
        // to the agent.
        assert_eq!(
            resolve_desktop(Some(launcher.clone()), dir.path()),
            Some(launcher)
        );
        // A missing override is an error, not a silent fallback to the
        // debug app that cannot show anything without its server.
        assert_eq!(
            resolve_desktop(Some(other.path().join("missing")), dir.path()),
            None
        );
        // An empty variable counts as unset.
        assert_eq!(resolve_desktop(Some(PathBuf::new()), dir.path()), Some(app));
    }

    fn long_running() -> Child {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 30 127.0.0.1 > NUL"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("30");
            command
        };
        command.spawn().unwrap()
    }

    #[test]
    fn a_starting_app_is_not_started_twice() {
        let now = Instant::now();
        let mut started = None;
        assert!(!still_starting(&mut started, now), "nothing started yet");
        started = Some(Started {
            child: long_running(),
            at: now,
        });
        assert!(still_starting(&mut started, now + Duration::from_secs(1)));
        // An app that has not connected for a long time may be stuck: a new
        // request starts another one.
        assert!(!still_starting(&mut started, now + STARTING));
        // An app that exited (closed, crashed) is reaped and started again.
        let previous = started.as_mut().unwrap();
        previous.child.kill().unwrap();
        previous.child.wait().unwrap();
        assert!(!still_starting(&mut started, now));
        assert!(started.is_none());
    }
}
