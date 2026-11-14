//! The lifecycle end to end: install, consent, enable, update and
//! escalation, rollback, revocation, uninstall, and the storage boundary.

#![cfg(feature = "sqlcipher")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    Env, ID, Verdicts, base_permissions, built, grants_of, id, key, package, package_for,
    plugin_files, with_notifications, without_background,
};
use deckpilot_plugin_manifest::PermissionKind;
use deckpilot_plugin_package::{Limits, PackageError, verify_package};
use deckpilot_plugin_store::{
    AuditRetention, Consent, GrantKey, GrantSource, GrantStatus, InstallSource, ManagerOptions,
    NoFailpoints, PluginManager, PluginState, PluginStore, StoreError, StoreKey, UpdateOutcome,
    VerdictChange, VersionRole,
};
use serde_json::json;

fn install_enabled(manager: &mut PluginManager, version: &str) {
    let package = package(version, &base_permissions());
    manager
        .install(
            &package,
            &InstallSource::Registry {
                registry: "https://registry.example".into(),
            },
            &Consent::all(&grants_of(&package), "user"),
        )
        .unwrap();
    manager.enable(&id()).unwrap();
}

fn key_of(text: &str) -> GrantKey {
    text.parse().unwrap()
}

fn state(manager: &PluginManager) -> PluginState {
    manager.store().plugin(&id()).unwrap().unwrap().state
}

// ------------------------------------------------------------------ install

#[test]
fn install_unpacks_into_the_version_directory_and_starts_disabled() {
    let env = Env::new();
    let mut manager = env.open();
    let package = package("1.0.0", &base_permissions());
    let consent = Consent::all(&grants_of(&package), "user");
    let record = manager
        .install(&package, &InstallSource::Sideload, &consent)
        .unwrap();

    assert_eq!(record.state, PluginState::Disabled);
    assert_eq!(record.source, InstallSource::Sideload);
    let active = record.active().unwrap();
    assert_eq!(active.version.to_string(), "1.0.0");
    assert_eq!(active.install_dir, "acme.github-prs/1.0.0");
    assert_eq!(&active.integrity, package.integrity());

    let dir = env.data_dir().join("plugins").join(ID).join("1.0.0");
    assert!(dir.join("plugin.js").is_file());
    assert!(
        std::fs::metadata(dir.join("manifest.json"))
            .unwrap()
            .permissions()
            .readonly()
    );
    let grants = manager.store().grants(&id()).unwrap();
    assert_eq!(grants.len(), grants_of(&package).len());
    assert!(grants.iter().all(|g| g.status == GrantStatus::Granted
        && g.source == GrantSource::Install
        && g.decided_by == "user"
        && g.granted_for_version == "1.0.0"));
    assert_eq!(manager.store().journal().unwrap(), vec![]);
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
    let audit = manager.store().audit(&id()).unwrap();
    assert!(
        audit
            .iter()
            .any(|a| a.op == "install" && a.decision == "ok")
    );
    // Disabled: nothing is effective yet.
    assert_eq!(manager.store().effective_grants(&id()).unwrap(), vec![]);
}

#[test]
fn installing_twice_is_refused() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let again = package("1.0.0", &base_permissions());
    let result = manager.install(&again, &InstallSource::Sideload, &Consent::none("user"));
    assert!(matches!(result, Err(StoreError::AlreadyInstalled { .. })));
    let newer = package("2.0.0", &base_permissions());
    let result = manager.install(&newer, &InstallSource::Sideload, &Consent::none("user"));
    assert!(matches!(result, Err(StoreError::AlreadyInstalled { .. })));
    assert_eq!(state(&manager), PluginState::Enabled);
}

#[test]
fn consent_for_an_undeclared_permission_is_refused_before_anything_is_written() {
    let env = Env::new();
    let mut manager = env.open();
    let package = package("1.0.0", &base_permissions());
    let consent = Consent::of([key_of("notifications")], "user");
    let result = manager.install(&package, &InstallSource::Sideload, &consent);
    assert!(matches!(result, Err(StoreError::UnknownGrant { .. })));
    assert_eq!(manager.store().plugins().unwrap(), vec![]);
    assert!(!env.data_dir().join("plugins").join(ID).exists());
}

#[test]
fn tampered_or_mismatched_packages_never_become_verified() {
    let (mut bytes, expected) = built(ID, "1.0.0", &base_permissions());
    let last = bytes.len() - 20;
    bytes[last] ^= 0xff;
    assert!(matches!(
        verify_package(&bytes, &expected, &Limits::default()),
        Err(PackageError::HashMismatch { .. })
    ));

    // Consent shown for other permissions than the package declares.
    let (bytes, mut expected) = built(ID, "1.0.0", &base_permissions());
    expected.permissions = Some(with_notifications());
    assert!(matches!(
        verify_package(&bytes, &expected, &Limits::default()),
        Err(PackageError::PermissionsMismatch)
    ));

    // A different plugin than pinned.
    let (bytes, mut expected) = built(ID, "1.0.0", &base_permissions());
    expected.version = semver::Version::new(1, 0, 1);
    assert!(matches!(
        verify_package(&bytes, &expected, &Limits::default()),
        Err(PackageError::WrongPlugin { .. })
    ));
}

#[test]
fn modified_files_are_detected_and_mark_the_plugin_broken() {
    let env = Env::new();
    let mut manager = env.open();
    let package = package("1.0.0", &base_permissions());
    manager
        .install(
            &package,
            &InstallSource::Sideload,
            &Consent::all(&grants_of(&package), "user"),
        )
        .unwrap();
    let bundle = env
        .data_dir()
        .join("plugins")
        .join(ID)
        .join("1.0.0")
        .join("plugin.js");
    let mut permissions = std::fs::metadata(&bundle).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&bundle, permissions).unwrap();
    std::fs::write(&bundle, "steal()").unwrap();

    let result = manager.enable(&id());
    assert!(matches!(result, Err(StoreError::Integrity { .. })));
    assert_eq!(state(&manager), PluginState::Broken);
    assert!(matches!(
        manager.enable(&id()),
        Err(StoreError::IllegalTransition {
            from: PluginState::Broken,
            to: PluginState::Enabled,
            ..
        })
    ));
    // A broken plugin is a known state, not an inconsistency; it can still
    // be uninstalled.
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
    manager.uninstall(&id()).unwrap();
    assert_eq!(manager.store().plugins().unwrap(), vec![]);
}

#[test]
fn recovery_marks_an_active_version_with_modified_files_broken() {
    let env = Env::new();
    {
        let mut manager = env.open();
        install_enabled(&mut manager, "1.0.0");
    }
    let extra = env
        .data_dir()
        .join("plugins")
        .join(ID)
        .join("1.0.0")
        .join("extra.js");
    std::fs::write(extra, "planted").unwrap();
    let mut manager = env.open();
    let report = manager.recover().unwrap();
    assert_eq!(report.broken, vec![id()]);
    assert_eq!(state(&manager), PluginState::Broken);
    assert_eq!(manager.store().effective_grants(&id()).unwrap(), vec![]);
}

// ------------------------------------------------------------------ consent

#[test]
fn enabling_requires_every_declared_permission_to_be_granted() {
    let env = Env::new();
    let mut manager = env.open();
    let package = package("1.0.0", &base_permissions());
    manager
        .install(&package, &InstallSource::Sideload, &Consent::none("user"))
        .unwrap();
    let Err(StoreError::ConsentRequired { missing, .. }) = manager.enable(&id()) else {
        panic!("enabled without consent");
    };
    assert_eq!(missing.len(), grants_of(&package).len());

    let all: Vec<GrantKey> = grants_of(&package).keys().cloned().collect();
    let (last, rest) = all.split_last().unwrap();
    manager
        .grant(&id(), &Consent::of(rest.iter().cloned(), "user"))
        .unwrap();
    let Err(StoreError::ConsentRequired { missing, .. }) = manager.enable(&id()) else {
        panic!("enabled with partial consent");
    };
    assert_eq!(missing, vec![last.clone()]);

    manager
        .grant(&id(), &Consent::of([last.clone()], "user"))
        .unwrap();
    manager.enable(&id()).unwrap();
    assert_eq!(state(&manager), PluginState::Enabled);
    assert_eq!(
        manager.store().effective_grants(&id()).unwrap().len(),
        all.len()
    );
    assert!(
        manager
            .store()
            .grants(&id())
            .unwrap()
            .iter()
            .all(|g| g.source == GrantSource::User)
    );
}

#[test]
fn revoking_a_grant_narrows_the_effective_set_and_blocks_re_enabling() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let network = key_of("network:api.github.com");
    manager.revoke(&id(), &network, "user").unwrap();
    assert_eq!(state(&manager), PluginState::Enabled);
    let effective = manager.store().effective_grants(&id()).unwrap();
    assert!(effective.iter().all(|(key, _)| *key != network));
    assert_eq!(effective.len(), 3);

    manager.disable(&id()).unwrap();
    assert!(matches!(
        manager.enable(&id()),
        Err(StoreError::ConsentRequired { missing, .. }) if missing == vec![network.clone()]
    ));
    manager
        .grant(&id(), &Consent::of([network.clone()], "user"))
        .unwrap();
    manager.enable(&id()).unwrap();

    assert!(matches!(
        manager.revoke(&id(), &key_of("notifications"), "user"),
        Err(StoreError::UnknownGrant { .. })
    ));
    let audit = manager.store().audit(&id()).unwrap();
    assert!(audit.iter().any(|a| a.op == "revoke"
        && a.target.as_deref() == Some("network:api.github.com")
        && a.decision == "revoked"));
}

// --------------------------------------------------------- illegal changes

#[test]
fn illegal_transitions_are_refused_and_change_nothing() {
    let env = Env::new();
    let mut manager = env.open();
    assert!(matches!(
        manager.enable(&id()),
        Err(StoreError::NotInstalled(_))
    ));
    install_enabled(&mut manager, "1.0.0");
    assert!(matches!(
        manager.enable(&id()),
        Err(StoreError::IllegalTransition {
            from: PluginState::Enabled,
            to: PluginState::Enabled,
            ..
        })
    ));
    manager.disable(&id()).unwrap();
    assert!(matches!(
        manager.disable(&id()),
        Err(StoreError::IllegalTransition {
            from: PluginState::Disabled,
            to: PluginState::Disabled,
            ..
        })
    ));
    assert!(matches!(
        manager.rollback(&id(), &Verdicts::default()),
        Err(StoreError::NoPreviousVersion(_))
    ));
    assert!(matches!(
        manager.approve_update(&id(), &Consent::none("user")),
        Err(StoreError::NoStagedUpdate(_))
    ));
    assert!(matches!(
        manager.decline_update(&id()),
        Err(StoreError::NoStagedUpdate(_))
    ));
    manager.quarantine(&id(), "revoked: malware").unwrap();
    assert!(matches!(
        manager.quarantine(&id(), "again"),
        Err(StoreError::IllegalTransition {
            from: PluginState::Quarantined,
            to: PluginState::Quarantined,
            ..
        })
    ));
    // `disable` never lifts a quarantine.
    assert!(matches!(
        manager.disable(&id()),
        Err(StoreError::IllegalTransition { .. })
    ));
    assert_eq!(state(&manager), PluginState::Quarantined);
}

#[test]
fn updates_must_be_newer_and_new() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.1.0");
    let same = package("1.1.0", &base_permissions());
    assert!(matches!(
        manager.update(&same, &InstallSource::Sideload),
        Err(StoreError::VersionInstalled { .. })
    ));
    let older = package("1.0.0", &base_permissions());
    assert!(matches!(
        manager.update(&older, &InstallSource::Sideload),
        Err(StoreError::NotNewer { .. })
    ));
    let other = package_for("acme.other-tool", "2.0.0", &base_permissions());
    assert!(matches!(
        manager.update(&other, &InstallSource::Sideload),
        Err(StoreError::NotInstalled(_))
    ));
    assert_eq!(state(&manager), PluginState::Enabled);
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

// ------------------------------------------------------------------ updates

#[test]
fn an_update_that_only_narrows_applies_at_once() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let update = package("1.1.0", &without_background());
    let outcome = manager.update(&update, &InstallSource::Sideload).unwrap();
    let UpdateOutcome::Applied { version, diff } = outcome else {
        panic!("narrowing waited for consent");
    };
    assert_eq!(version.to_string(), "1.1.0");
    assert_eq!(diff.removed, vec![key_of("background")]);
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    assert_eq!(plugin.state, PluginState::Enabled);
    assert_eq!(plugin.active().unwrap().version.to_string(), "1.1.0");
    assert_eq!(plugin.previous().unwrap().version.to_string(), "1.0.0");
    assert!(
        manager
            .store()
            .grants(&id())
            .unwrap()
            .iter()
            .all(|g| g.key != key_of("background"))
    );
    let files = plugin_files(&manager);
    assert!(files.contains("acme.github-prs/1.0.0/plugin.js"));
    assert!(files.contains("acme.github-prs/1.1.0/plugin.js"));
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

#[test]
fn an_escalating_update_waits_for_fresh_consent() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let update = package("1.1.0", &with_notifications());
    let outcome = manager.update(&update, &InstallSource::Sideload).unwrap();
    let UpdateOutcome::AwaitingConsent { diff, .. } = outcome else {
        panic!("escalation applied without consent");
    };
    assert_eq!(diff.escalations(), vec![key_of("notifications")]);

    // The running version is unchanged.
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    assert_eq!(plugin.state, PluginState::Enabled);
    assert_eq!(plugin.active().unwrap().version.to_string(), "1.0.0");
    assert_eq!(plugin.staged().unwrap().version.to_string(), "1.1.0");
    assert!(
        manager
            .store()
            .effective_grants(&id())
            .unwrap()
            .iter()
            .all(|(k, _)| k.kind != PermissionKind::Notifications)
    );
    let (version, pending) = manager.pending_update(&id()).unwrap().unwrap();
    assert_eq!(version.to_string(), "1.1.0");
    assert!(pending.is_escalation());

    // A second update while one waits is refused.
    let another = package("1.2.0", &with_notifications());
    assert!(matches!(
        manager.update(&another, &InstallSource::Sideload),
        Err(StoreError::UpdatePending { .. })
    ));
    // Approval without the escalated key is refused.
    assert!(matches!(
        manager.approve_update(&id(), &Consent::none("user")),
        Err(StoreError::ConsentRequired { missing, .. }) if missing == vec![key_of("notifications")]
    ));
    assert_eq!(state(&manager), PluginState::Enabled);

    manager
        .approve_update(&id(), &Consent::of([key_of("notifications")], "user"))
        .unwrap();
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    assert_eq!(plugin.state, PluginState::Enabled);
    assert_eq!(plugin.active().unwrap().version.to_string(), "1.1.0");
    assert!(plugin.staged().is_none());
    let notifications = manager
        .store()
        .grants(&id())
        .unwrap()
        .into_iter()
        .find(|g| g.key == key_of("notifications"))
        .unwrap();
    assert_eq!(notifications.source, GrantSource::Update);
    assert_eq!(notifications.granted_for_version, "1.1.0");
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

#[test]
fn a_broadened_scope_is_an_escalation() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let mut permissions = base_permissions();
    permissions[0]["methods"] = json!(["GET", "POST"]);
    let update = package("1.1.0", &permissions);
    let outcome = manager.update(&update, &InstallSource::Sideload).unwrap();
    let UpdateOutcome::AwaitingConsent { diff, .. } = outcome else {
        panic!("broadened methods applied without consent");
    };
    assert_eq!(diff.broadened.len(), 1);
    assert_eq!(diff.broadened[0].key, key_of("network:api.github.com"));
}

#[test]
fn declining_an_update_removes_its_files_and_keeps_the_running_version() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let update = package("1.1.0", &with_notifications());
    manager.update(&update, &InstallSource::Sideload).unwrap();
    manager.decline_update(&id()).unwrap();
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    assert!(plugin.staged().is_none());
    assert_eq!(plugin.active().unwrap().version.to_string(), "1.0.0");
    assert!(!plugin_files(&manager).contains("acme.github-prs/1.1.0"));
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

#[test]
fn only_one_previous_version_is_kept() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    for version in ["1.1.0", "1.2.0"] {
        let update = package(version, &base_permissions());
        assert!(matches!(
            manager.update(&update, &InstallSource::Sideload).unwrap(),
            UpdateOutcome::Applied { .. }
        ));
    }
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    let mut versions: Vec<_> = plugin
        .versions
        .iter()
        .map(|v| (v.version.to_string(), v.role))
        .collect();
    versions.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        versions,
        vec![
            ("1.1.0".into(), VersionRole::Previous),
            ("1.2.0".into(), VersionRole::Active)
        ]
    );
    let files = plugin_files(&manager);
    assert!(!files.contains("acme.github-prs/1.0.0"));
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

// ----------------------------------------------------------------- rollback

#[test]
fn rollback_restores_the_previous_version_and_drops_newer_grants() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let update = package("1.1.0", &with_notifications());
    manager.update(&update, &InstallSource::Sideload).unwrap();
    manager
        .approve_update(&id(), &Consent::of([key_of("notifications")], "user"))
        .unwrap();

    let version = manager.rollback(&id(), &Verdicts::default()).unwrap();
    assert_eq!(version.to_string(), "1.0.0");
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    assert_eq!(plugin.state, PluginState::Enabled);
    assert_eq!(plugin.active().unwrap().version.to_string(), "1.0.0");
    assert_eq!(plugin.previous().unwrap().version.to_string(), "1.1.0");
    assert!(
        manager
            .store()
            .grants(&id())
            .unwrap()
            .iter()
            .all(|g| g.key != key_of("notifications"))
    );
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

#[test]
fn rollback_to_a_version_that_needs_a_dropped_grant_ends_disabled() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let update = package("1.1.0", &without_background());
    manager.update(&update, &InstallSource::Sideload).unwrap();
    manager.rollback(&id(), &Verdicts::default()).unwrap();
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    assert_eq!(plugin.state, PluginState::Disabled);
    assert!(plugin.state_reason.is_some());
    assert!(matches!(
        manager.enable(&id()),
        Err(StoreError::ConsentRequired { missing, .. }) if missing == vec![key_of("background")]
    ));
}

#[test]
fn rollback_to_a_revoked_version_is_refused() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let update = package("1.1.0", &base_permissions());
    manager.update(&update, &InstallSource::Sideload).unwrap();
    let verdicts = Verdicts::disable(ID, "1.0.0", "malware");
    assert!(matches!(
        manager.rollback(&id(), &verdicts),
        Err(StoreError::Revoked { .. })
    ));
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    assert_eq!(plugin.active().unwrap().version.to_string(), "1.1.0");
}

// --------------------------------------------------------------- revocation

#[test]
fn a_revocation_verdict_quarantines_until_it_is_lifted() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let verdicts = Verdicts::disable(ID, "1.0.0", "exfiltrates tokens");
    let changes = manager.apply_verdicts(&verdicts).unwrap();
    assert!(
        matches!(&changes[..], [VerdictChange::Quarantined { reason, .. }]
        if reason.contains("exfiltrates tokens") && reason.contains("advisory"))
    );
    assert_eq!(state(&manager), PluginState::Quarantined);
    assert_eq!(manager.store().effective_grants(&id()).unwrap(), vec![]);
    assert!(matches!(
        manager.enable(&id()),
        Err(StoreError::IllegalTransition {
            from: PluginState::Quarantined,
            ..
        })
    ));
    // Applying the same verdicts again changes nothing; data is kept.
    assert_eq!(manager.apply_verdicts(&verdicts).unwrap(), vec![]);
    assert_eq!(manager.store().grants(&id()).unwrap().len(), 4);

    let changes = manager.apply_verdicts(&Verdicts::default()).unwrap();
    assert!(matches!(&changes[..], [VerdictChange::Released { .. }]));
    assert_eq!(state(&manager), PluginState::Disabled);
    manager.enable(&id()).unwrap();
}

#[test]
fn a_quarantined_plugin_can_update_to_a_version_that_is_not_revoked() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    manager
        .apply_verdicts(&Verdicts::disable(ID, "1.0.0", "bad"))
        .unwrap();
    let update = package("1.0.1", &base_permissions());
    manager.update(&update, &InstallSource::Sideload).unwrap();
    // Never straight back to enabled.
    assert_eq!(state(&manager), PluginState::Disabled);
    assert_eq!(
        manager
            .apply_verdicts(&Verdicts::disable(ID, "1.0.0", "bad"))
            .unwrap(),
        vec![]
    );
    manager.enable(&id()).unwrap();
}

#[test]
fn a_revoked_staged_update_is_dropped() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    let update = package("1.1.0", &with_notifications());
    manager.update(&update, &InstallSource::Sideload).unwrap();
    let changes = manager
        .apply_verdicts(&Verdicts::disable(ID, "1.1.0", "bad"))
        .unwrap();
    assert!(matches!(
        &changes[..],
        [VerdictChange::StagedDropped { .. }]
    ));
    assert_eq!(state(&manager), PluginState::Enabled);
    assert!(manager.pending_update(&id()).unwrap().is_none());
}

// ---------------------------------------------------------------- uninstall

#[test]
fn uninstall_leaves_no_files_and_no_rows() {
    let env = Env::new();
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    manager
        .update(
            &package("1.1.0", &base_permissions()),
            &InstallSource::Sideload,
        )
        .unwrap();
    manager
        .update(
            &package("1.2.0", &with_notifications()),
            &InstallSource::Sideload,
        )
        .unwrap();
    manager.uninstall(&id()).unwrap();

    assert_eq!(
        plugin_files(&manager),
        [".staging", ".trash"]
            .map(String::from)
            .into_iter()
            .collect()
    );
    let connection = manager.store().connection_for_tests();
    for table in [
        "plugins",
        "plugin_versions",
        "plugin_grants",
        "plugin_journal",
    ] {
        let rows: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
    assert_eq!(*env.hook.calls.lock().unwrap(), vec![ID.to_owned()]);
    // The audit outlives the plugin and holds keys, never values.
    let audit = manager.store().audit(&id()).unwrap();
    assert!(audit.iter().any(|a| a.op == "uninstall"));
    for row in &audit {
        if let Some(target) = &row.target {
            assert!(target.parse::<GrantKey>().is_ok(), "{target}");
        }
    }
    // The plugin can be installed again from scratch.
    install_enabled(&mut manager, "1.0.0");
}

#[test]
fn a_failing_secret_hook_leaves_the_uninstall_to_recovery() {
    let env = Env::new();
    *env.hook.fail_first.lock().unwrap() = 1;
    let mut manager = env.open();
    install_enabled(&mut manager, "1.0.0");
    assert!(matches!(
        manager.uninstall(&id()),
        Err(StoreError::Hook { .. })
    ));
    assert_eq!(state(&manager), PluginState::Uninstalling);
    assert!(matches!(
        manager.enable(&id()),
        Err(StoreError::IllegalTransition { .. })
    ));
    let report = manager.recover().unwrap();
    assert_eq!(report.completed.len(), 1);
    assert_eq!(manager.store().plugins().unwrap(), vec![]);
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

// ---------------------------------------------------------------- storage

#[test]
fn the_database_is_encrypted_and_a_wrong_key_fails() {
    let env = Env::new();
    {
        let mut manager = env.open();
        install_enabled(&mut manager, "1.0.0");
    }
    let path = env.data_dir().join("plugins").join("plugins.db");
    let header = std::fs::read(&path).unwrap();
    assert!(!header.starts_with(b"SQLite format 3\0"));

    let wrong = StoreKey::from_bytes([0x11; 32]);
    assert!(matches!(
        PluginManager::open(env.data_dir(), &wrong, ManagerOptions::default()),
        Err(StoreError::InvalidKey)
    ));
    assert!(matches!(
        PluginStore::open(&path, &wrong, Duration::from_secs(1)),
        Err(StoreError::InvalidKey)
    ));
    // Plain SQLite cannot read it either.
    let plain = rusqlite_plain_open(&path);
    assert!(plain.is_err());
    // The right key still works.
    let store = PluginStore::open(&path, &key(), Duration::from_secs(1)).unwrap();
    assert_eq!(store.plugins().unwrap().len(), 1);
}

fn rusqlite_plain_open(path: &std::path::Path) -> rusqlite::Result<i64> {
    // No key: SQLite reads the encrypted header as garbage.
    let connection = rusqlite::Connection::open(path)?;
    connection.query_row("SELECT count(*) FROM plugins", [], |r| r.get(0))
}

#[test]
fn plugin_data_never_reaches_the_configuration_database() {
    let env = Env::new();
    let config = env.data_dir().join("deckpilot.db");
    std::fs::write(&config, b"configuration database stand-in").unwrap();
    let before = std::fs::read(&config).unwrap();
    {
        let mut manager = env.open();
        assert_eq!(
            manager.store().path(),
            env.data_dir().join("plugins").join("plugins.db")
        );
        install_enabled(&mut manager, "1.0.0");
        manager
            .update(
                &package("1.1.0", &with_notifications()),
                &InstallSource::Sideload,
            )
            .unwrap();
        manager
            .approve_update(&id(), &Consent::of([key_of("notifications")], "user"))
            .unwrap();
        manager.rollback(&id(), &Verdicts::default()).unwrap();
        manager.uninstall(&id()).unwrap();
        install_enabled(&mut manager, "1.0.0");
    }
    assert_eq!(std::fs::read(&config).unwrap(), before);
    for path in common::data_files(env.data_dir()) {
        assert!(
            path.starts_with("plugins") || path == std::path::Path::new("deckpilot.db"),
            "{}",
            path.display()
        );
    }
    // Opening the configuration database as the plugin store is refused.
    assert!(matches!(
        PluginStore::open(&config, &key(), Duration::from_secs(1)),
        Err(StoreError::ConfigDatabase(_))
    ));
    assert_eq!(std::fs::read(&config).unwrap(), before);
}

// -------------------------------------------------------------- concurrency

#[test]
fn one_manager_per_data_directory() {
    let env = Env::new();
    let manager = env.open();
    assert!(matches!(
        PluginManager::open(env.data_dir(), &key(), ManagerOptions::default()),
        Err(StoreError::Locked(_))
    ));
    drop(manager);
    let _again = env.open();
}

#[test]
fn writes_wait_for_another_writer_and_fail_cleanly_after_the_busy_timeout() {
    let env = Env::new();
    let options = ManagerOptions {
        busy_timeout: Duration::from_millis(150),
        failpoints: Arc::new(NoFailpoints),
        ..ManagerOptions::default()
    };
    let mut manager = PluginManager::open(env.data_dir(), &key(), options).unwrap();
    let package = package("1.0.0", &base_permissions());
    manager
        .install(
            &package,
            &InstallSource::Sideload,
            &Consent::all(&grants_of(&package), "user"),
        )
        .unwrap();

    // A reader in another "process" holds the write lock.
    let other = PluginStore::open(
        &env.data_dir().join("plugins").join("plugins.db"),
        &key(),
        Duration::from_millis(150),
    )
    .unwrap();
    other
        .connection_for_tests()
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();
    let result = manager.enable(&id());
    assert!(
        matches!(&result, Err(StoreError::Sqlite(e)) if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)),
        "{result:?}"
    );
    let update = package_for(ID, "1.1.0", &base_permissions());
    assert!(manager.update(&update, &InstallSource::Sideload).is_err());
    // Readers still see a consistent store.
    assert_eq!(
        other.plugin(&id()).unwrap().unwrap().state,
        PluginState::Disabled
    );
    other
        .connection_for_tests()
        .execute_batch("ROLLBACK")
        .unwrap();

    manager.enable(&id()).unwrap();
    assert!(!plugin_files(&manager).contains("acme.github-prs/1.1.0"));
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

#[test]
fn a_concurrent_writer_is_serialized_by_the_busy_timeout() {
    let env = Env::new();
    let mut manager = env.open();
    let package = package("1.0.0", &base_permissions());
    manager
        .install(
            &package,
            &InstallSource::Sideload,
            &Consent::all(&grants_of(&package), "user"),
        )
        .unwrap();
    let path = env.data_dir().join("plugins").join("plugins.db");
    let holder = std::thread::spawn(move || {
        let other = PluginStore::open(&path, &key(), Duration::from_secs(5)).unwrap();
        other
            .connection_for_tests()
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        other
            .connection_for_tests()
            .execute_batch("COMMIT")
            .unwrap();
    });
    std::thread::sleep(Duration::from_millis(50));
    // Waits for the other writer instead of failing.
    manager.enable(&id()).unwrap();
    holder.join().unwrap();
    assert_eq!(state(&manager), PluginState::Enabled);
}

// -------------------------------------------------------------------- audit

#[test]
fn the_audit_log_is_pruned_by_count_and_age() {
    let env = Env::new();
    let now = Arc::new(std::sync::atomic::AtomicI64::new(1_000_000));
    let clock = now.clone();
    let options = ManagerOptions {
        clock: Arc::new(move || clock.load(std::sync::atomic::Ordering::SeqCst)),
        audit: AuditRetention {
            max_age_seconds: 100,
            max_rows_per_plugin: 5,
        },
        ..ManagerOptions::default()
    };
    let mut manager = PluginManager::open(env.data_dir(), &key(), options).unwrap();
    install_enabled(&mut manager, "1.0.0");
    let network = key_of("network:api.github.com");
    for _ in 0..10 {
        manager.revoke(&id(), &network, "user").unwrap();
        manager
            .grant(&id(), &Consent::of([network.clone()], "user"))
            .unwrap();
    }
    assert_eq!(manager.store().audit(&id()).unwrap().len(), 5);
    now.fetch_add(1_000, std::sync::atomic::Ordering::SeqCst);
    manager.disable(&id()).unwrap();
    let audit = manager.store().audit(&id()).unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].op, "disable");
}

#[test]
fn sources_are_recorded() {
    let env = Env::new();
    let mut manager = env.open();
    let source = InstallSource::Registry {
        registry: "https://registry.example".into(),
    };
    let package = package("1.0.0", &base_permissions());
    let record = manager
        .install(&package, &source, &Consent::none("user"))
        .unwrap();
    assert_eq!(record.source, source);
    manager
        .update(
            &package_for(ID, "1.1.0", &base_permissions()),
            &InstallSource::Dev,
        )
        .unwrap();
    let record = manager.store().plugin(&id()).unwrap().unwrap();
    assert_eq!(record.source, InstallSource::Dev);
}
