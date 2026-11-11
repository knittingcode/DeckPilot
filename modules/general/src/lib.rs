//! The `general` module: action types that do not belong to a specific
//! feature and have no side effects on the host system.

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    ExecutionContext, FeatureModule, ModuleId, ModuleManifest, ParameterKind, ParameterSpec,
    Portability,
};

pub const MODULE_ID: &str = "general";
pub const LOG: &str = "general.log";
pub const WAIT: &str = "general.wait";
/// Longest wait, in seconds.
pub const MAX_WAIT_SECONDS: i64 = 300;

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn log_id() -> ActionTypeId {
    ActionTypeId::new(LOG).expect("the action type id is valid")
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn wait_id() -> ActionTypeId {
    ActionTypeId::new(WAIT).expect("the action type id is valid")
}

#[derive(Debug)]
pub struct GeneralModule {
    manifest: ModuleManifest,
}

impl GeneralModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "General".into(),
                description:
                    "Basic actions such as writing a message to the execution log or waiting."
                        .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![Capability::ProfileRead],
                config: Vec::new(),
                dependencies: Vec::new(),
                enabled_by_default: true,
                // Only the enabled state travels; there are no settings.
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::new(),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
            },
        }
    }
}

impl Default for GeneralModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for GeneralModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, _config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        vec![Box::new(LogMessage::new()), Box::new(Wait::new())]
    }
}

/// Produces a message, expanding `{profile}`, `{project.key}` and
/// `{project.name}` from the execution context.
#[derive(Debug)]
pub struct LogMessage(ActionType);

impl LogMessage {
    pub fn new() -> Self {
        Self(ActionType {
            id: log_id(),
            name: "Log message".into(),
            description: "Writes a message to the execution log.".into(),
            category: "General".into(),
            parameters: vec![ParameterSpec {
                key: "message".into(),
                label: "Message".into(),
                description: Some(
                    "Supports {profile}, {project.key} and {project.name} placeholders.".into(),
                ),
                kind: ParameterKind::Text {
                    multiline: true,
                    max_length: Some(1000),
                },
                required: true,
                default: None,
            }],
            requires_project: false,
            node: deckpilot_core::graph::NodeSpec::default(),
        })
    }
}

impl Default for LogMessage {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for LogMessage {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let template = parameters.text("message").unwrap_or_default();
        let (profile, project) = match (context.profile(), context.project()) {
            (Ok(profile), Ok(project)) => (profile, project),
            (Err(denied), _) | (_, Err(denied)) => return denied.into(),
        };
        let mut output = template.replace("{profile}", &profile.name);
        if output.contains("{project.") {
            let Some(project) = project else {
                return ActionResult::failure(
                    "message refers to the active project, but none is active",
                );
            };
            output = output
                .replace("{project.key}", project.key.as_str())
                .replace("{project.name}", &project.name);
        }
        ActionResult::success(output)
    }
}

/// Waits a number of seconds. The reference implementation of the workflow
/// node contract (see `docs/modules.md`): it declares its outputs, timeout,
/// retry safety, idempotency and that it can be cancelled; it waits through
/// [`ExecutionContext::wait`], so a cancellation or its timeout stops it at
/// once; and it reports progress.
#[derive(Debug)]
pub struct Wait(ActionType);

impl Wait {
    pub fn new() -> Self {
        Self(ActionType {
            id: wait_id(),
            name: "Wait".into(),
            description: "Waits a number of seconds; stops at once when the workflow is cancelled."
                .into(),
            category: "General".into(),
            parameters: vec![ParameterSpec {
                key: "seconds".into(),
                label: "Seconds".into(),
                description: Some(format!("1 to {MAX_WAIT_SECONDS}")),
                kind: ParameterKind::Integer {
                    min: Some(1),
                    max: Some(MAX_WAIT_SECONDS),
                },
                required: true,
                default: Some(deckpilot_core::ParameterValue::Integer(5)),
            }],
            requires_project: false,
            node: deckpilot_core::graph::NodeSpec {
                version: 1,
                icon: Some("timer".into()),
                outputs: vec![deckpilot_core::graph::OutputSpec {
                    key: "waitedMs".into(),
                    label: "Waited (ms)".into(),
                    kind: deckpilot_core::graph::ValueKind::Integer,
                    always: true,
                    sensitive: false,
                }],
                // The longest wait plus a margin.
                timeout_ms: u32::try_from(MAX_WAIT_SECONDS * 1000 + 10_000).unwrap_or(u32::MAX),
                retry_safe: true,
                idempotent: true,
                changes_context: false,
                cancellable: true,
                effect: deckpilot_core::graph::ActionEffect::ReadOnly,
            },
        })
    }
}

impl Default for Wait {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for Wait {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let seconds = parameters
            .integer("seconds")
            .unwrap_or(1)
            .clamp(1, MAX_WAIT_SECONDS);
        let total = std::time::Duration::from_secs(seconds.unsigned_abs());
        let step = std::time::Duration::from_secs(1);
        let begun = std::time::Instant::now();
        let mut waited = std::time::Duration::ZERO;
        while waited < total {
            let percent = u8::try_from(waited.as_millis() * 100 / total.as_millis()).unwrap_or(100);
            context.report_progress(
                Some(percent),
                format!("{} of {seconds} s", waited.as_secs()),
            );
            if !context.wait(step.min(total.saturating_sub(waited))) {
                // Stopped by a cancellation or the timeout; the engine
                // reports which.
                return ActionResult::failure(format!(
                    "stopped after {} ms",
                    begun.elapsed().as_millis()
                ));
            }
            waited = begun.elapsed().min(total);
        }
        ActionResult::success(format!("waited {seconds} s"))
            .with_data("waitedMs", begun.elapsed().as_millis().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{MachineConfig, Os, ParameterValue, Profile, Project, ProjectKey};

    #[test]
    fn log_message_expands_placeholders() {
        let mut profile = Profile::new("Work");
        profile
            .projects
            .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
        let machine = MachineConfig::new(Os::Linux);
        let handler = LogMessage::new();
        let params = handler
            .definition()
            .normalize(&ActionParameters::new().with(
                "message",
                ParameterValue::Text("{profile}: {project.key} / {project.name}".into()),
            ))
            .unwrap();
        let mut context = ExecutionContext::new(&profile, Some(&profile.projects[0]), &machine);
        let result = handler.execute(&params, &context);
        assert_eq!(result.message, "Work: SHOP / Shop");

        context = ExecutionContext::new(&profile, None, &machine);
        assert!(!handler.execute(&params, &context).is_success());
    }

    #[test]
    fn action_types_are_namespaced() {
        let module = GeneralModule::new();
        for handler in module.action_handlers(&ActionParameters::new()) {
            assert_eq!(
                ModuleId::owner_of(&handler.definition().id),
                Some(module_id())
            );
        }
    }
}

#[cfg(test)]
mod wait_tests {
    use std::time::{Duration, Instant};

    use deckpilot_core::{
        CancellationToken, ExecutionControl, MachineConfig, Os, ParameterValue, Profile, Progress,
    };

    use super::*;

    fn parameters(seconds: i64) -> ActionParameters {
        Wait::new()
            .definition()
            .normalize(&ActionParameters::new().with("seconds", ParameterValue::Integer(seconds)))
            .unwrap()
    }

    #[test]
    fn waits_and_reports_progress_and_its_output() {
        let profile = Profile::new("Work");
        let machine = MachineConfig::new(Os::Linux);
        let token = CancellationToken::new();
        let reports = std::sync::Mutex::new(Vec::new());
        let progress = |p: Progress| reports.lock().unwrap().push(p);
        let control = ExecutionControl::new(&token, &progress);
        let context = ExecutionContext::new(&profile, None, &machine).with_control(&control);
        let result = Wait::new().execute(&parameters(1), &context);
        assert!(result.is_success(), "{}", result.message);
        assert!(result.data["waitedMs"].parse::<u64>().unwrap() >= 1000);
        assert_eq!(reports.lock().unwrap()[0].percent, Some(0));
    }

    #[test]
    fn stops_at_once_when_cancelled() {
        let profile = Profile::new("Work");
        let machine = MachineConfig::new(Os::Linux);
        let token = CancellationToken::new();
        let progress = |_: Progress| {};
        let control = ExecutionControl::new(&token, &progress);
        let context = ExecutionContext::new(&profile, None, &machine).with_control(&control);
        let canceller = {
            let token = token.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                token.cancel();
            })
        };
        let begun = Instant::now();
        let result = Wait::new().execute(&parameters(MAX_WAIT_SECONDS), &context);
        canceller.join().unwrap();
        assert!(!result.is_success());
        assert!(begun.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn declares_the_node_contract() {
        let wait = Wait::new();
        let spec = &wait.definition().node;
        assert!(spec.cancellable && spec.retry_safe && spec.idempotent);
        assert!(!spec.changes_context);
        assert_eq!(spec.outputs[0].key, "waitedMs");
        assert!(u64::from(spec.timeout_ms) > MAX_WAIT_SECONDS.unsigned_abs() * 1000);
    }
}
