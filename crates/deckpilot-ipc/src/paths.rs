//! Filesystem locations shared by the agent and the configuration app.

use std::io;
use std::path::{Path, PathBuf};

use crate::endpoint::Endpoint;

/// Application identifier; also the name of the data directory. Must match
/// the Tauri `identifier` so both programs use the same database.
pub const APP_IDENTIFIER: &str = "io.github.knittingcode.deckpilot";

/// Environment variable that overrides the data directory, e.g. to keep
/// development data separate.
pub const DATA_DIR_ENV: &str = "DECKPILOT_DATA_DIR";

/// Argument with which the configuration app's executable runs the agent
/// in its place. Used for AppImages, whose files only exist while the
/// AppImage runs: see [`appimage`].
pub const RUN_AGENT_ARG: &str = "--agent";

/// The AppImage file this process runs from, on Linux.
///
/// An AppImage mounts its contents at a new temporary path on every run
/// (`/tmp/.mount_…`) and unmounts it when its main process exits. Paths
/// inside it must therefore not be recorded (autostart) and programs that
/// outlive the process must not be started from it: they are started
/// through the AppImage file instead (`$APPIMAGE`, and [`RUN_AGENT_ARG`] for
/// the agent). `None` outside an AppImage, including when only the
/// variables were inherited by a program that does not run from it.
pub fn appimage() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    appimage_from(
        std::env::var_os("APPIMAGE").map(PathBuf::from),
        std::env::var_os("APPDIR").map(PathBuf::from),
        std::env::current_exe().ok(),
    )
    .filter(|path| path.is_file())
}

/// [`appimage`] for given values of `APPIMAGE`, `APPDIR` and the current
/// executable: the AppImage file if both are absolute and the executable
/// lies inside the mounted directory.
pub fn appimage_from(
    appimage: Option<PathBuf>,
    appdir: Option<PathBuf>,
    exe: Option<PathBuf>,
) -> Option<PathBuf> {
    let (appimage, appdir, exe) = (appimage?, appdir?, exe?);
    (appimage.is_absolute() && appdir.is_absolute() && exe.starts_with(&appdir)).then_some(appimage)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPaths {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    /// Held locked by the running agent; guarantees a single instance.
    pub lock_file: PathBuf,
    /// Random token clients must present; readable only by the user.
    pub token_file: PathBuf,
    pub log_dir: PathBuf,
}

impl AgentPaths {
    /// Uses `DECKPILOT_DATA_DIR` if set, otherwise the platform data
    /// directory (`~/.local/share`, `~/Library/Application Support`,
    /// `%APPDATA%`) joined with [`APP_IDENTIFIER`].
    pub fn resolve() -> io::Result<Self> {
        if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
            return Ok(Self::in_dir(PathBuf::from(dir)));
        }
        let base = dirs::data_dir().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no data directory for this user")
        })?;
        Ok(Self::in_dir(base.join(APP_IDENTIFIER)))
    }

    /// The platform data directory, ignoring `DECKPILOT_DATA_DIR`.
    pub fn resolve_default() -> io::Result<Self> {
        let base = dirs::data_dir().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no data directory for this user")
        })?;
        Ok(Self::in_dir(base.join(APP_IDENTIFIER)))
    }

    pub fn in_dir(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            database: data_dir.join("deckpilot.db"),
            lock_file: data_dir.join("agent.lock"),
            token_file: data_dir.join("agent.token"),
            log_dir: data_dir.join("logs"),
            data_dir,
        }
    }

    /// The IPC endpoint belonging to this data directory.
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::for_data_dir(&self.data_dir)
    }

    /// Creates the data and log directories if needed.
    pub fn create_dirs(&self) -> io::Result<()> {
        create_private_dir(&self.data_dir)?;
        std::fs::create_dir_all(&self.log_dir)
    }
}

/// Creates a directory accessible only by the current user (on Unix).
pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_of_data_directory() {
        let paths = AgentPaths::in_dir("/data/deckpilot");
        assert_eq!(paths.database, Path::new("/data/deckpilot/deckpilot.db"));
        assert_eq!(paths.lock_file, Path::new("/data/deckpilot/agent.lock"));
        assert_eq!(paths.token_file, Path::new("/data/deckpilot/agent.token"));
        assert_eq!(paths.log_dir, Path::new("/data/deckpilot/logs"));
    }

    #[test]
    fn the_appimage_is_used_only_for_programs_running_from_it() {
        let some = |p: &str| Some(PathBuf::from(p));
        let file = "/home/u/Apps/DeckPilot.AppImage";
        // The agent inside the mounted AppImage.
        assert_eq!(
            appimage_from(
                some(file),
                some("/tmp/.mount_DeckPiAbC123"),
                some("/tmp/.mount_DeckPiAbC123/usr/bin/deckpilot-agent"),
            ),
            some(file)
        );
        // Variables inherited by a program installed elsewhere (a .deb).
        assert_eq!(
            appimage_from(
                some(file),
                some("/tmp/.mount_DeckPiAbC123"),
                some("/usr/bin/deckpilot-agent"),
            ),
            None
        );
        // Not in an AppImage, or incomplete or relative values.
        assert_eq!(
            appimage_from(None, None, some("/usr/bin/deckpilot-agent")),
            None
        );
        assert_eq!(
            appimage_from(some(file), None, some("/tmp/.mount_X/usr/bin/a")),
            None
        );
        assert_eq!(
            appimage_from(
                some("DeckPilot.AppImage"),
                some("/tmp/.mount_X"),
                some("/tmp/.mount_X/usr/bin/a"),
            ),
            None
        );
    }
}
