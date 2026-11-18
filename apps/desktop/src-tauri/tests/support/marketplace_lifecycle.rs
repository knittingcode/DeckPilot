//! The second marketplace scenario: the plugin lifecycle where it can go
//! wrong, through the app's commands, against the same fixture as the
//! main scenario (an isolated data directory with a password-protected
//! configuration, a registry signed with throwaway keys and served over
//! local HTTPS, canned HTTP, a simulated MK.2).
//!
//! The sample is published as three versions made for it:
//!
//! * 1.0.0, as in the main scenario;
//! * 1.1.0, which asks for `notifications` (an added permission) and for
//!   `POST` to the host it already reaches (a broadened one);
//! * 1.2.0, which declares an action its module does not implement, so the
//!   sandboxed host refuses to load it.
//!
//! Failed updates (a registry whose signature does not verify, a package
//! that is not the pinned one, a download cut off, the agent killed in the
//! middle of the update) must leave the running version installed,
//! configured and working, with no files of the new one. An update that
//! asks for more must wait for consent to everything it asks for, keep the
//! old version running meanwhile (also across a restart) and grant nothing
//! when declined. Downgrades through an update are refused; a rollback is
//! the one way back, and not to a version the registry revoked. An older
//! registry is refused. A revoked installed version is quarantined, and
//! uninstalling it removes its data but nothing of the configuration.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use deckpilot_core::ActionStatus;
use serde_json::{Value, json};

use super::https::Route;
use super::{
    BERLIN, CREDENTIAL, Driver, Fixture, GREET, GREET_TYPE, HELLO, NETWORK, NOTIFICATIONS, Report,
    TEMPERATURE, TEMPERATURE_TYPE, Via, assert_plugin_data_kept, details, hello, manifest,
    permission, run_action,
};

/// The version that asks for more: `notifications` added, `POST` to the
/// weather host on top of `GET` (the same grant key, broadened).
pub fn escalating(version: &str) -> Value {
    let mut manifest = manifest(version);
    let permissions = manifest["permissions"].as_array_mut().unwrap();
    let network = permissions
        .iter_mut()
        .find(|p| p["kind"] == "network")
        .unwrap();
    network["methods"] = json!(["GET", "POST"]);
    manifest
}

/// [`escalating`] with one more action, `forecast`, which the sample's
/// module does not implement: the package verifies (the registry pins it),
/// but the host refuses the module because its actions are not the
/// manifest's.
pub fn broken(version: &str) -> Value {
    let mut manifest = escalating(version);
    let actions = manifest["contributes"]["actions"].as_array_mut().unwrap();
    let mut forecast = actions[0].clone();
    forecast["id"] = json!("forecast");
    forecast["name"] = json!("Forecast");
    actions.push(forecast);
    manifest
}

/// The package file of `version` as the registry serves it, and its path
/// on the server.
fn release(fx: &Fixture, version: &str) -> (String, Vec<u8>) {
    fn find(dir: &Path, suffix: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find(&path, suffix) {
                    return Some(found);
                }
            } else if path.to_string_lossy().ends_with(suffix) {
                return Some(path);
            }
        }
        None
    }
    let work = &fx.local().registry.work;
    let file = find(&work.join("releases"), &format!("-{version}.dpplugin"))
        .unwrap_or_else(|| panic!("no package of {version} under {}", work.display()));
    let served = file
        .strip_prefix(work)
        .unwrap()
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    (format!("/{served}"), std::fs::read(&file).unwrap())
}

/// A copy of the registry's site as it is now, to serve it again later
/// (a rollback attack: old, but validly signed).
fn snapshot(fx: &Fixture, name: &str) -> PathBuf {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let to = fx.root.path().join(name);
    copy(&fx.local().registry.work, &to);
    to
}

/// The current timestamp's signature with one character of the signature
/// line changed (the file is the minisign text in base64).
fn tampered_timestamp_signature(fx: &Fixture) -> Vec<u8> {
    let path = fx
        .local()
        .registry
        .work
        .join("site/v1/timestamp.json.minisig");
    let base64 = base64::engine::general_purpose::STANDARD;
    let text = std::fs::read_to_string(path).unwrap();
    let decoded = String::from_utf8(base64.decode(text.trim()).unwrap()).unwrap();
    let mut lines: Vec<String> = decoded.lines().map(str::to_owned).collect();
    let line = &mut lines[1];
    let at = line.len() / 2;
    let flipped = if line.as_bytes()[at] == b'A' {
        "B"
    } else {
        "A"
    };
    line.replace_range(at..=at, flipped);
    base64.encode(lines.join("\n") + "\n").into_bytes()
}

/// Revokes `version` of the sample in the registry and signs a new index.
fn revoke(fx: &Fixture, version: &str, why: &str) {
    let registry = &fx.local().registry;
    registry.run(&[
        "revoke",
        "--id",
        HELLO,
        "--versions",
        version,
        "--action",
        "disable",
        "--reason",
        why,
    ]);
    registry.sign();
}

/// The version directories of the sample on disk.
fn version_dirs(fx: &Fixture) -> BTreeSet<String> {
    let layout = deckpilot_plugin_store::Layout::new(&fx.paths.data_dir);
    std::fs::read_dir(layout.plugin_dir(&hello()))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// No unfinished operation: an empty journal and nothing left in staging.
fn assert_no_partial_files(fx: &Fixture) {
    let layout = deckpilot_plugin_store::Layout::new(&fx.paths.data_dir);
    let staging = layout.staging();
    let left: Vec<_> = std::fs::read_dir(&staging)
        .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "left in {}: {left:?}", staging.display());
    assert_eq!(fx.plugin_store().journal().unwrap(), [], "an open journal");
}

/// `version` is installed, enabled, configured and runs from its key; on
/// disk are exactly the directories of `on_disk`, and nothing is half
/// done.
fn assert_running(d: &mut dyn Driver, fx: &Fixture, version: &str, on_disk: &[&str]) {
    assert_plugin_data_kept(d, fx, version);
    assert_eq!(
        fx.press(0),
        ("Greet Ada".to_owned(), ActionStatus::Succeeded),
        "{version} does not run from its key"
    );
    let expected: BTreeSet<String> = on_disk.iter().map(|v| (*v).to_owned()).collect();
    assert_eq!(version_dirs(fx), expected);
    assert_no_partial_files(fx);
}

fn reason(error: &Value) -> &str {
    error["reason"].as_str().unwrap_or_default()
}

fn message(error: &Value) -> String {
    error["message"].as_str().unwrap_or_default().to_owned()
}

fn update(d: &mut dyn Driver, version: Option<&str>, consent: &[&str]) -> Result<Value, Value> {
    d.invoke(
        "update_plugin",
        json!({"plugin": HELLO, "version": version, "consent": consent}),
    )
}

/// The keys of a `changes` list (`added`, `broadened`).
fn keys(changes: &Value, list: &str) -> Vec<String> {
    changes[list]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap().to_owned())
        .collect()
}

/// The scenario, steps 1 to 12.
#[allow(clippy::too_many_lines)] // one path, step by step
pub fn run(d: &mut dyn Driver, fx: &mut Fixture) {
    let mut report = Report::new(&fx.artifacts, &format!("{}-update-failures", d.name()));
    let baseline_before = fx.stored_profile(fx.baseline);

    // 1. 1.0.0 installed with consent, enabled, configured, on two keys.
    let via = d.start(fx);
    assert!(fx.agent_unlocked(), "the agent is not unlocked");
    d.ok(
        "install_plugin",
        json!({"source": {"type": "registry", "plugin": HELLO, "version": "1.0.0"},
               "consent": [NETWORK, CREDENTIAL]}),
    );
    d.ok(
        "enable_plugin",
        json!({"plugin": HELLO, "consent": [NETWORK, CREDENTIAL]}),
    );
    d.configure();
    fx.seed_storage();
    let greet = d.ok(
        "create_action",
        json!({"profileId": fx.profile, "action": {
            "name": "Greet Ada", "actionType": GREET_TYPE,
            "parameters": {"name": {"type": "text", "value": "Ada"}}}}),
    );
    let weather = d.ok(
        "create_action",
        json!({"profileId": fx.profile, "action": {
            "name": "Berlin weather", "actionType": TEMPERATURE_TYPE,
            "parameters": {"city": {"type": "choice", "value": "berlin"}}}}),
    );
    let profile = super::profile_of(d, fx.profile);
    let mut layout: deckpilot_core::DeviceLayout =
        serde_json::from_value(profile["layouts"][0].clone()).unwrap();
    layout.pages[0].bindings = [&greet, &weather]
        .iter()
        .zip(0u16..)
        .map(|(action, index)| {
            let action: deckpilot_core::ActionInstance =
                serde_json::from_value((*action).clone()).unwrap();
            deckpilot_core::ControlBinding::new(
                deckpilot_device::ControlId::Key { index },
                deckpilot_core::Trigger::Press,
                deckpilot_core::BindingTarget::action(action.id),
            )
        })
        .collect();
    let saved = d.ok(
        "save_layout",
        json!({"profileId": fx.profile, "layout": layout}),
    );
    assert_eq!(saved["agentReload"]["state"], "applied", "{saved}");
    assert_running(d, fx, "1.0.0", &["1.0.0"]);
    let profile_before = super::profile_of(d, fx.profile);
    let stored_before = fx.stored_profile(fx.profile);
    let mut via = via;
    via.extend([Via::AppIpc, Via::AgentIpc, Via::Fixture]);
    report.step(1, "1.0.0 installed, enabled, configured, on keys", &via, "");

    // 2. 1.1.0 is published; the registry's new timestamp does not verify.
    // The update is refused before anything is downloaded.
    fx.local().registry.publish_manifest(&escalating("1.1.0"));
    let (package_path, package) = release(fx, "1.1.0");
    let server = &fx.local().server;
    server.route(
        "/site/v1/timestamp.json.minisig",
        Route::Body(tampered_timestamp_signature(fx)),
    );
    let refused = update(d, None, &[]).expect_err("an unverified registry was used");
    assert_eq!(reason(&refused), "registryUnavailable", "{refused}");
    assert!(
        fx.registry_log()
            .iter()
            .all(|p| !p.ends_with(&package_path)),
        "1.1.0 was downloaded: {:?}",
        fx.registry_log()
    );
    assert_running(d, fx, "1.0.0", &["1.0.0"]);
    report.step(
        2,
        "An update from a registry whose signature fails",
        &[Via::AppIpc, Via::Fixture],
        &message(&refused),
    );

    // 3. The registry verifies, the package is not the one it pins (one
    // byte changed): refused, nothing of 1.1.0 kept.
    let server = &fx.local().server;
    server.clear_routes();
    let mut tampered = package.clone();
    let middle = tampered.len() / 2;
    tampered[middle] ^= 0x40;
    server.route(&package_path, Route::Body(tampered));
    let refused = update(d, None, &[]).expect_err("a tampered package was installed");
    assert_eq!(reason(&refused), "invalidPackage", "{refused}");
    assert!(message(&refused).contains("pinned"), "{refused}");
    assert_running(d, fx, "1.0.0", &["1.0.0"]);
    report.step(
        3,
        "An update whose package hash does not match",
        &[Via::AppIpc, Via::Fixture],
        &message(&refused),
    );

    // 4. The download is cut off midway: refused, as above.
    let server = &fx.local().server;
    server.route(&package_path, Route::CutOff { sent: 4096 });
    let refused = update(d, None, &[]).expect_err("a cut-off package was installed");
    assert_eq!(reason(&refused), "registryUnavailable", "{refused}");
    assert_running(d, fx, "1.0.0", &["1.0.0"]);
    fx.local().server.clear_routes();
    report.step(
        4,
        "An update whose download is cut off",
        &[Via::AppIpc, Via::Fixture],
        &message(&refused),
    );

    // 5. The agent dies in the middle of the update (a test failpoint:
    // the files are unpacked, nothing is committed). After a restart the
    // store's recovery finishes or undoes it; 1.0.0 still runs.
    fx.next_agent_env.push((
        "DECKPILOT_TEST_PLUGIN_CRASH",
        "update:files_unpacked".into(),
    ));
    d.restart(fx);
    let interrupted = update(d, None, &[]);
    assert!(interrupted.is_err(), "{interrupted:?}");
    fx.wait_for_agent(false);
    d.restart(fx);
    let plugin = details(d);
    // The files matched the journal, so recovery finished the update:
    // 1.1.0 is staged and still needs consent; nothing of it is granted.
    assert_eq!(plugin["summary"]["stagedVersion"], "1.1.0", "{plugin}");
    assert_ne!(permission(&plugin, NOTIFICATIONS), "granted", "{plugin}");
    assert_running(d, fx, "1.0.0", &["1.0.0", "1.1.0"]);
    d.ok("decline_plugin_update", json!({"plugin": HELLO}));
    assert_running(d, fx, "1.0.0", &["1.0.0"]);
    report.step(
        5,
        "The agent killed during an update",
        &[Via::Fixture, Via::AppIpc, Via::AgentIpc],
        "recovery staged 1.1.0, which still waits for consent; declined",
    );

    // 6. The update asks for more: staged until consented, 1.0.0 runs on,
    // nothing new is granted.
    let staged = update(d, None, &[]).unwrap();
    let outcome = &staged["outcome"];
    assert_eq!(outcome["type"], "awaitingConsent", "{staged}");
    assert_eq!(outcome["version"], "1.1.0", "{staged}");
    assert_eq!(outcome["changes"]["escalation"], true, "{staged}");
    assert_eq!(keys(&outcome["changes"], "added"), [NOTIFICATIONS]);
    assert_eq!(keys(&outcome["changes"], "broadened"), [NETWORK]);
    let plugin = details(d);
    assert_eq!(plugin["summary"]["stagedVersion"], "1.1.0", "{plugin}");
    assert_eq!(plugin["pendingUpdate"]["version"], "1.1.0", "{plugin}");
    assert_ne!(permission(&plugin, NOTIFICATIONS), "granted", "{plugin}");
    assert_running(d, fx, "1.0.0", &["1.0.0", "1.1.0"]);
    let (_, text) = d.run_plugin_action(TEMPERATURE, json!({"city": "berlin"}));
    assert!(text.contains("21.5"), "{text}");
    // Consent to part of it is not enough.
    let refused = d
        .invoke(
            "approve_plugin_update",
            json!({"plugin": HELLO, "consent": [NOTIFICATIONS]}),
        )
        .expect_err("an update was applied without consent to its broadened grant");
    assert_eq!(reason(&refused), "consentRequired", "{refused}");
    let plugin = details(d);
    assert_ne!(permission(&plugin, NOTIFICATIONS), "granted", "{plugin}");
    assert_running(d, fx, "1.0.0", &["1.0.0", "1.1.0"]);
    report.step(
        6,
        "An update that adds and broadens permissions waits for consent",
        &[Via::AppIpc, Via::AgentIpc, Via::Fixture],
        "1.0.0 keeps running; consent to part of it is refused",
    );

    // 7. Through a restart it still waits; declined, 1.1.0 is gone and
    // nothing was granted.
    d.restart(fx);
    let plugin = details(d);
    assert_eq!(plugin["summary"]["stagedVersion"], "1.1.0", "{plugin}");
    assert_running(d, fx, "1.0.0", &["1.0.0", "1.1.0"]);
    d.ok("decline_plugin_update", json!({"plugin": HELLO}));
    let plugin = details(d);
    assert!(plugin["summary"]["stagedVersion"].is_null(), "{plugin}");
    assert!(plugin["pendingUpdate"].is_null(), "{plugin}");
    assert_eq!(permission(&plugin, NOTIFICATIONS), "absent", "{plugin}");
    assert_running(d, fx, "1.0.0", &["1.0.0"]);
    report.step(
        7,
        "The pending update survives a restart; declined",
        &[Via::AppIpc, Via::AgentIpc, Via::Fixture],
        "",
    );

    // 8. With consent to everything it asks for, the update applies in one
    // step; the data comes along.
    let applied = update(d, None, &[NOTIFICATIONS, NETWORK]).unwrap();
    assert_eq!(applied["outcome"]["type"], "applied", "{applied}");
    assert_eq!(applied["outcome"]["version"], "1.1.0", "{applied}");
    let plugin = details(d);
    assert_eq!(plugin["summary"]["previousVersion"], "1.0.0", "{plugin}");
    assert_eq!(permission(&plugin, NOTIFICATIONS), "granted", "{plugin}");
    assert_running(d, fx, "1.1.0", &["1.0.0", "1.1.0"]);
    report.step(
        8,
        "Update to 1.1.0 with consent",
        &[Via::AppIpc, Via::AgentIpc, Via::Fixture],
        "",
    );

    // 9. No way down but a rollback: an update to an older version and a
    // second install are refused. Once the registry revokes 1.0.0, the
    // rollback to it is refused too.
    let refused = update(d, Some("1.0.0"), &[]).expect_err("a downgrade was applied");
    assert_eq!(reason(&refused), "invalidState", "{refused}");
    assert!(message(&refused).contains("not newer"), "{refused}");
    let refused = d
        .invoke(
            "install_plugin",
            json!({"source": {"type": "registry", "plugin": HELLO, "version": "1.0.0"},
                   "consent": [NETWORK, CREDENTIAL]}),
        )
        .expect_err("an older version was installed over the newer one");
    assert_eq!(reason(&refused), "alreadyInstalled", "{refused}");
    let before_revocation = snapshot(fx, "site-before-revocation");
    revoke(fx, "1.0.0", "leaks the token");
    d.ok("list_available_plugins", json!({"refresh": true}));
    let refused = d
        .invoke("rollback_plugin", json!({"plugin": HELLO}))
        .expect_err("rolled back to a revoked version");
    assert_eq!(reason(&refused), "revoked", "{refused}");
    assert_running(d, fx, "1.1.0", &["1.0.0", "1.1.0"]);
    report.step(
        9,
        "Downgrades refused; no rollback to a revoked version",
        &[Via::AppIpc, Via::Fixture],
        &message(&refused),
    );

    // 10. The registry, rolled back to the site before the revocation
    // (older, validly signed): refused, the stored state stays.
    let seq = d.ok("get_plugin_runtime", json!({}))["registry"]["seq"].clone();
    assert!(seq.as_u64().is_some(), "{seq}");
    fx.local().server.set_root(&before_revocation);
    let refused = d
        .invoke("list_available_plugins", json!({"refresh": true}))
        .expect_err("an older registry was accepted");
    assert_eq!(reason(&refused), "registryUnavailable", "{refused}");
    assert!(message(&refused).contains("rollback"), "{refused}");
    assert_eq!(
        d.ok("get_plugin_runtime", json!({}))["registry"]["seq"],
        seq
    );
    let refused = d
        .invoke("rollback_plugin", json!({"plugin": HELLO}))
        .expect_err("the old registry's verdicts were used");
    assert_eq!(reason(&refused), "revoked", "{refused}");
    fx.local()
        .server
        .set_root(&fx.local().registry.work.clone());
    assert_running(d, fx, "1.1.0", &["1.0.0", "1.1.0"]);
    report.step(
        10,
        "A rolled-back registry is refused",
        &[Via::AppIpc, Via::Fixture],
        "",
    );

    // 11. 1.2.0's module is refused by the host: the update must not
    // leave the plugin broken. 1.1.0 stays and runs.
    fx.local().registry.publish_manifest(&broken("1.2.0"));
    let result = update(d, None, &[]);
    let plugin = details(d);
    let after = run_action(d, fx.profile, &greet);
    assert!(
        result.is_err(),
        "the update to a module the host refuses was applied: {result:?}; then {after:?}; \
         {plugin}"
    );
    let refused = result.unwrap_err();
    assert_eq!(reason(&refused), "broken", "{refused}");
    assert!(
        message(&refused).contains("refused its module"),
        "{refused}"
    );
    assert_eq!(after.status, ActionStatus::Succeeded, "{after:?}");
    let plugin = details(d);
    assert!(plugin["summary"]["stagedVersion"].is_null(), "{plugin}");
    assert_eq!(plugin["summary"]["previousVersion"], "1.0.0", "{plugin}");
    assert_running(d, fx, "1.1.0", &["1.0.0", "1.1.0"]);
    report.step(
        11,
        "An update whose module the host refuses",
        &[Via::AppIpc, Via::AgentIpc, Via::Fixture],
        &message(&refused),
    );

    // 12. The registry revokes the installed 1.1.0: quarantined, nothing
    // of it runs, it cannot be enabled. Uninstalling it removes its data;
    // the configuration is intact.
    revoke(fx, "1.1.0", "exfiltrates data");
    d.ok("list_available_plugins", json!({"refresh": true}));
    let plugin = details(d);
    assert_eq!(plugin["summary"]["state"], "quarantined", "{plugin}");
    assert_eq!(plugin["summary"]["version"], "1.1.0", "{plugin}");
    let refused = run_action(d, fx.profile, &greet);
    assert_eq!(refused.status, ActionStatus::Failed, "{refused:?}");
    assert_eq!(fx.press(0), ("Greet Ada".to_owned(), ActionStatus::Failed));
    let before = fx.fetched().len();
    assert_eq!(
        fx.press(1),
        ("Berlin weather".to_owned(), ActionStatus::Failed)
    );
    assert_eq!(fx.fetched().len(), before, "a quarantined plugin fetched");
    let refused = d
        .invoke(
            "invoke_plugin_action",
            json!({"plugin": HELLO, "action": GREET, "params": {"name": "Ada"}}),
        )
        .expect_err("a quarantined plugin ran");
    assert_eq!(reason(&refused), "notEnabled", "{refused}");
    assert!(message(&refused).contains("revoked"), "{refused}");
    let refused = d
        .invoke("enable_plugin", json!({"plugin": HELLO, "consent": []}))
        .expect_err("a quarantined plugin was enabled");
    assert_eq!(reason(&refused), "revoked", "{refused}");
    d.ok("uninstall_plugin", json!({"plugin": HELLO}));
    let installed = d.ok("list_plugins", json!({}));
    assert!(
        installed
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["id"] != HELLO),
        "{installed}"
    );
    let (storage, credentials, settings) = fx.plugin_data();
    assert!(storage.is_empty(), "{storage:?}");
    assert!(credentials.is_empty(), "{credentials:?}");
    assert!(settings.is_empty(), "{settings:?}");
    assert!(version_dirs(fx).is_empty(), "{:?}", version_dirs(fx));
    assert_no_partial_files(fx);
    assert_eq!(super::profile_of(d, fx.profile), profile_before);
    assert_eq!(fx.stored_profile(fx.profile), stored_before);
    assert_eq!(fx.stored_profile(fx.baseline), baseline_before);
    fx.assert_encrypted();
    assert!(
        fx.fetched().iter().all(|r| r.starts_with(BERLIN)),
        "{:?}",
        fx.fetched()
    );
    report.step(
        12,
        "A revoked installed version is quarantined; uninstalled",
        &[Via::AppIpc, Via::AgentIpc, Via::Fixture],
        "storage, credentials and settings gone; profiles and the encrypted config intact",
    );
}
