//! Seeding against a real plugin store in a temporary data directory, with
//! the test fixture `plugins/first-party/test-fixtures/example-default`
//! (a minimal ABI 1 module stands in for its built `plugin.wasm`; the end
//! to end test `tests/plugins_bundled.rs` runs the real one).

use std::path::Path;

use deckpilot_plugin_bundled::{BUNDLED_DIR, BundleClass, Pins};
use deckpilot_plugin_store::{
    GrantSource, GrantStatus, ManagerOptions, PluginState, Retention, StoreKey, TombstoneDecision,
};
use semver::Version;
use serde_json::{Value, json};

use super::*;

const ID: &str = "deckpilot.example-default";
const FIXTURE: &str = "../../plugins/first-party/test-fixtures/example-default";

fn id() -> PluginId {
    PluginId::parse(ID).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE).join(name)).unwrap()
}

/// A temporary data directory and application resources.
struct Env {
    root: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn bundled(&self) -> PathBuf {
        self.root.path().join("resources").join(BUNDLED_DIR)
    }

    /// The plugin store, as the agent opens it at start-up or unlock.
    fn open(&self) -> PluginManager {
        PluginManager::open(
            &self.data(),
            &StoreKey::from_bytes([0x42; 32]),
            ManagerOptions::default(),
        )
        .unwrap()
    }

    /// Writes the fixture at `version` with `permissions` into the
    /// resources and returns the bundle the agent would see, pinned to
    /// exactly these files.
    fn bundle(&self, version: &str, permissions: &Value, class: BundleClass) -> Bundle {
        let dir = self.bundled().join(ID);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let mut manifest: Value = serde_json::from_slice(&fixture("manifest.json")).unwrap();
        manifest["version"] = version.into();
        manifest["permissions"] = permissions.clone();
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
        std::fs::write(dir.join("LICENSE"), fixture("LICENSE")).unwrap();
        std::fs::write(dir.join("assets/icon-128.png"), fixture("assets/icon-128.png")).unwrap();
        let pins = Pins::compute(
            &self.bundled(),
            &[(id(), Version::parse(version).unwrap(), class)],
        )
        .unwrap();
        Bundle {
            dir: Some(self.bundled()),
            pins,
            trust: Ok(()),
        }
    }

    fn default_bundle(&self) -> Bundle {
        self.bundle("0.1.0", &notifications(), BundleClass::Default)
    }
}

fn notifications() -> Value {
    json!([{ "kind": "notifications", "justification": "Test fixture: low tier." }])
}

/// Notifications (low) and a network host (medium).
fn with_network() -> Value {
    json!([
        { "kind": "notifications", "justification": "Test fixture: low tier." },
        {
            "kind": "network",
            "hosts": ["api.example.com"],
            "justification": "Test fixture: medium tier, needs consent."
        }
    ])
}

fn only(report: Vec<(PluginId, Seeded)>) -> Seeded {
    assert_eq!(report.len(), 1, "{report:?}");
    report.into_iter().next().unwrap().1
}

fn record(manager: &PluginManager) -> Option<PluginRecord> {
    manager.store().plugin(&id()).unwrap()
}

#[test]
fn a_fresh_data_directory_is_seeded_offline() {
    let env = Env::new();
    let bundle = env.default_bundle();
    let mut manager = env.open();
    let outcome = only(seed(&mut manager, &bundle));
    if super::super::execution_supported() {
        assert_eq!(outcome, Seeded::Enabled);
    }
    let record = record(&manager).unwrap();
    assert_eq!(record.source, InstallSource::Bundled);
    assert_eq!(record.active().unwrap().version, Version::new(0, 1, 0));
    // Exactly the low-tier permission it declares, recorded as derived.
    let grants = manager.store().grants(&id()).unwrap();
    assert_eq!(grants.len(), 1, "{grants:?}");
    assert_eq!(grants[0].key.to_string(), "notifications");
    assert_eq!(grants[0].status, GrantStatus::Granted);
    assert_eq!(grants[0].source, GrantSource::Derived);
    assert_eq!(grants[0].decided_by, decided_by());

    // Idempotent: the next start finds the pinned copy intact.
    assert_eq!(only(seed(&mut manager, &bundle)), Seeded::Current);
    drop(manager);
    let mut manager = env.open();
    assert_eq!(only(seed(&mut manager, &bundle)), Seeded::Current);
    let dir = manager
        .layout()
        .resolve(&record.active().unwrap().install_dir);
    let record = manager.store().plugin(&id()).unwrap().unwrap();
    assert!(
        check_installed(&mut manager, &bundle, &record, &dir)
            .unwrap()
            .is_some()
    );
}

#[test]
fn nothing_pinned_means_nothing_seeded() {
    let env = Env::new();
    let mut bundle = env.default_bundle();
    bundle.pins = Pins::empty();
    let mut manager = env.open();
    assert_eq!(seed(&mut manager, &bundle), Vec::new());
    assert!(record(&manager).is_none());
}

#[test]
fn tampered_files_are_refused_at_seeding() {
    for file in ["plugin.wasm", "manifest.json", "LICENSE"] {
        let env = Env::new();
        let bundle = env.default_bundle();
        let path = env.bundled().join(ID).join(file);
        let mut bytes = std::fs::read(&path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        std::fs::write(&path, bytes).unwrap();
        let mut manager = env.open();
        let outcome = only(seed(&mut manager, &bundle));
        assert!(
            matches!(&outcome, Seeded::Failed(why) if why.contains(file)),
            "{file}: {outcome:?}"
        );
        assert!(record(&manager).is_none(), "{file}");
    }

    // A swapped-in extra file, or one missing.
    let env = Env::new();
    let bundle = env.default_bundle();
    std::fs::write(env.bundled().join(ID).join("plugin.js"), "x").unwrap();
    let mut manager = env.open();
    assert!(matches!(only(seed(&mut manager, &bundle)), Seeded::Failed(_)));
    std::fs::remove_file(env.bundled().join(ID).join("plugin.js")).unwrap();
    std::fs::remove_file(env.bundled().join(ID).join("LICENSE")).unwrap();
    assert!(matches!(only(seed(&mut manager, &bundle)), Seeded::Failed(_)));
    assert!(record(&manager).is_none());
}

#[test]
fn a_tampered_installed_copy_is_refused_at_load_and_marked_broken() {
    for file in ["plugin.wasm", "manifest.json"] {
        let env = Env::new();
        let bundle = env.default_bundle();
        let mut manager = env.open();
        seed(&mut manager, &bundle);
        let record = record(&manager).unwrap();
        let dir = manager
            .layout()
            .resolve(&record.active().unwrap().install_dir);
        let path = dir.join(file);
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&path, bytes).unwrap();

        let problem = check_installed(&mut manager, &bundle, &record, &dir).unwrap_err();
        assert!(problem.contains(file), "{problem}");
        assert_eq!(
            manager.store().plugin(&id()).unwrap().unwrap().state,
            PluginState::Broken
        );
        // And seeding at the next start reports it, without repairing it.
        assert!(matches!(only(seed(&mut manager, &bundle)), Seeded::Broken(_)));
    }

    // The bytes handed to the host are checked too.
    let env = Env::new();
    let bundle = env.default_bundle();
    let mut manager = env.open();
    seed(&mut manager, &bundle);
    let record = record(&manager).unwrap();
    let pin = bundle.pin_of(&record).cloned();
    assert!(check_module(&mut manager, &record, pin.as_ref(), b"\0asm\x01\0\0\0").is_err());
    let module = deckpilot_plugin_package::wasm::minimal_abi_module_with_note("0.1.0");
    // (The first check marked it broken; the right bytes still pass.)
    assert!(check_module(&mut manager, &record, pin.as_ref(), &module).is_ok());
}

#[test]
fn an_installed_bundled_version_the_agent_does_not_pin_is_refused() {
    let env = Env::new();
    let bundle = env.default_bundle();
    let mut manager = env.open();
    seed(&mut manager, &bundle);
    let record = record(&manager).unwrap();
    let dir = manager
        .layout()
        .resolve(&record.active().unwrap().install_dir);
    // Another agent build: it pins nothing (or another version).
    let other = Bundle {
        pins: Pins::empty(),
        ..bundle
    };
    let problem = check_installed(&mut manager, &other, &record, &dir).unwrap_err();
    assert!(problem.contains("pins no bundled"), "{problem}");
    assert_eq!(
        manager.store().plugin(&id()).unwrap().unwrap().state,
        PluginState::Broken
    );
}

#[test]
fn a_user_disable_sticks_across_restarts() {
    let env = Env::new();
    let bundle = env.default_bundle();
    {
        let mut manager = env.open();
        seed(&mut manager, &bundle);
        if record(&manager).unwrap().state != PluginState::Enabled {
            // Platforms that cannot run plugins: enable is refused anyway.
            return;
        }
        manager.disable_by_user(&id()).unwrap();
    }
    for _ in 0..2 {
        let mut manager = env.open();
        let outcome = only(seed(&mut manager, &bundle));
        assert!(
            matches!(&outcome, Seeded::Skipped(why) if why.contains("disabled by the user")),
            "{outcome:?}"
        );
        assert_eq!(record(&manager).unwrap().state, PluginState::Disabled);
    }
    // A newer app with a newer bundled version does not touch it either.
    let newer = env.bundle("0.2.0", &notifications(), BundleClass::Default);
    let mut manager = env.open();
    assert!(matches!(only(seed(&mut manager, &newer)), Seeded::Skipped(_)));
    let record = record(&manager).unwrap();
    assert_eq!(record.state, PluginState::Disabled);
    assert_eq!(record.active().unwrap().version, Version::new(0, 1, 0));
}

#[test]
fn a_user_uninstall_sticks_across_restarts() {
    let env = Env::new();
    let bundle = env.default_bundle();
    {
        let mut manager = env.open();
        seed(&mut manager, &bundle);
        manager.uninstall_by_user(&id(), Retention::default()).unwrap();
    }
    for _ in 0..2 {
        let mut manager = env.open();
        let outcome = only(seed(&mut manager, &bundle));
        assert!(
            matches!(&outcome, Seeded::Skipped(why) if why.contains("uninstalled by the user")),
            "{outcome:?}"
        );
        assert!(record(&manager).is_none());
        assert_eq!(
            manager.store().tombstone(&id()).unwrap().unwrap().decision,
            TombstoneDecision::Uninstalled
        );
    }
    // The agent's own uninstall (not the user's) leaves no tombstone.
    let env = Env::new();
    let mut manager = env.open();
    seed(&mut manager, &env.default_bundle());
    manager.uninstall(&id()).unwrap();
    assert!(manager.store().tombstone(&id()).unwrap().is_none());
}

#[test]
fn no_permission_beyond_the_low_tier_is_granted() {
    let env = Env::new();
    let bundle = env.bundle("0.1.0", &with_network(), BundleClass::Default);
    let mut manager = env.open();
    let outcome = only(seed(&mut manager, &bundle));
    let Seeded::AwaitingConsent(waiting) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        waiting.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["network:api.example.com"]
    );
    let record = record(&manager).unwrap();
    assert_eq!(record.state, PluginState::Disabled);
    let grants = manager.store().grants(&id()).unwrap();
    assert_eq!(
        grants
            .iter()
            .map(|g| g.key.to_string())
            .collect::<Vec<_>>(),
        ["notifications"]
    );
    assert!(grants.iter().all(|g| g.source == GrantSource::Derived));
    // A later start neither grants nor enables anything more.
    assert_eq!(only(seed(&mut manager, &bundle)), Seeded::Current);
    assert_eq!(manager.store().grants(&id()).unwrap(), grants);
    assert_eq!(record_state(&manager), PluginState::Disabled);
}

fn record_state(manager: &PluginManager) -> PluginState {
    record(manager).unwrap().state
}

#[test]
fn an_agent_disabled_plugin_is_never_re_enabled_by_seeding() {
    let env = Env::new();
    let bundle = env.default_bundle();
    let mut manager = env.open();
    seed(&mut manager, &bundle);
    if record_state(&manager) != PluginState::Enabled {
        return;
    }
    // A crash loop or a broker failure: the agent's own disable.
    manager.disable(&id()).unwrap();
    assert_eq!(only(seed(&mut manager, &bundle)), Seeded::Current);
    assert_eq!(record_state(&manager), PluginState::Disabled);
}

#[test]
fn migration_class_plugins_are_not_seeded() {
    let env = Env::new();
    let bundle = env.bundle("0.1.0", &notifications(), BundleClass::Migration);
    let mut manager = env.open();
    assert!(matches!(only(seed(&mut manager, &bundle)), Seeded::Skipped(_)));
    assert!(record(&manager).is_none());
}

#[test]
fn an_untrusted_bundle_is_not_seeded() {
    let env = Env::new();
    let mut bundle = env.default_bundle();
    bundle.trust = Err("the code seal does not verify".into());
    let mut manager = env.open();
    let outcome = only(seed(&mut manager, &bundle));
    assert!(
        matches!(&outcome, Seeded::Failed(why) if why.contains("code seal")),
        "{outcome:?}"
    );
    assert!(record(&manager).is_none());
}

#[test]
fn updates_go_forward_only_and_only_for_bundled_copies() {
    let env = Env::new();
    let mut manager = env.open();
    seed(&mut manager, &env.default_bundle());
    let enabled = record_state(&manager);

    // A newer app: the bundled copy is updated, in place.
    let newer = env.bundle("0.2.0", &notifications(), BundleClass::Default);
    assert_eq!(only(seed(&mut manager, &newer)), Seeded::Updated);
    let record = record(&manager).unwrap();
    assert_eq!(record.active().unwrap().version, Version::new(0, 2, 0));
    assert_eq!(record.state, enabled);
    assert_eq!(record.source, InstallSource::Bundled);

    // An older app never downgrades it.
    let older = env.bundle("0.1.0", &notifications(), BundleClass::Default);
    let outcome = only(seed(&mut manager, &older));
    assert!(matches!(&outcome, Seeded::Skipped(why) if why.contains("newer")), "{outcome:?}");

    // New medium-tier permissions in a bundled update wait for the user.
    let escalating = env.bundle("0.3.0", &with_network(), BundleClass::Default);
    assert_eq!(
        only(seed(&mut manager, &escalating)),
        Seeded::UpdateAwaitingConsent
    );
    let record = manager.store().plugin(&id()).unwrap().unwrap();
    assert_eq!(record.active().unwrap().version, Version::new(0, 2, 0));
    assert_eq!(record.staged().unwrap().version, Version::new(0, 3, 0));
}

#[test]
fn a_copy_from_a_registry_is_left_alone() {
    let env = Env::new();
    let bundle = env.default_bundle();
    let package =
        deckpilot_plugin_bundled::verify_bundled(&env.bundled().join(ID), &bundle.pins.plugins[0])
            .unwrap();
    let mut manager = env.open();
    manager
        .install(
            &package,
            &InstallSource::Registry {
                registry: "https://registry.example".into(),
            },
            &Consent::none("user"),
        )
        .unwrap();
    let newer = env.bundle("0.2.0", &notifications(), BundleClass::Default);
    let outcome = only(seed(&mut manager, &newer));
    assert!(
        matches!(&outcome, Seeded::Skipped(why) if why.contains("registry")),
        "{outcome:?}"
    );
    let record = record(&manager).unwrap();
    assert_eq!(record.active().unwrap().version, Version::new(0, 1, 0));
    // Registry copies are trusted by the registry, not by the pins.
    let dir = manager
        .layout()
        .resolve(&record.active().unwrap().install_dir);
    assert_eq!(
        check_installed(&mut manager, &newer, &record, &dir).unwrap(),
        None
    );
}
