//! The `docker` feature module: Docker availability, containers, Compose
//! projects (status, up, down, restart) and bounded logs.
//!
//! Every command names its Docker context (`docker --context <name>`), the
//! one configured for the action, for the module, or Docker's current
//! context, so it is always clear which Docker is meant; contexts that
//! point to remote engines work like the local one. Compose projects come
//! from resources: the `compose` resource of the run's project, of an
//! explicitly chosen project of the same profile, or one given to the
//! action.
//!
//! State-changing actions (`compose-up`, `compose-down`,
//! `compose-restart`) are off until allowed on this computer
//! (`allowStateChanges`, never exported or imported). `compose down` never
//! removes volumes or images, and no destructive flag is ever added
//! implicitly. There are no Swarm operations.

pub mod actions;
pub mod cli;

use std::collections::BTreeMap;
use std::time::Duration;

use deckpilot_core::{
    ActionHandler, ActionParameters, Capability, FeatureModule, ModuleId, ModuleManifest,
    ParameterKind, ParameterSpec, ParameterValue, Portability, SettingScope,
};

pub const MODULE_ID: &str = "docker";
/// Module setting: the `docker` program (path or name on `PATH`).
pub const EXECUTABLE: &str = "dockerExecutable";
/// Module setting: the Docker context to use; Docker's current one when
/// empty.
pub const CONTEXT: &str = "context";
/// Module setting: Compose projects may be started, stopped and restarted.
pub const ALLOW_STATE_CHANGES: &str = "allowStateChanges";
/// Module setting: the project resource that is the Compose project.
pub const COMPOSE_ROLE: &str = "composeRole";
/// Module setting: time limit of Compose changes, in seconds.
pub const TIMEOUT: &str = "timeoutSeconds";

/// Stable error codes of the module's actions.
pub mod codes {
    /// The `docker` program was not found.
    pub const DOCKER_NOT_FOUND: &str = "docker-not-found";
    /// The Docker engine of the context cannot be reached.
    pub const DAEMON_UNAVAILABLE: &str = "daemon-unavailable";
    /// The engine refused access (e.g. not in the `docker` group).
    pub const PERMISSION_DENIED: &str = "permission-denied";
    /// The Docker context does not exist.
    pub const CONTEXT_NOT_FOUND: &str = "context-not-found";
    /// `docker compose` (Compose v2) is not installed.
    pub const COMPOSE_UNAVAILABLE: &str = "compose-unavailable";
    /// No Compose file was found for the project.
    pub const COMPOSE_FILE_NOT_FOUND: &str = "compose-file-not-found";
    /// The container, service or resource does not exist.
    pub const NOT_FOUND: &str = "not-found";
    /// A context, container, service or project name is invalid.
    pub const INVALID_NAME: &str = "invalid-name";
    /// Changing Compose projects is not allowed on this computer.
    pub const CHANGES_DISABLED: &str = "changes-disabled";
    /// The explicit project is not one of the profile's.
    pub const PROJECT_NOT_IN_PROFILE: &str = "project-not-in-profile";
    /// Docker reported an error.
    pub const DOCKER_FAILED: &str = "docker-failed";
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

/// The module's settings.
#[derive(Debug, Clone)]
pub struct DockerSettings {
    pub executable: String,
    /// `None`: Docker's current context, asked for at run time.
    pub context: Option<String>,
    pub allow_changes: bool,
    pub compose_role: String,
    pub timeout: Duration,
}

pub fn settings(config: &ActionParameters) -> DockerSettings {
    let text = |key: &str| {
        config
            .text(key)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    DockerSettings {
        executable: text(EXECUTABLE).unwrap_or_else(|| "docker".into()),
        context: text(CONTEXT),
        allow_changes: config.boolean(ALLOW_STATE_CHANGES).unwrap_or(false),
        compose_role: text(COMPOSE_ROLE).unwrap_or_else(|| "compose".into()),
        timeout: Duration::from_secs(
            config
                .integer(TIMEOUT)
                .unwrap_or(300)
                .clamp(10, 1800)
                .unsigned_abs(),
        ),
    }
}

fn text_setting(key: &str, label: &str, description: &str, default: Option<&str>) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: label.into(),
        description: Some(description.into()),
        kind: ParameterKind::Text {
            multiline: false,
            max_length: Some(1024),
        },
        required: default.is_some(),
        default: default.map(|d| ParameterValue::Text(d.into())),
    }
}

#[derive(Debug)]
pub struct DockerModule {
    manifest: ModuleManifest,
}

impl DockerModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "Docker".into(),
                description: "Docker availability, containers, Compose projects and logs, in \
                              the Docker context you choose."
                    .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![
                    Capability::ProfileRead,
                    Capability::MachineResources,
                    Capability::ProcessExecution,
                ],
                config: vec![
                    text_setting(
                        EXECUTABLE,
                        "Docker program",
                        "A path to docker, or empty to use docker found on PATH. Only for this \
                         computer.",
                        None,
                    ),
                    text_setting(
                        CONTEXT,
                        "Docker context",
                        "The context commands use, e.g. a remote engine. Empty: Docker's \
                         current context. Only for this computer.",
                        None,
                    ),
                    ParameterSpec {
                        key: ALLOW_STATE_CHANGES.into(),
                        label: "Allow starting and stopping services".into(),
                        description: Some(
                            "Lets the Compose up, down and restart actions change containers on \
                             this computer's Docker context. Off by default; never exported or \
                             imported. Down never removes volumes."
                                .into(),
                        ),
                        kind: ParameterKind::Boolean,
                        required: true,
                        default: Some(ParameterValue::Boolean(false)),
                    },
                    text_setting(
                        COMPOSE_ROLE,
                        "Compose resource",
                        "The project resource with the Compose file or its folder, used when \
                         an action names none.",
                        Some("compose"),
                    ),
                    ParameterSpec {
                        key: TIMEOUT.into(),
                        label: "Time limit for Compose changes (s)".into(),
                        description: None,
                        kind: ParameterKind::Integer {
                            min: Some(10),
                            max: Some(1800),
                        },
                        required: true,
                        default: Some(ParameterValue::Integer(300)),
                    },
                ],
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::from([
                        (EXECUTABLE.to_owned(), SettingScope::MachineSpecific),
                        (CONTEXT.to_owned(), SettingScope::MachineSpecific),
                        (
                            ALLOW_STATE_CHANGES.to_owned(),
                            SettingScope::MachineSpecific,
                        ),
                        (COMPOSE_ROLE.to_owned(), SettingScope::Portable),
                        (TIMEOUT.to_owned(), SettingScope::Portable),
                    ]),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
            },
        }
    }
}

impl Default for DockerModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for DockerModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        actions::handlers(&std::sync::Arc::new(settings(config)))
    }
}
