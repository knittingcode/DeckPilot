//! The `git` feature module: reads the state of a project's repository —
//! status, branch, changes, last commit and how it compares with its
//! remote — and opens it.
//!
//! The repository is a folder resource: the one given to the action, or
//! the `repository` resource of the run's project or of an explicitly
//! chosen project of the same profile. Running an action never changes
//! DeckPilot's active project.
//!
//! Nothing here changes a repository. Git runs without a shell, prompts or
//! optional locks (`git --no-optional-locks`, so even `status` does not
//! refresh the index), with a repository's `core.fsmonitor` program
//! switched off and a minimal environment. There are no commit, reset,
//! clean, checkout or push actions. Local actions never use the network;
//! `git.remote-status` does only when asked to refresh, and then with
//! `git ls-remote`, which leaves the repository as it is.

pub mod actions;
pub mod porcelain;
pub mod repo;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use deckpilot_core::launch::{Launcher, SystemLauncher};
use deckpilot_core::{
    ActionHandler, ActionParameters, Capability, FeatureModule, ModuleId, ModuleManifest,
    ParameterKind, ParameterSpec, ParameterValue, Portability, SettingScope,
};

pub const MODULE_ID: &str = "git";
/// Module setting: the `git` program (path or name on `PATH`).
pub const EXECUTABLE: &str = "gitExecutable";
/// Module setting: the project resource that is the repository.
pub const REPOSITORY_ROLE: &str = "repositoryRole";
/// Module setting: time limit of network operations, in seconds.
pub const NETWORK_TIMEOUT: &str = "networkTimeoutSeconds";

/// Stable error codes of the module's actions.
pub mod codes {
    /// The repository folder does not exist.
    pub const NOT_FOUND: &str = "not-found";
    /// The folder is not a Git repository.
    pub const NOT_A_REPOSITORY: &str = "not-a-repository";
    /// Git refuses the repository because another user owns it.
    pub const UNSAFE_REPOSITORY: &str = "unsafe-repository";
    /// The `git` program was not found.
    pub const GIT_NOT_FOUND: &str = "git-not-found";
    /// Git reported an error.
    pub const GIT_FAILED: &str = "git-failed";
    /// The explicit project is not one of the profile's.
    pub const PROJECT_NOT_IN_PROFILE: &str = "project-not-in-profile";
    /// The branch has no commits yet.
    pub const NO_COMMITS: &str = "no-commits";
    /// The upstream's remote is not configured.
    pub const NO_REMOTE: &str = "no-remote";
    /// Contacting the remote failed.
    pub const NETWORK_FAILED: &str = "network-failed";
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

#[derive(Debug)]
pub struct GitModule {
    manifest: ModuleManifest,
    launcher: Arc<dyn Launcher>,
}

impl GitModule {
    pub fn new() -> Self {
        Self::with_launcher(Arc::new(SystemLauncher))
    }

    /// The module with another launcher for `git.open-repository`.
    pub fn with_launcher(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "Git".into(),
                description: "Status, branch, changes, last commit and remote state of project \
                              repositories. Never changes a repository."
                    .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![
                    Capability::ProfileRead,
                    Capability::MachineResources,
                    Capability::ProcessExecution,
                    Capability::OpenExternal,
                ],
                config: vec![
                    ParameterSpec {
                        key: EXECUTABLE.into(),
                        label: "Git program".into(),
                        description: Some(
                            "A path to git, or empty to use git found on PATH. Only for this \
                             computer."
                                .into(),
                        ),
                        kind: ParameterKind::Text {
                            multiline: false,
                            max_length: Some(1024),
                        },
                        required: false,
                        default: None,
                    },
                    ParameterSpec {
                        key: REPOSITORY_ROLE.into(),
                        label: "Repository resource".into(),
                        description: Some(
                            "The project resource used when an action names no repository.".into(),
                        ),
                        kind: ParameterKind::Text {
                            multiline: false,
                            max_length: Some(64),
                        },
                        required: true,
                        default: Some(ParameterValue::Text(repo::REPOSITORY.into())),
                    },
                    ParameterSpec {
                        key: NETWORK_TIMEOUT.into(),
                        label: "Network time limit (s)".into(),
                        description: Some("For refreshing the remote state.".into()),
                        kind: ParameterKind::Integer {
                            min: Some(5),
                            max: Some(120),
                        },
                        required: true,
                        default: Some(ParameterValue::Integer(20)),
                    },
                ],
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::from([
                        (EXECUTABLE.to_owned(), SettingScope::MachineSpecific),
                        (REPOSITORY_ROLE.to_owned(), SettingScope::Portable),
                        (NETWORK_TIMEOUT.to_owned(), SettingScope::Portable),
                    ]),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
            },
            launcher,
        }
    }
}

impl Default for GitModule {
    fn default() -> Self {
        Self::new()
    }
}

/// The module's settings from its validated configuration.
pub fn settings(config: &ActionParameters) -> repo::GitSettings {
    repo::GitSettings {
        executable: config
            .text(EXECUTABLE)
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .unwrap_or("git")
            .to_owned(),
        role: config
            .text(REPOSITORY_ROLE)
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .unwrap_or(repo::REPOSITORY)
            .to_owned(),
        network_timeout: Duration::from_secs(
            config
                .integer(NETWORK_TIMEOUT)
                .unwrap_or(20)
                .clamp(5, 120)
                .unsigned_abs(),
        ),
    }
}

impl FeatureModule for GitModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        actions::handlers(&Arc::new(settings(config)), &self.launcher)
    }
}
