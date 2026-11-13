//! The `system` feature module: opens applications, web pages, files and
//! folders, runs commands and reports whether programs are running.
//!
//! Everything machine-specific comes from logical resources bound on this
//! computer: an action refers to `application` or `repository`, never to a
//! path, so a workflow works on every machine once its resources are bound.
//!
//! Safety:
//!
//! - Nothing goes through a shell. Opening uses the operating system's
//!   launcher (`open`, `xdg-open`, `rundll32`) or starts the application
//!   directly ([`deckpilot_core::launch`]); commands are started with an
//!   argument list.
//! - Running commands is off until the user allows it on this computer
//!   (the `allowCommands` setting, which is never exported or imported).
//!   The program must be a bound resource, so an imported workflow cannot
//!   name one.
//! - Commands get a minimal environment, a time limit and bounded output;
//!   their output is available to later nodes but never kept in history.
//!
//! Error codes are listed in [`codes`].

pub mod actions;
pub mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use deckpilot_core::launch::{Launcher, SystemLauncher};
use deckpilot_core::{
    ActionHandler, ActionParameters, Capability, FeatureModule, ModuleId, ModuleManifest,
    ParameterKind, ParameterSpec, ParameterValue, Portability, SettingScope,
};

pub const MODULE_ID: &str = "system";
/// Module setting: user-chosen commands may run on this computer.
pub const ALLOW_COMMANDS: &str = "allowCommands";

/// Stable error codes of the module's actions.
pub mod codes {
    /// The URL is not a valid `http`, `https` or `mailto` address.
    pub const INVALID_URL: &str = "invalid-url";
    /// A relative path leaves its resource (`..`) or is absolute.
    pub const INVALID_PATH: &str = "invalid-path";
    /// Arguments are too many, too long or contain invalid characters.
    pub const INVALID_ARGUMENTS: &str = "invalid-arguments";
    /// The resolved file, folder or program does not exist.
    pub const NOT_FOUND: &str = "not-found";
    /// Running commands is not allowed on this computer.
    pub const COMMANDS_DISABLED: &str = "commands-disabled";
    /// The command finished with a non-zero exit code.
    pub const EXIT_NONZERO: &str = "exit-nonzero";
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

#[derive(Debug)]
pub struct SystemModule {
    manifest: ModuleManifest,
    launcher: Arc<dyn Launcher>,
}

impl SystemModule {
    pub fn new() -> Self {
        Self::with_launcher(Arc::new(SystemLauncher))
    }

    /// The module with another launcher, e.g. one that records plans in
    /// tests instead of opening windows.
    pub fn with_launcher(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "System".into(),
                description: "Opens applications, web pages, files and folders, runs commands \
                              and checks whether programs are running."
                    .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![
                    Capability::MachineResources,
                    Capability::OpenExternal,
                    Capability::ProcessExecution,
                    Capability::ProcessInspect,
                ],
                config: vec![ParameterSpec {
                    key: ALLOW_COMMANDS.into(),
                    label: "Allow running commands".into(),
                    description: Some(
                        "Lets “Run command” start the programs bound as resources on this \
                         computer. Off by default; never exported or imported."
                            .into(),
                    ),
                    kind: ParameterKind::Boolean,
                    required: true,
                    default: Some(ParameterValue::Boolean(false)),
                }],
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::from([(
                        ALLOW_COMMANDS.to_owned(),
                        SettingScope::MachineSpecific,
                    )]),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
                variables: Vec::new(),
            },
            launcher,
        }
    }
}

impl Default for SystemModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for SystemModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        let allow_commands = config.boolean(ALLOW_COMMANDS).unwrap_or(false);
        actions::handlers(&self.launcher, allow_commands)
    }
}
