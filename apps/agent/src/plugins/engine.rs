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
    ActionControls, ActionHandler, ActionParameters, ActionResult, ActionTitle, ActionType,
    ActionTypeId, ButtonState, ExecutionContext, FeatureModule, ModuleId, ModuleManifest,
    ParameterValue, Portability, RefreshPolicy, StateError,
};
use deckpilot_core::{RunOrigin, WorkflowCause};
use deckpilot_plugin_abi::capability::{
    RunCause, RunContext, RunSummary as AbiRunSummary, RunTrigger,
};
use deckpilot_plugin_abi::state::{STATE_NOT_PERMITTED, STATE_UNSUPPORTED};
use deckpilot_plugin_abi::{ActionOutcome, ParamValue, Params, StateOutcome};
use deckpilot_plugin_manifest::Action;
use deckpilot_plugin_supervisor::AbortSignal;
use tracing::warn;

use super::executor::project::ProjectSwitch;
use super::executor::resources::InvocationResources;
use super::key_state::{Asked, KeyId, Turn};
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
/// On top of a state call's time limit: starting the host.
const STATE_MARGIN: Duration = Duration::from_secs(8);

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
    /// platform, a permission not granted, a required dependency not
    /// there); `None`: they can.
    pub unavailable: Option<String>,
    /// Why single actions cannot run although the plugin can: an optional
    /// dependency they name (`requiresDependencies`) is not there.
    pub action_unavailable: std::collections::BTreeMap<String, String>,
    /// The ids of its required dependencies (plugins or built-in
    /// modules): the module registry then refuses to disable a built-in
    /// module an enabled plugin requires.
    pub requires: Vec<String>,
    /// Per action whose name is a translation (plugin API 1.6): every
    /// text it is named in, in any of the plugin's locales. A key titled
    /// with one of them shows the action's default name, which the agent
    /// draws in the app's language (`Agent::localize_faces`).
    pub default_names: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    /// The visual assets keys may draw now (plugin API 1.8): those of an
    /// enabled plugin whose files match the integrity record, decoded.
    pub visual_assets: Vec<(deckpilot_core::AssetId, deckpilot_render::LibraryAsset)>,
    /// It holds `projectContext` (plugin API 1.9): with it, and while it
    /// can run, its host-published variables have a value (X7).
    pub project_context: bool,
    /// It holds `resources` (plugin API 1.9): its invocations resolve the
    /// project's roles (`resource.resolve`, X2) even without a `resource`
    /// parameter, so its module reads the profile and the bindings.
    pub resources: bool,
}

impl CatalogPlugin {
    /// The name in the app's language of the action of type `action_type`
    /// when `title` is one of its default names, else `None` (a title the
    /// user wrote, or a plain name).
    pub fn default_name(&self, action_type: &str, title: &str) -> Option<String> {
        let action = action_type.strip_prefix(&self.id)?.strip_prefix('.')?;
        if !self.default_names.get(action)?.contains(title) {
            return None;
        }
        self.actions
            .iter()
            .find(|a| a.id == action)
            .map(|a| a.name.clone())
    }
}

/// A plugin as a feature module: no capabilities (a plugin reaches
/// nothing of DeckPilot's but through the broker; only a plugin with
/// `resource` parameters has its handler read the profile and the
/// bindings, to resolve them for the invocation), no settings, pages,
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
        // A plugin that stopped running (disabled, updated, uninstalled)
        // starts afresh when it runs again: no remembered state, no
        // back-off.
        if plugin.unavailable.is_some() {
            invoker.book().forget(&plugin.id);
        }
        let namespace = module_namespace(&id);
        let variables = declarations(&namespace, &plugin.variables);
        let host_context = plugin.unavailable.is_none() && plugin.project_context;
        let source = (!variables.is_empty()).then(|| {
            Arc::new(
                PluginVariableSource::new(plugin.id.clone(), namespace, values)
                    .with_host_context(&plugin.variables, host_context),
            ) as Arc<dyn VariableSource>
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
                capabilities: capabilities(&plugin),
                config: Vec::new(),
                // Only while it runs: a plugin that cannot run (for lack
                // of one of them, say) does not hold its dependencies.
                dependencies: if plugin.unavailable.is_none() {
                    plugin
                        .requires
                        .iter()
                        .filter_map(|id| ModuleId::new(id.clone()).ok())
                        .collect()
                } else {
                    Vec::new()
                },
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

/// What a plugin's module may use of the core: the profile and the
/// bindings for one that resolves resources; the profile and the project
/// context for one holding `projectContext` (X6), whose switches in a
/// workflow run are its results' `ContextEffect`s, which the action
/// registry refuses from any other module.
fn capabilities(plugin: &CatalogPlugin) -> Vec<deckpilot_core::Capability> {
    use deckpilot_core::Capability;
    let mut capabilities = Vec::new();
    if plugin.resources || takes_resources(&plugin.actions) {
        capabilities.extend([Capability::ProfileRead, Capability::MachineResources]);
    }
    if plugin.project_context {
        if !capabilities.contains(&Capability::ProfileRead) {
            capabilities.push(Capability::ProfileRead);
        }
        capabilities.push(Capability::ProjectContext);
    }
    capabilities
}

impl FeatureModule for PluginModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, _config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        // Keys of a plugin whose actions cannot run show no state: they
        // fall back to how the user designed them.
        let runs = self.plugin.unavailable.is_none();
        self.plugin
            .actions
            .iter()
            .filter_map(|action| {
                match PluginActionHandler::new(&self.plugin.id, action, self.invoker.clone()) {
                    Ok(handler) => {
                        let unmet = self.plugin.action_unavailable.get(&action.id).cloned();
                        let runs = runs && unmet.is_none();
                        Some(Box::new(handler.with_state(action, runs).with_unmet(unmet))
                            as Box<dyn ActionHandler>)
                    }
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
            // Plugin API 1.9's resource parameters (X1): a core resource
            // parameter, which the engine resolves for the invocation and
            // the plugin receives as a handle (`executor/resources.rs`).
            if let deckpilot_plugin_manifest::ParameterKind::Resource { kinds } = &parameter.kind {
                return Ok(resource_spec(parameter, kinds));
            }
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
    // Every plugin action can be stopped (the abort signal). One changes
    // the run's project only when its node declares it (plugin API 1.9,
    // a first-party manifest with `projectContext.select`, X6): its
    // `project.select` in a workflow run is the result's `ContextEffect`.
    node.cancellable = true;
    node.changes_context = action.changes_context();
    node.timeout_ms = u32::try_from(
        action
            .node
            .as_ref()
            .map_or(30_000, deckpilot_plugin_manifest::Node::timeout_ms),
    )
    .unwrap_or(MAX_TIMEOUT_MS)
    .clamp(1, MAX_TIMEOUT_MS);
    let editor_navigation = action
        .editor_navigation
        .as_ref()
        .map(|declared| editor_navigation(declared, &parameters))
        .transpose()?;
    let title = action.title.as_ref().map(action_title);
    Ok(ActionType {
        editor_navigation,
        title,
        id,
        name: action.name.clone(),
        description: action.description.clone(),
        category: action.category.clone(),
        parameters,
        requires_project: action.requires_project.unwrap_or(false),
        deprecated: action.deprecated.unwrap_or(false),
        // The manifest's `controls` (plugin API 1.3), as declared; without
        // them the legacy default, keys only. `device.requires` adds no
        // control kind (`docs/device-controls.md` §4).
        controls: action.controls.as_ref().map(|controls| ActionControls {
            bindings: controls.as_sets(),
        }),
        node,
    })
}

/// A `resource` parameter as the core's: one kind for the editor when the
/// manifest accepts exactly one the core knows (a file is the core's
/// `other`, so any binding is offered for it); the executor checks the
/// declared kinds again when it issues a handle. No default.
fn resource_spec(
    parameter: &deckpilot_plugin_manifest::Parameter,
    kinds: &[deckpilot_plugin_manifest::ResourceKind],
) -> deckpilot_core::ParameterSpec {
    use deckpilot_core::ResourceKind as Core;
    use deckpilot_plugin_manifest::ResourceKind as Declared;
    let kind = match kinds {
        [Declared::Directory] => Some(Core::Directory),
        [Declared::Application] => Some(Core::Application),
        [Declared::DockerEnvironment] => Some(Core::DockerEnvironment),
        _ => None,
    };
    deckpilot_core::ParameterSpec {
        key: parameter.key.clone(),
        label: parameter.label.clone(),
        description: parameter.description.clone(),
        kind: deckpilot_core::ParameterKind::Resource { kind },
        required: parameter.required,
        default: None,
    }
}

/// Whether an action of the plugin declares a `resource` parameter: its
/// module then reads the profile and this computer's bindings, so the
/// engine can resolve them for the invocation (never the plugin itself).
fn takes_resources(actions: &[Action]) -> bool {
    actions.iter().any(|action| {
        action.parameters.iter().any(|p| {
            matches!(
                p.kind,
                deckpilot_plugin_manifest::ParameterKind::Resource { .. }
            )
        })
    })
}

/// The manifest's `title` (plugin API 1.8) as the core's: data only, values
/// the manifest's validation already refused are left out.
fn action_title(declared: &deckpilot_plugin_manifest::ActionTitle) -> ActionTitle {
    use deckpilot_core::{TitleAlign, TitleFeature};
    let position = declared
        .position
        .as_deref()
        .and_then(|position| match position {
            "top" => Some(TitleAlign::Top),
            "middle" => Some(TitleAlign::Middle),
            "bottom" => Some(TitleAlign::Bottom),
            _ => None,
        });
    let supports = declared.supports.as_ref().map(|features| {
        let mut known: Vec<TitleFeature> = features
            .iter()
            .filter_map(|feature| match feature.as_str() {
                "visibility" => Some(TitleFeature::Visibility),
                "position" => Some(TitleFeature::Position),
                "font" => Some(TitleFeature::Font),
                "style" => Some(TitleFeature::Style),
                "color" => Some(TitleFeature::Color),
                "layout" => Some(TitleFeature::Layout),
                _ => None,
            })
            .collect();
        known.sort();
        known.dedup();
        known
    });
    ActionTitle {
        default: declared
            .default
            .as_ref()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty()),
        dynamic: declared.dynamic.unwrap_or(false),
        position,
        supports,
    }
}

/// The manifest's `editorNavigation` (plugin API 1.7) as the core's, checked
/// again against the core's allowlist and the action's own parameters: data
/// only, which the editor maps to its own navigation; the plugin is never
/// called for it.
fn editor_navigation(
    declared: &deckpilot_plugin_manifest::EditorNavigation,
    parameters: &[deckpilot_core::ParameterSpec],
) -> Result<deckpilot_core::EditorNavigation, String> {
    let operation = deckpilot_core::NavigationOperation::parse(&declared.operation)
        .ok_or_else(|| format!("`{}` is not a navigation operation", declared.operation))?;
    let navigation = deckpilot_core::EditorNavigation {
        operation,
        target_parameter: declared.target_parameter.clone(),
        page_parameter: declared.page_parameter.clone(),
    };
    navigation
        .check(parameters)
        .map_err(|e| format!("editorNavigation: {e}"))?;
    Ok(navigation)
}

/// Runs one plugin action through the plugin worker.
pub(crate) struct PluginActionHandler {
    definition: ActionType,
    plugin: String,
    action: String,
    outputs: BTreeSet<String>,
    timeout: Duration,
    invoker: Invoker,
    /// How often keys bound to the action are asked for their state;
    /// `None`: never (no `state` in the manifest, or the plugin's actions
    /// cannot run).
    state: Option<RefreshPolicy>,
    /// Why this action cannot run although its plugin can: a dependency it
    /// names is missing or disabled. It then fails at once, never reaching
    /// the plugin.
    unmet: Option<String>,
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
            state: None,
            unmet: None,
        })
    }

    fn with_unmet(mut self, unmet: Option<String>) -> Self {
        self.unmet = unmet;
        self
    }

    /// With the action's `state` from the manifest, while the plugin's
    /// actions run (`runs`).
    fn with_state(mut self, action: &Action, runs: bool) -> Self {
        self.state = action
            .state
            .as_ref()
            .filter(|_| runs)
            .map(super::key_state::policy);
        self
    }

    /// The state of a key bound to this action with `parameters`, within
    /// the budgets of `key_state.rs`. Runs on a provider thread of the
    /// agent's live key states, never on the agent's task.
    fn key_state(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ButtonState, StateError> {
        let params =
            params(parameters).map_err(|problem| StateError::new("invalid-parameters", problem))?;
        // Resource parameters never reach a state call (no handles
        // there), but keys bound to different resources are different keys.
        let references: Vec<&ParameterValue> = parameters
            .iter()
            .filter_map(|(_, value)| matches!(value, ParameterValue::Resource(_)).then_some(value))
            .collect();
        // A key whose binding follows the active project is another key
        // in another project.
        let scope = key_scope(&self.plugin, &self.definition, parameters, context);
        let key = KeyId {
            plugin: self.plugin.clone(),
            action: self.action.clone(),
            params: if references.is_empty() && scope.is_none() {
                serde_json::to_string(&params).unwrap_or_default()
            } else {
                serde_json::to_string(&(&params, references, &scope)).unwrap_or_default()
            },
        };
        let book = self.invoker.book();
        // The plugin's keys take turns; state calls are short, so the
        // turn comes soon, or the key keeps what it showed.
        let patience = Instant::now() + super::key_state::TURN_WAIT;
        loop {
            match book.begin(&key, Instant::now()) {
                Turn::Ask => break,
                Turn::Paused(shown) => return shown,
                Turn::Busy(shown) => {
                    if context.is_cancelled() || Instant::now() >= patience {
                        return shown;
                    }
                    std::thread::sleep(POLL);
                }
            }
        }
        // The turn is always given back, whatever happens while asking.
        let asked = catch_unwind(AssertUnwindSafe(|| self.ask(params, scope, context)))
            .unwrap_or_else(|_| {
                Asked::Failed(StateError::new(
                    "plugin-host-failed",
                    "the key's state could not be read",
                ))
            });
        book.finish(&key, asked, Instant::now())
    }

    fn ask(&self, params: Params, scope: Option<String>, context: &ExecutionContext<'_>) -> Asked {
        let failed = |code: &str, message: &str| Asked::Failed(StateError::new(code, message));
        let abort = AbortSignal::new();
        let Some(answer) = self.invoker.key_state(
            self.plugin.clone(),
            self.action.clone(),
            params,
            scope,
            abort.clone(),
        ) else {
            return Asked::Quiet;
        };
        // The call's own limit, and room for a host that has to start.
        let deadline = Instant::now() + super::key_state::STATE_CALL_TIMEOUT + STATE_MARGIN;
        loop {
            match answer.recv_timeout(POLL) {
                Ok(Ok(StateOutcome::Ok(state))) => {
                    return Asked::State(super::key_state::button_state(state));
                }
                // Not granted, or nothing to report for this action: the
                // key stays as the user designed it.
                Ok(Ok(StateOutcome::Error(failure)))
                    if failure.code == STATE_NOT_PERMITTED || failure.code == STATE_UNSUPPORTED =>
                {
                    return Asked::Quiet;
                }
                Ok(Ok(StateOutcome::Error(failure))) => {
                    return failed(&failure.code, &failure.message);
                }
                Ok(Err(EngineFailure { code, .. })) if code == "plugin-busy" => {
                    return Asked::Busy;
                }
                Ok(Err(EngineFailure { code, message })) => return failed(&code, &message),
                Err(RecvTimeoutError::Disconnected) => {
                    return failed("plugin-host-failed", "the plugin worker stopped");
                }
                Err(RecvTimeoutError::Timeout) => {
                    if context.is_cancelled() || Instant::now() >= deadline {
                        // The lane stops the call; its late answer is
                        // dropped with the channel.
                        abort.abort();
                        return failed("timeout", "the key's state took too long");
                    }
                }
            }
        }
    }

    fn run(&self, parameters: &ActionParameters, context: &ExecutionContext<'_>) -> ActionResult {
        let params = match params(parameters) {
            Ok(params) => params,
            Err(problem) => return ActionResult::error("invalid-parameters", problem),
        };
        let mut resources = InvocationResources::of(parameters, context).with_urls(
            &self.definition,
            parameters,
            context.provenance(),
        );
        // What a `project.select` of a workflow run selects (X6), for the
        // result.
        let switch = ProjectSwitch::for_action(self.definition.node.changes_context);
        resources.switch = switch.clone();
        let abort = AbortSignal::new();
        let run = run_context(context).with_key_scope(key_scope(
            &self.plugin,
            &self.definition,
            parameters,
            context,
        ));
        let Some(answer) = self.invoker.invoke(
            self.plugin.clone(),
            self.action.clone(),
            params,
            resources,
            run,
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
                Ok(reply) => return with_switch(self.result(reply), &switch),
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

/// `result` with the run's new project as its `ContextEffect` when the
/// action selected one in a workflow run and succeeded: the engine applies
/// it to the run's project, as a built-in project action's (and refuses it
/// from a module without the project context). Nothing for a failed or
/// skipped action.
fn with_switch(result: ActionResult, switch: &ProjectSwitch) -> ActionResult {
    if !result.is_success() {
        return result;
    }
    match switch
        .target()
        .map(|id| id.parse::<deckpilot_core::ProjectId>())
    {
        Some(Ok(project)) => {
            result.with_effect(deckpilot_core::ContextEffect::SelectProject { project })
        }
        Some(Err(_)) => ActionResult::failure("the plugin selected a project that does not exist"),
        None => result,
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
        if let Some(why) = &self.unmet {
            return ActionResult::error(
                "plugin-unavailable",
                format!("{} is unavailable: {why}", self.definition.name),
            );
        }
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

    fn state_policy(&self, _parameters: &ActionParameters) -> Option<RefreshPolicy> {
        self.state
    }

    fn button_state(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ButtonState, StateError> {
        self.key_state(parameters, context)
    }
}

/// What started the run of `context` and how its workflow run is going,
/// as the plugin may read it (`run.context`, G8): the invocation's own run
/// only, the workflow's name, counts and failed node ids. A run whose
/// origin nobody gave is `other`, which counts as not started by the user.
fn run_context(context: &ExecutionContext<'_>) -> RunContext {
    let summary = context.run_summary().map(|summary| AbiRunSummary {
        workflow: summary.workflow_name.clone(),
        elapsed_ms: summary.elapsed_ms,
        succeeded: summary.succeeded,
        failed: summary.failed,
        skipped: summary.skipped,
        cancelled: summary.cancelled,
        failed_nodes: summary.failed_nodes.clone(),
    });
    let direct = |trigger| RunContext {
        run: summary.clone(),
        ..RunContext::direct(trigger)
    };
    match context.origin() {
        Some(RunOrigin::Key) => direct(RunTrigger::Key),
        Some(RunOrigin::Gesture) => direct(RunTrigger::Gesture),
        Some(RunOrigin::App) => direct(RunTrigger::App),
        Some(RunOrigin::Test) => direct(RunTrigger::Test),
        Some(RunOrigin::Workflow { cause }) => RunContext::workflow(
            match cause {
                WorkflowCause::User => RunCause::User,
                WorkflowCause::Schedule => RunCause::Schedule,
                WorkflowCause::Event => RunCause::Event,
                WorkflowCause::Startup => RunCause::Startup,
                WorkflowCause::WorkSession => RunCause::WorkSession,
                WorkflowCause::Simulated | WorkflowCause::Other => RunCause::Other,
            },
            summary,
        ),
        // A press the app simulated is not the user's direct request: the
        // plugin sees `other`, so dialogs and `url.open` stay refused.
        Some(RunOrigin::Simulated) | None => direct(RunTrigger::Other),
    }
    .bounded()
}

/// Which key this is for its key state ([`RunContext::key_scope`], plugin
/// API 1.9): 128 bits of the SHA-256 of the plugin id, each `resource`
/// parameter of the action with its stored reference (never its path), and
/// the active project's id when a binding follows it (a project role, or a
/// parameter left empty, which the plugin resolves through the project's
/// role). The same for a key's state call and for every run with the same
/// bindings; `None` when the action has no `resource` parameter.
fn key_scope(
    plugin: &str,
    definition: &ActionType,
    parameters: &ActionParameters,
    context: &ExecutionContext<'_>,
) -> Option<String> {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let resources: Vec<&str> = definition
        .parameters
        .iter()
        .filter(|spec| matches!(spec.kind, deckpilot_core::ParameterKind::Resource { .. }))
        .map(|spec| spec.key.as_str())
        .collect();
    if resources.is_empty() {
        return None;
    }
    let mut hash = sha2::Sha256::new();
    hash.update(b"deckpilot.key-scope.v1\0");
    hash.update(plugin.as_bytes());
    let mut follows_project = false;
    for key in resources {
        hash.update(b"\0");
        hash.update(key.as_bytes());
        hash.update(b"=");
        if let Some(ParameterValue::Resource(reference)) = parameters.get(key) {
            follows_project |= matches!(reference, deckpilot_core::ResourceRef::ProjectRole { .. });
            hash.update(serde_json::to_vec(reference).unwrap_or_default());
        } else {
            follows_project = true;
            hash.update(b"unset");
        }
    }
    if follows_project {
        hash.update(b"\0project=");
        if let Ok(Some(project)) = context.project() {
            hash.update(project.id.to_string().as_bytes());
        }
    }
    let digest = hash.finalize();
    Some(
        digest[..16]
            .iter()
            .fold(String::with_capacity(32), |mut out, b| {
                let _ = write!(out, "{b:02x}");
                out
            }),
    )
}

/// Resolved parameters as ABI 1 values. Plugins see texts, integers and
/// booleans; a project parameter is the project's id. A resource
/// parameter is left out: it travels as [`InvocationResources`], and the
/// plugin receives a handle for it (`executor/resources.rs`).
fn params(parameters: &ActionParameters) -> Result<Params, String> {
    parameters
        .iter()
        .filter(|(_, value)| !matches!(value, ParameterValue::Resource(_)))
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
        // No `controls` in the manifest: the legacy default, keys only,
        // whatever `device.requires` says.
        assert_eq!(greet.controls, None);
        assert_eq!(*greet.effective_controls(), ActionControls::legacy());
    }

    /// Plugin API 1.3: the manifest's `controls` become the action type's,
    /// as sets; `device.requires` adds nothing.
    #[test]
    fn manifest_controls_become_the_action_types_controls() {
        use deckpilot_device::{ControlKind, InputCapability};
        let manifest = sample();
        let mut action = manifest.contributes.actions()[0].clone();
        action.device = Some(deckpilot_plugin_manifest::ActionDevice {
            requires: Some(vec![deckpilot_plugin_manifest::DeviceFamily::Dials]),
        });
        let legacy = action_type("acme.hello-weather", &action).unwrap();
        assert_eq!(legacy.controls, None);
        assert!(
            !legacy
                .effective_controls()
                .kinds()
                .contains(&ControlKind::Encoder)
        );

        action.controls = Some(
            serde_json::from_value(serde_json::json!({
                "key": ["press", "click", "press"],
                "encoder": ["rotate-counter-clockwise", "rotate-clockwise"],
                "touchZone": ["swipe"]
            }))
            .unwrap(),
        );
        let declared = action_type("acme.hello-weather", &action).unwrap();
        let expected = ActionControls::new()
            .with(
                ControlKind::Key,
                [InputCapability::Press, InputCapability::Click],
            )
            .with(
                ControlKind::Encoder,
                [
                    InputCapability::RotateClockwise,
                    InputCapability::RotateCounterClockwise,
                ],
            )
            .with(ControlKind::TouchZone, [InputCapability::Swipe]);
        assert_eq!(declared.controls, Some(expected));
        // What the manifest validator accepts, the core's rules accept.
        assert_eq!(
            declared.controls.as_ref().unwrap().issues(),
            Vec::<String>::new()
        );
        // It travels in the action type's JSON (IPC, stored definitions).
        let json = serde_json::to_value(&declared).unwrap();
        assert_eq!(
            json["controls"],
            serde_json::json!({"key": ["press", "click"],
                "encoder": ["rotate-clockwise", "rotate-counter-clockwise"],
                "touchZone": ["swipe"]})
        );
    }

    /// Plugin API 1.8: `title` becomes the action type's, data only; the
    /// manifest's names are the core's.
    #[test]
    fn titles_become_the_action_types() {
        use deckpilot_core::{TitleAlign, TitleFeature};
        let manifest = sample();
        let mut action = manifest.contributes.actions()[0].clone();
        assert_eq!(
            action_type("acme.hello-weather", &action).unwrap().title,
            None
        );
        action.title = Some(deckpilot_plugin_manifest::ActionTitle {
            default: Some(" CPU ".into()),
            dynamic: Some(true),
            position: Some("top".into()),
            supports: Some(vec!["color".into(), "position".into()]),
        });
        let title = action_type("acme.hello-weather", &action)
            .unwrap()
            .title
            .unwrap();
        assert_eq!(title.default.as_deref(), Some("CPU"));
        assert!(title.dynamic);
        assert_eq!(title.position, Some(TitleAlign::Top));
        assert_eq!(
            title.supports,
            Some(vec![TitleFeature::Position, TitleFeature::Color])
        );
        // Every name the manifest allows converts.
        for position in deckpilot_plugin_manifest::TITLE_POSITIONS {
            let converted = action_title(&deckpilot_plugin_manifest::ActionTitle {
                default: None,
                dynamic: None,
                position: Some(position.into()),
                supports: Some(
                    deckpilot_plugin_manifest::TITLE_FEATURES
                        .iter()
                        .map(|f| (*f).to_owned())
                        .collect(),
                ),
            });
            assert!(converted.position.is_some(), "{position}");
            assert_eq!(converted.supports.unwrap().len(), 6);
        }
    }

    /// Plugin API 1.7: `editorNavigation` becomes the action type's, data
    /// only, checked again against the core's allowlist and the action's
    /// own text parameters.
    #[test]
    fn editor_navigation_becomes_the_action_types_and_is_checked_again() {
        use deckpilot_core::NavigationOperation;
        // The manifest's allowlist is the core's.
        let manifest_ops: Vec<(&str, bool)> =
            deckpilot_plugin_manifest::EDITOR_NAVIGATION_OPERATIONS.to_vec();
        let core_ops: Vec<(&str, bool)> = NavigationOperation::ALL
            .iter()
            .map(|op| (op.as_str(), op.takes_target()))
            .collect();
        assert_eq!(manifest_ops, core_ops);

        let manifest = sample();
        let mut action = manifest.contributes.actions()[0].clone();
        assert_eq!(
            action_type("acme.hello-weather", &action)
                .unwrap()
                .editor_navigation,
            None
        );
        let declare = |operation: &str, target: Option<&str>| {
            Some(deckpilot_plugin_manifest::EditorNavigation {
                operation: operation.into(),
                target_parameter: target.map(str::to_owned),
                page_parameter: None,
            })
        };
        action.editor_navigation = declare("folder.open", Some("name"));
        let declared = action_type("acme.hello-weather", &action).unwrap();
        assert_eq!(
            declared.editor_navigation,
            Some(deckpilot_core::EditorNavigation::with_target(
                NavigationOperation::FolderOpen,
                "name"
            ))
        );
        let json = serde_json::to_value(&declared).unwrap();
        assert_eq!(
            json["editorNavigation"],
            serde_json::json!({"operation": "folder.open", "targetParameter": "name"})
        );
        // Whatever slipped past the manifest validator is refused here.
        for (operation, target) in [
            ("url.open", None),
            ("folder.open", None),
            ("folder.open", Some("style")),
            ("page.next", Some("name")),
        ] {
            action.editor_navigation = declare(operation, target);
            assert!(
                action_type("acme.hello-weather", &action).is_err(),
                "{operation} {target:?}"
            );
        }
    }

    /// Plugin API 1.9's key scope: the same for every action of a plugin
    /// with the same bindings and for every run, different for another
    /// binding, another plugin and, when a binding follows the active
    /// project, another project; none without `resource` parameters.
    #[test]
    fn the_key_scope_names_the_bindings_of_a_key() {
        use deckpilot_core::{
            MachineConfig, Profile, Project, ProjectKey, ResourceKey, ResourceRef,
        };
        let action = |id: &str| -> Action {
            serde_json::from_value(serde_json::json!({
                "id": id, "name": id, "description": "d", "category": "c",
                "parameters": [{"key": "compose", "label": "C", "required": false,
                    "kind": {"type": "resource", "kinds": ["directory"]}}]
            }))
            .unwrap()
        };
        let definition = |id: &str| action_type("deckpilot.docker", &action(id)).unwrap();
        let fixed = |key: &str| {
            ActionParameters::new().with(
                "compose",
                ParameterValue::Resource(ResourceRef::Fixed {
                    key: ResourceKey::new(key).unwrap(),
                }),
            )
        };
        let profile = Profile::new("P");
        let shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
        let cms = Project::new(ProjectKey::new("CMS").unwrap(), "Cms");
        let machine = MachineConfig::new(deckpilot_core::Os::Linux);
        let in_shop = ExecutionContext::new(&profile, Some(&shop), &machine);
        let in_cms = ExecutionContext::new(&profile, Some(&cms), &machine);
        let scope = |plugin: &str, id: &str, params: &ActionParameters, ctx| {
            key_scope(plugin, &definition(id), params, ctx).unwrap()
        };
        let a = scope(
            "deckpilot.docker",
            "compose-up",
            &fixed("shop.compose"),
            &in_shop,
        );
        assert_eq!(a.len(), 32);
        assert!(
            a.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        // The status and the up key of one project share it, in any project.
        assert_eq!(
            a,
            scope(
                "deckpilot.docker",
                "compose-status",
                &fixed("shop.compose"),
                &in_cms
            )
        );
        assert_ne!(
            a,
            scope(
                "deckpilot.docker",
                "compose-up",
                &fixed("cms.compose"),
                &in_shop
            )
        );
        assert_ne!(
            a,
            scope(
                "deckpilot.other",
                "compose-up",
                &fixed("shop.compose"),
                &in_shop
            )
        );
        // A role, or no binding (the project's role then), follows the
        // active project.
        let role = ActionParameters::new().with(
            "compose",
            ParameterValue::Resource(ResourceRef::ProjectRole {
                role: "compose".into(),
            }),
        );
        for params in [role, ActionParameters::new()] {
            assert_ne!(
                scope("deckpilot.docker", "compose-up", &params, &in_shop),
                scope("deckpilot.docker", "compose-up", &params, &in_cms)
            );
        }
        // No path is part of it, and an action without resources has none.
        let plain: Action = serde_json::from_value(serde_json::json!({
            "id": "status", "name": "s", "description": "d", "category": "c", "parameters": []
        }))
        .unwrap();
        let plain = action_type("deckpilot.docker", &plain).unwrap();
        assert_eq!(
            key_scope(
                "deckpilot.docker",
                &plain,
                &ActionParameters::new(),
                &in_shop
            ),
            None
        );
    }

    #[test]
    fn a_simulated_press_is_not_user_initiated_for_plugins() {
        use deckpilot_core::{MachineConfig, Profile};
        let profile = Profile::new("P");
        let machine = MachineConfig::new(deckpilot_core::Os::Linux);
        let base = ExecutionContext::new(&profile, None, &machine);
        let simulated = run_context(&base.with_origin(Some(RunOrigin::Simulated)));
        assert_eq!(simulated, RunContext::direct(RunTrigger::Other));
        assert!(!simulated.is_user_initiated());
        assert!(!RunOrigin::Simulated.is_user_initiated());
        let workflow = run_context(&base.with_origin(Some(RunOrigin::Workflow {
            cause: WorkflowCause::Simulated,
        })));
        assert_eq!(workflow.cause, Some(RunCause::Other));
        assert!(!workflow.is_user_initiated());
    }

    #[test]
    fn the_run_context_names_the_origin_and_the_own_run_only() {
        use deckpilot_core::{CancellationToken, ExecutionControl, MachineConfig, Profile};
        let profile = Profile::new("P");
        let machine = MachineConfig::new(deckpilot_core::Os::Linux);
        let base = ExecutionContext::new(&profile, None, &machine);
        assert_eq!(run_context(&base), RunContext::direct(RunTrigger::Other));
        assert!(!run_context(&base).is_user_initiated());
        for (origin, trigger) in [
            (RunOrigin::Key, RunTrigger::Key),
            (RunOrigin::Gesture, RunTrigger::Gesture),
            (RunOrigin::App, RunTrigger::App),
            (RunOrigin::Test, RunTrigger::Test),
        ] {
            let context = run_context(&base.with_origin(Some(origin)));
            assert_eq!(context, RunContext::direct(trigger));
            assert!(context.is_user_initiated(), "{origin:?}");
        }
        let token = CancellationToken::new();
        let progress = |_: deckpilot_core::Progress| {};
        let control =
            ExecutionControl::new(&token, &progress).with_summary(deckpilot_core::RunSummary {
                workflow: Some(deckpilot_core::WorkflowId::new()),
                workflow_name: Some("Nightly".into()),
                elapsed_ms: 900,
                succeeded: 2,
                failed: 1,
                skipped: 0,
                cancelled: 0,
                failed_nodes: vec!["deploy".into()],
            });
        let scheduled = base
            .with_control(&control)
            .with_origin(Some(RunOrigin::Workflow {
                cause: WorkflowCause::Schedule,
            }));
        let context = run_context(&scheduled);
        assert_eq!(context.trigger, RunTrigger::Workflow);
        assert_eq!(context.cause, Some(RunCause::Schedule));
        assert!(!context.is_user_initiated());
        let run = context.run.unwrap();
        assert_eq!(run.workflow.as_deref(), Some("Nightly"));
        assert_eq!((run.succeeded, run.failed), (2, 1));
        assert_eq!(run.failed_nodes, ["deploy"]);
        let by_user = run_context(&base.with_control(&control).with_origin(Some(
            RunOrigin::Workflow {
                cause: WorkflowCause::User,
            },
        )));
        assert!(by_user.is_user_initiated());
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

    /// Plugin API 1.9 (X1): a `resource` parameter is a core resource
    /// parameter (one kind for the editor when the manifest names exactly
    /// one the core knows), its value never travels as a plugin value, and
    /// only such a plugin's module reads the profile and the bindings.
    #[test]
    fn resource_parameters_become_core_resources_and_travel_apart() {
        use deckpilot_core::{ResourceKind as Core, ResourceRef};
        let mut action = sample().contributes.actions()[0].clone();
        let declare = |kinds: serde_json::Value| -> deckpilot_plugin_manifest::Parameter {
            serde_json::from_value(serde_json::json!({
                "key": "repository", "label": "Repository", "required": true,
                "kind": {"type": "resource", "kinds": kinds}
            }))
            .unwrap()
        };
        for (kinds, core) in [
            (serde_json::json!(["directory"]), Some(Core::Directory)),
            (serde_json::json!(["application"]), Some(Core::Application)),
            (serde_json::json!(["file"]), None),
            (serde_json::json!(["directory", "file"]), None),
        ] {
            action.parameters = vec![declare(kinds.clone())];
            let declared = action_type("deckpilot.git", &action).unwrap();
            let spec = &declared.parameters[0];
            assert_eq!(
                spec.kind,
                deckpilot_core::ParameterKind::Resource { kind: core },
                "{kinds}"
            );
            assert!(spec.required && spec.default.is_none());
        }
        let parameters = ActionParameters::new()
            .with("name", ParameterValue::Text("Ada".into()))
            .with(
                "repository",
                ParameterValue::Resource(ResourceRef::ProjectRole {
                    role: "repository".into(),
                }),
            );
        let values = params(&parameters).unwrap();
        assert_eq!(values.keys().collect::<Vec<_>>(), ["name"]);

        assert!(takes_resources(std::slice::from_ref(&action)));
        assert!(!takes_resources(sample().contributes.actions()));
    }
}
