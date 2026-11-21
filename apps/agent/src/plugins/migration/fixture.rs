//! TEST FIXTURE, never in a release build (features `plugins` and
//! `test-hooks`): a legacy built-in module, its alias entry and its
//! migration plan, so the migration engine is tested end to end on a
//! module of its own, independent of the shipped
//! [`deckpilot_core::alias::ACTION_ALIASES`] (whose GitHub block
//! `apps/agent/tests/plugins_cutover.rs` covers). An agent with the
//! fixture routes the fixture's table in place of the shipped one.
//!
//! The module `legacy-fixture` looks like the GitHub module: an API origin
//! (`apiUrl`, default `https://api.example.com`), a token credential
//! (`credential`, default `fixture.token`), a private-network switch
//! (`allowPrivateHost`) and one portable setting (`greeting`). Its action
//! `legacy-fixture.whoami` answers without any network
//! (`source = builtin`), so a test sees which implementation ran. The
//! plugin that replaces it is
//! `plugins/first-party/test-fixtures/legacy-fixture`
//! (`deckpilot.legacy-fixture`).
//!
//! The agent registers the module and routes the alias only when
//! [`ENV`] is `1`, so other agent tests see the modules they always saw.

use std::collections::BTreeMap;

use deckpilot_core::alias::{ActionAlias, ParamRule};
use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    CredentialId, ExecutionContext, FeatureModule, ModuleId, ModuleManifest, ParameterKind,
    ParameterSpec, ParameterValue, Portability, SettingScope,
};

use super::MigrationPlan;

/// `1`: register the fixture module and route its alias.
pub const ENV: &str = "DECKPILOT_TEST_LEGACY_FIXTURE";

pub const MODULE: &str = "legacy-fixture";
pub const ACTION: &str = "legacy-fixture.whoami";
pub const PLUGIN: &str = "deckpilot.legacy-fixture";
pub const DEFAULT_API_URL: &str = "https://api.example.com";
pub const DEFAULT_CREDENTIAL: &str = "fixture.token";

/// The fixture's alias entry: the shape of a real cut-over block.
pub const ALIASES: &[ActionAlias] = &[ActionAlias {
    legacy: ACTION,
    target: "deckpilot.legacy-fixture.whoami",
    module: MODULE,
    plugin: PLUGIN,
    since: "0.0.0-test",
    params: &[ParamRule::DropIfDefault("credential")],
}];

const _: () = deckpilot_core::alias::assert_valid(ALIASES);

/// What the alias table does not say about the fixture module.
pub const PLAN: MigrationPlan = MigrationPlan {
    module: MODULE,
    credentials: &[("credential", "token")],
    origin: Some(super::OriginPlan {
        setting: "apiUrl",
        default: Some(DEFAULT_API_URL),
        entered: false,
    }),
    private_network: Some("allowPrivateHost"),
    open_url_hosts: &[],
    extraction: None,
};

/// Whether this agent runs with the fixture (an agent test asked for it).
pub fn enabled() -> bool {
    std::env::var(ENV).is_ok_and(|v| v == "1")
}

fn text(key: &str, label: &str, max: usize, default: &str) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: label.into(),
        description: None,
        kind: ParameterKind::Text {
            multiline: false,
            max_length: Some(max),
        },
        required: false,
        default: (!default.is_empty()).then(|| ParameterValue::Text(default.into())),
    }
}

/// `legacy-fixture.whoami`: the contract the plugin's
/// `deckpilot.legacy-fixture.whoami` keeps (but the `credential` override).
pub fn whoami_type() -> ActionType {
    let mut node = NodeSpec {
        outputs: vec![
            OutputSpec::new("who", "Who", ValueKind::Text),
            OutputSpec::new("source", "Source", ValueKind::Text),
        ],
        timeout_ms: 5_000,
        retry_safe: true,
        idempotent: true,
        cancellable: true,
        effect: ActionEffect::ReadOnly,
        ..NodeSpec::default()
    };
    node.icon = Some("user".into());
    ActionType {
        editor_navigation: None,
        title: None,
        id: ActionTypeId::new(ACTION).expect("a valid id"),
        name: "Who am I".into(),
        description: "TEST FIXTURE: answers without asking anyone.".into(),
        category: "Legacy Fixture".into(),
        parameters: vec![
            text("prefix", "Prefix", 32, ""),
            ParameterSpec {
                kind: ParameterKind::Credential,
                ..text("credential", "Token credential", 128, "")
            },
        ],
        requires_project: false,
        node,
        deprecated: false,
        // As the built-in it stands in for: keys only.
        controls: Some(deckpilot_core::ActionControls::keys()),
    }
}

struct Whoami {
    definition: ActionType,
    /// The module's `credential` setting, used when the override is empty.
    credential: String,
}

impl ActionHandler for Whoami {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        // The built-in uses the configured credential; only whether it is
        // there shows, never the secret.
        let credential = parameters
            .text("credential")
            .filter(|c| !c.trim().is_empty())
            .unwrap_or(&self.credential);
        let who = match CredentialId::new(credential)
            .ok()
            .map(|id| context.credential(&id))
        {
            Some(Ok(_)) => "token-holder",
            _ => "anonymous",
        };
        ActionResult::success(format!(
            "{}builtin, {who}",
            parameters.text("prefix").unwrap_or_default()
        ))
        .with_data("who", who)
        .with_data("source", "builtin")
    }
}

/// The fixture legacy module.
pub struct FixtureModule(ModuleManifest);

impl FixtureModule {
    pub fn new() -> Self {
        Self(ModuleManifest {
            id: ModuleId::new(MODULE).expect("a valid id"),
            version: "0.0.0".into(),
            name: "Legacy Fixture".into(),
            description: "TEST FIXTURE: a built-in that a bundled plugin replaces.".into(),
            platforms: Vec::new(),
            pages: Vec::new(),
            capabilities: vec![Capability::Network, Capability::Credentials],
            config: vec![
                text("apiUrl", "API URL", 512, DEFAULT_API_URL),
                ParameterSpec {
                    kind: ParameterKind::Credential,
                    ..text("credential", "Token credential", 128, DEFAULT_CREDENTIAL)
                },
                ParameterSpec {
                    key: "allowPrivateHost".into(),
                    label: "Private network".into(),
                    description: None,
                    kind: ParameterKind::Boolean,
                    required: false,
                    default: Some(ParameterValue::Boolean(false)),
                },
                text("greeting", "Greeting", 32, "Hello"),
            ],
            dependencies: Vec::new(),
            enabled_by_default: true,
            portability: Portability {
                version: 1,
                settings: BTreeMap::from([
                    ("apiUrl".to_owned(), SettingScope::MachineSpecific),
                    ("credential".to_owned(), SettingScope::Portable),
                    ("allowPrivateHost".to_owned(), SettingScope::MachineSpecific),
                    ("greeting".to_owned(), SettingScope::Portable),
                ]),
            },
            migrations: Vec::new(),
            windows: Vec::new(),
            variables: Vec::new(),
        })
    }
}

impl FeatureModule for FixtureModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.0
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        vec![Box::new(Whoami {
            definition: whoami_type(),
            credential: config
                .text("credential")
                .filter(|c| !c.trim().is_empty())
                .unwrap_or(DEFAULT_CREDENTIAL)
                .to_owned(),
        })]
    }
}
