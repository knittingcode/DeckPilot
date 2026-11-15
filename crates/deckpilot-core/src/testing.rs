//! Minimal action handlers and modules for the core's own tests. Real action
//! types live in feature module crates, which the core does not depend on.

use crate::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    ContextEffect, ExecutionContext, FeatureModule, KeyFace, ModuleId, ModuleManifest, ModulePage,
    Os, ParameterKind, ParameterSpec, Project, ProjectSwitcher, render_label,
};

/// A registry with [`Echo`] and [`Select`].
pub(crate) fn registry() -> crate::ActionRegistry {
    let mut registry = crate::ActionRegistry::default();
    registry.register(Box::new(Echo::new())).unwrap();
    registry.register(Box::new(Select::new())).unwrap();
    registry
}

/// A profile with projects using `keys` as key and name.
pub(crate) fn with_projects(name: &str, keys: &[&str]) -> crate::Profile {
    let mut profile = crate::Profile::new(name);
    for key in keys {
        profile
            .projects
            .push(Project::new(crate::ProjectKey::new(*key).unwrap(), *key));
    }
    profile
}

pub(crate) fn echo_type() -> ActionTypeId {
    ActionTypeId::new("test.echo").unwrap()
}

pub(crate) fn select_type() -> ActionTypeId {
    ActionTypeId::new("test.select").unwrap()
}

fn text(key: &str, required: bool) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: key.into(),
        description: None,
        kind: ParameterKind::Text {
            multiline: false,
            max_length: Some(100),
        },
        required,
        default: None,
    }
}

/// `test.echo`: succeeds with its rendered `message`, or fails when the
/// message is `fail`.
pub(crate) struct Echo(ActionType);

impl Echo {
    pub(crate) fn new() -> Self {
        Self(ActionType {
            id: echo_type(),
            name: "Echo".into(),
            description: String::new(),
            category: "Test".into(),
            parameters: vec![text("message", true)],
            requires_project: false,
            deprecated: false,
            node: crate::graph::NodeSpec::default(),
        })
    }

    /// The same handler for the action type `id`.
    pub(crate) fn with_id(id: &str) -> Self {
        let mut echo = Self::new();
        echo.0.id = ActionTypeId::new(id).unwrap();
        echo
    }
}

impl ActionHandler for Echo {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let message = parameters.text("message").unwrap_or_default();
        if message == "fail" {
            return ActionResult::failure("failed on purpose");
        }
        let (profile, project) = match (context.profile(), context.project()) {
            (Ok(profile), Ok(project)) => (profile, project),
            (Err(denied), _) | (_, Err(denied)) => return denied.into(),
        };
        ActionResult::success(render_label(message, &profile.name, project))
    }

    /// Live state: the rendered message as the label, refreshed every 5 s;
    /// fails for `fail`.
    fn state_policy(&self, _parameters: &ActionParameters) -> Option<crate::RefreshPolicy> {
        Some(crate::RefreshPolicy::every(std::time::Duration::from_secs(
            5,
        )))
    }

    fn button_state(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<crate::ButtonState, crate::StateError> {
        let message = parameters.text("message").unwrap_or_default();
        if message == "fail" {
            return Err(crate::StateError::new("failed", "failed on purpose"));
        }
        let profile = context.profile()?;
        let project = context.project()?;
        Ok(crate::ButtonState::new()
            .with_label(render_label(message, &profile.name, project))
            .with_indicator(crate::Indicator::Ok))
    }
}

/// `test.select`: makes the configured project active, or the next one when
/// no project is configured. Faces show the active project's key, or are
/// highlighted while the configured project is active.
pub(crate) struct Select(ActionType);

impl Select {
    pub(crate) fn new() -> Self {
        Self(ActionType {
            id: select_type(),
            name: "Select".into(),
            description: String::new(),
            category: "Test".into(),
            parameters: vec![ParameterSpec {
                key: "project".into(),
                label: "Project".into(),
                description: None,
                kind: ParameterKind::Project,
                required: false,
                default: None,
            }],
            requires_project: false,
            deprecated: false,
            node: crate::graph::NodeSpec {
                changes_context: true,
                ..crate::graph::NodeSpec::default()
            },
        })
    }
}

impl ActionHandler for Select {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Some(project) = parameters.project("project") {
            return ActionResult::success("selected")
                .with_effect(ContextEffect::SelectProject { project });
        }
        let (profile, project) = match (context.profile(), context.project()) {
            (Ok(profile), Ok(project)) => (profile, project),
            (Err(denied), _) | (_, Err(denied)) => return denied.into(),
        };
        match ProjectSwitcher::cycle(
            &profile.projects,
            project.map(|p| p.id),
            crate::CycleDirection::Next,
        ) {
            Some(project) => ActionResult::success("switched")
                .with_effect(ContextEffect::SelectProject { project }),
            None => ActionResult::failure("no projects"),
        }
    }

    fn check(&self, parameters: &ActionParameters, profile: &crate::Profile) -> Vec<String> {
        parameters
            .project("project")
            .filter(|id| profile.project(*id).is_none())
            .map(|_| vec!["project belongs to another profile".to_owned()])
            .unwrap_or_default()
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        parameters: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        match parameters.project("project") {
            Some(target) => face.highlighted(project.is_some_and(|p| p.id == target)),
            None if template.contains("{project.") => face,
            None => face.with_detail(project.map_or("-", |p| p.key.as_str())),
        }
    }
}

struct Noop(ActionType);

impl ActionHandler for Noop {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(&self, _: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        ActionResult::success("")
    }
}

pub(crate) struct TestModule {
    manifest: ModuleManifest,
    source: Option<std::sync::Arc<dyn crate::variables::VariableSource>>,
}

impl TestModule {
    fn manifest(id: &str, name: &str, page: &str, order: i32) -> ModuleManifest {
        ModuleManifest {
            id: ModuleId::new(id).unwrap(),
            version: "1.0.0".into(),
            name: name.into(),
            description: String::new(),
            platforms: Vec::new(),
            pages: vec![ModulePage {
                id: id.into(),
                title: page.into(),
                icon: "box".into(),
                order,
            }],
            capabilities: vec![Capability::ProjectContext],
            config: Vec::new(),
            dependencies: Vec::new(),
            enabled_by_default: true,
            portability: crate::Portability::default(),
            migrations: Vec::new(),
            windows: Vec::new(),
            variables: Vec::new(),
        }
    }

    /// Module `test` providing `test.echo` and `test.select`.
    pub(crate) fn base() -> Self {
        let mut manifest = Self::manifest("test", "Test", "Test page", 20);
        manifest.config = vec![text("prefix", false)];
        manifest.windows = vec![crate::ModuleWindow {
            id: "test.panel".into(),
            title: "Test panel".into(),
            size: crate::WindowSize::Medium,
            blocking: false,
        }];
        Self {
            manifest,
            source: None,
        }
    }

    /// Module `test-extra`, Linux only, depending on `test`.
    pub(crate) fn dependent() -> Self {
        let mut manifest = Self::manifest("test-extra", "Extra", "Extra", 10);
        manifest.platforms = vec![Os::Linux];
        manifest.dependencies = vec![ModuleId::new("test").unwrap()];
        Self {
            manifest,
            source: None,
        }
    }

    /// Module `other` that wrongly provides `test.echo`.
    pub(crate) fn outside_namespace() -> Self {
        Self {
            manifest: Self::manifest("other", "Other", "Other", 30),
            source: None,
        }
    }
}

impl TestModule {
    /// Declares `variables`, resolved by `source`.
    pub(crate) fn with_variables(
        mut self,
        variables: Vec<crate::variables::VariableDeclaration>,
        source: Option<std::sync::Arc<dyn crate::variables::VariableSource>>,
    ) -> Self {
        self.manifest.variables = variables;
        self.source = source;
        self
    }

    /// Declares a window with `id`, which may be outside the namespace.
    pub(crate) fn set_window(&mut self, id: &str) {
        self.manifest.windows = vec![crate::ModuleWindow {
            id: id.into(),
            title: id.into(),
            size: crate::WindowSize::Fit,
            blocking: false,
        }];
    }
}

impl FeatureModule for TestModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn variable_source(&self) -> Option<std::sync::Arc<dyn crate::variables::VariableSource>> {
        self.source.clone()
    }

    fn action_handlers(&self, _config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        match self.manifest.id.as_str() {
            "test-extra" => vec![Box::new(Noop(ActionType {
                id: ActionTypeId::new("test-extra.noop").unwrap(),
                name: "Noop".into(),
                description: String::new(),
                category: "Test".into(),
                parameters: Vec::new(),
                requires_project: false,
                deprecated: false,
                node: crate::graph::NodeSpec::default(),
            }))],
            "other" => vec![Box::new(Echo::new())],
            _ => vec![Box::new(Echo::new()), Box::new(Select::new())],
        }
    }
}

/// Fixture aliases: the built-in `legacy.fetch` of module `legacy`, served
/// by the first-party plugin fixture `deckpilot.legacy`. The shipped table
/// is empty, so routing is tested with these.
pub(crate) const FIXTURE_ALIASES: &[crate::alias::ActionAlias] = &[crate::alias::ActionAlias {
    legacy: "legacy.fetch",
    target: "deckpilot.legacy.fetch",
    module: "legacy",
    plugin: "deckpilot.legacy",
    since: "0.0.0-test",
    params: &[crate::alias::ParamRule::DropIfDefault("credential")],
}];

const _: () = crate::alias::assert_valid(FIXTURE_ALIASES);

/// The value of the `legacy` module's `credential` setting.
pub(crate) const LEGACY_CREDENTIAL: &str = "legacy-token";

fn fetch_type(id: &str, message_length: usize) -> ActionType {
    ActionType {
        id: ActionTypeId::new(id).unwrap(),
        name: format!("Fetch ({id})"),
        description: String::new(),
        category: "Test".into(),
        parameters: vec![ParameterSpec {
            kind: ParameterKind::Text {
                multiline: false,
                max_length: Some(message_length),
            },
            ..text("message", true)
        }],
        requires_project: false,
        deprecated: false,
        node: crate::graph::NodeSpec {
            outputs: vec![crate::graph::OutputSpec::new(
                "body",
                "Body",
                crate::graph::ValueKind::Text,
            )],
            ..crate::graph::NodeSpec::default()
        },
    }
}

/// The built-in `legacy.fetch`: `message`, and an optional `credential`
/// override a plugin cannot declare.
pub(crate) fn legacy_fetch() -> ActionType {
    let mut definition = fetch_type("legacy.fetch", 100);
    definition.parameters.push(ParameterSpec {
        kind: ParameterKind::Credential,
        ..text("credential", false)
    });
    definition
}

/// The plugin's `deckpilot.legacy.fetch`: `message` only, longer.
pub(crate) fn plugin_fetch() -> ActionType {
    fetch_type("deckpilot.legacy.fetch", 200)
}

/// Succeeds with `<tag>: <message> [<parameter keys>]`, so a test sees who
/// ran and with which parameters.
pub(crate) struct Fetch {
    definition: ActionType,
    tag: &'static str,
}

impl ActionHandler for Fetch {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        _context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let keys: Vec<&str> = parameters.iter().map(|(key, _)| key.as_str()).collect();
        ActionResult::success(format!(
            "{}: {} [{}]",
            self.tag,
            parameters.text("message").unwrap_or_default(),
            keys.join(",")
        ))
    }

    fn runs_detached(&self) -> bool {
        self.tag == "plugin"
    }
}

/// The built-in module `legacy` ("Legacy"): `legacy.fetch` (aliased) and
/// `legacy.other` (not), a `credential` setting, the network capability.
pub(crate) struct LegacyModule(ModuleManifest);

impl LegacyModule {
    pub(crate) fn new() -> Self {
        let mut manifest = TestModule::manifest("legacy", "Legacy", "Legacy", 40);
        manifest.pages.clear();
        manifest.capabilities = vec![Capability::Network];
        manifest.config = vec![ParameterSpec {
            default: Some(crate::ParameterValue::Text(LEGACY_CREDENTIAL.into())),
            ..text("credential", false)
        }];
        Self(manifest)
    }
}

impl FeatureModule for LegacyModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.0
    }

    fn action_handlers(&self, _config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        vec![
            Box::new(Fetch {
                definition: legacy_fetch(),
                tag: "builtin",
            }),
            Box::new(Fetch {
                definition: fetch_type("legacy.other", 100),
                tag: "builtin",
            }),
        ]
    }
}

/// The first-party plugin fixture `deckpilot.legacy` ("Legacy plugin"),
/// providing `deckpilot.legacy.fetch` with the credentials capability.
pub(crate) struct PluginFixture {
    manifest: ModuleManifest,
    fetch: ActionType,
    reason: Option<String>,
}

impl PluginFixture {
    /// Enabled and healthy.
    pub(crate) fn new() -> Self {
        let mut manifest =
            TestModule::manifest("deckpilot.legacy", "Legacy plugin", "Legacy plugin", 50);
        manifest.pages.clear();
        manifest.capabilities = vec![Capability::Credentials];
        Self {
            manifest,
            fetch: plugin_fetch(),
            reason: None,
        }
    }

    /// Not enabled because of a fault (broken, quarantined, ...).
    pub(crate) fn faulty(reason: &str) -> Self {
        let mut plugin = Self::new();
        plugin.manifest.enabled_by_default = false;
        plugin.reason = Some(reason.to_owned());
        plugin
    }

    /// The same module under another id.
    pub(crate) fn with_module_id(mut self, id: &str) -> Self {
        self.manifest.id = ModuleId::new(id).unwrap();
        self
    }

    /// Providing `fetch` in place of [`plugin_fetch`].
    pub(crate) fn providing(fetch: ActionType) -> Self {
        Self {
            fetch,
            ..Self::new()
        }
    }
}

impl FeatureModule for PluginFixture {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, _config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        vec![Box::new(Fetch {
            definition: self.fetch.clone(),
            tag: "plugin",
        })]
    }

    fn unavailable_reason(&self) -> Option<String> {
        self.reason.clone()
    }
}
