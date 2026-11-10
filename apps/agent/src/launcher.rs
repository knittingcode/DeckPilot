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
    let path = desktop_executable().ok_or("the DeckPilot configuration app was not found")?;
    spawn_detached(Command::new(&path)).map_err(|e| format!("cannot start {}: {e}", path.display()))
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
