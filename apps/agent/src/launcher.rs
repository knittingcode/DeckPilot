//! Starts the configuration application.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Overrides the configuration app executable.
pub const DESKTOP_BIN_ENV: &str = "DECKPILOT_DESKTOP_BIN";

const CANDIDATES: &[&str] = &["deckpilot-desktop", "DeckPilot"];

/// Locates the configuration app: `DECKPILOT_DESKTOP_BIN`, or an executable
/// next to the agent (development builds and installed bundles).
pub fn desktop_executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(DESKTOP_BIN_ENV).map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    CANDIDATES
        .iter()
        .map(|name| dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
        .find(|path| path.is_file())
}

/// Starts the configuration app detached from the agent, so neither process
/// terminates the other.
pub fn open_desktop() -> Result<(), String> {
    let path = desktop_program()?;
    spawn_detached(Command::new(&path)).map_err(|e| format!("cannot start {}: {e}", path.display()))
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

pub fn spawn_detached(mut command: Command) -> std::io::Result<()> {
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
    command.spawn().map(|_| ())
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
}
