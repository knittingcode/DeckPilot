//! What must never leave this computer in an export: stored credentials
//! (secrets and their descriptions), machine-specific and sensitive module
//! settings, machine resource values, security settings and runtime state;
//! and data of profiles that were not selected.
//!
//! Every value that must not travel carries a marker, and the test looks
//! for the markers in the decrypted payload, in the encrypted archive and in
//! a store an export was imported into.

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, CredentialId,
    CredentialSource, MachineConfig, MachineResourceBinding, ModuleId, ModuleSettings, OnFailure,
    Os, ParameterValue, Profile, Project, ProjectKey, ResourceKey, ResourceKind, Secret, Workflow,
};
use deckpilot_device::ModelCatalog;
use deckpilot_portability::import::validate;
use deckpilot_portability::payload::{self, ExportOptions};
use deckpilot_portability::{ImportOptions, plan};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore, SqliteStore};

const ALL: ExportOptions = ExportOptions {
    module_settings: true,
    app_settings: true,
};

/// Values that must never appear in an export.
const NEVER_EXPORTED: &[&str] = &[
    // Credential secrets and descriptions.
    "SECRET-ghp-7f3a9c",
    "SECRET-jira-51e0d2",
    "CREDENTIAL-DESCRIPTION-marker",
    // Machine-specific module settings.
    "ghe-internal-marker.example",
    "private-host-marker.example",
    // Machine resource values.
    "/home/MACHINE-PATH-marker/src/shop",
    // Security settings and runtime state.
    "idle15Minutes",
    "security.confirmedVersion",
];

fn profile(name: &str, project_key: &str, resource: &str) -> Profile {
    let mut profile = Profile::new(name);
    let mut project = Project::new(
        ProjectKey::new(project_key).unwrap(),
        format!("{name} project"),
    );
    project
        .resources
        .insert("repository".into(), ResourceKey::new(resource).unwrap());
    profile.projects.push(project);
    let action = ActionInstance {
        id: ActionInstanceId::new(),
        name: format!("{name} action"),
        action_type: ActionTypeId::new("projects.repository").unwrap(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    };
    profile
        .workflows
        .push(Workflow::new(format!("{name} workflow")).step(action.id, OnFailure::Stop));
    profile.actions.push(action);
    profile
}

/// A store holding everything that must stay here.
fn populated() -> (SqliteStore, Profile, Profile) {
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile("WorkIsolationMarker", "WORKX", "work.repo");
    let home = profile("Home", "HOME", "home.repo");
    store.save_profile(&work).unwrap();
    store.save_profile(&home).unwrap();
    store.set_active_profile(Some(work.id)).unwrap();
    store
        .set_active_project(work.id, Some(work.projects[0].id))
        .unwrap();

    store
        .set_credential(
            &CredentialId::new("github.token").unwrap(),
            "CREDENTIAL-DESCRIPTION-marker",
            Some(&Secret::new("SECRET-ghp-7f3a9c")),
        )
        .unwrap();
    store
        .set_credential(
            &CredentialId::new("jira.token").unwrap(),
            "",
            Some(&Secret::new("SECRET-jira-51e0d2")),
        )
        .unwrap();

    store
        .set_module_state(
            &ModuleId::new("github").unwrap(),
            &ModuleSettings {
                enabled: true,
                config: ActionParameters::new()
                    .with(
                        "apiUrl",
                        ParameterValue::Text("https://ghe-internal-marker.example/api".into()),
                    )
                    .with("credential", ParameterValue::Text("github.token".into()))
                    .with("timeoutSeconds", ParameterValue::Integer(17))
                    .with("allowPrivateHost", ParameterValue::Boolean(true)),
            },
        )
        .unwrap();
    store
        .set_module_state(
            &ModuleId::new("http").unwrap(),
            &ModuleSettings {
                enabled: true,
                config: ActionParameters::new()
                    .with("allowPrivateNetworks", ParameterValue::Boolean(true))
                    .with(
                        "privateHosts",
                        ParameterValue::Text("private-host-marker.example".into()),
                    ),
            },
        )
        .unwrap();

    let mut machine = MachineConfig::new(Os::Linux);
    machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new("work.repo").unwrap(),
        kind: ResourceKind::Directory,
        value: "/home/MACHINE-PATH-marker/src/shop".into(),
    });
    store.save_machine_config(&machine).unwrap();
    store
        .set_setting("security.autoLock", "idle15Minutes")
        .unwrap();
    store
        .set_setting("security.confirmedVersion", "0.6.0")
        .unwrap();
    store.set_device_brightness(42).unwrap();
    (store, work, home)
}

fn modules(store: &SqliteStore) -> deckpilot_core::ModuleRegistry {
    let (registry, problems) = deckpilot_modules::configured(&store.module_states().unwrap());
    assert_eq!(problems, Vec::<String>::new());
    registry
}

fn seal(payload: &payload::Payload) -> Vec<u8> {
    payload::seal(
        payload,
        &Password::new(export_password()),
        KdfParams::for_tests(),
    )
    .unwrap()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// The export password of this test run: generated, so that it is never a
/// constant that could pass for (or be reused as) a real credential.
fn export_password() -> &'static str {
    static PASSWORD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PASSWORD.get_or_init(generated_password)
}

/// A new random password, different from every other one.
fn generated_password() -> String {
    let mut bytes = [0u8; 16];
    deckpilot_storage::crypto::random_bytes(&mut bytes).unwrap();
    format!("test-export-{:032x}", u128::from_le_bytes(bytes))
}

#[test]
fn an_export_never_contains_secrets_or_values_of_this_computer() {
    let (store, _, _) = populated();
    let payload = payload::export(&store, &modules(&store), None, ALL, 1, "0.6.0").unwrap();
    let text = serde_json::to_string(&payload).unwrap();
    for marker in NEVER_EXPORTED {
        assert!(!text.contains(marker), "the export contains {marker}");
    }
    // Not vacuous: portable settings and credential *names* do travel.
    let github = &payload.modules["github"];
    assert_eq!(github.settings.text("credential"), Some("github.token"));
    assert_eq!(github.settings.integer("timeoutSeconds"), Some(17));
    assert_eq!(github.settings.text("apiUrl"), None);
    assert_eq!(github.settings.boolean("allowPrivateHost"), None);
    assert_eq!(payload.modules["http"].settings.text("privateHosts"), None);
    assert!(text.contains("WorkIsolationMarker"));

    // The archive is encrypted: not even the profile names are readable,
    // neither in the plaintext header nor in the decoded ciphertext. The
    // needles are structured text of the payload (quotes, dots), so they
    // cannot match base64 text or random bytes by chance.
    let sealed = seal(&payload);
    let envelope: serde_json::Value = serde_json::from_slice(&sealed).unwrap();
    let header = envelope["header"].as_str().unwrap().as_bytes();
    let ciphertext = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(envelope["ciphertext"].as_str().unwrap())
            .unwrap()
    };
    let visible = [
        r#""name":"WorkIsolationMarker""#,
        r#""key":"WORKX""#,
        r#""credential":"#,
        "work.repo",
    ];
    for needle in visible {
        assert!(text.contains(needle), "{needle} is not in the payload");
        for (part, bytes) in [
            ("file", &sealed[..]),
            ("header", header),
            ("ciphertext", &ciphertext),
        ] {
            assert!(!contains(bytes, needle), "the {part} shows {needle}");
        }
    }
    for marker in NEVER_EXPORTED {
        assert!(!contains(&sealed, marker), "the archive contains {marker}");
    }
}

#[test]
fn importing_never_brings_or_replaces_credentials() {
    let (source, _, _) = populated();
    let payload = payload::export(&source, &modules(&source), None, ALL, 1, "0.6.0").unwrap();
    // A crafted export that carries credentials and sensitive settings
    // anyway: unknown fields are ignored and non-portable settings dropped.
    let mut value = serde_json::to_value(&payload).unwrap();
    value["credentials"] = serde_json::json!([
        {"id": "github.token", "secret": "SECRET-crafted-0a1b"},
    ]);
    value["modules"]["github"]["settings"]["apiUrl"] = serde_json::to_value(ParameterValue::Text(
        "https://ghe-internal-marker.example/api".into(),
    ))
    .unwrap();
    let crafted: payload::Payload = serde_json::from_value(value).unwrap();
    let sealed = seal(&crafted);
    let opened = payload::open(&sealed, &Password::new(export_password())).unwrap();
    validate(&opened, &ModelCatalog::builtin()).unwrap();

    let target = SqliteStore::open_in_memory().unwrap();
    let local = CredentialId::new("github.token").unwrap();
    target
        .set_credential(&local, "here", Some(&Secret::new("local-secret")))
        .unwrap();
    let options = ImportOptions {
        profiles: opened.profiles.iter().map(|p| p.id).collect(),
        module_settings: true,
        app_settings: true,
        ..ImportOptions::default()
    };
    let (changes, _) = plan(&opened, &options, &target, &modules(&target)).unwrap();
    target.apply_changes(&changes).unwrap();

    let credentials = target.load_credentials().unwrap();
    assert_eq!(
        credentials.secret(&local),
        Some(Secret::new("local-secret"))
    );
    assert_eq!(target.credentials().unwrap().len(), 1);
    let github = &target.module_states().unwrap()[&ModuleId::new("github").unwrap()];
    // The machine-specific URL was dropped; this computer keeps its own.
    assert_eq!(github.config.text("apiUrl"), Some("https://api.github.com"));
    assert_eq!(github.config.text("credential"), Some("github.token"));
    // Nothing of the source computer arrived.
    assert_eq!(target.load_machine_config().unwrap(), None);
    assert_eq!(target.setting("security.autoLock").unwrap(), None);
    assert_eq!(target.device_brightness().unwrap(), None);
    assert_eq!(target.active_profile().unwrap(), None);
}

#[test]
fn an_export_of_one_profile_contains_nothing_of_the_others() {
    let (store, work, home) = populated();
    let payload =
        payload::export(&store, &modules(&store), Some(&[home.id]), ALL, 1, "0.6.0").unwrap();
    assert_eq!(payload.profiles.len(), 1);
    assert_eq!(payload.profiles[0].id, home.id);
    let text = serde_json::to_string(&payload).unwrap();
    let work_values = [
        work.id.to_string(),
        work.name.clone(),
        work.projects[0].id.to_string(),
        "WORKX".into(),
        "work.repo".into(),
        work.actions[0].id.to_string(),
        work.actions[0].name.clone(),
        work.workflows[0].id.to_string(),
        work.workflows[0].name.clone(),
    ];
    for value in &work_values {
        assert!(
            !text.contains(value.as_str()),
            "the export contains {value}"
        );
    }
    // Only the selected profile's resources are listed.
    let keys: Vec<String> = payload.resources.keys().map(ToString::to_string).collect();
    assert_eq!(keys, ["home.repo"]);
}

#[test]
fn imported_profiles_stay_apart_from_local_ones() {
    let (store, work, home) = populated();
    let payload = payload::export(&store, &modules(&store), None, ALL, 1, "0.6.0").unwrap();
    // Importing the configuration into itself keeps both copies apart:
    // the imports get new identifiers for the profile and its projects.
    let options = ImportOptions {
        profiles: vec![work.id, home.id],
        ..ImportOptions::default()
    };
    let (changes, summary) = plan(&payload, &options, &store, &modules(&store)).unwrap();
    store.apply_changes(&changes).unwrap();
    assert_eq!(summary.renamed.len(), 2);
    let imported: Vec<Profile> = summary
        .imported
        .iter()
        .map(|(id, _)| store.load_profile(*id).unwrap().unwrap())
        .collect();
    for copy in &imported {
        assert_ne!(copy.id, work.id);
        assert_ne!(copy.id, home.id);
        for project in &copy.projects {
            assert!(
                !work
                    .projects
                    .iter()
                    .chain(&home.projects)
                    .any(|p| p.id == project.id),
                "a project is in two profiles"
            );
            // A local profile cannot select the imported project, nor
            // the other way round.
            assert!(store.set_active_project(work.id, Some(project.id)).is_err());
        }
        assert!(
            store
                .set_active_project(copy.id, Some(work.projects[0].id))
                .is_err()
        );
        // The active project of the original is not carried over.
        assert_eq!(store.active_project(copy.id).unwrap(), None);
    }
    assert_eq!(
        store.active_project(work.id).unwrap(),
        Some(work.projects[0].id)
    );
    assert_eq!(store.active_profile().unwrap(), Some(work.id));
    assert_eq!(store.load_profile(work.id).unwrap().unwrap(), work);
}
