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

/// The user's Restore of the plugin they uninstalled (`restoreBundledPlugin`)
/// routes the migrated module's actions to it again.
#[test]
fn a_restore_after_the_users_uninstall_routes_to_the_plugin_again() {
    let env = Env::new();
    let bundle = env.bundle();
    let mut manager = cut_over(&env, &bundle);
    manager
        .uninstall_by_user(&plugin_id(), Retention::default())
        .unwrap();
    assert_eq!(route(&manager, &bundle), LegacyRoute::PluginWithdrawn);
    super::super::bundled::restore(&mut manager, &bundle, &plugin_id()).unwrap();
    assert!(manager.store().tombstone(&plugin_id()).unwrap().is_none());
    assert!(manager.store().plugin(&plugin_id()).unwrap().is_some());
    assert_eq!(route(&manager, &bundle), LegacyRoute::Plugin);
    assert_eq!(record(&manager).phase, MigrationPhase::CutOver);
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
    let registry = deckpilot_modules::builtin(Os::current()).with_aliases(&[]);
    assert!(
        LegacySnapshot::read(&env.config, &registry)
            .unwrap()
            .modules
            .is_empty()
    );
    // The shipped table: GitHub's block, with its plan, ...
    let registry = deckpilot_modules::builtin(Os::current());
    let shipped = LegacySnapshot::read(&env.config, &registry).unwrap();
    assert_eq!(
        shipped
            .modules
            .iter()
            .map(|m| m.module.as_str())
            .collect::<Vec<_>>(),
        [
            "docker",
            "git",
            "github",
            "gitlab",
            "jira",
            "notifications",
            "projects",
            "workspace"
        ]
    );
    // None of the extraction set is used by this data: no backup is due.
    assert!(!shipped.needs_backup());
    let github = shipped
        .modules
        .iter()
        .find(|m| m.module.as_str() == "github")
        .unwrap();
    assert_eq!(github.module.as_str(), "github");
    assert_eq!(github.plugin.as_str(), "deckpilot.github");
    assert_eq!(github.aliases.len(), 6);
    assert_eq!(github.plan, super::plan_for("github"));
    assert_eq!(github.credentials[0].setting, "credential");
    assert_eq!(github.credentials[0].slot, "token");
    assert_eq!(
        github.credentials[0].id.as_ref().map(CredentialId::as_str),
        Some("github.token"),
        "the module's default credential"
    );
    // ... and the notifications block (P10), with its plan,
    // never used by this data and still eligible, because its plugin is
    // of the `default` class (§4.3); a `migration`-class plugin is not.
    let notifications = shipped
        .modules
        .iter()
        .find(|m| m.module.as_str() == "notifications")
        .unwrap_or_else(|| panic!("{shipped:?}"));
    assert_eq!(notifications.plugin.as_str(), "deckpilot.notifications");
    assert_eq!(notifications.aliases.len(), 2);
    assert_eq!(notifications.plan, super::plan_for("notifications"));
    assert!(notifications.credentials.is_empty());
    assert_eq!(
        (notifications.row.as_ref(), notifications.instances),
        (None, 0)
    );
    if crate::plugins::execution_supported() {
        assert_eq!(eligibility(notifications, true), None);
    }
    assert_eq!(
        eligibility(notifications, false).as_deref(),
        Some("the built-in module is not used on this computer")
    );
    let mut off = notifications.clone();
    off.enabled = false;
    assert_eq!(
        eligibility(&off, true).as_deref(),
        Some("the built-in module is disabled")
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

/// The shipped GitHub plan (P9): GitHub's API URL and the module's own
/// credential are eligible, and so is a GitHub Enterprise Server (another
/// `apiUrl`, carried into the plugin's user-origin slot and asked for),
/// also with private hosts allowed (asked for separately). A private
/// api.github.com, a server no plugin can reach and a per-action
/// credential other than the module's keep the built-in, each with its
/// reason.
#[test]
fn the_github_plan_takes_enterprise_servers_and_keeps_private_hosts_and_overrides() {
    fn github(config: &[(&str, ParameterValue)], credential: Option<&str>) -> Option<String> {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut parameters = ActionParameters::new();
        for (key, value) in config {
            parameters = parameters.with(*key, value.clone());
        }
        store
            .set_module_state(
                &ModuleId::new("github").unwrap(),
                &ModuleSettings {
                    enabled: true,
                    config: parameters,
                },
            )
            .unwrap();
        let mut profile = Profile::new("Work");
        let mut params =
            ActionParameters::new().with("repository", ParameterValue::Text("octo/site".into()));
        if let Some(credential) = credential {
            params = params.with("credential", ParameterValue::Text(credential.into()));
        }
        profile.actions.push(ActionInstance {
            id: ActionInstanceId::new(),
            name: "PRs".into(),
            action_type: ActionTypeId::new("github.pull-requests").unwrap(),
            parameters: params,
            description: String::new(),
            icon: None,
            label: None,
        });
        store.save_profile(&profile).unwrap();
        let mut registry = deckpilot_modules::builtin(Os::current());
        registry.apply_states(&store.module_states().unwrap());
        let snapshot = LegacySnapshot::read(&store, &registry).unwrap();
        let module = snapshot
            .modules
            .iter()
            .find(|m| m.module.as_str() == "github")
            .unwrap_or_else(|| panic!("{snapshot:?}"));
        assert_eq!(module.plan, plan_for("github"));
        eligibility(module, false)
    }
    let text = |s: &str| ParameterValue::Text(s.into());
    let eligible = |reason: Option<String>| {
        if crate::plugins::execution_supported() {
            assert_eq!(reason, None);
        }
    };
    eligible(github(&[], None));
    eligible(github(
        &[("apiUrl", text("https://api.github.com/"))],
        Some(""),
    ));
    eligible(github(
        &[("credential", text("work.github"))],
        Some("work.github"),
    ));
    eligible(github(&[], Some("github.token")));

    // GitHub Enterprise Server: eligible; the consent to the server (and,
    // with private hosts allowed, to the private network) is asked when
    // the migration runs.
    eligible(github(
        &[("apiUrl", text("https://ghe.example.com/api/v3"))],
        None,
    ));
    eligible(github(
        &[
            ("apiUrl", text("https://ghe.internal/api/v3")),
            ("allowPrivateHost", ParameterValue::Boolean(true)),
        ],
        None,
    ));
    let plain = github(&[("apiUrl", text("http://ghe.example.com/api/v3"))], None).unwrap();
    assert!(
        plain.contains("is not a server a plugin can reach"),
        "{plain}"
    );
    let private = github(&[("allowPrivateHost", ParameterValue::Boolean(true))], None).unwrap();
    assert!(private.contains("`allowPrivateHost`"), "{private}");
    let other = github(&[], Some("other.token")).unwrap();
    assert!(
        other.contains("`PRs` sets `credential`") && other.contains("G5"),
        "{other}"
    );
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
/// never the secret), and only what waits can be decided.
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
}

/// A decline keeps the built-in across starts and grants nothing, Retry
/// asks again, and the approval cuts over.
#[test]
fn a_decline_holds_until_retry() {
    let env = Env::new();
    let bundle = env.bundle();
    let snapshot = env.snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);

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

// ------------------------------------------- a server the user entered

/// The fixture's plan for a plugin that takes a server the user entered
/// (as the shipped GitLab plan): `apiUrl` is the plugin's user-origin
/// slot, `https://api.example.com` its declared default.
static ENTERED: MigrationPlan = MigrationPlan {
    module: MODULE,
    credentials: &[("credential", "token")],
    origin: Some(OriginPlan {
        setting: "apiUrl",
        default: Some(fixture::DEFAULT_API_URL),
        entered: true,
    }),
    private_network: Some("allowPrivateHost"),
    open_url_hosts: &[],
    extraction: None,
};

/// The server the user entered in the built-in.
const OWN: &str = "https://code.example.com/gitlab/api/v4";
const SLOT: &str = "network:user-origin:apiUrl";
const PRIVATE: &str = "network:user-origin:apiUrl:private-network";

impl Env {
    /// The fixture plugin with the origin setting `apiUrl` as a
    /// user-origin slot (a path, a private network), the token bound to
    /// it.
    fn entered_bundle(&self) -> Bundle {
        self.bundle_with("0.1.0", |manifest| {
            manifest["permissions"][1]["userOrigins"] = json!([
                {"setting": "apiUrl", "pathPrefix": true, "privateNetwork": true}
            ]);
            manifest["contributes"]["settings"]["fields"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "key": "apiUrl", "label": "API URL", "required": true,
                    "kind": {"type": "text", "maxLength": 512},
                    "default": {"type": "text", "value": "https://api.example.com"},
                    "portability": "machineSpecific"
                }));
            manifest["contributes"]["credentials"][0]["userOrigin"] = json!("apiUrl");
        })
    }

    /// The snapshot with the [`ENTERED`] plan.
    fn entered_snapshot(&self) -> LegacySnapshot {
        let mut snapshot = self.snapshot();
        snapshot.modules[0].plan = Some(&ENTERED);
        snapshot
    }
}

fn user_origins(manager: &PluginManager) -> Vec<(String, bool, String)> {
    manager
        .store()
        .user_origins(&plugin_id())
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.consent.display(),
                row.consent.private_network,
                row.decided_by,
            )
        })
        .collect()
}

fn setting(manager: &PluginManager, key: &str) -> Option<Value> {
    manager
        .store()
        .settings(&plugin_id())
        .unwrap()
        .get(key)
        .cloned()
}

fn waiting(info: &ipc::ModuleMigrationInfo) -> Vec<&str> {
    info.consent
        .iter()
        .filter(|i| i.decision == ipc::ConsentDecision::Pending)
        .map(|i| i.key.as_str())
        .collect()
}

/// The server the user entered in the built-in is carried into the
/// plugin's origin setting and allowed only by the user's decision on its
/// own item: never derived, never by another item's approval.
#[test]
#[allow(clippy::too_many_lines)] // one migration, step by step
fn an_entered_server_is_carried_and_allowed_only_by_its_own_consent() {
    let env = Env::new();
    env.save_module(
        &json!({"greeting": "Hi", "apiUrl": "https://Code.Example.com:443/gitlab/api/v4/"}),
    );
    let bundle = env.entered_bundle();
    let snapshot = env.entered_snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);

    // Copied, normalized as the agent stores what a user saves there.
    let migration = record(&manager);
    assert_eq!(
        migration.phase,
        MigrationPhase::AwaitingConsent,
        "{migration:?}"
    );
    assert_eq!(setting(&manager, "apiUrl"), Some(json!(OWN)));
    assert_eq!(setting(&manager, "greeting"), Some(json!("Hi")));
    // Nothing allowed: no consent to the server, and neither the slot nor
    // the default origin (which the built-in did not talk to) is derived.
    assert_eq!(user_origins(&manager), []);
    let granted = grants(manager.store());
    assert!(
        !granted
            .iter()
            .any(|(key, ..)| key == SLOT || key == "network:api.example.com"),
        "{granted:?}"
    );
    // The journal names the setting, never the server.
    let reason = migration.reason.unwrap();
    assert!(
        reason.contains("the server entered in `apiUrl`"),
        "{reason}"
    );
    assert!(!reason.contains("code.example.com"), "{reason}");

    // The app is told which server, what goes there, and why.
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert_eq!(
        waiting(&info),
        ["network:api.example.com", SLOT, "credentials:token"],
        "{info:?}"
    );
    let server = info.consent.iter().find(|i| i.key == SLOT).unwrap();
    assert_eq!(
        server.origin,
        Some(ipc::MigrationUserOrigin {
            setting: "apiUrl".into(),
            origin: OWN.into(),
            private_network: false,
            methods: vec!["GET".into()],
            credentials: vec!["token".into()],
            opens_pages: false,
        })
    );
    assert_eq!(server.permission.tier, ipc::PermissionTier::High);
    assert!(server.why.contains("exactly this server"), "{}", server.why);
    let token = info
        .consent
        .iter()
        .find(|i| i.key == "credentials:token")
        .unwrap();
    let import = token.import.as_ref().unwrap();
    assert_eq!(import.user_origin.as_deref(), Some("apiUrl"));
    assert!(
        token
            .why
            .contains("api.example.com and the server you allow in `apiUrl`"),
        "{}",
        token.why
    );

    // Approving other items allows no server.
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &decide_items(&["credentials:token", "network:api.example.com"], &[]),
    )
    .unwrap();
    assert_eq!(record(&manager).phase, MigrationPhase::AwaitingConsent);
    assert_eq!(user_origins(&manager), []);
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert_eq!(waiting(&info), [SLOT]);

    // Its own item: allowed, exactly that server, by this consent.
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &decide_items(&[SLOT], &[]),
    )
    .unwrap();
    assert_eq!(
        record(&manager).phase,
        MigrationPhase::CutOver,
        "{:?}",
        record(&manager)
    );
    assert_eq!(
        user_origins(&manager),
        [(OWN.to_owned(), false, format!("user:migration:{MODULE}"))]
    );
    assert_eq!(route(&manager, &bundle), LegacyRoute::Plugin);
    assert_eq!(
        manager.store().credentials(&plugin_id()).unwrap().len(),
        1,
        "the token was imported"
    );
    // Nothing waits any more.
    assert_eq!(
        infos(&manager, &bundle, Some(&snapshot))
            .pop()
            .unwrap()
            .consent,
        []
    );
}

/// A slot grant made elsewhere (Settings › Plugins) never allows the
/// server: the migration still asks, and records the consent only with
/// the decision that names it.
#[test]
fn a_slot_granted_elsewhere_does_not_allow_the_server() {
    let env = Env::new();
    env.save_module(&json!({ "apiUrl": OWN }));
    let bundle = env.entered_bundle();
    let snapshot = env.entered_snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);
    manager
        .grant(
            &plugin_id(),
            &Consent::of([SLOT.parse::<GrantKey>().unwrap()], "user"),
        )
        .unwrap();
    start(&mut manager, &bundle, &snapshot);
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &decide_items(&["credentials:token", "network:api.example.com"], &[]),
    )
    .unwrap();
    assert_eq!(record(&manager).phase, MigrationPhase::AwaitingConsent);
    assert_eq!(user_origins(&manager), []);
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert_eq!(waiting(&info), [SLOT]);
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &decide_items(&[SLOT], &[]),
    )
    .unwrap();
    assert_eq!(record(&manager).phase, MigrationPhase::CutOver);
    assert_eq!(user_origins(&manager).len(), 1);
}

/// A server the built-in reached on a private network: the private
/// network is an item of its own, and the server is allowed only with
/// both, in one decision.
#[test]
fn a_private_network_is_asked_separately_for_exactly_that_server() {
    for url in [
        // A public name the built-in was allowed private addresses for.
        OWN,
        // A host that is private itself.
        "https://gitlab.internal/api/v4",
        "https://10.0.0.5/api/v4",
    ] {
        let env = Env::new();
        env.save_module(&json!({"apiUrl": url, "allowPrivateHost": true}));
        let bundle = env.entered_bundle();
        let snapshot = env.entered_snapshot();
        let mut manager = env.open();
        start(&mut manager, &bundle, &snapshot);
        let migration = record(&manager);
        assert_eq!(
            migration.phase,
            MigrationPhase::AwaitingConsent,
            "{url}: {migration:?}"
        );
        assert!(
            migration
                .reason
                .as_deref()
                .unwrap()
                .contains("the private network for the server entered in `apiUrl`"),
            "{migration:?}"
        );
        let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
        let private = info.consent.iter().find(|i| i.key == PRIVATE).unwrap();
        assert_eq!(private.permission.tier, ipc::PermissionTier::High);
        assert_eq!(private.decision, ipc::ConsentDecision::Pending);
        let asked = private.origin.as_ref().unwrap();
        assert!(asked.private_network);
        assert_eq!(asked.origin, url);

        // Everything but the private network: the server is not allowed.
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &decide_items(&["credentials:token", "network:api.example.com", SLOT], &[]),
        )
        .unwrap();
        assert_eq!(
            record(&manager).phase,
            MigrationPhase::AwaitingConsent,
            "{url}"
        );
        assert_eq!(user_origins(&manager), [], "{url}");
        // The private network alone does not allow the server either: the
        // server's item waits again, with it.
        let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
        assert_eq!(waiting(&info), [SLOT, PRIVATE], "{url}");
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &decide_items(&[PRIVATE], &[]),
        )
        .unwrap();
        assert_eq!(user_origins(&manager), [], "{url}");
        // Both, in one decision.
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &decide_items(&[SLOT, PRIVATE], &[]),
        )
        .unwrap();
        assert_eq!(
            record(&manager).phase,
            MigrationPhase::CutOver,
            "{url}: {:?}",
            record(&manager)
        );
        assert_eq!(
            user_origins(&manager),
            [(url.to_owned(), true, format!("user:migration:{MODULE}"))]
        );
        // The private-network item was never stored as a grant.
        assert!(
            !manager
                .store()
                .grants(&plugin_id())
                .unwrap()
                .iter()
                .any(|g| g.key.to_string() == PRIVATE)
        );
    }
}

/// Declining the private network declines the server: the built-in
/// stays, at every start, until Retry.
#[test]
fn a_declined_private_network_keeps_the_builtin() {
    let env = Env::new();
    env.save_module(&json!({"apiUrl": OWN, "allowPrivateHost": true}));
    let bundle = env.entered_bundle();
    let snapshot = env.entered_snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &decide_items(
            &["credentials:token", "network:api.example.com", SLOT],
            &[PRIVATE],
        ),
    )
    .unwrap();
    let declined = record(&manager);
    assert_eq!(declined.phase, MigrationPhase::NotEligible, "{declined:?}");
    assert!(
        declined
            .reason
            .as_deref()
            .unwrap()
            .contains(&format!("you declined {SLOT}")),
        "{declined:?}"
    );
    assert_eq!(user_origins(&manager), []);
    assert_eq!(manager.store().credentials(&plugin_id()).unwrap(), []);
    start(&mut manager, &bundle, &snapshot);
    assert_eq!(record(&manager).phase, MigrationPhase::NotEligible);
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert!(info.can_retry);
    let private = info.consent.iter().find(|i| i.key == PRIVATE).unwrap();
    assert_eq!(private.decision, ipc::ConsentDecision::Declined);
    // Retry asks again; allowing everything cuts over.
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &Decision::Retry(MODULE.into()),
    )
    .unwrap();
    assert_eq!(record(&manager).phase, MigrationPhase::AwaitingConsent);
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &Decision::Consent(MODULE.into()),
    )
    .unwrap();
    assert_eq!(
        record(&manager).phase,
        MigrationPhase::CutOver,
        "{:?}",
        record(&manager)
    );
    assert!(user_origins(&manager)[0].1);
}

/// What a plugin can never reach keeps the built-in: plain `http`, this
/// computer, a user name in the address; and the private switch for the
/// default origin, which has no per-server consent.
#[test]
fn servers_a_plugin_cannot_reach_stay_on_the_builtin() {
    for (config, expect) in [
        (
            json!({"apiUrl": "http://code.example.com/api/v4"}),
            "`apiUrl` is not a server a plugin can reach",
        ),
        (
            json!({"apiUrl": "http://127.0.0.1:8080/api/v4", "allowPrivateHost": true}),
            "`apiUrl` is not a server a plugin can reach",
        ),
        (
            json!({"apiUrl": "https://localhost/api/v4", "allowPrivateHost": true}),
            "`apiUrl` is not a server a plugin can reach",
        ),
        (
            json!({"apiUrl": "https://user:pw@code.example.com/api/v4"}),
            "`apiUrl` is not a server a plugin can reach",
        ),
        (
            json!({"allowPrivateHost": true}),
            "`allowPrivateHost` allows private network hosts",
        ),
    ] {
        let env = Env::new();
        env.save_module(&config);
        let bundle = env.entered_bundle();
        let mut manager = env.open();
        start(&mut manager, &bundle, &env.entered_snapshot());
        let migration = record(&manager);
        assert_eq!(migration.phase, MigrationPhase::NotEligible, "{config}");
        let reason = migration.reason.unwrap();
        assert!(reason.contains(expect), "{config}: {reason}");
        // Never the value.
        assert!(
            !reason.contains("example.com") && !reason.contains("pw"),
            "{reason}"
        );
        assert!(
            manager.store().plugin(&plugin_id()).unwrap().is_none(),
            "{config}"
        );
    }
    // The default origin: nothing to carry, nothing to ask about a server;
    // its host is derived as before.
    let env = Env::new();
    let bundle = env.entered_bundle();
    let snapshot = env.entered_snapshot();
    let mut manager = env.open();
    start(&mut manager, &bundle, &snapshot);
    assert_eq!(setting(&manager, "apiUrl"), None);
    assert!(grants(manager.store()).iter().any(|(key, source, _)| key
        == "network:api.example.com"
        && *source == GrantSource::Derived));
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    // The slot itself is high tier (a private network): asked, without a
    // server.
    assert_eq!(waiting(&info), [SLOT, "credentials:token"]);
    assert_eq!(info.consent[0].origin, None);
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &Decision::Consent(MODULE.into()),
    )
    .unwrap();
    assert_eq!(record(&manager).phase, MigrationPhase::CutOver);
    assert_eq!(user_origins(&manager), []);
}

/// An older app pointed the built-in at another server after the
/// cut-over: the settings are copied again, the built-in serves again,
/// and the new server waits for its own consent.
#[test]
fn another_server_after_the_cut_over_asks_again() {
    let env = Env::new();
    env.save_module(&json!({ "apiUrl": OWN }));
    let bundle = env.entered_bundle();
    let mut manager = env.open();
    let snapshot = env.entered_snapshot();
    start(&mut manager, &bundle, &snapshot);
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &Decision::Consent(MODULE.into()),
    )
    .unwrap();
    assert_eq!(record(&manager).phase, MigrationPhase::CutOver);

    let other = "https://git.example.org/api/v4";
    env.save_module(&json!({ "apiUrl": other }));
    let snapshot = env.entered_snapshot();
    start(&mut manager, &bundle, &snapshot);
    let migration = record(&manager);
    assert_eq!(
        migration.phase,
        MigrationPhase::AwaitingConsent,
        "{migration:?}"
    );
    assert!(
        migration
            .reason
            .as_deref()
            .unwrap()
            .contains("waits for your consent"),
        "{migration:?}"
    );
    assert_eq!(route(&manager, &bundle), LegacyRoute::Builtin);
    assert_eq!(setting(&manager, "apiUrl"), Some(json!(other)));
    // The consent to the first server allows nothing for the second.
    let info = infos(&manager, &bundle, Some(&snapshot)).pop().unwrap();
    assert_eq!(waiting(&info), [SLOT], "{info:?}");
    let server = info.consent.iter().find(|i| i.key == SLOT).unwrap();
    assert_eq!(server.origin.as_ref().unwrap().origin, other);
    decide(
        &mut manager,
        &bundle,
        &snapshot,
        &decide_items(&[SLOT], &[]),
    )
    .unwrap();
    assert_eq!(
        record(&manager).phase,
        MigrationPhase::CutOver,
        "{:?}",
        record(&manager)
    );
    assert_eq!(user_origins(&manager)[0].0, other);
    assert_eq!(route(&manager, &bundle), LegacyRoute::Plugin);
}

/// A module as an alpha.3 configuration has it: saved with `config`,
/// enabled, and used by one action instance of `action`.
fn shipped_module(
    id: &str,
    action: &str,
    config: &[(&str, ParameterValue)],
    credential: Option<&str>,
) -> LegacyModule {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut parameters = ActionParameters::new();
    for (key, value) in config {
        parameters = parameters.with(*key, value.clone());
    }
    store
        .set_module_state(
            &ModuleId::new(id).unwrap(),
            &ModuleSettings {
                enabled: true,
                config: parameters,
            },
        )
        .unwrap();
    let mut profile = Profile::new("Work");
    let mut params = ActionParameters::new();
    if let Some(credential) = credential {
        params = params.with("credential", ParameterValue::Text(credential.into()));
    }
    profile.actions.push(ActionInstance {
        id: ActionInstanceId::new(),
        name: "Mine".into(),
        action_type: ActionTypeId::new(action).unwrap(),
        parameters: params,
        description: String::new(),
        icon: None,
        label: None,
    });
    store.save_profile(&profile).unwrap();
    let mut registry = deckpilot_modules::builtin(Os::current());
    registry.apply_states(&store.module_states().unwrap());
    let snapshot = LegacySnapshot::read(&store, &registry).unwrap();
    let module = snapshot
        .modules
        .into_iter()
        .find(|m| m.module.as_str() == id)
        .unwrap();
    assert_eq!(module.plan, plan_for(id));
    module
}

/// The shipped GitLab and Jira plans (P11): GitLab.com and a server the
/// user entered are eligible, also with the private switch for such a
/// server; what a plugin cannot reach and a per-action credential other
/// than the module's keep the built-in. Jira has no default: its site is
/// always a server the user entered.
#[test]
fn the_gitlab_and_jira_plans_take_a_server_the_user_entered() {
    let text = |s: &str| ParameterValue::Text(s.into());
    let on = ParameterValue::Boolean(true);
    let eligible = |m: &LegacyModule| {
        if crate::plugins::execution_supported() {
            assert_eq!(eligibility(m, false), None, "{:?}", m.config);
        }
    };
    let gitlab = |config: &[(&str, ParameterValue)], credential: Option<&str>| {
        shipped_module("gitlab", "gitlab.merge-requests", config, credential)
    };
    let jira =
        |config: &[(&str, ParameterValue)]| shipped_module("jira", "jira.issues", config, None);

    // GitLab.com: the declared host, nothing entered.
    let m = gitlab(&[], None);
    eligible(&m);
    assert_eq!(entered_origin(&m), None);
    assert_eq!(m.credentials[0].slot, "token");
    assert_eq!(
        m.credentials[0].id.as_ref().map(CredentialId::as_str),
        Some("gitlab.token")
    );
    eligible(&gitlab(
        &[("apiUrl", text("https://gitlab.com/api/v4/"))],
        Some("gitlab.token"),
    ));
    // A self-managed GitLab, under a path, also on a private network.
    for config in [
        vec![("apiUrl", text("https://code.example.com/gitlab/api/v4"))],
        vec![
            ("apiUrl", text("https://gitlab.internal/api/v4")),
            ("allowPrivateHost", on.clone()),
        ],
        vec![
            ("apiUrl", text("https://10.0.0.5/api/v4")),
            ("allowPrivateHost", on.clone()),
        ],
    ] {
        let m = gitlab(&config, None);
        eligible(&m);
        assert!(matches!(entered_origin(&m), Some((_, Ok(_)))), "{config:?}");
    }
    // Not reachable for a plugin, each with its reason.
    for (config, expect) in [
        (
            vec![("apiUrl", text("http://127.0.0.1:8080/api/v4"))],
            "is not a server a plugin can reach",
        ),
        (
            vec![("apiUrl", text("http://gitlab.example.com/api/v4"))],
            "is not a server a plugin can reach",
        ),
        (
            vec![("allowPrivateHost", on.clone())],
            "`allowPrivateHost` allows private network hosts",
        ),
    ] {
        let reason = eligibility(&gitlab(&config, None), false).unwrap();
        assert!(reason.contains(expect), "{config:?}: {reason}");
    }
    let other = eligibility(&gitlab(&[], Some("other.token")), false).unwrap();
    assert!(
        other.contains("`Mine` sets `credential`") && other.contains("G5"),
        "{other}"
    );

    // Jira: no default; the site is what the user entered.
    let m = jira(&[("siteUrl", text("https://team.atlassian.net"))]);
    eligible(&m);
    assert!(matches!(entered_origin(&m), Some((_, Ok(_)))));
    assert_eq!(m.credentials[0].slot, "credential");
    assert_eq!(
        m.credentials[0].id.as_ref().map(CredentialId::as_str),
        Some("jira.token")
    );
    eligible(&jira(&[
        ("siteUrl", text("https://jira.internal/jira")),
        ("deployment", ParameterValue::Choice("data-center".into())),
        ("allowPrivateHost", on.clone()),
    ]));
    // No site yet: nothing to carry or to ask about.
    let unset = jira(&[]);
    eligible(&unset);
    assert_eq!(entered_origin(&unset), None);
    let local = eligibility(&jira(&[("siteUrl", text("http://localhost:8080"))]), false).unwrap();
    assert!(
        local.contains("`siteUrl` is not a server a plugin can reach"),
        "{local}"
    );
}

// ------------------------------------------- the extraction (WP-C1)

/// The modules extracted on 2026-11-20 (`git`, `docker`, `workspace`,
/// `projects`) against their shipped plans, aliases and the real
/// manifests of `plugins/first-party/<module>` (a minimal ABI module stands
/// in for each `plugin.wasm`; `tests/plugins_cutover.rs` runs the real
/// Git plugin end to end).
mod extraction {
    use super::*;

    const MODULES: [&str; 4] = ["git", "docker", "workspace", "projects"];
    const BACKUP: &str = "deckpilot-pre-plugin-extraction-1.db";

    fn plugin(module: &str) -> PluginId {
        PluginId::parse(&format!("deckpilot.{module}")).unwrap()
    }

    fn source(module: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../plugins/first-party")
            .join(module)
    }

    /// A configuration store and the application resources.
    struct Shipped {
        root: tempfile::TempDir,
        config: SqliteStore,
        profile: Profile,
    }

    impl Shipped {
        fn new() -> Self {
            Self {
                root: tempfile::tempdir().unwrap(),
                config: SqliteStore::open_in_memory().unwrap(),
                profile: Profile::new("Work"),
            }
        }

        /// The built-in's `modules` row, enabled, with `config`.
        fn save(&self, module: &str, config: &[(&str, ParameterValue)]) {
            let parameters = config
                .iter()
                .fold(ActionParameters::new(), |p, (k, v)| p.with(*k, v.clone()));
            self.config
                .set_module_state(
                    &ModuleId::new(module).unwrap(),
                    &ModuleSettings {
                        enabled: true,
                        config: parameters,
                    },
                )
                .unwrap();
        }

        /// An instance of the legacy `action` in the profile.
        fn uses(&mut self, name: &str, action: &str, parameters: &[(&str, ParameterValue)]) {
            let parameters = parameters
                .iter()
                .fold(ActionParameters::new(), |p, (k, v)| p.with(*k, v.clone()));
            self.profile.actions.push(ActionInstance {
                id: ActionInstanceId::new(),
                name: name.into(),
                action_type: ActionTypeId::new(action).unwrap(),
                parameters,
                description: String::new(),
                icon: None,
                label: None,
            });
            self.config.save_profile(&self.profile).unwrap();
        }

        fn open(&self) -> PluginManager {
            PluginManager::open(
                &self.root.path().join("data"),
                &StoreKey::from_bytes([0x24; 32]),
                ManagerOptions::default(),
            )
            .unwrap()
        }

        /// The four plugins as `build.sh` stages them (manifest, licence,
        /// icons, locales), each pinned with class `migration`.
        fn bundle(&self) -> Bundle {
            let bundled = self.root.path().join("resources").join(BUNDLED_DIR);
            let mut pinned = Vec::new();
            for module in MODULES {
                let id = plugin(module);
                let dir = bundled.join(id.as_str());
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(dir.join("assets")).unwrap();
                std::fs::create_dir_all(dir.join("locales")).unwrap();
                let manifest: Value = serde_json::from_slice(
                    &std::fs::read(source(module).join("manifest.json")).unwrap(),
                )
                .unwrap();
                let mut files = vec!["manifest.json".to_owned(), "LICENSE".to_owned()];
                files.extend(
                    manifest["icons"]
                        .as_object()
                        .unwrap()
                        .values()
                        .map(|v| v.as_str().unwrap().to_owned()),
                );
                files.extend(
                    manifest["locales"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|tag| format!("locales/{}.json", tag.as_str().unwrap())),
                );
                for file in files {
                    std::fs::copy(source(module).join(&file), dir.join(&file)).unwrap();
                }
                let version = manifest["version"].as_str().unwrap();
                std::fs::write(
                    dir.join("plugin.wasm"),
                    deckpilot_plugin_package::wasm::minimal_abi_module_with_note(version),
                )
                .unwrap();
                pinned.push((id, Version::parse(version).unwrap(), BundleClass::Migration));
            }
            Bundle {
                dir: Some(bundled.clone()),
                pins: Pins::compute(&bundled, &pinned).unwrap(),
                trust: Ok(()),
            }
        }

        /// What the service task hands the worker, with the backup it took.
        fn snapshot(&self, backup: Option<Result<String, String>>) -> LegacySnapshot {
            let mut registry = deckpilot_modules::builtin(Os::current());
            registry.apply_states(&self.config.module_states().unwrap());
            let mut snapshot = LegacySnapshot::read(&self.config, &registry).unwrap();
            snapshot.backup = backup;
            snapshot
        }

        fn backed_up(&self) -> LegacySnapshot {
            self.snapshot(Some(Ok(BACKUP.into())))
        }
    }

    fn phase(manager: &PluginManager, module: &str) -> Option<ModuleMigration> {
        manager.store().module_migration(module).unwrap()
    }

    fn route_of(manager: &PluginManager, bundle: &Bundle, module: &str) -> LegacyRoute {
        routing(manager, bundle).route(&ModuleId::new(module).unwrap())
    }

    fn granted(manager: &PluginManager, module: &str) -> Vec<(String, String)> {
        manager
            .store()
            .grants(&plugin(module))
            .unwrap()
            .into_iter()
            .filter(|g| g.status == GrantStatus::Granted)
            .map(|g| (g.key.to_string(), g.decided_by))
            .collect()
    }

    fn consent_to(
        manager: &mut PluginManager,
        bundle: &Bundle,
        snapshot: &LegacySnapshot,
        module: &str,
    ) {
        decide(manager, bundle, snapshot, &Decision::Consent(module.into())).unwrap();
    }

    fn text(value: &str) -> ParameterValue {
        ParameterValue::Text(value.into())
    }

    /// D4: nothing is installed for a module this computer does not use,
    /// and nothing is backed up.
    #[test]
    fn a_user_with_no_use_of_a_module_gets_nothing_installed() {
        let mut env = Shipped::new();
        env.uses("Note", "general.log", &[]);
        let bundle = env.bundle();
        let snapshot = env.snapshot(None);
        assert!(!snapshot.needs_backup());
        let mut manager = env.open();
        start(&mut manager, &bundle, &snapshot);
        for module in MODULES {
            let record = phase(&manager, module).unwrap();
            assert_eq!(record.phase, MigrationPhase::NotEligible, "{module}");
            assert_eq!(
                record.reason.as_deref(),
                Some("the built-in module is not used on this computer"),
                "{module}"
            );
            assert!(
                manager.store().plugin(&plugin(module)).unwrap().is_none(),
                "{module}"
            );
            assert_eq!(
                route_of(&manager, &bundle, module),
                LegacyRoute::Builtin,
                "{module}"
            );
        }
        assert_eq!(manager.store().host_setting(BACKUP_NOTE).unwrap(), None);
        assert_eq!(manager.store().host_setting("tools.git").unwrap(), None);
    }

    /// Git, end to end through the engine: eligible by its keys; installed
    /// and waiting for the high tier with its keys unavailable (no
    /// fallback while it waits) and `deckpilot.db` untouched; the user's
    /// consent cuts it over after the backup is journaled; reruns change
    /// nothing; the user's disable withdraws it.
    #[test]
    #[allow(clippy::too_many_lines)] // one migration, step by step
    fn git_waits_unavailable_for_consent_then_runs_through_the_plugin() {
        let mut env = Shipped::new();
        env.save(
            "git",
            &[
                ("gitExecutable", text("/opt/git/bin/git")),
                ("networkTimeoutSeconds", ParameterValue::Integer(60)),
                ("repositoryRole", text("code")),
            ],
        );
        env.uses(
            "Branch",
            "git.status",
            &[(
                "project_id",
                ParameterValue::Project(deckpilot_core::ProjectId::new()),
            )],
        );
        let stored = env.config.load_profile(env.profile.id).unwrap().unwrap();
        let bundle = env.bundle();
        let snapshot = env.backed_up();
        assert!(snapshot.needs_backup());
        let mut manager = env.open();
        start(&mut manager, &bundle, &snapshot);

        let record = phase(&manager, "git").unwrap();
        assert_eq!(record.phase, MigrationPhase::AwaitingConsent, "{record:?}");
        let reason = record.reason.unwrap();
        for item in ["tools:git", "tools:git:network", "resources:open"] {
            assert!(reason.contains(item), "{reason}");
        }
        assert_eq!(
            route_of(&manager, &bundle, "git"),
            LegacyRoute::AwaitingConsent
        );
        let info = info(&manager, &bundle, Some(&snapshot), "git").unwrap();
        assert_eq!(info.route, ipc::MigrationRoute::PluginWithdrawn);
        assert!(
            info.route_reason
                .unwrap()
                .contains("waits for your consent")
        );
        // Derived: the low tier and the medium tier the built-in used;
        // never the high tier.
        let derived: BTreeSet<String> = granted(&manager, "git")
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(
            derived,
            BTreeSet::from([
                "device:state".to_owned(),
                "profile".to_owned(),
                "resources".to_owned(),
                "resources:paths".to_owned(),
            ]),
            "{derived:?}"
        );
        // The plugin's settings; the host's tool settings; no backup yet.
        assert_eq!(
            manager.store().settings(&plugin("git")).unwrap(),
            BTreeMap::from([
                ("networkTimeoutSeconds".to_owned(), json!(60)),
                ("repositoryRole".to_owned(), json!("code")),
            ])
        );
        assert_eq!(
            manager.store().host_setting("tools.git").unwrap(),
            Some(json!({"gitExecutable": "/opt/git/bin/git", "networkTimeoutSeconds": 60}))
        );
        assert_eq!(manager.store().host_setting(BACKUP_NOTE).unwrap(), None);
        // `deckpilot.db`'s profile is exactly as stored.
        assert_eq!(
            env.config.load_profile(env.profile.id).unwrap().unwrap(),
            stored
        );

        consent_to(&mut manager, &bundle, &snapshot, "git");
        let record = phase(&manager, "git").unwrap();
        assert_eq!(record.phase, MigrationPhase::CutOver, "{record:?}");
        assert_eq!(route_of(&manager, &bundle, "git"), LegacyRoute::Plugin);
        assert_eq!(
            manager.store().host_setting(BACKUP_NOTE).unwrap(),
            Some(json!({"file": BACKUP}))
        );
        for key in ["tools:git", "tools:git:network", "resources:open"] {
            assert!(
                granted(&manager, "git")
                    .contains(&(key.to_owned(), "user:migration:git".to_owned())),
                "{key}"
            );
        }
        assert_eq!(
            manager
                .store()
                .plugin(&plugin("git"))
                .unwrap()
                .unwrap()
                .state,
            PluginState::Enabled
        );

        // Reruns: nothing changes, the backup is journaled once.
        let done = phase(&manager, "git").unwrap();
        for _ in 0..2 {
            drop(manager);
            manager = env.open();
            start(
                &mut manager,
                &bundle,
                &env.snapshot(Some(Ok("another.db".into()))),
            );
        }
        assert_eq!(phase(&manager, "git").unwrap(), done);
        assert_eq!(
            manager.store().host_setting(BACKUP_NOTE).unwrap(),
            Some(json!({"file": BACKUP}))
        );
        assert_eq!(
            env.config.load_profile(env.profile.id).unwrap().unwrap(),
            stored
        );

        // The user's disable: unavailable, no fallback.
        manager.disable_by_user(&plugin("git")).unwrap();
        assert_eq!(
            route_of(&manager, &bundle, "git"),
            LegacyRoute::PluginWithdrawn
        );
    }

    /// A decline keeps the keys unavailable (recoverable) until Retry asks
    /// again; a backup that could not be written fails the cut-over, and
    /// the built-in serves while it is compiled in.
    #[test]
    fn a_decline_waits_and_a_failed_backup_falls_back() {
        let mut env = Shipped::new();
        env.uses("Branch", "git.branch", &[]);
        let bundle = env.bundle();
        let snapshot = env.backed_up();
        let mut manager = env.open();
        start(&mut manager, &bundle, &snapshot);
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &Decision::Decide {
                module: "git".into(),
                approve: BTreeSet::new(),
                decline: BTreeSet::from([GrantKey::new(PermissionKind::Tools, "git")]),
            },
        )
        .unwrap();
        let record = phase(&manager, "git").unwrap();
        assert_eq!(record.phase, MigrationPhase::NotEligible);
        assert_eq!(
            route_of(&manager, &bundle, "git"),
            LegacyRoute::AwaitingConsent
        );
        start(&mut manager, &bundle, &snapshot);
        assert_eq!(
            route_of(&manager, &bundle, "git"),
            LegacyRoute::AwaitingConsent
        );
        let reason = info(&manager, &bundle, Some(&snapshot), "git")
            .unwrap()
            .route_reason
            .unwrap();
        assert!(reason.contains("you declined"), "{reason}");
        decide(
            &mut manager,
            &bundle,
            &snapshot,
            &Decision::Retry("git".into()),
        )
        .unwrap();
        assert_eq!(
            phase(&manager, "git").unwrap().phase,
            MigrationPhase::AwaitingConsent
        );

        // The backup failed: consent given, but no cut-over.
        let failing = env.snapshot(Some(Err("disk full".into())));
        consent_to(&mut manager, &bundle, &failing, "git");
        let record = phase(&manager, "git").unwrap();
        assert_eq!(record.phase, MigrationPhase::Failed, "{record:?}");
        assert!(record.reason.unwrap().contains("could not be backed up"));
        assert_eq!(route_of(&manager, &bundle, "git"), LegacyRoute::Builtin);
        assert_eq!(manager.store().host_setting(BACKUP_NOTE).unwrap(), None);
        // The next start with a backup retries and waits no more.
        start(&mut manager, &bundle, &snapshot);
        assert_eq!(
            phase(&manager, "git").unwrap().phase,
            MigrationPhase::CutOver
        );
    }

    /// Docker: `tools:docker:modify` is offered only when
    /// `allowStateChanges` was on, never derived; its tool settings are
    /// the host's; a stored `projectName` does not stop the migration and
    /// is named in the journal (that key is unavailable until cleared).
    #[test]
    fn docker_offers_modify_only_when_the_builtin_allowed_changes() {
        let modify = GrantKey::new(PermissionKind::Tools, "docker:modify");
        for allowed in [false, true] {
            let mut env = Shipped::new();
            env.save(
                "docker",
                &[
                    ("dockerExecutable", text("/usr/local/bin/docker")),
                    ("context", text("colima")),
                    ("timeoutSeconds", ParameterValue::Integer(900)),
                    ("allowStateChanges", ParameterValue::Boolean(allowed)),
                ],
            );
            env.uses("Up", "docker.compose-up", &[("projectName", text("shop"))]);
            let bundle = env.bundle();
            let snapshot = env.backed_up();
            let mut manager = env.open();
            start(&mut manager, &bundle, &snapshot);
            let record = phase(&manager, "docker").unwrap();
            assert_eq!(record.phase, MigrationPhase::AwaitingConsent, "{record:?}");
            let reason = record.reason.unwrap();
            assert!(reason.contains("tools:docker"), "{reason}");
            assert_eq!(reason.contains("tools:docker:modify"), allowed, "{reason}");
            assert!(reason.contains("`Up` sets `projectName`"), "{reason}");
            let items: Vec<String> = info(&manager, &bundle, Some(&snapshot), "docker")
                .unwrap()
                .consent
                .into_iter()
                .map(|item| item.key)
                .collect();
            assert_eq!(items.contains(&modify.to_string()), allowed, "{items:?}");
            assert_eq!(
                manager.store().host_setting("tools.docker").unwrap(),
                Some(json!({
                    "context": "colima",
                    "dockerExecutable": "/usr/local/bin/docker",
                    "timeoutSeconds": 900,
                }))
            );
            assert_eq!(
                manager.store().settings(&plugin("docker")).unwrap(),
                BTreeMap::from([("timeoutSeconds".to_owned(), json!(900))])
            );
            consent_to(&mut manager, &bundle, &snapshot, "docker");
            let modify_granted = granted(&manager, "docker")
                .iter()
                .any(|(key, _)| *key == modify.to_string());
            assert_eq!(modify_granted, allowed);
            if allowed {
                let record = phase(&manager, "docker").unwrap();
                assert_eq!(record.phase, MigrationPhase::CutOver, "{record:?}");
                assert!(record.reason.unwrap().contains("`Up` sets `projectName`"));
                assert_eq!(route_of(&manager, &bundle, "docker"), LegacyRoute::Plugin);
            }
        }
    }

    /// A plugin whose actions cannot replace the built-in's (the known
    /// gaps of `crates/deckpilot-modules/tests/contracts.rs`) fails before
    /// the user is asked: nothing waits, the built-in serves, and it is
    /// retried at the next start. Projects counts its projects as use.
    #[test]
    fn a_plugin_that_cannot_replace_the_builtin_fails_with_fallback() {
        let mut env = Shipped::new();
        env.profile.projects.push(deckpilot_core::Project::new(
            deckpilot_core::ProjectKey::new("SHOP").unwrap(),
            "Shop",
        ));
        env.uses("Edit", "workspace.open-urls", &[]);
        let bundle = env.bundle();
        let snapshot = env.backed_up();
        let projects = snapshot
            .modules
            .iter()
            .find(|m| m.module.as_str() == "projects")
            .unwrap();
        assert!(projects.data_in_use && projects.instances == 0);
        let mut manager = env.open();
        start(&mut manager, &bundle, &snapshot);
        for (module, gap) in [
            (
                "workspace",
                "`deckpilot.workspace.open-in-editor` cannot replace",
            ),
            ("projects", "`deckpilot.projects.cycle` cannot replace"),
        ] {
            let record = phase(&manager, module).unwrap();
            assert_eq!(record.phase, MigrationPhase::Failed, "{module}: {record:?}");
            let reason = record.reason.unwrap();
            assert!(
                reason.starts_with("health check: ") && reason.contains(gap),
                "{reason}"
            );
            assert_eq!(
                route_of(&manager, &bundle, module),
                LegacyRoute::Builtin,
                "{module}"
            );
            assert_eq!(
                manager
                    .store()
                    .plugin(&plugin(module))
                    .unwrap()
                    .unwrap()
                    .state,
                PluginState::Disabled
            );
            assert!(
                !granted(&manager, module)
                    .iter()
                    .any(|(_, by)| by.starts_with("user:")),
                "{module}"
            );
        }
        // Retried at the next start, failing the same way.
        start(&mut manager, &bundle, &snapshot);
        assert_eq!(phase(&manager, "projects").unwrap().attempts, 2);
    }

    /// The extraction plans and their aliases agree: one plan per
    /// extracted module, each of the four bundled with class `migration`.
    #[test]
    fn every_extracted_module_has_its_plan() {
        for module in MODULES {
            let plan = plan_for(module).unwrap();
            assert!(plan.extraction.is_some(), "{module}");
            assert!(plan.credentials.is_empty() && plan.origin.is_none());
            assert!(
                deckpilot_core::alias::ACTION_ALIASES
                    .iter()
                    .any(|a| a.module == module && a.plugin == plugin(module).as_str()),
                "{module}"
            );
        }
        for module in ["github", "gitlab", "jira", "notifications"] {
            assert!(plan_for(module).unwrap().extraction.is_none(), "{module}");
        }
        let released: Value = serde_json::from_slice(
            &std::fs::read(source("bundled.json").with_file_name("bundled.json")).unwrap(),
        )
        .unwrap();
        for module in MODULES {
            assert!(
                released["plugins"]
                    .as_array()
                    .unwrap()
                    .contains(&json!({"path": module, "class": "migration"})),
                "{module}"
            );
        }
        assert!(
            released["plugins"]
                .as_array()
                .unwrap()
                .contains(&json!({"path": "workflows", "class": "default"}))
        );
    }
}
