//! The migration engine against a real plugin store and a real (in-memory)
//! configuration store, with the test fixture: the legacy module
//! `legacy-fixture` (`fixture.rs`) and the bundled plugin
//! `plugins/first-party/test-fixtures/legacy-fixture` (its manifest; a
//! minimal ABI 1 module stands in for its `plugin.wasm`, the end-to-end
//! test `tests/plugins_migration.rs` runs the real one).

use std::path::{Path, PathBuf};

use deckpilot_core::alias::LegacyRoute;
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, CredentialId, ModuleId,
    ModuleSettings, Os, ParameterValue, Profile, Secret,
};
use deckpilot_plugin_bundled::{BUNDLED_DIR, BundleClass, Pins};
use deckpilot_plugin_store::{
    GrantSource, ManagerOptions, MigrationPhase, PluginStore, Retention, StoreKey,
    TombstoneDecision,
};
use deckpilot_storage::{ConfigStore, SqliteStore};
use semver::Version;
use serde_json::{Value, json};

use super::fixture::{self, ACTION, MODULE, PLUGIN};
use super::*;

const FIXTURE: &str = "../../plugins/first-party/test-fixtures/legacy-fixture";
const TOKEN: &str = "s3cret-token";

fn plugin_id() -> PluginId {
    PluginId::parse(PLUGIN).unwrap()
}

fn module_id() -> ModuleId {
    ModuleId::new(MODULE).unwrap()
}

fn fixture_file(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURE)
            .join(name),
    )
    .unwrap()
}

/// A data directory, application resources and an alpha.3-style
/// configuration: the fixture module saved, enabled and used, its
/// credential stored.
struct Env {
    root: tempfile::TempDir,
    config: SqliteStore,
}

impl Env {
    fn new() -> Self {
        let env = Self {
            root: tempfile::tempdir().unwrap(),
            config: SqliteStore::open_in_memory().unwrap(),
        };
        env.save_module(&json!({"greeting": "Hi"}));
        env.config
            .set_credential(
                &CredentialId::new(fixture::DEFAULT_CREDENTIAL).unwrap(),
                "fixture",
                Some(&Secret::new(TOKEN)),
            )
            .unwrap();
        let mut profile = Profile::new("Work");
        profile.actions.push(instance(None));
        env.config.save_profile(&profile).unwrap();
        env
    }

    /// The `modules` row as an older app writes it: `config` as plain
    /// JSON (text and booleans).
    fn save_module(&self, config: &Value) {
        let mut parameters = ActionParameters::new();
        for (key, value) in config.as_object().unwrap() {
            let value = match value {
                Value::Bool(b) => ParameterValue::Boolean(*b),
                Value::String(s) => ParameterValue::Text(s.clone()),
                other => panic!("{other}"),
            };
            parameters = parameters.with(key.clone(), value);
        }
        self.config
            .set_module_state(
                &module_id(),
                &ModuleSettings {
                    enabled: true,
                    config: parameters,
                },
            )
            .unwrap();
    }

    fn bundled(&self) -> PathBuf {
        self.root.path().join("resources").join(BUNDLED_DIR)
    }

    fn open(&self) -> PluginManager {
        PluginManager::open(
            &self.root.path().join("data"),
            &StoreKey::from_bytes([0x42; 32]),
            ManagerOptions::default(),
        )
        .unwrap()
    }

    /// The fixture plugin in the resources, `edit` applied to its
    /// manifest, pinned to exactly these files.
    fn bundle_with(&self, version: &str, edit: impl FnOnce(&mut Value)) -> Bundle {
        let dir = self.bundled().join(PLUGIN);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let mut manifest: Value = serde_json::from_slice(&fixture_file("manifest.json")).unwrap();
        manifest["version"] = version.into();
        edit(&mut manifest);
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("plugin.wasm"),
            deckpilot_plugin_package::wasm::minimal_abi_module_with_note(version),
        )
        .unwrap();
        std::fs::write(dir.join("LICENSE"), fixture_file("LICENSE")).unwrap();
        std::fs::write(
            dir.join("assets/icon-128.png"),
            fixture_file("assets/icon-128.png"),
        )
        .unwrap();
        let pins = Pins::compute(
            &self.bundled(),
            &[(
                plugin_id(),
                Version::parse(version).unwrap(),
                BundleClass::Migration,
            )],
        )
        .unwrap();
        Bundle {
            dir: Some(self.bundled()),
            pins,
            trust: Ok(()),
        }
    }

    fn bundle(&self) -> Bundle {
        self.bundle_with("0.1.0", |_| {})
    }

    /// What the agent's service task hands the worker.
    fn snapshot(&self) -> LegacySnapshot {
        let mut registry = deckpilot_modules::builtin(Os::current()).with_aliases(fixture::ALIASES);
        registry
            .register(Box::new(fixture::FixtureModule::new()))
            .unwrap();
        registry.apply_states(&self.config.module_states().unwrap());
        LegacySnapshot::read(&self.config, &registry).unwrap()
    }
}

/// An instance of the legacy type, with a `credential` override or not.
fn instance(credential: Option<&str>) -> ActionInstance {
    let mut parameters = ActionParameters::new();
    if let Some(credential) = credential {
        parameters = parameters.with("credential", ParameterValue::Text(credential.into()));
    }
    ActionInstance {
        id: ActionInstanceId::new(),
        name: "Who".into(),
        action_type: ActionTypeId::new(ACTION).unwrap(),
        parameters,
        description: String::new(),
        icon: None,
        label: None,
    }
}

fn record(manager: &PluginManager) -> ModuleMigration {
    manager.store().module_migration(MODULE).unwrap().unwrap()
}

fn route(manager: &PluginManager, bundle: &Bundle) -> LegacyRoute {
    routing(manager, bundle).route(&module_id())
}

/// Grants as `(key, source, decided_by)`.
fn grants(store: &PluginStore) -> Vec<(String, GrantSource, String)> {
    store
        .grants(&plugin_id())
        .unwrap()
        .into_iter()
        .filter(|g| g.status == GrantStatus::Granted)
        .map(|g| (g.key.to_string(), g.source, g.decided_by))
        .collect()
}

fn consent(manager: &mut PluginManager, bundle: &Bundle, snapshot: &LegacySnapshot) {
    decide(manager, bundle, snapshot, &Decision::Consent(MODULE.into())).unwrap();
}

/// Migrated through the user's consent, as at the end of the happy path.
fn cut_over(env: &Env, bundle: &Bundle) -> PluginManager {
    let mut manager = env.open();
    let snapshot = env.snapshot();
    start(&mut manager, bundle, &snapshot);
    consent(&mut manager, bundle, &snapshot);
    assert_eq!(
        record(&manager).phase,
        MigrationPhase::CutOver,
        "{:?}",
        record(&manager)
    );
    manager
}

#[test]
fn an_alpha3_configuration_migrates_after_consent() {
    let env = Env::new();
    let bundle = env.bundle();
    let snapshot = env.snapshot();
    let mut manager = env.open();

    // Nothing routed before the engine runs.
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);
    let touched = start(&mut manager, &bundle, &snapshot);
    assert_eq!(touched, BTreeSet::from([plugin_id()]));

    // Installed from the bundle, disabled: the credential slot is high
    // tier, so it waits for the user, and nothing was imported.
    let migration = record(&manager);
    assert_eq!(migration.phase, MigrationPhase::AwaitingConsent);
    let reason = migration.reason.unwrap();
    assert!(reason.contains("credentials:token"), "{reason}");
    assert!(reason.contains("`fixture.token`"), "{reason}");
    assert!(!reason.contains(TOKEN), "{reason}");
    assert_eq!(migration.attempts, 0);
    assert!(migration.source_digest.is_some());
    let store = manager.store();
    let plugin = store.plugin(&plugin_id()).unwrap().unwrap();
    assert_eq!(plugin.source, InstallSource::Bundled);
    assert_eq!(plugin.state, PluginState::Disabled);
    assert_eq!(store.credentials(&plugin_id()).unwrap(), []);
    // Derived per policy: the low tier, and the medium-tier origin the
    // built-in used; never the credential.
    let derived = format!("migration:{MODULE}");
    assert_eq!(
        grants(store),
        [
            (
                "network:api.example.com".to_owned(),
                GrantSource::Derived,
                derived.clone()
            ),
            (
                "notifications".to_owned(),
                GrantSource::Derived,
                derived.clone()
            ),
        ]
    );
    // The built-in's saved settings, one to one; origin and credential
    // settings are never copied.
    assert_eq!(
        store.settings(&plugin_id()).unwrap(),
        BTreeMap::from([("greeting".to_owned(), json!("Hi"))])
    );
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);

    // The user consents: the token is imported, the plugin enabled and
    // healthy, and the module cut over.
    consent(&mut manager, &bundle, &snapshot);
    let migration = record(&manager);
    assert_eq!(migration.phase, MigrationPhase::CutOver, "{migration:?}");
    assert_eq!(migration.reason, None);
    let store = manager.store();
    assert_eq!(
        store.plugin(&plugin_id()).unwrap().unwrap().state,
        PluginState::Enabled
    );
    let data = deckpilot_plugin_store::PluginData::open(
        &manager.layout().database(),
        &StoreKey::from_bytes([0x42; 32]),
        deckpilot_plugin_store::DEFAULT_BUSY_TIMEOUT,
    )
    .unwrap();
    assert_eq!(
        data.credential_secret(&plugin_id(), "token")
            .unwrap()
            .unwrap()
            .expose(),
        TOKEN
    );
    assert!(grants(store).contains(&(
        "credentials:token".to_owned(),
        GrantSource::User,
        format!("user:migration:{MODULE}")
    )));
    assert_eq!(route(&manager, &bundle), LegacyRoute::Plugin);

    // deckpilot.db is as it was: the row, the credential and the profile.
    let rows = env.config.module_states().unwrap();
    assert_eq!(
        rows[&module_id()].config,
        ActionParameters::new().with("greeting", ParameterValue::Text("Hi".into()))
    );
    assert_eq!(env.config.credentials().unwrap().len(), 1);
    let profiles = env.config.list_profiles().unwrap();
    let profile = env.config.load_profile(profiles[0].id).unwrap().unwrap();
    assert_eq!(profile.actions[0].action_type.as_str(), ACTION);
}

#[test]
fn reruns_are_idempotent() {
    let env = Env::new();
    let bundle = env.bundle();
    let snapshot = env.snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);
    let before = (record(&manager), grants(manager.store()));
    let audit = manager.store().audit(&plugin_id()).unwrap().len();
    for _ in 0..2 {
        drop(manager);
        manager = env.open();
        start(&mut manager, &bundle, &snapshot);
    }
    let after = (record(&manager), grants(manager.store()));
    assert_eq!(before.0.phase, after.0.phase);
    assert_eq!(before.0.source_digest, after.0.source_digest);
    assert_eq!(before.1, after.1);
    assert_eq!(manager.store().audit(&plugin_id()).unwrap().len(), audit);

    // Interrupted after each step: the next pass resumes from the record.
    for phase in [
        MigrationPhase::Pending,
        MigrationPhase::Installed,
        MigrationPhase::SettingsCopied,
    ] {
        let env = Env::new();
        let mut manager = env.open();
        let snapshot = env.snapshot();
        manager
            .record_migration(&MigrationStep {
                module_id: MODULE.into(),
                plugin_id: plugin_id(),
                phase,
                reason: None,
                source_digest: None,
                app_version: "0.0.0".into(),
            })
            .unwrap();
        start(&mut manager, &bundle, &snapshot);
        assert_eq!(
            record(&manager).phase,
            MigrationPhase::AwaitingConsent,
            "{phase}"
        );
        consent(&mut manager, &bundle, &snapshot);
        assert_eq!(record(&manager).phase, MigrationPhase::CutOver, "{phase}");
    }

    // A cut-over module is left as it is.
    drop(manager);
    let manager = cut_over(&env, &bundle);
    let done = record(&manager);
    drop(manager);
    let mut manager = env.open();
    start(&mut manager, &bundle, &env.snapshot());
    assert_eq!(record(&manager), done);
}

#[test]
fn a_plain_grant_of_the_slot_imports_nothing() {
    let env = Env::new();
    let bundle = env.bundle();
    let snapshot = env.snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);
    // Settings › Plugins: the user allows the slot, for a token of their
    // own; that is not the migration's consent to import.
    manager
        .grant(
            &plugin_id(),
            &Consent::of(
                [GrantKey::new(PermissionKind::Credentials, "token")],
                "user",
            ),
        )
        .unwrap();
    start(&mut manager, &bundle, &snapshot);
    let migration = record(&manager);
    assert_eq!(migration.phase, MigrationPhase::AwaitingConsent);
    assert!(
        migration.reason.unwrap().contains("import"),
        "nothing imported"
    );
    assert_eq!(manager.store().credentials(&plugin_id()).unwrap(), []);
    // A token the user stored themselves needs no import.
    manager
        .set_credential(&plugin_id(), "token", "their-own")
        .unwrap();
    start(&mut manager, &bundle, &snapshot);
    assert_eq!(record(&manager).phase, MigrationPhase::CutOver);
}

#[test]
fn a_medium_permission_the_builtin_did_not_use_needs_consent() {
    let env = Env::new();
    let bundle = env.bundle_with("0.1.0", |manifest| {
        // A second host the built-in never used.
        manifest["permissions"][1]["hosts"] = json!(["api.example.com", "other.example.com"]);
    });
    let mut manager = env.open();
    start(&mut manager, &bundle, &env.snapshot());
    let reason = record(&manager).reason.unwrap();
    assert!(reason.contains("network:other.example.com"), "{reason}");
    assert!(
        !grants(manager.store())
            .iter()
            .any(|(key, ..)| key == "network:other.example.com")
    );
}

#[test]
fn modules_that_cannot_migrate_stay_on_the_builtin() {
    let not_eligible = |env: &Env, bundle: &Bundle, expect: &str| {
        let mut manager = env.open();
        start(&mut manager, bundle, &env.snapshot());
        let migration = record(&manager);
        assert_eq!(migration.phase, MigrationPhase::NotEligible, "{expect}");
        let reason = migration.reason.unwrap();
        assert!(reason.contains(expect), "{expect}: {reason}");
        assert_eq!(migration.attempts, 0);
        assert!(
            manager.store().plugin(&plugin_id()).unwrap().is_none(),
            "{expect}"
        );
        assert_eq!(route(&manager, bundle), LegacyRoute::Builtin);
    };
    // GitHub Enterprise-style: another origin.
    let env = Env::new();
    env.save_module(&json!({"apiUrl": "https://ghe.example.com/api"}));
    not_eligible(&env, &env.bundle(), "`apiUrl` is not the default origin");
    // Private network.
    let env = Env::new();
    env.save_module(&json!({"allowPrivateHost": true}));
    not_eligible(&env, &env.bundle(), "private network");
    // Disabled.
    let env = Env::new();
    env.config
        .set_module_state(
            &module_id(),
            &ModuleSettings {
                enabled: false,
                config: ActionParameters::new(),
            },
        )
        .unwrap();
    not_eligible(&env, &env.bundle(), "disabled");
    // An instance naming another credential (G5).
    let env = Env::new();
    let mut profile = Profile::new("Other");
    profile.actions.push(instance(Some("someone.else")));
    env.config.save_profile(&profile).unwrap();
    not_eligible(&env, &env.bundle(), "`Who` sets `credential`");
    // The default credential, or none, translates.
    let env = Env::new();
    let mut profile = Profile::new("Other");
    profile
        .actions
        .push(instance(Some(fixture::DEFAULT_CREDENTIAL)));
    profile.actions.push(instance(Some("")));
    env.config.save_profile(&profile).unwrap();
    let mut manager = env.open();
    start(&mut manager, &env.bundle(), &env.snapshot());
    assert_eq!(record(&manager).phase, MigrationPhase::AwaitingConsent);
    // No bundled plugin.
    let env = Env::new();
    let mut bundle = env.bundle();
    bundle.pins = Pins::empty();
    not_eligible(&env, &bundle, "does not bundle");
    // Not used: no saved row, no instance (a fresh install).
    let env = Env {
        root: tempfile::tempdir().unwrap(),
        config: SqliteStore::open_in_memory().unwrap(),
    };
    not_eligible(&env, &env.bundle(), "not used");
}

#[test]
fn a_failed_health_check_leaves_the_builtin_and_retries_are_limited() {
    // A plugin whose action times out sooner than the built-in's: it
    // cannot replace it.
    let env = Env::new();
    let bundle = env.bundle_with("0.1.0", |manifest| {
        manifest["contributes"]["actions"][0]["node"]["timeoutMs"] = json!(1000);
    });
    let snapshot = env.snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);
    consent(&mut manager, &bundle, &snapshot);
    let migration = record(&manager);
    assert_eq!(migration.phase, MigrationPhase::Failed);
    let reason = migration.reason.clone().unwrap();
    assert!(reason.starts_with("health check:"), "{reason}");
    assert!(reason.contains("timeout"), "{reason}");
    assert_eq!(migration.attempts, 1);
    // The built-in takes over again; the plugin is back to disabled
    // (the agent's own, no tombstone).
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);
    assert_eq!(
        manager.store().plugin(&plugin_id()).unwrap().unwrap().state,
        PluginState::Disabled
    );
    assert_eq!(manager.store().tombstone(&plugin_id()).unwrap(), None);

    // Retried at the next starts, up to MAX_MIGRATION_ATTEMPTS.
    for attempts in 2..=deckpilot_plugin_store::MAX_MIGRATION_ATTEMPTS {
        start(&mut manager, &bundle, &snapshot);
        assert_eq!(record(&manager).attempts, attempts);
    }
    let exhausted = record(&manager);
    assert!(!exhausted.retries_automatically());
    start(&mut manager, &bundle, &snapshot);
    assert_eq!(record(&manager), exhausted, "no more automatic attempts");

    // A fixed bundle (an app update) is still not retried on its own…
    let fixed = env.bundle_with("0.1.1", |_| {});
    start(&mut manager, &fixed, &snapshot);
    assert_eq!(record(&manager), exhausted);
    // …only by the user's Retry.
    decide(
        &mut manager,
        &fixed,
        &snapshot,
        &Decision::Retry(MODULE.into()),
    )
    .unwrap();
    let migration = record(&manager);
    assert_eq!(migration.phase, MigrationPhase::CutOver, "{migration:?}");
    assert_eq!(migration.attempts, 0);
    assert_eq!(route(&manager, &fixed), LegacyRoute::Plugin);
}

#[test]
fn a_failed_install_is_journaled_and_retried() {
    let env = Env::new();
    let bundle = env.bundle();
    std::fs::remove_dir_all(env.bundled().join(PLUGIN)).unwrap();
    let snapshot = env.snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);
    let migration = record(&manager);
    assert_eq!(migration.phase, MigrationPhase::Failed);
    assert_eq!(migration.attempts, 1);
    assert!(manager.store().plugin(&plugin_id()).unwrap().is_none());
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);
    // The files are back: the automatic retry installs it.
    let bundle = env.bundle();
    start(&mut manager, &bundle, &snapshot);
    assert_eq!(record(&manager).phase, MigrationPhase::AwaitingConsent);
}

#[test]
fn drift_copies_again_or_keeps_the_plugins_settings() {
    // The plugin's settings are unchanged since the copy: copied again.
    let env = Env::new();
    let bundle = env.bundle();
    let mut manager = cut_over(&env, &bundle);
    env.save_module(&json!({"greeting": "Hello there"}));
    start(&mut manager, &bundle, &env.snapshot());
    let migration = record(&manager);
    assert_eq!(migration.phase, MigrationPhase::CutOver);
    assert!(migration.reason.unwrap().contains("copied again"));
    assert_eq!(
        manager.store().settings(&plugin_id()).unwrap()["greeting"],
        json!("Hello there")
    );
    // Recorded: the next start changes nothing.
    let settled = record(&manager);
    start(&mut manager, &bundle, &env.snapshot());
    assert_eq!(record(&manager), settled);

    // The user changed the plugin's settings: they are kept, with a note.
    manager
        .set_settings(
            &plugin_id(),
            &BTreeMap::from([("greeting".to_owned(), json!("Yo"))]),
        )
        .unwrap();
    env.save_module(&json!({"greeting": "Hey"}));
    start(&mut manager, &bundle, &env.snapshot());
    let migration = record(&manager);
    assert_eq!(migration.phase, MigrationPhase::CutOver);
    assert!(migration.reason.unwrap().contains("were kept"));
    assert_eq!(
        manager.store().settings(&plugin_id()).unwrap()["greeting"],
        json!("Yo")
    );
    assert_eq!(route(&manager, &bundle), LegacyRoute::Plugin);

    // An older app pointed the built-in at another origin: the built-in
    // serves again.
    env.save_module(&json!({"greeting": "Hey", "apiUrl": "https://ghe.example.com/api"}));
    start(&mut manager, &bundle, &env.snapshot());
    assert_eq!(record(&manager).phase, MigrationPhase::NotEligible);
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);
    // And back: migrated again, without asking twice.
    env.save_module(&json!({"greeting": "Hey"}));
    start(&mut manager, &bundle, &env.snapshot());
    assert_eq!(record(&manager).phase, MigrationPhase::CutOver);
}

#[test]
fn the_users_disable_or_uninstall_withdraws_the_legacy_actions() {
    let env = Env::new();
    let bundle = env.bundle();
    let mut manager = cut_over(&env, &bundle);
    manager.disable_by_user(&plugin_id()).unwrap();
    assert_eq!(route(&manager, &bundle), LegacyRoute::PluginWithdrawn);
    start(&mut manager, &bundle, &env.snapshot());
    assert_eq!(route(&manager, &bundle), LegacyRoute::PluginWithdrawn);
    assert_eq!(
        manager.store().plugin(&plugin_id()).unwrap().unwrap().state,
        PluginState::Disabled
    );
    // The user enables it again: their undo.
    manager.clear_tombstone(&plugin_id()).unwrap();
    manager.enable(&plugin_id()).unwrap();
    assert_eq!(route(&manager, &bundle), LegacyRoute::Plugin);

    // Uninstalled, then an app update with a newer bundled copy: never
    // installed again.
    manager
        .uninstall_by_user(&plugin_id(), Retention::default())
        .unwrap();
    assert_eq!(
        manager
            .store()
            .tombstone(&plugin_id())
            .unwrap()
            .unwrap()
            .decision,
        TombstoneDecision::Uninstalled
    );
    assert_eq!(route(&manager, &bundle), LegacyRoute::PluginWithdrawn);
    let newer = env.bundle_with("0.2.0", |_| {});
    start(&mut manager, &newer, &env.snapshot());
    assert!(manager.store().plugin(&plugin_id()).unwrap().is_none());
    assert_eq!(route(&manager, &newer), LegacyRoute::PluginWithdrawn);
}

#[test]
fn a_build_without_the_plugin_routes_to_the_builtin() {
    let env = Env::new();
    let bundle = env.bundle();
    let manager = cut_over(&env, &bundle);
    assert_eq!(route(&manager, &bundle), LegacyRoute::Plugin);
    // An agent that pins no such copy cannot load it: the built-in serves.
    let mut other = bundle.clone();
    other.pins = Pins::empty();
    assert_eq!(route(&manager, &other), LegacyRoute::Builtin);
}

#[test]
fn derived_grants_are_never_high_tier_and_never_overwrite() {
    let env = Env::new();
    let bundle = env.bundle();
    let mut manager = env.open();
    start(&mut manager, &bundle, &env.snapshot());
    let credential = GrantKey::new(PermissionKind::Credentials, "token");
    assert!(
        manager
            .derive_grants(
                &plugin_id(),
                &BTreeSet::from([credential]),
                "migration:legacy-fixture"
            )
            .is_err()
    );
    // A revocation stays.
    let network = GrantKey::new(PermissionKind::Network, "api.example.com");
    manager.revoke(&plugin_id(), &network, "user").unwrap();
    assert_eq!(
        manager
            .derive_grants(
                &plugin_id(),
                &BTreeSet::from([network]),
                "migration:legacy-fixture"
            )
            .unwrap(),
        []
    );
    start(&mut manager, &bundle, &env.snapshot());
    assert!(
        !grants(manager.store())
            .iter()
            .any(|(key, ..)| key == "network:api.example.com")
    );
}

#[test]
fn the_snapshot_reads_nothing_without_aliases() {
    let env = Env::new();
    let registry = deckpilot_modules::builtin(Os::current());
    assert!(
        LegacySnapshot::read(&env.config, &registry)
            .unwrap()
            .modules
            .is_empty()
    );
    let snapshot = env.snapshot();
    let module = &snapshot.modules[0];
    assert_eq!(module.module, module_id());
    assert_eq!(module.instances, 1);
    assert_eq!(module.credentials.len(), 1);
    assert_eq!(module.credentials[0].slot, "token");
    // Secrets never show in debug output.
    assert!(!format!("{snapshot:?}").contains(TOKEN));
}

fn decide_items(approve: &[&str], decline: &[&str]) -> Decision {
    let keys = |items: &[&str]| {
        items
            .iter()
            .map(|k| k.parse::<GrantKey>().unwrap())
            .collect()
    };
    Decision::Decide {
        module: MODULE.into(),
        approve: keys(approve),
        decline: keys(decline),
    }
}

/// The consent dialog item by item: the app lists what waits (by name,
/// never the secret), a decline keeps the built-in across starts and
/// grants nothing, Retry asks again, and the approval cuts over.
#[test]
fn consent_is_decided_item_by_item() {
    let env = Env::new();
    let bundle = env.bundle();
    let snapshot = env.snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);

    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert_eq!(info.phase, ipc::MigrationPhase::AwaitingConsent);
    assert_eq!(info.module_name, "Legacy Fixture");
    assert_eq!(info.route, ipc::MigrationRoute::Builtin);
    assert!(!info.can_retry);
    assert_eq!(info.aliases.len(), 1);
    let [item] = info.consent.as_slice() else {
        panic!("{info:?}");
    };
    assert_eq!(item.key, "credentials:token");
    assert_eq!(item.decision, ipc::ConsentDecision::Pending);
    let import = item.import.as_ref().unwrap();
    assert_eq!(import.credential, fixture::DEFAULT_CREDENTIAL);
    assert_eq!(import.hosts, ["api.example.com"]);
    assert!(!serde_json::to_string(&info).unwrap().contains(TOKEN));
    // Without the snapshot only the journal is known.
    let bare = infos(&manager, &bundle, None).pop().unwrap();
    assert_eq!(bare.module_name, MODULE);
    assert_eq!(bare.consent, []);

    // What does not wait, and a Retry of a waiting migration, are refused.
    assert!(matches!(
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &decide_items(&["notifications"], &[])
        ),
        Err(Refusal::UnknownItem(_))
    ));
    assert!(matches!(
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &Decision::Retry(MODULE.into())
        ),
        Err(Refusal::InvalidState(_))
    ));
    assert!(matches!(
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &Decision::Retry("someone-else".into())
        ),
        Err(Refusal::Unknown(_))
    ));

    // Declined: nothing granted or imported, the built-in stays, at every
    // start.
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &decide_items(&[], &["credentials:token"]),
    )
    .unwrap();
    let declined = record(&manager);
    assert_eq!(declined.phase, MigrationPhase::NotEligible);
    assert!(
        declined
            .reason
            .as_deref()
            .unwrap()
            .contains("you declined credentials:token")
    );
    assert_eq!(manager.store().credentials(&plugin_id()).unwrap(), []);
    assert!(
        !grants(manager.store())
            .iter()
            .any(|(key, ..)| key == "credentials:token")
    );
    for _ in 0..2 {
        start(&mut manager, &bundle, &snapshot);
        assert_eq!(record(&manager).phase, MigrationPhase::NotEligible);
    }
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert!(info.can_retry);
    assert_eq!(info.consent[0].decision, ipc::ConsentDecision::Declined);
    assert!(matches!(
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &decide_items(&["credentials:token"], &[])
        ),
        Err(Refusal::InvalidState(_))
    ));

    // Retry asks again; the approval cuts over.
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &Decision::Retry(MODULE.into()),
    )
    .unwrap();
    assert_eq!(record(&manager).phase, MigrationPhase::AwaitingConsent);
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert_eq!(info.consent[0].decision, ipc::ConsentDecision::Pending);
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &decide_items(&["credentials:token"], &[]),
    )
    .unwrap();
    assert_eq!(record(&manager).phase, MigrationPhase::CutOver);
    assert_eq!(route(&manager, &bundle), LegacyRoute::Plugin);
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert_eq!(info.route, ipc::MigrationRoute::Plugin);
    assert_eq!(info.consent, []);
    assert!(matches!(
        event_of(&record(&manager)),
        ipc::PluginEvent::MigrationChanged {
            phase: ipc::MigrationPhase::CutOver,
            ..
        }
    ));
}
