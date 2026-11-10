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
}
