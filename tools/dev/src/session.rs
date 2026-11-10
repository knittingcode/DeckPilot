//! Paths and the persisted state of a development session.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::process::{self, OwnedProcess};

/// Locations used by the development environment. Everything lives in
/// `.dev/` at the repository root, which Git ignores.
#[derive(Debug, Clone)]
pub struct DevPaths {
    pub root: PathBuf,
    /// Isolated agent/app data directory (`DECKPILOT_DATA_DIR`).
    pub data: PathBuf,
    pub logs: PathBuf,
    /// Example project folders referenced by the seeded resource bindings.
    pub workspace: PathBuf,
    pub session: PathBuf,
    pub stop_request: PathBuf,
}

impl DevPaths {
    pub fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = root.canonicalize().unwrap_or(root);
        let dev = root.join(".dev");
        Self {
            data: dev.join("data"),
            logs: dev.join("logs"),
            workspace: dev.join("workspace"),
            session: dev.join("session.json"),
            stop_request: dev.join("stop-requested"),
            root,
        }
    }

    /// Paths for an isolated data directory outside `.dev`, e.g. for the
    /// release validation (`deckpilot-dev validation`).
    pub fn isolated(dir: &Path) -> Self {
        let dev = Self::new();
        Self {
            data: dir.to_path_buf(),
            logs: dir.join("logs"),
            workspace: dir.join("workspace"),
            session: dir.join("session.json"),
            stop_request: dir.join("stop-requested"),
            root: dev.root,
        }
    }

    pub fn create(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data)?;
        std::fs::create_dir_all(&self.logs)?;
        std::fs::create_dir_all(&self.workspace)
    }

    pub fn agent_paths(&self) -> deckpilot_ipc::AgentPaths {
        deckpilot_ipc::AgentPaths::in_dir(&self.data)
    }

    /// Refuses to run if the development data directory would be the user's
    /// real configuration.
    pub fn ensure_isolated(&self) -> Result<(), String> {
        let production = deckpilot_ipc::AgentPaths::resolve_default()
            .map(|p| p.data_dir)
            .ok();
        if production.is_some_and(|dir| same_path(&dir, &self.data)) {
            return Err("the development data directory is the production one".into());
        }
        Ok(())
    }

    pub fn agent_binary(&self) -> PathBuf {
        self.root
            .join("target/debug")
            .join(format!("deckpilot-agent{}", std::env::consts::EXE_SUFFIX))
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub started_at: u64,
    pub supervisor: OwnedProcess,
    pub processes: Vec<OwnedProcess>,
    /// Device simulation passed to the agent.
    pub devices: String,
}

impl Session {
    pub fn load(paths: &DevPaths) -> Option<Self> {
        let text = std::fs::read_to_string(&paths.session).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, paths: &DevPaths) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let temporary = paths.session.with_extension("json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(temporary, &paths.session)
    }

    pub fn remove(paths: &DevPaths) {
        let _ = std::fs::remove_file(&paths.session);
        let _ = std::fs::remove_file(&paths.stop_request);
    }

    pub fn supervisor_alive(&self) -> bool {
        process::is_alive(&self.supervisor)
    }

    pub fn process(&self, role: &str) -> Option<&OwnedProcess> {
        self.processes.iter().find(|p| p.role == role)
    }
}
