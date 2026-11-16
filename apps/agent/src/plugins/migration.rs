//! The module migration engine (`docs/marketplace/module-migration.md`
//! §5.5–§5.9): moves a built-in module with an alias block to its bundled
//! first-party plugin, one journaled, idempotent step at a time, and
//! decides the [`LegacyRouting`] the core routes the module's legacy action
//! types with.
//!
//! The plugin is bundled with class `migration` (installed here, only for
//! a computer whose data uses the built-in) or `default` (seeded on every
//! computer by `bundled.rs`, §4.3). A `default`-class plugin is the
//! built-in's replacement everywhere, so it does not need the user's data
//! to use the built-in: a fresh install, with nothing to migrate, cuts
//! over at its first start, and an upgrade copies the built-in's settings
//! first. A built-in the user turned off stays off: the seeded plugin is
//! disabled once, when the module is first journaled (the enabled state is
//! copied once, §5.5).
//!
//! # Where it runs
//!
//! The agent's service task owns `deckpilot.db`: it reads a
//! [`LegacySnapshot`] (`legacy.rs`, read only) whenever it loads the
//! modules and hands it to the plugin worker, which owns `plugins.db`. The
//! first snapshot after the plugin store opened (at start-up or unlock)
//! runs the steps of every module ([`start`]); the routing is computed
//! from the journal after every change ([`routing`]) and given back to the
//! service task, which sets it on the module registry before it builds the
//! action registry (`service.rs`, `load_modules`). Until then, and
//! whenever the plugin store is not open, every module is `Builtin`.
//!
//! # The state machine (`module_migrations.phase`)
//!
//! ```text
//!            ┌──────────── not_eligible ◀─── (any non-final phase; a cut-over
//!            │  (re-checked at every start)      module whose source changed)
//!            ▼
//!   (none) ─▶ pending ─▶ installed ─▶ settings_copied ─┬─▶ cut_over
//!            ▲                                         │      ▲
//!            │                                         ▼      │ consent
//!            │                                  awaiting_consent
//!            │
//!    failed ─┘ automatically at the next start while attempts < 3,
//!              afterwards only by the user's Retry
//! ```
//!
//! Each step checks its target first, so a rerun (after a crash, a restart
//! or a decision) repeats nothing that was done. Any step that fails
//! records `failed` with a reason (never a value) and counts an attempt;
//! the routing stays `Builtin`, so the built-in keeps serving.
//!
//! 1. **Eligibility.** The built-in is compiled in and enabled, the user's
//!    data uses it (a saved `modules` row or a stored instance), no stored
//!    value needs a translation the alias cannot make, its origin is the
//!    plugin's default or (for a plugin that takes one, [`OriginPlan`]) a
//!    server the user entered that a plugin can reach, it allows private
//!    hosts only for such a server, and plugins can run here. A tombstone
//!    (the user removed or disabled the plugin) and a bundle without the
//!    plugin are reasons too.
//! 2. **Install** the pinned bundled copy (pins plus the macOS code seal,
//!    `bundled.rs`), or keep a copy that is not older, and record the
//!    derived grants (below).
//! 3. **Settings.** Copy the built-in's saved settings that the plugin
//!    declares, and the server the user entered into the plugin's origin
//!    setting, only if the plugin has none yet, and record the source
//!    digest. The `modules` row is never changed.
//! 4. **Consent.** Anything not derived waits for the user
//!    (`awaiting_consent`): high-tier permissions always, the import of a
//!    stored credential always, and a server the user entered always
//!    (with its private network as a separate item). With the consent,
//!    the secret is copied into the plugin's empty slot (the
//!    `credentials` row stays) and the consent to exactly that server is
//!    recorded.
//! 5. **Health check** (enabled, files match the pins, every alias target
//!    declared and compatible with the built-in, required settings set),
//!    then **cut over**: the routing becomes `Plugin`.
//!
//! # Consent
//!
//! Low-tier permissions, and medium-tier ones the built-in demonstrably
//! used on this computer (its default origin for `network` and its web
//! hosts for `openUrl`, when that is what it talked to), are granted as
//! `derived`, decided by `migration:<module>`. High-tier permissions
//! (credentials, the private network) are never derived, and a stored
//! credential is imported only when the user granted its slot through the
//! migration's consent (decided by `user:migration:<module>`); a grant
//! made for other reasons does not import anything.
//!
//! A server the user entered in the built-in (a self-managed GitLab, a
//! Jira site) is never derived either, although the built-in used it: the
//! consent to a user-entered origin (G2) is the user's, to exactly that
//! server. It is an item of the same dialog, recorded only by the
//! decision that names it; reaching it on a private network is one more
//! item, and the server is allowed only with both.
//!
//! The app asks with `decideModuleMigration`, item by item ([`decide`],
//! [`infos`] lists the items). A declined item is a refusal decided by
//! `user:migration:<module>`: the module stays on its built-in
//! (`not_eligible`, at every start) until the user presses Retry, which
//! forgets those refusals and asks again. Nothing is ever granted without
//! an approval.
//!
//! # After the cut-over
//!
//! At every start a cut-over module is reconciled: if the built-in's
//! settings changed (an older app edited them), the plugin's settings are
//! copied again when they are unchanged since the copy and kept otherwise,
//! with a note; a source that is no longer eligible (another origin) goes
//! back to the built-in. The user disabling or uninstalling the plugin
//! (its tombstone) makes the routing `PluginWithdrawn`: the legacy actions
//! are unavailable, with no fallback. A plugin that breaks falls back to
//! the built-in in the core; a bundled copy this agent does not pin (a
//! build without the plugin) is routed to the built-in here.
//!
//! Nothing here writes `deckpilot.db`, deletes or rewrites a `modules` or
//! `credentials` row, or rewrites a stored profile.

use std::collections::{BTreeMap, BTreeSet};

use deckpilot_core::alias::{LegacyRoute, LegacyRouting};
use deckpilot_core::{Capability, ModuleId, ModuleRegistry, ModuleStates, ParameterValue};
use deckpilot_ipc::plugins as ipc;
use deckpilot_plugin_broker::{OriginError, UserOrigin};
use deckpilot_plugin_bundled::{BundleClass, Pin};
use deckpilot_plugin_manifest::{Manifest, PermissionKind, PluginId};
use deckpilot_plugin_store::{
    Consent, GrantKey, GrantSet, GrantStatus, InstallSource, MAX_MIGRATION_REASON_CHARS,
    MigrationPhase, MigrationStep, ModuleMigration, PluginManager, PluginState, Tier,
    missing_grants,
};
use serde_json::Value;
use tracing::{info, warn};

use super::bundled::{self, Bundle, Seeded};

#[cfg(feature = "test-hooks")]
pub(crate) mod fixture;
pub(crate) mod legacy;
#[cfg(feature = "test-hooks")]
mod standin;
#[cfg(all(test, feature = "test-hooks"))]
mod tests;

pub(crate) use legacy::{LegacyModule, LegacySnapshot};

/// What the alias table does not say about a module: which settings name
/// its credentials and origin. Static, like the alias table: one plan per
/// alias block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MigrationPlan {
    pub module: &'static str,
    /// Credential settings of the built-in and the plugin's slot for each.
    pub credentials: &'static [(&'static str, &'static str)],
    /// The setting with the server the built-in talks to.
    pub origin: Option<OriginPlan>,
    /// The boolean setting that lets the built-in reach private addresses.
    /// `true` is never migrated by itself: for a server the user entered
    /// ([`OriginPlan::entered`]) the user is asked, for exactly that
    /// server; otherwise the module stays on its built-in (G3).
    pub private_network: Option<&'static str>,
    /// Hosts the built-in opens pages on (`openUrl`, derived when the
    /// built-in has the `open-external` capability).
    pub open_url_hosts: &'static [&'static str],
}

/// The setting of a built-in with the server it talks to, and how its
/// plugin takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OriginPlan {
    /// The built-in's setting; the plugin's setting has the same key.
    pub setting: &'static str,
    /// The built-in's default, a host the plugin declares (it needs no
    /// consent of its own). `None`: the module has no fixed host (Jira).
    pub default: Option<&'static str>,
    /// The plugin takes any other server the user entered there, through
    /// the user-origin slot of that setting (G2): the value is copied
    /// into the plugin's setting and the user is asked to allow exactly
    /// that server (never derived). `false`: only the default is
    /// eligible.
    pub entered: bool,
}

/// Every plan, one per module of [`deckpilot_core::alias::ACTION_ALIASES`].
pub(crate) const PLANS: &[MigrationPlan] = &[GITHUB, GITLAB, JIRA, NOTIFICATIONS];

/// GitHub (P9) to `deckpilot.github`:
///
/// - the credential setting `credential` (a credential id, default
///   `github.token`) is imported into the plugin's slot `token`, only with
///   the user's consent;
/// - the origin `apiUrl`: only `https://api.github.com`, the plugin's
///   default, is eligible; GitHub Enterprise and any other origin keep the
///   built-in (`not_eligible`) until G2;
/// - `allowPrivateHost` on keeps the built-in (G3, never migrated);
/// - `github.com` is the web host the built-in opens pages on (`openUrl`,
///   derived: the built-in has `open-external`).
///
/// A per-action `credential` override other than empty or the module's
/// own makes the module `not_eligible` ([`ParamRule::DropIfDefault`] in the
/// alias block, until G5).
///
/// [`ParamRule::DropIfDefault`]: deckpilot_core::alias::ParamRule::DropIfDefault
const GITHUB: MigrationPlan = MigrationPlan {
    module: deckpilot_modules::github::MODULE_ID,
    credentials: &[(deckpilot_modules::github::CREDENTIAL, "token")],
    origin: Some(OriginPlan {
        setting: deckpilot_modules::github::API_URL,
        default: Some(deckpilot_modules::github::DEFAULT_API_URL),
        entered: false,
    }),
    private_network: Some(deckpilot_modules::github::ALLOW_PRIVATE),
    open_url_hosts: &["github.com"],
};

/// GitLab (P11) to `deckpilot.gitlab`:
///
/// - the credential setting `credential` (a credential id, default
///   `gitlab.token`) is imported into the plugin's slot `token`, only with
///   the user's consent;
/// - the origin `apiUrl`: `https://gitlab.com/api/v4` is the plugin's
///   declared host; a self-managed GitLab is carried into the plugin's
///   `apiUrl` setting (its user-origin slot), and the user is asked to
///   allow exactly that server;
/// - `allowPrivateHost` on, for such a server, asks the user's separate
///   consent to reach it on a private network; on for GitLab.com keeps
///   the built-in;
/// - `gitlab.com` is the web host the built-in opens pages on (`openUrl`,
///   derived when GitLab.com is what the built-in used).
const GITLAB: MigrationPlan = MigrationPlan {
    module: deckpilot_modules::gitlab::MODULE_ID,
    credentials: &[(deckpilot_modules::gitlab::CREDENTIAL, "token")],
    origin: Some(OriginPlan {
        setting: deckpilot_modules::gitlab::API_URL,
        default: Some(deckpilot_modules::gitlab::DEFAULT_API_URL),
        entered: true,
    }),
    private_network: Some(deckpilot_modules::gitlab::ALLOW_PRIVATE),
    open_url_hosts: &["gitlab.com"],
};

/// Jira (P11) to `deckpilot.jira`:
///
/// - the credential setting `credential` (a credential id, default
///   `jira.token`) is imported into the plugin's slot `credential`, only
///   with the user's consent;
/// - the origin `siteUrl` has no default (Jira has no fixed host): the
///   site is carried into the plugin's `siteUrl` setting (its user-origin
///   slot), and the user is asked to allow exactly that server;
/// - `allowPrivateHost` on asks the user's separate consent to reach the
///   site on a private network;
/// - `deployment` and `timeoutSeconds` are declared by the plugin with the
///   same keys and copied one to one.
const JIRA: MigrationPlan = MigrationPlan {
    module: deckpilot_modules::jira::MODULE_ID,
    credentials: &[(deckpilot_modules::jira::CREDENTIAL, "credential")],
    origin: Some(OriginPlan {
        setting: deckpilot_modules::jira::SITE_URL,
        default: None,
        entered: true,
    }),
    private_network: Some(deckpilot_modules::jira::ALLOW_PRIVATE),
    open_url_hosts: &[],
};

/// Notifications (P10) to `deckpilot.notifications`, class `default`:
/// nothing the plan handles itself (no credential, origin, private network
/// or web host). Its three settings, `desktop`, `statusBar` and
/// `minimumSeverity`, are declared by the plugin with the same keys and
/// copied one to one; the agent routes the plugin's notifications by them
/// (the manifest's `notifications.routing`, G9).
const NOTIFICATIONS: MigrationPlan = MigrationPlan {
    module: deckpilot_modules::notifications::MODULE_ID,
    credentials: &[],
    origin: None,
    private_network: None,
    open_url_hosts: &[],
};

/// The plan of `module`, if it has one.
pub(crate) fn plan_for(module: &str) -> Option<&'static MigrationPlan> {
    #[cfg(feature = "test-hooks")]
    if module == fixture::MODULE {
        return Some(&fixture::PLAN);
    }
    PLANS.iter().find(|plan| plan.module == module)
}

/// The built-in modules with their stored states (the agent's
/// `load_modules`); with the test fixture when an agent test asked for it
/// (feature `test-hooks`).
pub(crate) fn configured(states: &ModuleStates) -> (ModuleRegistry, Vec<String>) {
    #[cfg(feature = "test-hooks")]
    if fixture::enabled() {
        let mut registry = deckpilot_modules::builtin(deckpilot_core::Os::current())
            .with_aliases(fixture::ALIASES);
        registry
            .register(Box::new(fixture::FixtureModule::new()))
            .expect("the fixture module does not conflict");
        let problems = registry.apply_states(states);
        return (registry, problems);
    }
    #[cfg(feature = "test-hooks")]
    {
        let mut registry = deckpilot_modules::builtin(deckpilot_core::Os::current());
        if standin::apply(&mut registry) {
            let problems = registry.apply_states(states);
            return (registry, problems);
        }
    }
    deckpilot_modules::configured(states)
}

/// A decision of the user's: the requests `decideModuleMigration` and
/// `retryModuleMigration` of the app's migration notice and consent
/// dialog. Agent tests can also give `Consent` and `Retry` through
/// `DECKPILOT_TEST_MIGRATION_DECISIONS` (feature `test-hooks`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Approves everything the migration waits for (grants what is not
    /// derived and imports the stored credentials). Test hooks only.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    Consent(String),
    /// The consent dialog, item by item: grant keys the user approves and
    /// declines. Every item approved: the migration goes on; any item
    /// declined: the built-in stays until Retry.
    Decide {
        module: String,
        approve: BTreeSet<GrantKey>,
        decline: BTreeSet<GrantKey>,
    },
    /// Retry in the migration notice: a failed migration starts again, and
    /// one the user declined asks again.
    Retry(String),
}

impl Decision {
    pub fn module(&self) -> &str {
        match self {
            Self::Consent(module) | Self::Retry(module) | Self::Decide { module, .. } => module,
        }
    }
}

/// Why a decision was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// No migration of that module is known here.
    Unknown(String),
    /// The migration is not in a phase the decision applies to.
    InvalidState(String),
    /// An item the migration does not wait for.
    UnknownItem(String),
    /// The plugin store refused.
    Failed(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(why)
            | Self::InvalidState(why)
            | Self::UnknownItem(why)
            | Self::Failed(why) => f.write_str(why),
        }
    }
}

/// `migration:<module>`: who decided the derived grants.
fn derived_by(module: &ModuleId) -> String {
    format!("migration:{module}")
}

/// `user:migration:<module>`: the user's consent in the migration dialog,
/// the only decision that imports a stored credential.
fn consented_by(module: &ModuleId) -> String {
    format!("user:migration:{module}")
}

/// Why the steps stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stop {
    /// Not eligible on this computer (not counted as an attempt).
    NotEligible(String),
    /// A step failed (counted).
    Failed(String),
}

type Step<T> = Result<T, Stop>;

fn failed(why: impl std::fmt::Display) -> Stop {
    Stop::Failed(why.to_string())
}

/// The order of the forward phases; `None` for `not_eligible` and
/// `failed`, which any step may leave.
fn rank(phase: MigrationPhase) -> Option<u8> {
    match phase {
        MigrationPhase::Pending => Some(0),
        MigrationPhase::Installed => Some(1),
        MigrationPhase::SettingsCopied => Some(2),
        MigrationPhase::AwaitingConsent => Some(3),
        MigrationPhase::CutOver => Some(4),
        _ => None,
    }
}

/// One line of at most [`MAX_MIGRATION_REASON_CHARS`] characters.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_MIGRATION_REASON_CHARS)
        .collect()
}

/// The hex sha256 of what the migration took from the built-in: the
/// settings it copies (`settings`), the credentials it uses by id, and the
/// settings that decide eligibility (origin, private network). Never a
/// secret.
fn source_digest(
    settings: &BTreeMap<String, Value>,
    credentials: &BTreeMap<String, String>,
    builtin: &BTreeMap<String, Value>,
) -> String {
    let document = serde_json::json!({
        "settings": settings,
        "credentials": credentials,
        "builtin": builtin,
    });
    let bytes = serde_json::to_vec(&document).unwrap_or_default();
    deckpilot_plugin_package::sha256(&bytes).to_string()
}

/// The setting keys the plan handles itself (never copied).
fn handled_keys(m: &LegacyModule) -> BTreeSet<&'static str> {
    let mut keys = BTreeSet::new();
    if let Some(plan) = m.plan {
        keys.extend(plan.credentials.iter().map(|(setting, _)| *setting));
        keys.extend(plan.origin.map(|origin| origin.setting));
        keys.extend(plan.private_network);
    }
    keys
}

/// The effective values of the settings that decide eligibility. Not the
/// origin setting of a plan that carries the server into the plugin's
/// settings ([`OriginPlan::entered`]): there it is part of the copied
/// settings, so another server is a change of those, copied again like
/// any other.
fn builtin_part(m: &LegacyModule) -> BTreeMap<String, Value> {
    let carried = m
        .plan
        .and_then(|plan| plan.origin)
        .filter(|origin| origin.entered)
        .map(|origin| origin.setting);
    handled_keys(m)
        .into_iter()
        .filter(|key| Some(*key) != carried)
        .map(|key| {
            let value = m
                .config
                .get(key)
                .and_then(|v| serde_json::to_value(v).ok())
                .unwrap_or(Value::Null);
            (key.to_owned(), value)
        })
        .collect()
}

/// A plain setting value as the plugin stores it.
fn json_of(value: &ParameterValue) -> Option<Value> {
    Some(match value {
        ParameterValue::Text(text) | ParameterValue::Choice(text) => Value::String(text.clone()),
        ParameterValue::Integer(n) => Value::from(*n),
        ParameterValue::Boolean(b) => Value::Bool(*b),
        _ => return None,
    })
}

/// The built-in's saved settings that `manifest` declares, one to one,
/// checked against the plugin's declaration. Keys only in messages.
fn projection(m: &LegacyModule, manifest: &Manifest) -> Step<BTreeMap<String, Value>> {
    let handled = handled_keys(m);
    let declared = manifest.contributes.settings();
    let mut values = BTreeMap::new();
    let saved = m.row.iter().flat_map(|row| row.config.iter());
    for (key, value) in saved {
        if handled.contains(key.as_str()) || !declared.iter().any(|s| &s.key == key) {
            continue;
        }
        let json = json_of(value).ok_or_else(|| {
            Stop::NotEligible(format!(
                "the setting `{key}` is not a plain value the plugin can store"
            ))
        })?;
        values.insert(key.clone(), json);
    }
    // The server the user entered in the built-in goes into the plugin's
    // origin setting, as the agent stores what a user saves there
    // (normalized). It reaches nothing until the user allowed exactly it.
    if let Some((origin, Ok(server))) = entered_origin(m) {
        let slot = deckpilot_plugin_store::declared_slot(manifest, origin.setting)
            .map(|(slot, _)| slot)
            .ok_or_else(|| {
                Stop::NotEligible(format!(
                    "{} has no origin setting `{}`",
                    manifest.id, origin.setting
                ))
            })?;
        let server = server.to_string();
        if let Err(problem) = super::origins::parse(slot, &server) {
            return Err(Stop::NotEligible(format!(
                "the server in `{}` {problem}",
                origin.setting
            )));
        }
        values.insert(origin.setting.to_owned(), Value::String(server));
    }
    super::data::merge_settings(manifest, &BTreeMap::new(), &values).map_err(|_| {
        Stop::NotEligible(format!(
            "the settings {} do not fit {}'s declaration",
            values.keys().cloned().collect::<Vec<_>>().join(", "),
            manifest.id
        ))
    })
}

/// The host of an origin URL (`https://api.github.com` → `api.github.com`).
fn host_of(origin: &str) -> Option<String> {
    url::Url::parse(origin)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
}

/// An origin as compared: trimmed, without a trailing `/`.
fn normalized(origin: &str) -> &str {
    origin.trim().trim_end_matches('/')
}

/// The server the built-in talks to: its origin setting, else the default.
fn configured_origin<'m>(m: &'m LegacyModule, origin: &OriginPlan) -> Option<&'m str> {
    m.config
        .text(origin.setting)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(origin.default)
}

/// Whether the built-in talks to its default origin (or, without a
/// default, to nothing yet): a host the plugin declares, nothing to carry.
fn uses_default(m: &LegacyModule, origin: &OriginPlan) -> bool {
    match (configured_origin(m, origin), origin.default) {
        (None, _) => true,
        (Some(configured), Some(default)) => normalized(configured) == normalized(default),
        (Some(_), None) => false,
    }
}

/// The server the user entered in the built-in, when the plugin takes
/// such a server ([`OriginPlan::entered`]) and it is not the default: as
/// the broker parses a user-entered origin, or why it is not one (plain
/// `http`, this computer, a user name…). Never the value in the error.
fn entered_origin(m: &LegacyModule) -> Option<(OriginPlan, Result<UserOrigin, OriginError>)> {
    let origin = m.plan?.origin.filter(|origin| origin.entered)?;
    if uses_default(m, &origin) {
        return None;
    }
    let configured = configured_origin(m, &origin)?;
    Some((origin, UserOrigin::parse(configured)))
}

/// Whether the built-in may reach private addresses (its switch is on).
fn allows_private(m: &LegacyModule) -> bool {
    m.plan
        .and_then(|plan| plan.private_network)
        .is_some_and(|setting| m.config.get(setting) == Some(&ParameterValue::Boolean(true)))
}

/// The consent item of the separate private-network consent for the
/// server in `setting`. Not a grant: it is never stored, and decides only
/// how the consent to that server is recorded.
fn private_item(setting: &str) -> GrantKey {
    GrantKey::new(
        PermissionKind::Network,
        format!("user-origin:{setting}:private-network"),
    )
}

/// A server the plugin's origin setting holds that the migration must ask
/// the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginAsk {
    setting: String,
    /// Normalized, with its path prefix: what the consent names.
    origin: String,
    /// The user allowed the server already; only the private network
    /// waits.
    allowed: bool,
    /// Reaching it needs the separate private-network consent: its host
    /// is private, or the built-in was allowed private addresses for it.
    private: bool,
    methods: Vec<String>,
    credentials: Vec<String>,
    opens_pages: bool,
}

/// What the plugin's origin setting of `m`'s plan holds that is not
/// allowed yet. Read from the plugin's own settings (what the copy put
/// there, or what the user saved since): the consent is always for what
/// the plugin would talk to.
fn origin_asks(manager: &PluginManager, m: &LegacyModule, manifest: &Manifest) -> Vec<OriginAsk> {
    let Some(plan) = m.plan.and_then(|plan| plan.origin).filter(|o| o.entered) else {
        return Vec::new();
    };
    let store = manager.store();
    let saved = store.settings(&m.plugin).unwrap_or_default();
    let grants = store.grants(&m.plugin).unwrap_or_default();
    let rows = store.user_origins(&m.plugin).unwrap_or_default();
    // The built-in's private switch counts for the server it was set for.
    let builtin_private = allows_private(m)
        .then(|| entered_origin(m))
        .flatten()
        .and_then(|(_, server)| server.ok())
        .map(|server| server.to_string());
    super::origins::infos(manifest, &saved, &grants, &rows)
        .into_iter()
        .filter(|info| info.setting == plan.setting)
        .filter_map(|info| {
            let origin = info.origin?;
            let private = info.private_host || builtin_private.as_deref() == Some(&origin);
            let allowed = match info.status {
                ipc::UserOriginStatus::NeedsConsent | ipc::UserOriginStatus::SlotNotGranted => {
                    false
                }
                ipc::UserOriginStatus::Granted if private && !info.private_network_granted => true,
                _ => return None,
            };
            Some(OriginAsk {
                setting: info.setting.clone(),
                origin,
                allowed,
                private,
                methods: info.methods,
                credentials: info.credentials,
                opens_pages: manifest.opens_user_origin(&info.setting),
            })
        })
        .collect()
}

/// Why `m` cannot migrate on this computer, if it cannot (§5.6 step 1).
/// `default`: the plugin is of the bundled `default` class, which replaces
/// the built-in whether or not the user's data uses it (§4.3, §5.8).
fn eligibility(m: &LegacyModule, default: bool) -> Option<String> {
    if m.manifest.is_none() {
        return Some("the built-in module is not part of this app".into());
    }
    if !m.enabled {
        return Some("the built-in module is disabled".into());
    }
    if !default && m.row.is_none() && m.instances == 0 {
        return Some("the built-in module is not used on this computer".into());
    }
    if !m.untranslatable.is_empty() {
        return Some(format!(
            "{} cannot be translated for the plugin (it needs credential slots, G5)",
            m.untranslatable.join("; ")
        ));
    }
    if let Some(plan) = m.plan {
        let entered = entered_origin(m);
        if let Some(origin) = plan.origin
            && !uses_default(m, &origin)
        {
            let setting = origin.setting;
            match &entered {
                None => {
                    return Some(format!(
                        "`{setting}` is not the default origin, the only one {} talks to",
                        m.plugin
                    ));
                }
                Some((_, Err(problem))) => {
                    return Some(format!(
                        "`{setting}` is not a server a plugin can reach ({problem})"
                    ));
                }
                Some((_, Ok(_))) => {}
            }
        }
        // A private network is asked for with a server the user entered,
        // for exactly that server; there is no such consent for a host
        // the plugin declares.
        if let Some(setting) = plan.private_network
            && allows_private(m)
            && !matches!(entered, Some((_, Ok(_))))
        {
            return Some(format!(
                "`{setting}` allows private network hosts, which is never migrated (G3)"
            ));
        }
    }
    if !super::execution_supported() {
        return Some("plugins cannot run on this platform".into());
    }
    None
}

/// Whether the built-in demonstrably used `key` on this computer: its
/// default origin for `network` and its web hosts for `openUrl` when that
/// is what it talked to, and opening pages of the server in its origin
/// setting. Never a server the user entered: that is asked.
fn demonstrated(m: &LegacyModule, key: &GrantKey) -> bool {
    let Some(manifest) = &m.manifest else {
        return false;
    };
    let has = |capability| manifest.capabilities.contains(&capability);
    let Some(plan) = m.plan else {
        return false;
    };
    // The default origin and its web hosts count only when the default is
    // what the built-in talked to here.
    let on_default = plan.origin.is_none_or(|origin| uses_default(m, &origin));
    match key.kind {
        PermissionKind::Network => {
            has(Capability::Network)
                && on_default
                && plan
                    .origin
                    .and_then(|origin| origin.default)
                    .and_then(host_of)
                    .is_some_and(|host| host == key.key)
        }
        // Pages of the server in the built-in's origin setting: the
        // built-in opened them, and the grant opens nothing until the user
        // allowed that server.
        PermissionKind::OpenUrl
            if deckpilot_plugin_store::open_url_user_origin_setting(key).is_some() =>
        {
            has(Capability::OpenExternal)
                && plan
                    .origin
                    .filter(|origin| origin.entered)
                    .is_some_and(|origin| {
                        deckpilot_plugin_store::open_url_user_origin_setting(key)
                            == Some(origin.setting)
                    })
        }
        PermissionKind::OpenUrl => {
            has(Capability::OpenExternal)
                && on_default
                && plan.open_url_hosts.contains(&key.key.as_str())
        }
        _ => false,
    }
}

/// The engine over one open plugin store.
struct Engine<'a> {
    manager: &'a mut PluginManager,
    bundle: &'a Bundle,
    /// Plugins whose state or grants changed (the broker syncs them).
    touched: BTreeSet<PluginId>,
    /// Test only: the agent exits right after recording this phase.
    exit_after: Option<MigrationPhase>,
}

impl Engine<'_> {
    fn record(
        &mut self,
        m: &LegacyModule,
        phase: MigrationPhase,
        reason: Option<&str>,
        source_digest: Option<String>,
    ) -> Step<()> {
        let step = MigrationStep {
            module_id: m.module.to_string(),
            plugin_id: m.plugin.clone(),
            phase,
            reason: reason.map(one_line),
            source_digest,
            app_version: crate::service::VERSION.to_owned(),
        };
        let recorded = self.manager.record_migration(&step).map_err(failed)?;
        info!(module = %m.module, plugin = %m.plugin, phase = %recorded.phase,
            reason = recorded.reason.as_deref().unwrap_or(""), attempts = recorded.attempts,
            "module migration");
        if self.exit_after == Some(phase) {
            warn!(%phase, "test hook: the agent exits after this migration step");
            std::process::exit(86);
        }
        Ok(())
    }

    /// Records `phase` unless the module is already past it.
    fn advance(
        &mut self,
        m: &LegacyModule,
        phase: MigrationPhase,
        reason: Option<&str>,
        digest: Option<String>,
    ) -> Step<()> {
        let current = self
            .manager
            .store()
            .module_migration(m.module.as_str())
            .map_err(failed)?;
        let behind = current
            .as_ref()
            .and_then(|c| rank(c.phase))
            .is_none_or(|current| rank(phase).is_some_and(|next| next >= current));
        match current {
            _ if behind => self.record(m, phase, reason, digest),
            // Past it already: only a new digest is recorded.
            Some(current) if digest.is_some() => {
                self.record(m, current.phase, current.reason.as_deref(), digest)
            }
            _ => Ok(()),
        }
    }

    fn migration(&self, m: &LegacyModule) -> Option<ModuleMigration> {
        match self.manager.store().module_migration(m.module.as_str()) {
            Ok(record) => record,
            Err(error) => {
                warn!(module = %m.module, %error, "the migration record cannot be read");
                None
            }
        }
    }

    /// The pin of `m`'s plugin, of either class, if this app bundles it.
    fn pin(&self, m: &LegacyModule) -> Option<Pin> {
        self.bundle
            .pins
            .plugins
            .iter()
            .find(|pin| pin.id == m.plugin)
            .cloned()
    }

    /// The user turned the built-in off: its `default`-class replacement,
    /// which seeding enabled, is turned off too, once, when the module is
    /// first journaled (the agent's own disable, no tombstone). The user
    /// can turn the plugin on in Settings › Plugins; nothing here turns it
    /// off again.
    fn keep_off(&mut self, m: &LegacyModule) -> Step<()> {
        if self.migration(m).is_some() {
            return Ok(());
        }
        let record = self.manager.store().plugin(&m.plugin).map_err(failed)?;
        if record
            .is_some_and(|r| r.source == InstallSource::Bundled && r.state == PluginState::Enabled)
        {
            self.manager.disable(&m.plugin).map_err(failed)?;
            self.touched.insert(m.plugin.clone());
            info!(module = %m.module, plugin = %m.plugin,
                "the built-in is disabled, so its replacement is disabled too");
        }
        Ok(())
    }

    /// Runs the steps of `m` that are due. `start`: the first pass after
    /// the store opened (drift, automatic retries).
    fn run(&mut self, m: &LegacyModule, start: bool) {
        let record = self.migration(m);
        if let Some(record) = &record {
            match record.phase {
                MigrationPhase::CutOver => {
                    if start && let Err(stop) = self.reconcile(m, record) {
                        self.stopped(m, stop);
                    }
                    return;
                }
                MigrationPhase::Failed if !(start && record.retries_automatically()) => return,
                _ => {}
            }
        }
        if let Err(stop) = self.steps(m) {
            self.stopped(m, stop);
        }
    }

    fn stopped(&mut self, m: &LegacyModule, stop: Stop) {
        let (phase, reason) = match stop {
            Stop::NotEligible(reason) => (MigrationPhase::NotEligible, reason),
            Stop::Failed(reason) => (MigrationPhase::Failed, reason),
        };
        if let Err(Stop::Failed(error) | Stop::NotEligible(error)) =
            self.record(m, phase, Some(&reason), None)
        {
            warn!(module = %m.module, %error, "the migration step cannot be recorded");
        }
    }

    fn steps(&mut self, m: &LegacyModule) -> Step<()> {
        let pin = self.pin(m);
        let default = pin
            .as_ref()
            .is_some_and(|pin| pin.class == BundleClass::Default);
        if let Some(reason) = eligibility(m, default) {
            if default && !m.enabled {
                self.keep_off(m)?;
            }
            return Err(Stop::NotEligible(reason));
        }
        if let Some(tombstone) = self.manager.store().tombstone(&m.plugin).map_err(failed)? {
            return Err(Stop::NotEligible(format!(
                "{} was {} by you",
                m.plugin, tombstone.decision
            )));
        }
        let pin =
            pin.ok_or_else(|| Stop::NotEligible(format!("this app does not bundle {}", m.plugin)))?;
        self.advance(m, MigrationPhase::Pending, None, None)?;
        self.install(m, &pin)?;
        self.advance(m, MigrationPhase::Installed, None, None)?;
        let digest = self.copy_settings(m)?;
        self.advance(m, MigrationPhase::SettingsCopied, None, digest)?;
        // The user declined in the consent dialog: the built-in stays, at
        // every start, until the user asks again (Retry).
        let declined = self.declined(m)?;
        if !declined.is_empty() {
            return Err(Stop::NotEligible(format!(
                "you declined {}; the built-in stays until you ask again",
                declined
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let waiting = self.consent_needed(m)?;
        if !waiting.is_empty() {
            return self.advance(
                m,
                MigrationPhase::AwaitingConsent,
                Some(&format!("waiting for your consent: {}", waiting.join(", "))),
                None,
            );
        }
        self.import_credentials(m)?;
        self.enable(m)?;
        if let Err(why) = self.health(m) {
            // The built-in takes over again: the plugin goes back to how
            // the migration found it (the agent's own disable, no
            // tombstone).
            let _ = self.manager.disable(&m.plugin);
            self.touched.insert(m.plugin.clone());
            return Err(Stop::Failed(format!("health check: {why}")));
        }
        self.record(m, MigrationPhase::CutOver, None, None)
    }

    fn active_manifest(&self, m: &LegacyModule) -> Step<Manifest> {
        let record = self
            .manager
            .store()
            .plugin(&m.plugin)
            .map_err(failed)?
            .ok_or_else(|| failed(format!("{} is not installed", m.plugin)))?;
        record
            .active()
            .map(|active| active.manifest.clone())
            .ok_or_else(|| failed(format!("{} has no active version", m.plugin)))
    }

    /// §5.6 step 2, and the derived grants.
    fn install(&mut self, m: &LegacyModule, pin: &Pin) -> Step<()> {
        let existing = self.manager.store().plugin(&m.plugin).map_err(failed)?;
        match existing {
            None => {
                let package = bundled::verified(self.bundle, pin).map_err(Stop::Failed)?;
                let (low, _) = bundled::split_by_tier(package.manifest());
                self.manager
                    .install(
                        &package,
                        &InstallSource::Bundled,
                        &Consent::of(low, derived_by(&m.module)),
                    )
                    .map_err(failed)?;
                self.touched.insert(m.plugin.clone());
            }
            Some(record) if record.source == InstallSource::Bundled => {
                match bundled::refresh(self.manager, self.bundle, pin, &record) {
                    Seeded::Broken(why) | Seeded::Failed(why) => return Err(Stop::Failed(why)),
                    Seeded::Updated => {
                        self.touched.insert(m.plugin.clone());
                    }
                    _ => {}
                }
            }
            Some(record) => {
                let older = record.active().is_none_or(|a| a.version < pin.version);
                if older {
                    return Err(Stop::NotEligible(format!(
                        "an older copy of {} from {} is installed; update it first",
                        m.plugin,
                        record.source.as_str()
                    )));
                }
            }
        }
        let manifest = self.active_manifest(m)?;
        let keys: BTreeSet<GrantKey> = GrantSet::from_manifest(&manifest)
            .iter()
            .filter(|(key, scope)| match scope.tier() {
                Tier::Low => true,
                Tier::Medium => demonstrated(m, key),
                Tier::High => false,
            })
            .map(|(key, _)| key.clone())
            .collect();
        let granted = self
            .manager
            .derive_grants(&m.plugin, &keys, &derived_by(&m.module))
            .map_err(failed)?;
        if !granted.is_empty() {
            self.touched.insert(m.plugin.clone());
        }
        Ok(())
    }

    /// §5.6 step 3: the digest to record, if any.
    fn copy_settings(&mut self, m: &LegacyModule) -> Step<Option<String>> {
        let manifest = self.active_manifest(m)?;
        let values = projection(m, &manifest)?;
        let saved = self.manager.store().settings(&m.plugin).map_err(failed)?;
        let digest = source_digest(&values, &m.credential_ids(), &builtin_part(m));
        let copied = saved.is_empty() && !values.is_empty();
        if copied {
            self.manager
                .set_settings(&m.plugin, &values)
                .map_err(failed)?;
        }
        let recorded = self
            .migration(m)
            .and_then(|record| record.source_digest)
            .is_some();
        Ok((copied || !recorded).then_some(digest))
    }

    /// The credentials to import: stored in `credentials`, and the
    /// plugin's slot is empty.
    fn imports<'m>(&self, m: &'m LegacyModule) -> Step<Vec<&'m legacy::LegacyCredential>> {
        let filled = self
            .manager
            .store()
            .credentials(&m.plugin)
            .map_err(failed)?;
        Ok(m.credentials
            .iter()
            .filter(|c| c.secret.is_some() && !filled.iter().any(|f| f.key == c.slot))
            .collect())
    }

    /// Whether the user consented to importing into `slot` (the
    /// migration's consent, not any grant of the slot).
    fn import_consented(&self, m: &LegacyModule, slot: &str) -> Step<bool> {
        let key = GrantKey::new(PermissionKind::Credentials, slot);
        let consented = consented_by(&m.module);
        Ok(self
            .manager
            .store()
            .grants(&m.plugin)
            .map_err(failed)?
            .iter()
            .any(|g| g.key == key && g.status == GrantStatus::Granted && g.decided_by == consented))
    }

    /// What waits for the user's consent (§5.6 step 4): permissions not
    /// granted (high tier, and medium tier the built-in did not use here),
    /// and every credential import. Keys and credential ids only.
    fn consent_needed(&self, m: &LegacyModule) -> Step<Vec<String>> {
        let manifest = self.active_manifest(m)?;
        let stored = self.manager.store().grants(&m.plugin).map_err(failed)?;
        let mut waiting: Vec<String> = missing_grants(&stored, &GrantSet::from_manifest(&manifest))
            .iter()
            .map(ToString::to_string)
            .collect();
        for credential in self.imports(m)? {
            if !self.import_consented(m, &credential.slot)? {
                waiting.push(format!(
                    "import of the stored credential `{}` into `{}`",
                    credential
                        .id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    credential.slot
                ));
            }
        }
        // The server the built-in talked to, by its setting (the value is
        // in the consent item, not in the journal).
        for ask in origin_asks(self.manager, m, &manifest) {
            if !ask.allowed {
                waiting.push(format!("the server entered in `{}`", ask.setting));
            }
            if ask.private {
                waiting.push(format!(
                    "the private network for the server entered in `{}`",
                    ask.setting
                ));
            }
        }
        Ok(waiting)
    }

    /// The keys the user's consent grants: everything not granted, and the
    /// slots of the credentials to import.
    fn consent_keys(&self, m: &LegacyModule) -> Step<BTreeSet<GrantKey>> {
        let manifest = self.active_manifest(m)?;
        let stored = self.manager.store().grants(&m.plugin).map_err(failed)?;
        let mut keys: BTreeSet<GrantKey> =
            missing_grants(&stored, &GrantSet::from_manifest(&manifest))
                .into_iter()
                .collect();
        for credential in self.imports(m)? {
            keys.insert(GrantKey::new(PermissionKind::Credentials, &credential.slot));
        }
        for ask in origin_asks(self.manager, m, &manifest) {
            if !ask.allowed {
                keys.insert(deckpilot_plugin_store::user_origin_key(&ask.setting));
            }
            if ask.private {
                keys.insert(private_item(&ask.setting));
            }
        }
        Ok(keys)
    }

    /// Records the user's consent to each server the migration asked
    /// about (§5.5, G2): only for a server whose item is in `approve`,
    /// the items the user allowed in this very decision, and, when
    /// reaching it needs a private network, only with that separate item
    /// in it too. Nothing decided earlier or elsewhere counts: a server
    /// is allowed by the decision that named it. The consent names
    /// exactly what the plugin's setting holds.
    fn allow_origins(
        &mut self,
        m: &LegacyModule,
        approve: &BTreeSet<GrantKey>,
    ) -> Result<(), Refusal> {
        let failed = |why: String| Refusal::Failed(why);
        let manifest = self.active_manifest(m).map_err(|stop| match stop {
            Stop::Failed(why) | Stop::NotEligible(why) => Refusal::Failed(why),
        })?;
        let by = consented_by(&m.module);
        for ask in origin_asks(self.manager, m, &manifest) {
            let slot = deckpilot_plugin_store::user_origin_key(&ask.setting);
            let server = ask.allowed || approve.contains(&slot);
            let network = !ask.private || approve.contains(&private_item(&ask.setting));
            if !server || !network {
                continue;
            }
            let saved = self
                .manager
                .store()
                .settings(&m.plugin)
                .map_err(|e| failed(e.to_string()))?;
            let consent =
                super::origins::consent(&manifest, &saved, &ask.setting, &ask.origin, ask.private)
                    .map_err(failed)?;
            self.manager
                .grant_user_origin(&m.plugin, &consent, &by)
                .map_err(|e| failed(e.to_string()))?;
            self.touched.insert(m.plugin.clone());
            info!(module = %m.module, plugin = %m.plugin, setting = %ask.setting,
                private_network = ask.private, "the user allowed the server the built-in used");
        }
        Ok(())
    }

    /// Copies each consented credential into its empty slot. The
    /// `credentials` row is never changed.
    fn import_credentials(&mut self, m: &LegacyModule) -> Step<()> {
        let imports: Vec<(String, String)> = self
            .imports(m)?
            .into_iter()
            .filter_map(|c| Some((c.slot.clone(), c.secret.as_ref()?.expose().to_owned())))
            .collect();
        for (slot, secret) in imports {
            if !self.import_consented(m, &slot)? {
                return Err(failed(format!("the import into `{slot}` is not consented")));
            }
            self.manager
                .set_credential(&m.plugin, &slot, &secret)
                .map_err(failed)?;
            info!(module = %m.module, plugin = %m.plugin, %slot, "stored credential imported");
        }
        Ok(())
    }

    fn enable(&mut self, m: &LegacyModule) -> Step<()> {
        let record = self
            .manager
            .store()
            .plugin(&m.plugin)
            .map_err(failed)?
            .ok_or_else(|| failed(format!("{} is not installed", m.plugin)))?;
        match record.state {
            PluginState::Enabled => Ok(()),
            PluginState::Disabled => {
                self.manager.enable(&m.plugin).map_err(failed)?;
                self.touched.insert(m.plugin.clone());
                Ok(())
            }
            other => Err(failed(format!("{} is {other}", m.plugin))),
        }
    }

    /// §5.6 step 5: the plugin can replace the built-in right now.
    fn health(&mut self, m: &LegacyModule) -> Result<(), String> {
        let record = self
            .manager
            .store()
            .plugin(&m.plugin)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{} is not installed", m.plugin))?;
        if record.state != PluginState::Enabled {
            return Err(format!("{} is {}", m.plugin, record.state));
        }
        let active = record
            .active()
            .ok_or_else(|| format!("{} has no active version", m.plugin))?
            .clone();
        let dir = self.manager.layout().resolve(&active.install_dir);
        bundled::check_installed(self.manager, self.bundle, &record, &dir)?;
        let actions = active.manifest.contributes.actions();
        for alias in &m.aliases {
            let builtin = m
                .builtin_types
                .iter()
                .find(|t| t.id.as_str() == alias.legacy)
                .ok_or_else(|| {
                    format!("the built-in `{}` is not part of this app", alias.legacy)
                })?;
            let action = actions
                .iter()
                .find(|a| format!("{}.{}", m.plugin, a.id) == alias.target)
                .ok_or_else(|| format!("{} does not provide `{}`", m.plugin, alias.target))?;
            let target = super::engine::action_type(m.plugin.as_str(), action)?;
            let problems = deckpilot_core::alias::incompatibilities(alias, builtin, &target);
            if !problems.is_empty() {
                return Err(format!(
                    "`{}` cannot replace `{}`: {}",
                    alias.target,
                    alias.legacy,
                    problems.join("; ")
                ));
            }
        }
        let saved = self
            .manager
            .store()
            .settings(&m.plugin)
            .map_err(|e| e.to_string())?;
        let (_, missing) = super::data::effective_settings(&active.manifest, &saved);
        if !missing.is_empty() {
            return Err(format!(
                "required settings are not set: {}",
                missing.join(", ")
            ));
        }
        Ok(())
    }

    /// A cut-over module at a start (§5.6 "Re-runs and drift").
    fn reconcile(&mut self, m: &LegacyModule, record: &ModuleMigration) -> Step<()> {
        if self
            .manager
            .store()
            .tombstone(&m.plugin)
            .map_err(failed)?
            .is_some()
        {
            return Ok(());
        }
        if self
            .manager
            .store()
            .plugin(&m.plugin)
            .map_err(failed)?
            .is_none()
        {
            return Ok(());
        }
        let manifest = self.active_manifest(m)?;
        let ids = m.credential_ids();
        let builtin = builtin_part(m);
        let values = projection(m, &manifest);
        let now = values
            .as_ref()
            .ok()
            .map(|v| source_digest(v, &ids, &builtin));
        if now.is_some() && now == record.source_digest {
            return Ok(());
        }
        // The built-in's settings changed after the migration.
        let default = self
            .pin(m)
            .is_some_and(|pin| pin.class == BundleClass::Default);
        if let Some(reason) = eligibility(m, default) {
            return Err(Stop::NotEligible(format!(
                "{reason} (changed after the migration; the built-in serves again)"
            )));
        }
        let values = values?;
        let now = source_digest(&values, &ids, &builtin);
        let saved = self.manager.store().settings(&m.plugin).map_err(failed)?;
        let unchanged =
            record.source_digest.as_deref() == Some(source_digest(&saved, &ids, &builtin).as_str());
        let note = if unchanged {
            self.manager
                .set_settings(&m.plugin, &values)
                .map_err(failed)?;
            // Another server than the user allowed: the built-in serves
            // again until the user allowed this one too.
            if !self.consent_needed(m)?.is_empty() {
                return self.record(
                    m,
                    MigrationPhase::AwaitingConsent,
                    Some(
                        "the built-in's settings changed after the migration; they were copied \
                         again, and what changed waits for your consent",
                    ),
                    Some(now),
                );
            }
            "the built-in's settings changed after the migration; they were copied again"
        } else {
            "the built-in's settings changed after the migration; the plugin's own settings \
             were kept"
        };
        self.record(m, MigrationPhase::CutOver, Some(note), Some(now))
    }

    /// The keys the user declined in the migration's consent dialog (revoked
    /// by `user:migration:<module>`) among those the migration needs.
    fn declined(&self, m: &LegacyModule) -> Step<BTreeSet<GrantKey>> {
        let consented = consented_by(&m.module);
        let stored = self.manager.store().grants(&m.plugin).map_err(failed)?;
        let needed = self.consent_keys(m)?;
        Ok(stored
            .into_iter()
            .filter(|g| {
                g.status == GrantStatus::Revoked
                    && g.decided_by == consented
                    && needed.contains(&g.key)
            })
            .map(|g| g.key)
            .collect())
    }

    /// The user's decisions in the consent dialog (§5.5), item by item:
    /// approvals are grants decided by `user:migration:<module>`, which
    /// also allow the imports; declines are refusals decided the same way,
    /// which keep the built-in. `approve` and `decline` name only what the
    /// migration waits for. Then the steps run on.
    fn consent(
        &mut self,
        m: &LegacyModule,
        approve: Option<&BTreeSet<GrantKey>>,
        decline: &BTreeSet<GrantKey>,
    ) -> Result<(), Refusal> {
        let record = self.migration(m);
        if record.as_ref().map(|r| r.phase) != Some(MigrationPhase::AwaitingConsent) {
            return Err(Refusal::InvalidState(format!(
                "the migration of {} waits for no consent",
                m.module
            )));
        }
        let stop = |stop: Stop| match stop {
            Stop::Failed(why) | Stop::NotEligible(why) => Refusal::Failed(why),
        };
        let waiting = self.consent_keys(m).map_err(stop)?;
        // `None`: everything that waits (the test hooks' `consent:`).
        let approve = approve.cloned().unwrap_or_else(|| waiting.clone());
        if let Some(key) = approve
            .iter()
            .chain(decline)
            .find(|key| !waiting.contains(key))
        {
            return Err(Refusal::UnknownItem(format!(
                "the migration of {} does not wait for `{key}`",
                m.module
            )));
        }
        let by = consented_by(&m.module);
        // The private-network items are not grants: they decide how the
        // consent to their server is recorded (`allow_origins`), and a
        // declined one declines the server.
        let private: BTreeMap<GrantKey, GrantKey> = waiting
            .iter()
            .filter_map(|key| {
                let setting = deckpilot_plugin_store::user_origin_setting(key)?
                    .strip_suffix(":private-network")?;
                Some((
                    key.clone(),
                    deckpilot_plugin_store::user_origin_key(setting),
                ))
            })
            .collect();
        let grants: Vec<GrantKey> = approve
            .iter()
            .filter(|key| !private.contains_key(key))
            .cloned()
            .collect();
        if !grants.is_empty() {
            self.manager
                .grant(&m.plugin, &Consent::of(grants, by.clone()))
                .map_err(|e| Refusal::Failed(e.to_string()))?;
            self.touched.insert(m.plugin.clone());
        }
        for key in decline {
            let key = private.get(key).unwrap_or(key);
            self.manager
                .revoke(&m.plugin, key, &by)
                .map_err(|e| Refusal::Failed(e.to_string()))?;
            self.touched.insert(m.plugin.clone());
        }
        if decline.is_empty() {
            self.allow_origins(m, &approve)?;
        }
        info!(module = %m.module, plugin = %m.plugin, approved = approve.len(),
            declined = decline.len(), "the user decided on the migration");
        self.run(m, false);
        Ok(())
    }

    /// The user's Retry: a failed migration starts again with no failed
    /// attempts; one the user declined (or that was not eligible) is
    /// checked again, and what the user declined is asked again.
    fn retry(&mut self, m: &LegacyModule) -> Result<(), Refusal> {
        let phase = self.migration(m).map(|r| r.phase);
        if !matches!(
            phase,
            Some(MigrationPhase::Failed | MigrationPhase::NotEligible)
        ) {
            return Err(Refusal::InvalidState(format!(
                "the migration of {} has not failed",
                m.module
            )));
        }
        if self
            .manager
            .store()
            .plugin(&m.plugin)
            .map_err(|e| Refusal::Failed(e.to_string()))?
            .is_some()
        {
            let forgotten = self
                .manager
                .forget_declines(&m.plugin, &consented_by(&m.module))
                .map_err(|e| Refusal::Failed(e.to_string()))?;
            if !forgotten.is_empty() {
                self.touched.insert(m.plugin.clone());
            }
        }
        self.manager
            .retry_migration(m.module.as_str())
            .map_err(|e| Refusal::Failed(e.to_string()))?
            .ok_or_else(|| Refusal::Unknown(format!("`{}` has no migration to retry", m.module)))?;
        info!(module = %m.module, "the user retries the migration");
        self.run(m, false);
        Ok(())
    }
}

/// The first pass after the plugin store opened: every module's due steps.
/// Returns the plugins whose state or grants changed.
pub(super) fn start(
    manager: &mut PluginManager,
    bundle: &Bundle,
    snapshot: &LegacySnapshot,
) -> BTreeSet<PluginId> {
    let mut engine = Engine {
        manager,
        bundle,
        touched: BTreeSet::new(),
        exit_after: test_hooks::exit_after(),
    };
    // Test hooks only (an empty list in every other build): what an agent
    // test decided before this start, as the requests would.
    let decisions = test_hooks::decisions();
    for decision in &decisions {
        if let Decision::Retry(module) = decision {
            apply(&mut engine, snapshot, decision, module);
        }
    }
    for module in &snapshot.modules {
        engine.run(module, true);
    }
    for decision in &decisions {
        if let Decision::Consent(module) = decision {
            apply(&mut engine, snapshot, decision, module);
        }
    }
    engine.touched
}

fn apply(engine: &mut Engine<'_>, snapshot: &LegacySnapshot, decision: &Decision, module: &str) {
    if let Err(why) = decide_with(engine, snapshot, decision) {
        warn!(%module, %why, "a migration decision was not applied");
    }
}

fn decide_with(
    engine: &mut Engine<'_>,
    snapshot: &LegacySnapshot,
    decision: &Decision,
) -> Result<(), Refusal> {
    let module = decision.module();
    let m = snapshot
        .modules
        .iter()
        .find(|m| m.module.as_str() == module)
        .ok_or_else(|| Refusal::Unknown(format!("no module `{module}` can migrate here")))?;
    match decision {
        Decision::Consent(_) => engine.consent(m, None, &BTreeSet::new()),
        Decision::Decide {
            approve, decline, ..
        } => engine.consent(m, Some(approve), decline),
        Decision::Retry(_) => engine.retry(m),
    }
}

/// Applies a decision of the user's (`decideModuleMigration`,
/// `retryModuleMigration`) and runs that module's steps. Returns the
/// plugins whose state or grants changed.
pub(super) fn decide(
    manager: &mut PluginManager,
    bundle: &Bundle,
    snapshot: &LegacySnapshot,
    decision: &Decision,
) -> Result<BTreeSet<PluginId>, Refusal> {
    let mut engine = Engine {
        manager,
        bundle,
        touched: BTreeSet::new(),
        exit_after: None,
    };
    decide_with(&mut engine, snapshot, decision)?;
    Ok(engine.touched)
}

/// The route of one journaled module (§5.2), and why it is unavailable
/// when the user withdrew the plugin.
fn route_of(
    manager: &PluginManager,
    bundle: &Bundle,
    record: &ModuleMigration,
) -> (LegacyRoute, Option<String>) {
    let store = manager.store();
    if record.phase != MigrationPhase::CutOver {
        return (LegacyRoute::Builtin, None);
    }
    if let Some(tombstone) = store.tombstone(&record.plugin_id).ok().flatten() {
        let name = store
            .known()
            .unwrap_or_default()
            .into_iter()
            .find(|k| k.id == record.plugin_id)
            .map(|k| k.display_name)
            .filter(|name| !name.is_empty());
        let who = match name {
            Some(name) => format!("{name} ({})", record.plugin_id),
            None => record.plugin_id.to_string(),
        };
        return (
            LegacyRoute::PluginWithdrawn,
            Some(format!("{who} was {} by you", tombstone.decision)),
        );
    }
    match store.plugin(&record.plugin_id).ok().flatten() {
        Some(plugin)
            if plugin.source == InstallSource::Bundled && bundle.pin_of(&plugin).is_none() =>
        {
            (LegacyRoute::Builtin, None)
        }
        _ => (LegacyRoute::Plugin, None),
    }
}

/// The routing of every module with a migration record (§5.2): `Plugin`
/// once cut over, `PluginWithdrawn` when the user then disabled or
/// uninstalled the plugin, `Builtin` otherwise, and for a bundled copy
/// this agent does not pin (a build without the plugin).
pub(super) fn routing(manager: &PluginManager, bundle: &Bundle) -> LegacyRouting {
    let store = manager.store();
    let Ok(records) = store.module_migrations() else {
        return LegacyRouting::new();
    };
    let mut routing = LegacyRouting::new();
    for record in records {
        let Ok(module) = ModuleId::new(record.module_id.clone()) else {
            continue;
        };
        if record.phase != MigrationPhase::CutOver {
            continue;
        }
        let (route, _) = route_of(manager, bundle, &record);
        routing = routing.with(module, route);
    }
    routing
}

/// Every journaled migration as the app shows it
/// (`listModuleMigrations`): phase, reason, attempts, route, aliases and
/// the consent items. `snapshot` is what `deckpilot.db` said at the last
/// module load (names, aliases, the credentials to import); without it
/// only the journal is known. Never a value or a secret: credentials by
/// id, settings by key.
pub(super) fn infos(
    manager: &PluginManager,
    bundle: &Bundle,
    snapshot: Option<&LegacySnapshot>,
) -> Vec<ipc::ModuleMigrationInfo> {
    let Ok(records) = manager.store().module_migrations() else {
        return Vec::new();
    };
    records
        .iter()
        .map(|record| info_of(manager, bundle, snapshot, record))
        .collect()
}

/// The migration of `module`, if it is journaled.
pub(super) fn info(
    manager: &PluginManager,
    bundle: &Bundle,
    snapshot: Option<&LegacySnapshot>,
    module: &str,
) -> Option<ipc::ModuleMigrationInfo> {
    let record = manager.store().module_migration(module).ok().flatten()?;
    Some(info_of(manager, bundle, snapshot, &record))
}

fn phase(phase: MigrationPhase) -> ipc::MigrationPhase {
    match phase {
        MigrationPhase::Pending => ipc::MigrationPhase::Pending,
        MigrationPhase::NotEligible => ipc::MigrationPhase::NotEligible,
        MigrationPhase::Installed => ipc::MigrationPhase::Installed,
        MigrationPhase::SettingsCopied => ipc::MigrationPhase::SettingsCopied,
        MigrationPhase::AwaitingConsent => ipc::MigrationPhase::AwaitingConsent,
        MigrationPhase::CutOver => ipc::MigrationPhase::CutOver,
        MigrationPhase::Failed => ipc::MigrationPhase::Failed,
    }
}

/// The event of a journaled migration.
pub(super) fn event_of(record: &ModuleMigration) -> ipc::PluginEvent {
    ipc::PluginEvent::MigrationChanged {
        module: record.module_id.clone(),
        plugin: record.plugin_id.to_string(),
        phase: phase(record.phase),
    }
}

fn info_of(
    manager: &PluginManager,
    bundle: &Bundle,
    snapshot: Option<&LegacySnapshot>,
    record: &ModuleMigration,
) -> ipc::ModuleMigrationInfo {
    let m = snapshot.and_then(|s| {
        s.modules
            .iter()
            .find(|m| m.module.as_str() == record.module_id)
    });
    let plugin = manager.store().plugin(&record.plugin_id).ok().flatten();
    let manifest = plugin
        .as_ref()
        .and_then(|p| p.active())
        .map(|a| a.manifest.clone());
    let (route, route_reason) = route_of(manager, bundle, record);
    let consent = match (m, &manifest, record.phase) {
        (
            Some(m),
            Some(manifest),
            MigrationPhase::AwaitingConsent | MigrationPhase::NotEligible,
        ) => consent_items(manager, m, manifest),
        _ => Vec::new(),
    };
    // Not eligible for another reason: nothing was declined, nothing to show.
    let consent = if record.phase == MigrationPhase::NotEligible
        && !consent
            .iter()
            .any(|item| item.decision == ipc::ConsentDecision::Declined)
    {
        Vec::new()
    } else {
        consent
    };
    let declined = record.phase == MigrationPhase::NotEligible && !consent.is_empty();
    ipc::ModuleMigrationInfo {
        module: record.module_id.clone(),
        module_name: m.and_then(|m| m.manifest.as_ref()).map_or_else(
            || record.module_id.clone(),
            |manifest| manifest.name.clone(),
        ),
        plugin: record.plugin_id.to_string(),
        plugin_name: manifest.as_ref().map(|manifest| manifest.name.clone()),
        phase: phase(record.phase),
        reason: record.reason.clone(),
        attempts: record.attempts,
        max_attempts: deckpilot_plugin_store::MAX_MIGRATION_ATTEMPTS,
        can_retry: record.phase == MigrationPhase::Failed || declined,
        route: match route {
            LegacyRoute::Builtin => ipc::MigrationRoute::Builtin,
            LegacyRoute::Plugin => ipc::MigrationRoute::Plugin,
            LegacyRoute::PluginWithdrawn => ipc::MigrationRoute::PluginWithdrawn,
        },
        route_reason,
        aliases: m
            .map(|m| {
                m.aliases
                    .iter()
                    .map(|alias| ipc::ModuleActionAlias {
                        legacy: alias.legacy.to_owned(),
                        target: alias.target.to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        consent,
        updated_at: record.updated_at,
    }
}

/// What the migration of `m` asks the user (§5.5): every permission it
/// needs that is not granted, and every credential import, with the
/// user's decision so far. Keys, ids and hosts only.
fn consent_items(
    manager: &PluginManager,
    m: &LegacyModule,
    manifest: &Manifest,
) -> Vec<ipc::MigrationConsentItem> {
    let Ok(stored) = manager.store().grants(&m.plugin) else {
        return Vec::new();
    };
    let filled = manager.store().credentials(&m.plugin).unwrap_or_default();
    let declared = GrantSet::from_manifest(manifest);
    let missing: BTreeSet<GrantKey> = missing_grants(&stored, &declared).into_iter().collect();
    let consented = consented_by(&m.module);
    let imports: BTreeMap<&str, &legacy::LegacyCredential> = m
        .credentials
        .iter()
        .filter(|c| c.secret.is_some() && !filled.iter().any(|f| f.key == c.slot))
        .map(|c| (c.slot.as_str(), c))
        .collect();
    let asks = origin_asks(manager, m, manifest);
    let mut items: Vec<ipc::MigrationConsentItem> = declared
        .iter()
        .filter_map(|(key, scope)| {
            let decided = stored
                .iter()
                .find(|g| &g.key == key && g.decided_by == consented);
            let import = (key.kind == PermissionKind::Credentials)
                .then(|| imports.get(key.key.as_str()))
                .flatten();
            // The server the plugin's origin setting holds, not allowed yet.
            let ask = deckpilot_plugin_store::user_origin_setting(key)
                .and_then(|setting| asks.iter().find(|a| a.setting == setting && !a.allowed));
            if !missing.contains(key) && import.is_none() && decided.is_none() && ask.is_none() {
                return None;
            }
            let decision = match decided.map(|g| g.status) {
                // Granted, but its server still waits: not decided.
                Some(GrantStatus::Granted) if ask.is_some() => ipc::ConsentDecision::Pending,
                Some(GrantStatus::Granted) => ipc::ConsentDecision::Approved,
                Some(GrantStatus::Revoked) => ipc::ConsentDecision::Declined,
                _ => ipc::ConsentDecision::Pending,
            };
            let status = match stored.iter().find(|g| &g.key == key) {
                Some(g) if g.status == GrantStatus::Granted && g.scope.covers(scope) => {
                    ipc::PermissionStatus::Granted
                }
                Some(g) if g.status == GrantStatus::Revoked => ipc::PermissionStatus::Revoked,
                _ => ipc::PermissionStatus::NotGranted,
            };
            let (hosts, bound) = match scope {
                deckpilot_plugin_store::GrantScope::Credential {
                    hosts, user_origin, ..
                } => (hosts.clone(), user_origin.clone()),
                _ => (Vec::new(), None),
            };
            let why = match (import, scope.tier()) {
                _ if ask.is_some() => format!(
                    "The built-in {} talked to this server. The plugin may send requests there, \
                     and the credential it gets, only with your consent to exactly this server; \
                     nothing is sent before.",
                    m.module
                ),
                (Some(c), _) => format!(
                    "The built-in {} used your stored credential `{}`. The plugin gets a copy in \
                     its slot `{}`, sent only to {}. The stored credential is kept.",
                    m.module,
                    c.id.as_ref().map(ToString::to_string).unwrap_or_default(),
                    c.slot,
                    match (hosts.is_empty(), &bound) {
                        (true, None) => "the hosts the plugin declares".to_owned(),
                        (true, Some(setting)) => format!("the server you allow in `{setting}`"),
                        (false, None) => hosts.join(", "),
                        (false, Some(setting)) => format!(
                            "{} and the server you allow in `{setting}`",
                            hosts.join(", ")
                        ),
                    }
                ),
                (None, Tier::High) if key.kind == PermissionKind::Credentials => format!(
                    "The plugin's credential slot `{}`: credentials are never granted without \
                     your consent. Nothing is imported.",
                    key.key
                ),
                (None, Tier::High) => {
                    "Private network access is never granted without your consent.".to_owned()
                }
                (None, _) => format!(
                    "The built-in {} did not use this on this computer, so it is not granted \
                     without your consent.",
                    m.module
                ),
            };
            Some(ipc::MigrationConsentItem {
                key: key.to_string(),
                permission: super::convert::permission(manifest, key, scope, status),
                import: import.map(|c| ipc::CredentialImport {
                    setting: c.setting.clone(),
                    credential: c.id.as_ref().map(ToString::to_string).unwrap_or_default(),
                    slot: c.slot.clone(),
                    hosts: hosts.clone(),
                    user_origin: bound.clone(),
                }),
                origin: ask.map(|ask| origin_of_ask(ask, false)),
                why,
                decision,
            })
        })
        .collect();
    // The private network, for each server that needs it: an item of its
    // own, never part of another.
    for ask in asks.iter().filter(|ask| ask.private) {
        let key = private_item(&ask.setting);
        let slot = deckpilot_plugin_store::user_origin_key(&ask.setting);
        let declined = stored.iter().any(|g| {
            g.key == slot && g.status == GrantStatus::Revoked && g.decided_by == consented
        });
        items.push(ipc::MigrationConsentItem {
            key: key.to_string(),
            permission: super::convert::permission(
                manifest,
                &key,
                &deckpilot_plugin_store::GrantScope::PrivateNetwork,
                ipc::PermissionStatus::NotGranted,
            ),
            import: None,
            origin: Some(origin_of_ask(ask, true)),
            why: format!(
                "The built-in {} was allowed to reach this server on a private network. A \
                 plugin reaches a private network only with your separate consent, for exactly \
                 this server; this computer and link-local addresses are never reachable.",
                m.module
            ),
            decision: if declined {
                ipc::ConsentDecision::Declined
            } else {
                ipc::ConsentDecision::Pending
            },
        });
    }
    items
}

fn origin_of_ask(ask: &OriginAsk, private_network: bool) -> ipc::MigrationUserOrigin {
    ipc::MigrationUserOrigin {
        setting: ask.setting.clone(),
        origin: ask.origin.clone(),
        private_network,
        methods: ask.methods.clone(),
        credentials: ask.credentials.clone(),
        opens_pages: ask.opens_pages,
    }
}

/// Test only (feature `test-hooks`, never in a release build): the user's
/// decisions and a crash point, from the environment, so an end-to-end
/// test can decide before the agent starts (the app decides with the
/// requests) or end the agent after a step.
#[cfg(feature = "test-hooks")]
mod test_hooks {
    use deckpilot_plugin_store::MigrationPhase;

    use super::Decision;

    /// Whitespace-separated `consent:<module>` and `retry:<module>`,
    /// applied at the first pass after the store opened.
    pub const DECISIONS_ENV: &str = "DECKPILOT_TEST_MIGRATION_DECISIONS";
    /// A phase: the agent exits (86) right after recording it, as a power
    /// cut would.
    pub const EXIT_ENV: &str = "DECKPILOT_TEST_MIGRATION_EXIT";

    pub fn decisions() -> Vec<Decision> {
        std::env::var(DECISIONS_ENV)
            .unwrap_or_default()
            .split_whitespace()
            .filter_map(|item| match item.split_once(':')? {
                ("consent", module) => Some(Decision::Consent(module.to_owned())),
                ("retry", module) => Some(Decision::Retry(module.to_owned())),
                _ => None,
            })
            .collect()
    }

    pub fn exit_after() -> Option<MigrationPhase> {
        MigrationPhase::parse(&std::env::var(EXIT_ENV).ok()?)
    }
}

#[cfg(not(feature = "test-hooks"))]
mod test_hooks {
    use deckpilot_plugin_store::MigrationPhase;

    pub fn decisions() -> Vec<super::Decision> {
        Vec::new()
    }

    pub fn exit_after() -> Option<MigrationPhase> {
        None
    }
}
