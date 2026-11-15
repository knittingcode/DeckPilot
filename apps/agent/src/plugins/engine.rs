//! Plugin actions in the workflow engine (design §3.7): one
//! [`PluginModule`] per installed plugin, registered in the agent's
//! module registry under the plugin id, with one [`PluginActionHandler`]
//! per declared action (`<plugin id>.<action>`).
//!
//! A handler never runs plugin code. It hands the run to the plugin worker,
//! which prepares it and queues it on the plugin's lane (one thread per
//! host); the handler waits for the answer on the thread that called it:
//! a workflow node's execution thread, or the agent's thread for one key
//! press or `runAction` ([`deckpilot_core::ActionHandler::runs_detached`]:
//! never the service task). A cancelled run, or one past its deadline,
//! raises the invocation's abort signal: a capability call that is running
//! (an HTTP request) is stopped at once and its result dropped, further
//! calls are refused, and the host has 2 s to end the invocation before it
//! is stopped. The action is cancelled whatever the plugin still answers.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use deckpilot_core::graph::{MAX_TIMEOUT_MS, NodeSpec};
use deckpilot_core::variables::{VariableSource, module_namespace};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, ExecutionContext,
    FeatureModule, ModuleId, ModuleManifest, ParameterValue, Portability,
};
use deckpilot_plugin_abi::{ActionOutcome, ParamValue, Params};
use deckpilot_plugin_manifest::Action;
use deckpilot_plugin_supervisor::AbortSignal;
use tracing::warn;

use super::variables::{PluginVariableSource, PublishedVariables, declarations};
use super::worker::{EngineFailure, Invoker};

/// How long the host may take to answer after an abort (the lane's
/// `abort_grace`), plus a margin for the answer to travel back.
const ABORT_WAIT: Duration = Duration::from_millis(2_500);
/// How often a waiting handler looks at its run's cancellation.
const POLL: Duration = Duration::from_millis(25);
/// On top of the action's time limit: queueing behind other invocations
/// of the same plugin, starting the host.
const QUEUE_MARGIN: Duration = Duration::from_secs(30);

/// An installed plugin as the engine sees it, from the plugin store.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CatalogPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// The active version's actions.
    pub actions: Vec<Action>,
    /// The active version's variables (`contributes.variables`).
    pub variables: Vec<deckpilot_plugin_manifest::Variable>,
    /// Why its actions cannot run (disabled, broken, quarantined, this
    /// platform, a permission not granted); `None`: they can.
    pub unavailable: Option<String>,
}

/// A plugin as a feature module: no capabilities (a plugin reaches
/// nothing of DeckPilot's but through the broker), no settings, pages,
/// windows or migrations. Its variables are those it declares, in its
/// namespace (`plugins.<publisherCamel>.<nameCamel>.…`), resolved from
/// the values it published (`variables.rs`). Enabled exactly when its
/// actions can run; while it is not, its variables are unavailable.
pub(crate) struct PluginModule {
    manifest: ModuleManifest,
    plugin: CatalogPlugin,
    invoker: Invoker,
    source: Option<Arc<dyn VariableSource>>,
}

impl PluginModule {
    /// `None` when the plugin id is not a module id (never, for an
    /// installed plugin: both grammars are the same).
    pub fn new(
        plugin: CatalogPlugin,
        invoker: Invoker,
        values: PublishedVariables,
    ) -> Option<Self> {
        let id = ModuleId::new(plugin.id.clone()).ok()?;
        let namespace = module_namespace(&id);
        let variables = declarations(&namespace, &plugin.variables);
        let source = (!variables.is_empty()).then(|| {
            Arc::new(PluginVariableSource::new(
                plugin.id.clone(),
                namespace,
                values,
            )) as Arc<dyn VariableSource>
        });
        Some(Self {
            source,
            manifest: ModuleManifest {
                id,
                version: plugin.version.clone(),
                name: plugin.name.clone(),
                description: plugin.description.clone(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: Vec::new(),
                config: Vec::new(),
                dependencies: Vec::new(),
                enabled_by_default: plugin.unavailable.is_none(),
                portability: Portability::default(),
                migrations: Vec::new(),
                windows: Vec::new(),
                variables,
            },
            plugin,
            invoker,
        })
    }
}

impl FeatureModule for PluginModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, _config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        self.plugin
            .actions
            .iter()
            .filter_map(|action| {
                match PluginActionHandler::new(&self.plugin.id, action, self.invoker.clone()) {
                    Ok(handler) => Some(Box::new(handler) as Box<dyn ActionHandler>),
                    Err(problem) => {
                        warn!(plugin = %self.plugin.id, action = %action.id, %problem,
                            "a plugin action cannot be offered");
                        None
                    }
                }
            })
            .collect()
    }

    fn variable_source(&self) -> Option<Arc<dyn VariableSource>> {
        self.source.clone()
    }

    fn unavailable_reason(&self) -> Option<String> {
        self.plugin
            .unavailable
            .as_ref()
            .map(|why| format!("{} ({}): {why}", self.plugin.name, self.plugin.id))
    }
}

/// The action type of a manifest action. The manifest's parameter and node
/// schemas mirror the core's one to one (`deckpilot-plugin-manifest`), so
/// they convert through JSON; anything that does not convert is refused.
pub(crate) fn action_type(plugin: &str, action: &Action) -> Result<ActionType, String> {
    let id = ActionTypeId::new(format!("{plugin}.{}", action.id)).map_err(|e| e.to_string())?;
    let parameters = action
        .parameters
        .iter()
        .map(|parameter| {
            // Through `core_spec`: the kind's constraints (`maxLength`, …)
            // are camel case in the manifest and snake case in the core.
            serde_json::to_value(parameter)
                .map_err(|e| e.to_string())
                .and_then(super::data::core_spec)
                .map_err(|e| format!("parameter {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut node: NodeSpec = match &action.node {
        Some(node) => serde_json::to_value(node)
            .and_then(serde_json::from_value)
            .map_err(|e| format!("node: {e}"))?,
        None => NodeSpec::default(),
    };
    // Every plugin action can be stopped (the abort signal), and none
    // can change the run's project (plugins have no project context).
    node.cancellable = true;
    node.changes_context = false;
    node.timeout_ms = u32::try_from(
        action
            .node
            .as_ref()
            .map_or(30_000, deckpilot_plugin_manifest::Node::timeout_ms),
    )
    .unwrap_or(MAX_TIMEOUT_MS)
    .clamp(1, MAX_TIMEOUT_MS);
    Ok(ActionType {
        id,
        name: action.name.clone(),
        description: action.description.clone(),
        category: action.category.clone(),
        parameters,
        requires_project: action.requires_project.unwrap_or(false),
        deprecated: action.deprecated.unwrap_or(false),
        node,
    })
}

/// Runs one plugin action through the plugin worker.
pub(crate) struct PluginActionHandler {
    definition: ActionType,
    plugin: String,
    action: String,
    outputs: BTreeSet<String>,
    timeout: Duration,
    invoker: Invoker,
}

impl PluginActionHandler {
    fn new(plugin: &str, action: &Action, invoker: Invoker) -> Result<Self, String> {
        let definition = action_type(plugin, action)?;
        Ok(Self {
            timeout: Duration::from_millis(u64::from(definition.node.timeout_ms)),
            outputs: super::lane::declared_outputs(action),
            definition,
            plugin: plugin.to_owned(),
            action: action.id.clone(),
            invoker,
        })
    }

    fn run(&self, parameters: &ActionParameters, context: &ExecutionContext<'_>) -> ActionResult {
        let params = match params(parameters) {
            Ok(params) => params,
            Err(problem) => return ActionResult::error("invalid-parameters", problem),
        };
        let abort = AbortSignal::new();
        let Some(answer) = self.invoker.invoke(
            self.plugin.clone(),
            self.action.clone(),
            params,
            abort.clone(),
        ) else {
            return ActionResult::error(
                "plugin-unavailable",
                "plugins are not running in this agent",
            );
        };
        let deadline = Instant::now() + self.timeout + QUEUE_MARGIN;
        let reason = loop {
            match answer.recv_timeout(POLL) {
                Ok(reply) => return self.result(reply),
                Err(RecvTimeoutError::Disconnected) => {
                    return ActionResult::error(
                        "plugin-host-failed",
                        "the plugin worker stopped before the action finished",
                    );
                }
                Err(RecvTimeoutError::Timeout) => {
                    if context.is_cancelled() {
                        break "the action was cancelled";
                    }
                    if Instant::now() >= deadline {
                        break "the action took longer than its time limit";
                    }
                }
            }
        };
        // Abort: a capability call that is running stops at once and the
        // host gets its grace to finish. Whatever the plugin answers now is
        // late and never applied (design §6.3: discarded); waiting only
        // lets a cooperative host end the invocation before the action is
        // reported, so the next one finds the lane free.
        abort.abort();
        let _ = answer.recv_timeout(ABORT_WAIT);
        ActionResult::error(
            "cancelled",
            format!("{reason}; the plugin was asked to stop and its result is discarded"),
        )
    }

    fn result(&self, reply: Result<ActionOutcome, EngineFailure>) -> ActionResult {
        match reply {
            Ok(ActionOutcome::Ok { message, data }) => {
                let mut result = ActionResult::success(
                    message.unwrap_or_else(|| format!("{} finished", self.definition.name)),
                );
                for (key, value) in data {
                    if self.outputs.contains(&key) {
                        result = result.with_data(key, text(&value));
                    }
                }
                result
            }
            Ok(ActionOutcome::Skipped { message }) => ActionResult::skipped(
                message.unwrap_or_else(|| format!("{} skipped", self.definition.name)),
            ),
            Ok(ActionOutcome::Failed { code, message, .. }) => ActionResult::error(
                code.clone(),
                message.unwrap_or_else(|| format!("{} failed: {code}", self.definition.name)),
            ),
            Err(EngineFailure { code, message }) => ActionResult::error(code, message),
        }
    }
}

impl ActionHandler for PluginActionHandler {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        catch_unwind(AssertUnwindSafe(|| self.run(parameters, context))).unwrap_or_else(|_| {
            warn!(plugin = %self.plugin, action = %self.action, "a plugin action handler panicked");
            ActionResult::error(
                "plugin-host-failed",
                "the plugin action failed inside DeckPilot",
            )
        })
    }

    /// Always: the plugin runs in its host process, possibly for minutes.
    fn runs_detached(&self) -> bool {
        true
    }
}

/// Resolved parameters as ABI 1 values. Plugins see texts, integers and
/// booleans; a project parameter is the project's id.
fn params(parameters: &ActionParameters) -> Result<Params, String> {
    parameters
        .iter()
        .map(|(key, value)| {
            let value = match value {
                ParameterValue::Text(text) | ParameterValue::Choice(text) => {
                    ParamValue::String(text.clone())
                }
                ParameterValue::Integer(i) => ParamValue::Integer(*i),
                ParameterValue::Boolean(b) => ParamValue::Bool(*b),
                ParameterValue::Project(project) => ParamValue::String(project.to_string()),
                ParameterValue::Resource(_)
                | ParameterValue::Reference(_)
                | ParameterValue::Template(_) => {
                    return Err(format!(
                        "parameter `{key}` is not a value a plugin can receive"
                    ));
                }
            };
            Ok((key.clone(), value))
        })
        .collect()
}

/// An output value as text (the engine's outputs are texts).
fn text(value: &ParamValue) -> String {
    match value {
        ParamValue::String(s) => s.clone(),
        ParamValue::Integer(i) => i.to_string(),
        ParamValue::Number(n) => n.to_string(),
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::List(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> deckpilot_plugin_manifest::Manifest {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/examples/hello-weather/manifest.json"
        ))
        .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn manifest_actions_become_action_types() {
        let manifest = sample();
        let actions = manifest.contributes.actions();
        let greet = action_type("acme.hello-weather", &actions[0]).unwrap();
        assert_eq!(greet.id.as_str(), "acme.hello-weather.greet");
        assert_eq!(
            ModuleId::owner_of(&greet.id).unwrap().as_str(),
            "acme.hello-weather"
        );
        assert_eq!(greet.parameters.len(), 2);
        // The camel-case constraint reached the core's kind.
        assert_eq!(
            greet.parameters[0].kind,
            deckpilot_core::ParameterKind::Text {
                multiline: false,
                max_length: Some(64)
            }
        );
        assert!(greet.parameters[0].required);
        assert_eq!(greet.node.timeout_ms, 5_000);
        assert!(greet.node.cancellable && !greet.node.changes_context);
        assert_eq!(greet.node.outputs[0].key, "greeting");
        // The defaults are the core's: normalizing fills them in.
        let normalized = greet
            .normalize(&ActionParameters::new().with("name", ParameterValue::Text("Ada".into())))
            .unwrap();
        assert_eq!(
            normalized.get("style"),
            Some(&ParameterValue::Choice("plain".into()))
        );
        let weather = action_type("acme.hello-weather", &actions[1]).unwrap();
        assert!(
            weather
                .normalize(
                    &ActionParameters::new().with("city", ParameterValue::Choice("paris".into()))
                )
                .is_err()
        );
    }

    #[test]
    fn values_reach_the_plugin_as_abi_values() {
        let parameters = ActionParameters::new()
            .with("name", ParameterValue::Text("Ada".into()))
            .with("count", ParameterValue::Integer(2))
            .with("on", ParameterValue::Boolean(true))
            .with("city", ParameterValue::Choice("berlin".into()));
        let params = params(&parameters).unwrap();
        assert_eq!(params["name"], ParamValue::String("Ada".into()));
        assert_eq!(params["count"], ParamValue::Integer(2));
        assert_eq!(params["on"], ParamValue::Bool(true));
        assert_eq!(params["city"], ParamValue::String("berlin".into()));
        assert_eq!(text(&ParamValue::Number(21.5)), "21.5");
        assert_eq!(text(&ParamValue::List(vec![ParamValue::Integer(1)])), "[1]");
    }
}
