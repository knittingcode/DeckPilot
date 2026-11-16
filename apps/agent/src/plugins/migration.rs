//! The module migration engine (`docs/marketplace/module-migration.md`
//! §5.5–§5.9): moves a built-in module with an alias block to its
//! `migration`-class bundled first-party plugin, one journaled,
//! idempotent step at a time, and decides the [`LegacyRouting`] the core
//! routes the module's legacy action types with.
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
//!    plugin's default and it does not allow private hosts, and plugins
//!    can run here. A tombstone (the user removed or disabled the plugin)
//!    and a bundle without the plugin are reasons too.
//! 2. **Install** the pinned bundled copy (pins plus the macOS code seal,
//!    `bundled.rs`), or keep a copy that is not older, and record the
//!    derived grants (below).
//! 3. **Settings.** Copy the built-in's saved settings that the plugin
//!    declares, only if the plugin has none yet, and record the source
//!    digest. The `modules` row is never changed.
//! 4. **Consent.** Anything not derived waits for the user
//!    (`awaiting_consent`): high-tier permissions always, and the import
//!    of a stored credential always. With the consent, the secret is
//!    copied into the plugin's empty slot; the `credentials` row stays.
//! 5. **Health check** (enabled, files match the pins, every alias target
//!    declared and compatible with the built-in, required settings set),
//!    then **cut over**: the routing becomes `Plugin`.
//!
//! # Consent
//!
//! Low-tier permissions, and medium-tier ones the built-in demonstrably
//! used on this computer (its configured origin for `network`, its web
//! hosts for `openUrl`), are granted as `derived`, decided by
//! `migration:<module>`. High-tier permissions (credentials, the private
//! network) are never derived, and a stored credential is imported only
//! when the user granted its slot through the migration's consent
//! (decided by `user:migration:<module>`); a grant made for other reasons
//! does not import anything.
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
#[cfg(all(test, feature = "test-hooks"))]
mod tests;

pub(crate) use legacy::{LegacyModule, LegacySnapshot};

/// What the alias table does not say about a module: which settings name
/// its credentials and origin. Static, like the alias table, and empty
/// until a module is cut over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MigrationPlan {
    pub module: &'static str,
    /// Credential settings of the built-in and the plugin's slot for each.
    pub credentials: &'static [(&'static str, &'static str)],
    /// The setting with the origin the built-in talks to, and its default:
    /// the only origin the plugin may use until G2 (user-entered origins).
    pub origin: Option<(&'static str, &'static str)>,
    /// The boolean setting that lets the built-in reach private addresses;
    /// `true` is never migrated (G3).
    pub private_network: Option<&'static str>,
    /// Hosts the built-in opens pages on (`openUrl`, derived when the
    /// built-in has the `open-external` capability).
    pub open_url_hosts: &'static [&'static str],
}

/// Every plan. Empty until a module is cut over (P9: GitHub).
pub(crate) const PLANS: &[MigrationPlan] = &[];

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
    deckpilot_modules::configured(states)
}

/// A decision of the user's (P7 makes them requests; agent tests take
/// them from `DECKPILOT_TEST_MIGRATION_DECISIONS`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub(crate) enum Decision {
    /// The one-time consent dialog: grant what is not derived and import
    /// the stored credentials.
    Consent(String),
    /// Retry in Settings › Plugins: a failed migration starts again.
    Retry(String),
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
        keys.extend(plan.origin.map(|(setting, _)| setting));
        keys.extend(plan.private_network);
    }
    keys
}

/// The effective values of the settings that decide eligibility.
fn builtin_part(m: &LegacyModule) -> BTreeMap<String, Value> {
    handled_keys(m)
        .into_iter()
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
    let Some(row) = &m.row else {
        return Ok(BTreeMap::new());
    };
    let handled = handled_keys(m);
    let declared = manifest.contributes.settings();
    let mut values = BTreeMap::new();
    for (key, value) in row.config.iter() {
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

/// Why `m` cannot migrate on this computer, if it cannot (§5.6 step 1).
fn eligibility(m: &LegacyModule) -> Option<String> {
    if m.manifest.is_none() {
        return Some("the built-in module is not part of this app".into());
    }
    if !m.enabled {
        return Some("the built-in module is disabled".into());
    }
    if m.row.is_none() && m.instances == 0 {
        return Some("the built-in module is not used on this computer".into());
    }
    if !m.untranslatable.is_empty() {
        return Some(format!(
            "{} cannot be translated for the plugin (it needs credential slots, G5)",
            m.untranslatable.join("; ")
        ));
    }
    if let Some(plan) = m.plan {
        if let Some((setting, default)) = plan.origin {
            let configured = m.config.text(setting).unwrap_or(default);
            if normalized(configured) != normalized(default) {
                return Some(format!(
                    "`{setting}` is not the default origin (user-entered origins need G2)"
                ));
            }
        }
        if let Some(setting) = plan.private_network
            && m.config.get(setting) == Some(&ParameterValue::Boolean(true))
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
/// (default) origin for `network`, its web hosts for `openUrl`.
fn demonstrated(m: &LegacyModule, key: &GrantKey) -> bool {
    let Some(manifest) = &m.manifest else {
        return false;
    };
    let has = |capability| manifest.capabilities.contains(&capability);
    let Some(plan) = m.plan else {
        return false;
    };
    match key.kind {
        PermissionKind::Network => {
            has(Capability::Network)
                && plan
                    .origin
                    .and_then(|(_, default)| host_of(default))
                    .is_some_and(|host| host == key.key)
        }
        PermissionKind::OpenUrl => {
            has(Capability::OpenExternal) && plan.open_url_hosts.contains(&key.key.as_str())
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

    /// The `migration`-class pin of `m`'s plugin, if this app bundles it.
    fn pin(&self, m: &LegacyModule) -> Option<Pin> {
        self.bundle
            .pins
            .plugins
            .iter()
            .find(|pin| pin.id == m.plugin && pin.class == BundleClass::Migration)
            .cloned()
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
        if let Some(reason) = eligibility(m) {
            return Err(Stop::NotEligible(reason));
        }
        if let Some(tombstone) = self.manager.store().tombstone(&m.plugin).map_err(failed)? {
            return Err(Stop::NotEligible(format!(
                "{} was {} by you",
                m.plugin, tombstone.decision
            )));
        }
        let pin = self
            .pin(m)
            .ok_or_else(|| Stop::NotEligible(format!("this app does not bundle {}", m.plugin)))?;
        self.advance(m, MigrationPhase::Pending, None, None)?;
        self.install(m, &pin)?;
        self.advance(m, MigrationPhase::Installed, None, None)?;
        let digest = self.copy_settings(m)?;
        self.advance(m, MigrationPhase::SettingsCopied, None, digest)?;
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
        Ok(keys)
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
        if let Some(reason) = eligibility(m) {
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
            "the built-in's settings changed after the migration; they were copied again"
        } else {
            "the built-in's settings changed after the migration; the plugin's own settings \
             were kept"
        };
        self.record(m, MigrationPhase::CutOver, Some(note), Some(now))
    }

    /// The user's consent (§5.5): grants what is not derived, decided by
    /// `user:migration:<module>`, which also allows the imports.
    fn consent(&mut self, m: &LegacyModule) -> Result<(), String> {
        let record = self.migration(m);
        if record.as_ref().map(|r| r.phase) != Some(MigrationPhase::AwaitingConsent) {
            return Err(format!(
                "the migration of {} waits for no consent",
                m.module
            ));
        }
        let keys = self.consent_keys(m).map_err(|stop| match stop {
            Stop::Failed(why) | Stop::NotEligible(why) => why,
        })?;
        if !keys.is_empty() {
            self.manager
                .grant(&m.plugin, &Consent::of(keys, consented_by(&m.module)))
                .map_err(|e| e.to_string())?;
            self.touched.insert(m.plugin.clone());
        }
        info!(module = %m.module, plugin = %m.plugin, "the user consented to the migration");
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
) -> Result<(), String> {
    let (Decision::Consent(module) | Decision::Retry(module)) = decision;
    let m = snapshot
        .modules
        .iter()
        .find(|m| m.module.as_str() == module)
        .ok_or_else(|| format!("no module `{module}` can migrate"))?;
    match decision {
        Decision::Consent(_) => engine.consent(m),
        Decision::Retry(_) => {
            engine
                .manager
                .retry_migration(module)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("`{module}` has no migration to retry"))?;
            info!(%module, "the user retries the migration");
            engine.run(m, false);
            Ok(())
        }
    }
}

/// Applies a decision of the user's and runs that module's steps
/// (the consent dialog and Retry of P7).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn decide(
    manager: &mut PluginManager,
    bundle: &Bundle,
    snapshot: &LegacySnapshot,
    decision: &Decision,
) -> Result<BTreeSet<PluginId>, String> {
    let mut engine = Engine {
        manager,
        bundle,
        touched: BTreeSet::new(),
        exit_after: None,
    };
    decide_with(&mut engine, snapshot, decision)?;
    Ok(engine.touched)
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
        let route = if store.tombstone(&record.plugin_id).ok().flatten().is_some() {
            LegacyRoute::PluginWithdrawn
        } else {
            match store.plugin(&record.plugin_id).ok().flatten() {
                Some(plugin)
                    if plugin.source == InstallSource::Bundled
                        && bundle.pin_of(&plugin).is_none() =>
                {
                    LegacyRoute::Builtin
                }
                _ => LegacyRoute::Plugin,
            }
        };
        routing = routing.with(module, route);
    }
    routing
}

/// Test only (feature `test-hooks`, never in a release build): the user's
/// decisions and a crash point, from the environment, so an end-to-end
/// test drives the engine through a real agent before P7 adds the
/// requests.
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
