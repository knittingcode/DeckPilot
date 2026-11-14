//! Crash safety: every operation is crashed at every one of its failpoints
//! (and recovery itself is crashed), then recovered. The result must be
//! consistent and equal to either the state before or the state after the
//! operation, recovery must be idempotent, and an undone operation must
//! succeed when retried.

#![cfg(feature = "sqlcipher")]

mod common;

use std::sync::Arc;

use common::{
    Env, Snapshot, Verdicts, base_permissions, grants_of, id, package, snapshot,
    with_notifications, without_background,
};
use deckpilot_plugin_store::{
    Consent, CrashOnce, Failpoint, InstallSource, Operation, PluginManager, Result, StoreError,
};

type Step = fn(&mut PluginManager) -> Result<()>;

struct Scenario {
    name: &'static str,
    op: Operation,
    setup: Step,
    run: Step,
}

fn install(manager: &mut PluginManager, version: &str) -> Result<()> {
    let package = package(version, &base_permissions());
    let consent = Consent::all(&grants_of(&package), "user");
    manager.install(&package, &InstallSource::Sideload, &consent)?;
    Ok(())
}

fn install_enabled(manager: &mut PluginManager) -> Result<()> {
    install(manager, "1.0.0")?;
    manager.enable(&id())
}

fn update(
    manager: &mut PluginManager,
    version: &str,
    permissions: &[serde_json::Value],
) -> Result<()> {
    manager
        .update(&package(version, permissions), &InstallSource::Sideload)
        .map(drop)
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "install",
            op: Operation::Install,
            setup: |_| Ok(()),
            run: |m| install(m, "1.0.0"),
        },
        Scenario {
            name: "update (narrowing, applied)",
            op: Operation::Update,
            setup: install_enabled,
            run: |m| update(m, "1.1.0", &without_background()),
        },
        Scenario {
            name: "update (escalation, staged)",
            op: Operation::Update,
            setup: install_enabled,
            run: |m| update(m, "1.1.0", &with_notifications()),
        },
        Scenario {
            name: "update (retires the oldest version)",
            op: Operation::Update,
            setup: |m| {
                install_enabled(m)?;
                update(m, "1.1.0", &base_permissions())
            },
            run: |m| update(m, "1.2.0", &base_permissions()),
        },
        Scenario {
            name: "approve update",
            op: Operation::ApproveUpdate,
            setup: |m| {
                install_enabled(m)?;
                update(m, "1.1.0", &with_notifications())
            },
            run: |m| {
                let consent = Consent::of(["notifications".parse().unwrap()], "user");
                m.approve_update(&id(), &consent).map(drop)
            },
        },
        Scenario {
            name: "decline update",
            op: Operation::DeclineUpdate,
            setup: |m| {
                install_enabled(m)?;
                update(m, "1.1.0", &with_notifications())
            },
            run: |m| m.decline_update(&id()),
        },
        Scenario {
            name: "rollback",
            op: Operation::Rollback,
            setup: |m| {
                install_enabled(m)?;
                update(m, "1.1.0", &base_permissions())
            },
            run: |m| m.rollback(&id(), &Verdicts::default()).map(drop),
        },
        Scenario {
            name: "uninstall",
            op: Operation::Uninstall,
            setup: |m| {
                install_enabled(m)?;
                update(m, "1.1.0", &base_permissions())
            },
            run: |m| m.uninstall(&id()),
        },
    ]
}

/// The state after `scenario` without any crash.
fn reference(scenario: &Scenario) -> Snapshot {
    let env = Env::new();
    let mut manager = env.open();
    (scenario.setup)(&mut manager).unwrap();
    (scenario.run)(&mut manager).unwrap();
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
    snapshot(&manager)
}

/// Runs `scenario` crashing at `point`; returns the environment (manager
/// dropped, as after a process death) and the state before the operation.
fn crash(scenario: &Scenario, point: Failpoint) -> (Env, Snapshot) {
    let env = Env::new();
    let failpoint = Arc::new(CrashOnce::new(scenario.op, point));
    {
        let mut manager = env.open();
        (scenario.setup)(&mut manager).unwrap();
    }
    let before = {
        let mut manager = env.open_with(failpoint.clone());
        let before = snapshot(&manager);
        let error = (scenario.run)(&mut manager).unwrap_err();
        assert!(
            matches!(error, StoreError::Crashed { op, point: p } if op == scenario.op && p == point),
            "{}: {error}",
            scenario.name
        );
        before
    };
    assert!(
        failpoint.fired(),
        "{}: {point:?} never reached",
        scenario.name
    );
    (env, before)
}

fn check_recovered(
    scenario: &Scenario,
    point: Failpoint,
    manager: &mut PluginManager,
    before: &Snapshot,
    after: &Snapshot,
) {
    let label = format!("{} at {point:?}", scenario.name);
    assert_eq!(manager.check_consistency().unwrap(), vec![], "{label}");
    assert!(manager.store().journal().unwrap().is_empty(), "{label}");
    let recovered = snapshot(manager);
    assert!(
        recovered == *before || recovered == *after,
        "{label}: neither before nor after\nrecovered: {recovered:#?}\nbefore: {before:#?}\nafter: {after:#?}"
    );
    // Recovery is idempotent.
    let again = manager.recover().unwrap();
    assert!(again.is_clean(), "{label}: second recovery did {again:?}");
    assert_eq!(snapshot(manager), recovered, "{label}");
    // An undone operation can simply be retried.
    if recovered == *before && before != after {
        (scenario.run)(manager).unwrap();
        assert_eq!(snapshot(manager), *after, "{label}: retry");
        assert!(manager.check_consistency().unwrap().is_empty(), "{label}");
    }
}

#[test]
fn every_failpoint_of_every_operation_recovers_to_before_or_after() {
    let mut cases = 0;
    let mut completed = 0;
    let mut undone = 0;
    for scenario in scenarios() {
        let after = reference(&scenario);
        for &point in scenario.op.failpoints() {
            let (env, before) = crash(&scenario, point);
            let mut manager = env.open();
            let report = manager.recover().unwrap();
            if snapshot(&manager) == after {
                completed += 1;
            } else {
                undone += 1;
            }
            let _ = report;
            check_recovered(&scenario, point, &mut manager, &before, &after);
            cases += 1;
        }
    }
    // 4 install + 3 × 4 update + 5 uninstall + approve, decline, rollback.
    assert_eq!(cases, 4 + 12 + 5 + 3);
    assert!(
        completed > 0 && undone > 0,
        "{completed} completed, {undone} undone"
    );
}

#[test]
fn a_crash_during_recovery_is_repaired_by_the_next_recovery() {
    let mut crashed_recoveries = 0;
    for scenario in scenarios() {
        let after = reference(&scenario);
        for &point in scenario.op.failpoints() {
            let (env, before) = crash(&scenario, point);
            let failpoint = Arc::new(CrashOnce::new(
                Operation::Recover,
                Failpoint::RecoveryFilesResolved,
            ));
            {
                let mut manager = env.open_with(failpoint.clone());
                match manager.recover() {
                    Err(error) => {
                        assert!(error.is_crash(), "{error}");
                        crashed_recoveries += 1;
                    }
                    Ok(_) => assert!(!failpoint.fired()),
                }
            }
            let mut manager = env.open();
            manager.recover().unwrap();
            check_recovered(&scenario, point, &mut manager, &before, &after);
        }
    }
    // Every crash that leaves a journal entry passes the recovery failpoint.
    assert!(crashed_recoveries >= 10, "{crashed_recoveries}");
}

#[test]
fn the_failpoint_list_matches_the_operations() {
    for op in Operation::ALL {
        assert!(!op.failpoints().is_empty(), "{op}");
    }
    assert_eq!(Operation::Install.failpoints().len(), 4);
    assert_eq!(Operation::Uninstall.failpoints().len(), 5);
}

#[test]
fn staging_leftovers_and_orphan_directories_are_swept() {
    let env = Env::new();
    {
        let mut manager = env.open();
        install_enabled(&mut manager).unwrap();
    }
    let root = env.data_dir().join("plugins");
    std::fs::create_dir_all(root.join(".staging/unpack-x/files/assets")).unwrap();
    std::fs::write(root.join(".staging/unpack-x/files/plugin.js"), "half").unwrap();
    std::fs::create_dir_all(root.join("acme.github-prs/9.9.9")).unwrap();
    std::fs::create_dir_all(root.join("acme.ghost/1.0.0")).unwrap();
    std::fs::write(root.join(".trash/old"), "x").unwrap();

    let mut manager = env.open();
    let inconsistencies = manager.check_consistency().unwrap();
    assert_eq!(inconsistencies.len(), 3, "{inconsistencies:?}");
    let report = manager.recover().unwrap();
    let mut orphans = report.orphans_removed.clone();
    orphans.sort();
    assert_eq!(orphans, vec!["acme.ghost", "acme.github-prs/9.9.9"]);
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
    assert_eq!(
        manager.store().plugin(&id()).unwrap().unwrap().state,
        deckpilot_plugin_store::PluginState::Enabled
    );
}

#[test]
fn a_damaged_previous_version_is_dropped_by_recovery() {
    let env = Env::new();
    {
        let mut manager = env.open();
        install_enabled(&mut manager).unwrap();
        update(&mut manager, "1.1.0", &base_permissions()).unwrap();
    }
    let removed = env
        .data_dir()
        .join("plugins/acme.github-prs/1.0.0/README.md");
    let mut permissions = std::fs::metadata(&removed).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&removed, permissions).unwrap();
    std::fs::remove_file(removed).unwrap();

    let mut manager = env.open();
    let report = manager.recover().unwrap();
    assert_eq!(report.dropped_versions.len(), 1);
    let plugin = manager.store().plugin(&id()).unwrap().unwrap();
    assert!(plugin.previous().is_none());
    assert!(matches!(
        manager.rollback(&id(), &Verdicts::default()),
        Err(StoreError::NoPreviousVersion(_))
    ));
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

#[test]
fn a_file_system_error_during_install_is_undone_at_once() {
    let env = Env::new();
    let mut manager = env.open();
    // A file where the plugin directory must go.
    std::fs::write(
        env.data_dir().join("plugins").join(common::ID),
        "in the way",
    )
    .unwrap();
    let result = install(&mut manager, "1.0.0");
    assert!(
        matches!(result, Err(StoreError::Io { .. } | StoreError::Package(_))),
        "{result:?}"
    );
    assert_eq!(manager.store().plugins().unwrap(), vec![]);
    assert_eq!(manager.store().journal().unwrap(), vec![]);
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
    install(&mut manager, "1.0.0").unwrap();
}

#[test]
fn an_operation_on_a_plugin_with_an_unfinished_one_is_refused() {
    let env = Env::new();
    let failpoint = Arc::new(CrashOnce::new(Operation::Update, Failpoint::FilesUnpacked));
    let mut manager = env.open_with(failpoint);
    install_enabled(&mut manager).unwrap();
    assert!(
        update(&mut manager, "1.1.0", &base_permissions())
            .unwrap_err()
            .is_crash()
    );
    // Same process, no recovery yet: everything on that plugin waits.
    assert!(matches!(
        manager.disable(&id()),
        Err(StoreError::IllegalTransition { .. })
    ));
    assert!(matches!(
        manager.uninstall(&id()),
        Err(StoreError::OperationInProgress {
            op: Operation::Update,
            ..
        })
    ));
    assert!(matches!(
        manager.quarantine(&id(), "x"),
        Err(StoreError::OperationInProgress { .. })
    ));
    manager.recover().unwrap();
    manager.uninstall(&id()).unwrap();
    assert_eq!(manager.check_consistency().unwrap(), vec![]);
}

#[test]
fn an_interrupted_uninstall_deletes_the_secrets_exactly_by_completion() {
    for &point in Operation::Uninstall.failpoints() {
        let env = Env::new();
        {
            let mut manager = env.open_with(Arc::new(CrashOnce::new(Operation::Uninstall, point)));
            install_enabled(&mut manager).unwrap();
            assert!(manager.uninstall(&id()).unwrap_err().is_crash());
        }
        let mut manager = env.open();
        manager.recover().unwrap();
        assert!(manager.store().plugins().unwrap().is_empty(), "{point:?}");
        let calls = env.hook.calls.lock().unwrap();
        assert!(!calls.is_empty(), "{point:?}: secrets never deleted");
        assert!(calls.iter().all(|c| c == common::ID));
    }
}
