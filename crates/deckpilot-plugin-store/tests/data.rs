//! Per-plugin data: storage (isolation, quota, limits), credential
//! secrets, settings, `plugin_known`, and what an uninstall keeps.

#![cfg(feature = "sqlcipher")]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use common::{Env, base_permissions, grants_of, id, key, package, package_for};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_store::{
    Consent, CrashOnce, Failpoint, InstallSource, Operation, PluginData, PluginManager, Retention,
    StorageError, StorageQuota, StoreError,
};
use serde_json::{Value, json};

const OTHER: &str = "other.github-prs";

fn install(manager: &mut PluginManager, plugin: &str) {
    let package = package_for(plugin, "1.0.0", &base_permissions());
    manager
        .install(
            &package,
            &InstallSource::Sideload,
            &Consent::all(&grants_of(&package), "user"),
        )
        .unwrap();
}

fn data(env: &Env) -> PluginData {
    PluginData::open(
        &env.data_dir().join("plugins/plugins.db"),
        &key(),
        Duration::from_secs(5),
    )
    .unwrap()
}

fn other() -> PluginId {
    PluginId::parse(OTHER).unwrap()
}

const QUOTA: StorageQuota = StorageQuota {
    bytes: 1_000,
    keys: 3,
};

#[test]
fn storage_is_isolated_per_plugin() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, common::ID);
    install(&mut manager, OTHER);
    let mut data = data(&env);
    data.storage_set(&id(), "shared", &json!("mine"), QUOTA)
        .unwrap();
    data.storage_set(&other(), "shared", &json!({"theirs": [1, 2]}), QUOTA)
        .unwrap();
    assert_eq!(
        data.storage_get(&id(), "shared").unwrap(),
        Some(json!("mine"))
    );
    assert_eq!(
        data.storage_get(&other(), "shared").unwrap(),
        Some(json!({"theirs": [1, 2]}))
    );
    assert!(data.storage_delete(&id(), "shared").unwrap());
    assert_eq!(data.storage_get(&id(), "shared").unwrap(), None);
    assert!(data.storage_get(&other(), "shared").unwrap().is_some());
    assert_eq!(data.storage_keys(&other(), None).unwrap(), ["shared"]);
    assert_eq!(data.storage_keys(&id(), None).unwrap().len(), 0);
    // Each plugin's quota counts only its own rows.
    assert_eq!(data.storage_usage(&id()).unwrap().keys, 0);
    assert_eq!(data.storage_usage(&other()).unwrap().keys, 1);
    // A plugin that is not installed has no storage to write to.
    let ghost = PluginId::parse("ghost.plugin").unwrap();
    assert!(matches!(
        data.storage_set(&ghost, "k", &json!(1), QUOTA),
        Err(StorageError::NotInstalled)
    ));
}

#[test]
fn storage_quota_counts_bytes_and_keys() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, common::ID);
    let mut data = data(&env);
    // "a" + "\"…\"": 1 + 402 bytes.
    let big = json!("x".repeat(400));
    let usage = data.storage_set(&id(), "a", &big, QUOTA).unwrap();
    assert_eq!(usage.bytes, 403);
    assert_eq!(data.storage_usage(&id()).unwrap().bytes, 403);
    data.storage_set(&id(), "b", &big, QUOTA).unwrap();
    // The third would need 1,209 bytes.
    let refused = data.storage_set(&id(), "c", &big, QUOTA).unwrap_err();
    assert!(
        matches!(
            refused,
            StorageError::QuotaExceeded {
                needed: 1_209,
                quota: 1_000
            }
        ),
        "{refused:?}"
    );
    assert!(refused.to_string().starts_with("storage-quota"));
    // Overwriting a key replaces its size.
    data.storage_set(&id(), "a", &json!(1), QUOTA).unwrap();
    assert_eq!(data.storage_usage(&id()).unwrap().bytes, 405);
    data.storage_set(&id(), "c", &json!(2), QUOTA).unwrap();
    let refused = data.storage_set(&id(), "d", &json!(3), QUOTA).unwrap_err();
    assert!(matches!(refused, StorageError::TooManyKeys { max: 3 }));
    // Over the quota (a smaller one, as after a revoked grant), a write
    // that does not grow the storage is still allowed; one that grows is
    // not.
    let small = StorageQuota { bytes: 5, keys: 3 };
    data.storage_set(&id(), "b", &json!("y"), small).unwrap();
    assert!(matches!(
        data.storage_set(&id(), "c", &json!("longer than two"), small),
        Err(StorageError::QuotaExceeded { .. })
    ));
}

#[test]
fn storage_keys_and_values_are_bounded() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, common::ID);
    let mut data = data(&env);
    let quota = StorageQuota {
        bytes: 10 << 20,
        keys: 1_000,
    };
    for bad in [String::new(), "k".repeat(129), "line\nbreak".into()] {
        assert!(
            matches!(
                data.storage_set(&id(), &bad, &json!(1), quota),
                Err(StorageError::InvalidKey)
            ),
            "{bad:?}"
        );
    }
    data.storage_set(&id(), &"é".repeat(128), &json!(1), quota)
        .unwrap();
    let value = json!("v".repeat(64 * 1024));
    assert!(matches!(
        data.storage_set(&id(), "v", &value, quota),
        Err(StorageError::ValueTooLarge)
    ));
    data.storage_set(&id(), "v", &json!("v".repeat(64 * 1024 - 2)), quota)
        .unwrap();
    let keys = data.storage_keys(&id(), Some("v")).unwrap();
    assert_eq!(keys, ["v"]);
}

#[test]
fn credentials_are_per_plugin_slot_and_never_audited_by_value() {
    const SECRET: &str = "ghp_s3cr3t-value-that-must-not-leak";
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, common::ID);
    install(&mut manager, OTHER);
    manager.set_credential(&id(), "token", SECRET).unwrap();
    // Only declared slots.
    assert!(matches!(
        manager.set_credential(&id(), "password", SECRET),
        Err(StoreError::InvalidData(_))
    ));
    for bad in ["", "two\nlines"] {
        assert!(matches!(
            manager.set_credential(&id(), "token", bad),
            Err(StoreError::InvalidData(_))
        ));
    }
    let status = manager.store().credentials(&id()).unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].key, "token");
    assert_eq!(manager.store().credentials(&other()).unwrap().len(), 0);
    let data = data(&env);
    assert_eq!(
        data.credential_secret(&id(), "token")
            .unwrap()
            .unwrap()
            .expose(),
        SECRET
    );
    assert!(data.credential_secret(&other(), "token").unwrap().is_none());
    let secret = data.credential_secret(&id(), "token").unwrap().unwrap();
    assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    // The audit names the slot, never the value.
    let audit = manager.store().audit(&id()).unwrap();
    assert!(
        audit
            .iter()
            .any(|a| a.op == "credential.set" && a.target.as_deref() == Some("credentials:token"))
    );
    assert!(!format!("{audit:?}").contains(SECRET));
    assert!(manager.delete_credential(&id(), "token").unwrap());
    assert!(!manager.delete_credential(&id(), "token").unwrap());
    assert!(data.credential_secret(&id(), "token").unwrap().is_none());
    // The file is encrypted: the secret is not in it as plain text.
    manager.set_credential(&id(), "token", SECRET).unwrap();
    drop(data);
    drop(manager);
    for entry in std::fs::read_dir(env.data_dir().join("plugins")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
                "{} holds the secret in clear text",
                path.display()
            );
        }
    }
}

#[test]
fn settings_are_per_plugin_and_only_declared_keys() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, common::ID);
    install(&mut manager, OTHER);
    let values = BTreeMap::from([("pollMinutes".to_owned(), json!(10))]);
    manager.set_settings(&id(), &values).unwrap();
    assert_eq!(manager.store().settings(&id()).unwrap(), values);
    assert_eq!(manager.store().settings(&other()).unwrap(), BTreeMap::new());
    assert_eq!(data(&env).settings(&id()).unwrap(), values);
    let unknown = BTreeMap::from([("token".to_owned(), json!("x"))]);
    assert!(matches!(
        manager.set_settings(&id(), &unknown),
        Err(StoreError::InvalidData(_))
    ));
    manager.set_settings(&id(), &BTreeMap::new()).unwrap();
    assert_eq!(manager.store().settings(&id()).unwrap(), BTreeMap::new());
    let ghost = PluginId::parse("ghost.plugin").unwrap();
    assert!(matches!(
        manager.set_settings(&ghost, &values),
        Err(StoreError::NotInstalled(_))
    ));
}

fn fill(env: &Env, manager: &mut PluginManager) {
    manager.set_credential(&id(), "token", "s3cret").unwrap();
    manager
        .set_settings(
            &id(),
            &BTreeMap::from([("pollMinutes".to_owned(), json!(10))]),
        )
        .unwrap();
    data(env)
        .storage_set(&id(), "k", &json!(1), DEFAULT)
        .unwrap();
}

const DEFAULT: StorageQuota = StorageQuota {
    bytes: 256 * 1024,
    keys: 1_000,
};

#[test]
fn uninstall_deletes_the_data_and_keeps_the_name() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, common::ID);
    install(&mut manager, OTHER);
    fill(&env, &mut manager);
    data(&env)
        .storage_set(&other(), "k", &json!(2), DEFAULT)
        .unwrap();
    manager.uninstall(&id()).unwrap();
    let store = manager.store();
    assert_eq!(store.storage_keys(&id()).unwrap().len(), 0);
    assert_eq!(store.credentials(&id()).unwrap().len(), 0);
    assert_eq!(store.settings(&id()).unwrap(), BTreeMap::new());
    // The other plugin keeps its own.
    assert_eq!(store.storage_keys(&other()).unwrap(), ["k"]);
    // `plugin_known` keeps the name, the namespace and the actions.
    let known = store.known().unwrap();
    let mine = known.iter().find(|k| k.id == id()).unwrap();
    assert_eq!(mine.display_name, "GitHub PR Count");
    assert_eq!(mine.namespace, "plugins.acme.githubPrs");
    assert_eq!(mine.last_version, "1.0.0");
    assert_ne!(mine.actions.len(), 0);
    assert!(known.iter().any(|k| k.id == other()));
    // Reinstalling starts empty and keeps first_seen_at.
    let first = mine.first_seen_at;
    install(&mut manager, common::ID);
    let store = manager.store();
    assert_eq!(store.credentials(&id()).unwrap().len(), 0);
    assert_eq!(store.settings(&id()).unwrap(), BTreeMap::new());
    let again = store.known().unwrap();
    assert_eq!(
        again.iter().find(|k| k.id == id()).unwrap().first_seen_at,
        first
    );
}

#[test]
fn uninstall_can_keep_settings_and_credentials() {
    let env = Env::new();
    let mut manager = env.open();
    install(&mut manager, common::ID);
    fill(&env, &mut manager);
    manager
        .uninstall_keeping(
            &id(),
            Retention {
                settings: true,
                credentials: true,
            },
        )
        .unwrap();
    assert!(manager.store().plugin(&id()).unwrap().is_none());
    // Storage is never kept.
    assert_eq!(manager.store().storage_keys(&id()).unwrap().len(), 0);
    assert_eq!(manager.store().credentials(&id()).unwrap().len(), 1);
    assert_eq!(manager.store().settings(&id()).unwrap().len(), 1);
    // The external secrets hook was not asked to delete anything.
    assert_eq!(env.hook.calls.lock().unwrap().len(), 0);
    // A later install of the same plugin finds them.
    install(&mut manager, common::ID);
    assert_eq!(
        data(&env)
            .credential_secret(&id(), "token")
            .unwrap()
            .unwrap()
            .expose(),
        "s3cret"
    );
}

#[test]
fn an_interrupted_uninstall_keeps_what_it_was_asked_to_keep() {
    let env = Env::new();
    {
        let mut manager = env.open();
        install(&mut manager, common::ID);
        fill(&env, &mut manager);
    }
    let crash = Arc::new(CrashOnce::new(
        Operation::Uninstall,
        Failpoint::FilesTrashed,
    ));
    let mut manager = env.open_with(crash.clone());
    let result = manager.uninstall_keeping(
        &id(),
        Retention {
            settings: false,
            credentials: true,
        },
    );
    assert!(
        matches!(result, Err(StoreError::Crashed { .. })),
        "{result:?}"
    );
    assert!(crash.fired());
    drop(manager);
    let mut manager = env.open();
    manager.recover().unwrap();
    assert!(manager.store().plugin(&id()).unwrap().is_none());
    assert_eq!(manager.store().credentials(&id()).unwrap().len(), 1);
    assert_eq!(manager.store().settings(&id()).unwrap(), BTreeMap::new());
    assert_eq!(manager.store().storage_keys(&id()).unwrap().len(), 0);
}

#[test]
fn updates_refresh_the_known_record() {
    let env = Env::new();
    let mut manager = env.open();
    let first = package("1.0.0", &base_permissions());
    manager
        .install(
            &first,
            &InstallSource::Sideload,
            &Consent::all(&grants_of(&first), "user"),
        )
        .unwrap();
    let next = package("1.1.0", &base_permissions());
    manager.update(&next, &InstallSource::Sideload).unwrap();
    let known = manager.store().known().unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].last_version, "1.1.0");
    let value: Value = serde_json::to_value(&known[0].actions).unwrap();
    assert!(value.is_array());
}
