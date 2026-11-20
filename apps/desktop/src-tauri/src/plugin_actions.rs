//! Action types of installed plugins, as the agent reported them
//! (experimental; IPC protocol 10, capability `plugins`,
//! `docs/marketplace/agent-integration.md`).
//!
//! The agent is the source of truth: every answer of the plugin commands
//! that lists or changes installed plugins is recorded here (see
//! `commands::plugins`), and nothing else adds to it. Plugin actions never
//! run in this process; the definitions only let the editors validate and
//! save actions, workflow nodes and keys that use them, exactly as the
//! agent's engine will (`apps/agent/src/plugins/engine.rs`). Until the
//! frontend asks for the installed plugins (only builds with
//! `VITE_DECKPILOT_PLUGINS=1` do) this stays empty and the registry is
//! the built-in modules' alone.

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionHandler, ActionParameters, ActionRegistry, ActionResult, ActionType, ExecutionContext,
    ModuleId,
};
use deckpilot_ipc::plugins::PluginSummary;

/// `definition` with its editor navigation (plugin API 1.7) only when the
/// host's own rules accept it: an allowlisted operation whose target is one
/// of the action's text parameters. The editor maps it to its own
/// navigation; nothing of the plugin runs for it.
fn checked(definition: &ActionType) -> ActionType {
    let mut definition = definition.clone();
    if definition
        .editor_navigation
        .as_ref()
        .is_some_and(|navigation| navigation.check(&definition.parameters).is_err())
    {
        definition.editor_navigation = None;
    }
    definition
}

/// The installed plugins of the last answer, by plugin id.
#[derive(Debug, Default)]
pub struct PluginActions {
    plugins: BTreeMap<String, PluginSummary>,
}

impl PluginActions {
    /// The installed plugins are exactly `plugins` (a `listPlugins` answer).
    pub fn replace(&mut self, plugins: &[PluginSummary]) {
        self.plugins = plugins
            .iter()
            .map(|plugin| (plugin.id.clone(), plugin.clone()))
            .collect();
    }

    /// One plugin changed (the answer to `getPlugin`, an install, enable…).
    pub fn upsert(&mut self, plugin: &PluginSummary) {
        self.plugins.insert(plugin.id.clone(), plugin.clone());
    }

    /// The plugin was uninstalled.
    pub fn remove(&mut self, plugin: &str) {
        self.plugins.remove(plugin);
    }

    /// Plugins cannot be asked for (no agent, plugins off, locked): none
    /// is known, so their actions are unknown action types again.
    pub fn clear(&mut self) {
        self.plugins.clear();
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Adds the plugin action types to `registry`: available ones as
    /// definitions (they never run here), the others as unavailable with
    /// the agent's reason, worded as the agent's engine words it. Entries
    /// outside their plugin's namespace are ignored, so an answer can never
    /// replace or hide a built-in action type.
    pub fn extend(&self, registry: &mut ActionRegistry) {
        for plugin in self.plugins.values() {
            let Ok(module) = ModuleId::new(plugin.id.clone()) else {
                continue;
            };
            if !module.is_plugin() {
                continue;
            }
            let name = if plugin.name.is_empty() {
                plugin.id.as_str()
            } else {
                plugin.name.as_str()
            };
            for action in &plugin.actions {
                let Some(definition) = &action.definition else {
                    continue;
                };
                if definition.id.as_str() != action.action_type
                    || ModuleId::owner_of(&definition.id).as_ref() != Some(&module)
                {
                    continue;
                }
                if action.available {
                    // A duplicate cannot happen inside one namespace; if it
                    // did, the first definition stays.
                    let _ = registry.register_for_module(
                        module.clone(),
                        &[],
                        Box::new(AgentAction {
                            definition: checked(definition),
                        }),
                    );
                } else {
                    let why = action
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("the plugin cannot run now");
                    registry.mark_unavailable(
                        [definition.id.clone()],
                        format!("{name} ({}): {why}", plugin.id),
                    );
                }
            }
        }
    }
}

/// A plugin action type for validation only: the agent runs it.
struct AgentAction {
    definition: ActionType,
}

impl ActionHandler for AgentAction {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        _parameters: &ActionParameters,
        _context: &ExecutionContext<'_>,
    ) -> ActionResult {
        ActionResult::failure("plugin actions run in the DeckPilot agent")
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_core::{ActionTypeId, ParameterValue, RegistryError};
    use deckpilot_ipc::plugins::{PluginActionInfo, PluginOrigin, PluginState};

    use super::*;

    fn definition(id: &str) -> ActionType {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": "Greet",
            "description": "Says hello.",
            "category": "Fun",
            "parameters": [
                {"key": "name", "label": "Name", "kind": {"type": "text"}, "required": true}
            ],
            "requiresProject": false,
        }))
        .unwrap()
    }

    fn plugin(id: &str, available: bool) -> PluginSummary {
        let action_type = format!("{id}.greet");
        PluginSummary {
            id: id.into(),
            name: "Hello Weather".into(),
            publisher: "acme".into(),
            version: "1.0.0".into(),
            state: if available {
                PluginState::Enabled
            } else {
                PluginState::Disabled
            },
            state_reason: None,
            origin: PluginOrigin::Sideload,
            previous_version: None,
            staged_version: None,
            actions: vec![PluginActionInfo {
                action: "greet".into(),
                action_type: action_type.clone(),
                name: "Greet".into(),
                description: None,
                available,
                unavailable_reason: (!available).then(|| "the plugin is disabled".into()),
                definition: Some(definition(&action_type)),
            }],
            action_count: None,
            health: None,
            navigation: Vec::new(),
            installed_for: None,
            visual_assets: Vec::new(),
        }
    }

    fn greet() -> ActionTypeId {
        ActionTypeId::new("acme.hello-weather.greet").unwrap()
    }

    fn registry(actions: &PluginActions) -> ActionRegistry {
        let mut registry =
            deckpilot_modules::builtin(deckpilot_core::Os::current()).action_registry();
        actions.extend(&mut registry);
        registry
    }

    /// Plugin API 1.7: a plugin's editor navigation reaches the catalog
    /// only as the host's rules allow; the plugin never runs for it.
    #[test]
    fn editor_navigation_reaches_the_catalog_only_when_it_checks_out() {
        use deckpilot_core::{EditorNavigation, NavigationOperation};
        let with = |navigation: serde_json::Value| {
            let mut summary = plugin("acme.hello-weather", true);
            let mut definition = serde_json::to_value(definition(&greet().to_string())).unwrap();
            definition["editorNavigation"] = navigation;
            summary.actions[0].definition = Some(serde_json::from_value(definition).unwrap());
            let mut actions = PluginActions::default();
            actions.replace(&[summary]);
            registry(&actions)
                .definition(&greet())
                .unwrap()
                .editor_navigation
                .clone()
        };
        assert_eq!(
            with(serde_json::json!({"operation": "folder.open", "targetParameter": "name"})),
            Some(EditorNavigation::with_target(
                NavigationOperation::FolderOpen,
                "name"
            ))
        );
        assert_eq!(
            with(serde_json::json!({"operation": "folder.open", "targetParameter": "nope"})),
            None
        );
        assert_eq!(
            with(serde_json::json!({"operation": "page.next", "targetParameter": "name"})),
            None
        );
    }

    #[test]
    fn nothing_known_changes_nothing() {
        let before = deckpilot_modules::builtin(deckpilot_core::Os::current()).action_registry();
        let after = registry(&PluginActions::default());
        assert_eq!(
            before.action_types().map(|t| &t.id).collect::<Vec<_>>(),
            after.action_types().map(|t| &t.id).collect::<Vec<_>>()
        );
        assert!(after.definition(&greet()).is_none());
        assert!(after.unavailable_reason(&greet()).is_none());
    }

    #[test]
    fn available_actions_validate_like_in_the_agent() {
        let mut actions = PluginActions::default();
        actions.replace(&[plugin("acme.hello-weather", true)]);
        let registry = registry(&actions);
        assert_eq!(
            registry.module_of(&greet()).map(ModuleId::as_str),
            Some("acme.hello-weather")
        );
        let normalized = registry
            .normalize(
                &greet(),
                &ActionParameters::new().with("name", ParameterValue::Text("Ada".into())),
            )
            .unwrap();
        assert_eq!(
            normalized.get("name"),
            Some(&ParameterValue::Text("Ada".into()))
        );
        // A required parameter is required here as in the agent.
        assert!(matches!(
            registry.normalize(&greet(), &ActionParameters::new()),
            Err(RegistryError::InvalidParameters(_))
        ));
    }

    #[test]
    fn unavailable_actions_say_why_and_cannot_be_created() {
        let mut actions = PluginActions::default();
        actions.replace(&[plugin("acme.hello-weather", false)]);
        let registry = registry(&actions);
        assert!(registry.definition(&greet()).is_none());
        assert_eq!(
            registry.unavailable_reason(&greet()),
            Some("Hello Weather (acme.hello-weather): the plugin is disabled")
        );
        assert!(matches!(
            registry.create_instance(
                "Greet",
                greet(),
                &ActionParameters::new().with("name", ParameterValue::Text("Ada".into()))
            ),
            Err(RegistryError::Unavailable(..))
        ));
    }

    #[test]
    fn changes_follow_the_agent() {
        let mut actions = PluginActions::default();
        actions.replace(&[plugin("acme.hello-weather", true)]);
        actions.upsert(&plugin("acme.hello-weather", false));
        assert!(registry(&actions).definition(&greet()).is_none());
        actions.remove("acme.hello-weather");
        assert!(actions.is_empty());
        let registry = registry(&actions);
        assert!(registry.definition(&greet()).is_none());
        assert!(registry.unavailable_reason(&greet()).is_none());
        actions.upsert(&plugin("acme.hello-weather", true));
        actions.clear();
        assert!(actions.is_empty());
    }

    #[test]
    fn answers_cannot_reach_outside_their_namespace() {
        // A built-in type claimed by a plugin, a plugin id that is not a
        // plugin id, and a type of another plugin: all ignored.
        let mut claims_builtin = plugin("acme.hello-weather", false);
        claims_builtin.actions[0].action_type = "projects.next".into();
        claims_builtin.actions[0].definition = Some(definition("projects.next"));
        let mut not_a_plugin = plugin("projects", true);
        not_a_plugin.actions[0].action_type = "projects.greet".into();
        not_a_plugin.actions[0].definition = Some(definition("projects.greet"));
        let mut other = plugin("acme.hello-weather", true);
        other.id = "acme.other".into();
        let mut actions = PluginActions::default();
        actions.replace(&[claims_builtin, not_a_plugin, other]);
        let registry = registry(&actions);
        let next = ActionTypeId::new("projects.next").unwrap();
        assert!(registry.definition(&next).is_some());
        assert!(registry.unavailable_reason(&next).is_none());
        assert!(
            registry
                .definition(&ActionTypeId::new("projects.greet").unwrap())
                .is_none()
        );
        assert!(registry.definition(&greet()).is_none());
    }
}
