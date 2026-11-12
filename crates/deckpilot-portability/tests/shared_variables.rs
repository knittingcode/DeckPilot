//! Shared variables in exports and imports (decision D5): definitions and
//! the values of the exported profiles and their projects travel with the
//! profiles, global values only with the application settings, and
//! sensitive values never. A `secretRef` value is a credential name; the
//! credential's secret is never exported.

use deckpilot_core::variables::{
    ProjectSource, Resolution, ResolutionContext, ScopeKey, VariableDefinition, VariableKey,
    VariableScope, VariableType, VariableValue,
};
use deckpilot_core::{CredentialId, ModuleId, Os, Profile, Project, ProjectKey, Secret};
use deckpilot_device::ModelCatalog;
use deckpilot_portability::import::{ConflictResolution, validate};
use deckpilot_portability::payload::{self, ExportOptions, ExportedValue, Payload};
use deckpilot_portability::{ImportOptions, ImportSummary, PAYLOAD_VERSION, plan, preview};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore, SqliteStore, VariableOrigin};

const WITH_SETTINGS: ExportOptions = ExportOptions {
    module_settings: false,
    app_settings: true,
};
const PROFILES_ONLY: ExportOptions = ExportOptions {
    module_settings: false,
    app_settings: false,
};

const SENSITIVE_VALUE: &str = "SENSITIVE-VALUE-marker";
const SENSITIVE_DEFAULT: &str = "SENSITIVE-DEFAULT-marker";
const SECRET: &str = "SECRET-credential-9d2f";
const GLOBAL_VALUE: &str = "GLOBAL-VALUE-marker";
const OTHER_PROFILE_VALUE: &str = "OTHER-PROFILE-marker";

const ALL_SCOPES: &[VariableScope] = &[
    VariableScope::Global,
    VariableScope::Profile,
    VariableScope::Project,
];

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

fn text(value: &str) -> VariableValue {
    VariableValue::String(value.to_owned())
}

fn profile(name: &str, keys: &[&str]) -> Profile {
    let mut profile = Profile::new(name);
    for project in keys {
        profile
            .projects
            .push(Project::new(ProjectKey::new(*project).unwrap(), *project));
    }
    profile
}

fn modules() -> deckpilot_core::ModuleRegistry {
    deckpilot_modules::builtin(Os::Linux)
}

/// Work (SHOP, CMR) and Home (BLOG) with variables at every scope.
fn populated() -> (SqliteStore, Profile, Profile) {
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile("Work", &["SHOP", "CMR"]);
    let home = profile("Home", &["BLOG"]);
    store.save_profile(&work).unwrap();
    store.save_profile(&home).unwrap();
    let set = |name: &str, scope: ScopeKey, value: VariableValue| {
        store
            .set_shared_variable_value(&key(name), &scope, &value)
            .unwrap();
    };

    let mut root = VariableDefinition::new(key("paths.root"), VariableType::String, ALL_SCOPES);
    root.label = "Source root".into();
    store.save_shared_variable(&root).unwrap();
    set("paths.root", ScopeKey::Global, text(GLOBAL_VALUE));
    set(
        "paths.root",
        ScopeKey::Profile { profile: work.id },
        text("/work"),
    );
    set(
        "paths.root",
        ScopeKey::Project {
            profile: work.id,
            project: work.projects[0].id,
        },
        text("/work/shop"),
    );
    set(
        "paths.root",
        ScopeKey::Profile { profile: home.id },
        text(OTHER_PROFILE_VALUE),
    );

    let favorite = VariableDefinition::new(
        key("projects.favorite"),
        VariableType::ProjectId,
        &[VariableScope::Profile],
    );
    store.save_shared_variable(&favorite).unwrap();
    set(
        "projects.favorite",
        ScopeKey::Profile { profile: work.id },
        VariableValue::ProjectId(work.projects[1].id),
    );

    let mut password =
        VariableDefinition::new(key("db.password"), VariableType::String, ALL_SCOPES);
    password.sensitive = true;
    password.default = Some(text(SENSITIVE_DEFAULT));
    store.save_shared_variable(&password).unwrap();
    set("db.password", ScopeKey::Global, text(SENSITIVE_VALUE));
    set(
        "db.password",
        ScopeKey::Profile { profile: work.id },
        text(SENSITIVE_VALUE),
    );

    let token = CredentialId::new("api.token").unwrap();
    store
        .set_credential(&token, "", Some(&Secret::new(SECRET)))
        .unwrap();
    let reference = VariableDefinition::new(
        key("api.token"),
        VariableType::SecretRef,
        &[VariableScope::Profile],
    );
    store.save_shared_variable(&reference).unwrap();
    set(
        "api.token",
        ScopeKey::Profile { profile: work.id },
        VariableValue::SecretRef(token),
    );
    (store, work, home)
}

fn export(
    store: &SqliteStore,
    profiles: Option<&[deckpilot_core::ProfileId]>,
    options: ExportOptions,
) -> Payload {
    let payload = payload::export(store, &modules(), profiles, options, 1, "0.6.0").unwrap();
    let bytes = payload::seal(&payload, &Password::new("pw"), KdfParams::for_tests()).unwrap();
    payload::open(&bytes, &Password::new("pw")).unwrap()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn apply(store: &SqliteStore, payload: &Payload, options: &ImportOptions) -> ImportSummary {
    validate(payload, &ModelCatalog::builtin()).unwrap();
    let (changes, summary) = plan(payload, options, store, &modules()).unwrap();
    store.apply_changes(&changes).unwrap();
    summary
}

fn values_of(store: &SqliteStore, name: &str) -> Vec<(ScopeKey, VariableValue)> {
    store
        .shared_variable_values(&key(name))
        .unwrap()
        .into_iter()
        .map(|v| (v.scope, v.value))
        .collect()
}

#[test]
fn sensitive_values_secrets_and_other_profiles_never_leave() {
    let (store, work, _) = populated();
    for options in [WITH_SETTINGS, PROFILES_ONLY] {
        let payload = export(&store, Some(&[work.id]), options);
        assert_eq!(payload.version, PAYLOAD_VERSION);
        let plain = serde_json::to_vec(&payload).unwrap();
        let sealed = payload::seal(&payload, &Password::new("pw"), KdfParams::for_tests()).unwrap();
        for marker in [
            SENSITIVE_VALUE,
            SENSITIVE_DEFAULT,
            SECRET,
            OTHER_PROFILE_VALUE,
        ] {
            assert!(!contains(&plain, marker), "{marker} in the payload");
            assert!(!contains(&sealed, marker), "{marker} in the archive");
        }
        // The sensitive definition travels, without its default and values.
        let password = payload
            .variables
            .definitions
            .iter()
            .find(|d| d.key == key("db.password"))
            .unwrap();
        assert!(password.sensitive);
        assert_eq!(password.default, None);
        assert!(
            !payload
                .variables
                .values
                .iter()
                .any(|v| v.key == key("db.password"))
        );
        // The secret reference is only the credential's name.
        assert!(
            payload
                .variables
                .values
                .iter()
                .any(|v| v.value
                    == VariableValue::SecretRef(CredentialId::new("api.token").unwrap()))
        );
        // Global values only with the application settings.
        assert_eq!(
            contains(&plain, GLOBAL_VALUE),
            options.app_settings,
            "{options:?}"
        );
    }
}

#[test]
fn an_import_creates_the_variables_and_values_of_the_profile() {
    let (source, work, _) = populated();
    let payload = export(&source, Some(&[work.id]), WITH_SETTINGS);

    let target = SqliteStore::open_in_memory().unwrap();
    let shown = preview(&payload, &target, &modules(), Vec::new()).unwrap();
    assert_eq!(shown.variables.new.len(), 4);
    assert_eq!(shown.variables.global_values, 1);
    let summary = apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            app_settings: true,
            ..ImportOptions::default()
        },
    );
    assert_eq!(summary.variables.len(), 4);
    assert_eq!(summary.variable_values, 5);
    assert_eq!(summary.skipped_values, Vec::<String>::new());
    let stored = target.shared_variables().unwrap();
    assert!(stored.iter().all(|s| s.origin == VariableOrigin::Import));
    assert_eq!(values_of(&target, "db.password"), Vec::new());
    assert_eq!(
        values_of(&target, "paths.root"),
        values_of(&source, "paths.root")
            .into_iter()
            .filter(|(_, v)| *v != text(OTHER_PROFILE_VALUE))
            .collect::<Vec<_>>()
    );
    // The imported values resolve like at home.
    let variables = target.shared_variables_for(work.id).unwrap();
    let at_shop = ResolutionContext::preview(work.id)
        .with_project(Some(work.projects[0].id), ProjectSource::Explicit);
    assert!(matches!(
        variables.resolve(&key("paths.root"), &at_shop, &[]),
        Resolution::Value { value, .. } if value == text("/work/shop")
    ));
    // The credential is not: the reference names one that does not exist.
    assert_eq!(target.credentials().unwrap(), Vec::new());

    // Without the application settings the global value stays out.
    let target = SqliteStore::open_in_memory().unwrap();
    apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            ..ImportOptions::default()
        },
    );
    assert!(
        !values_of(&target, "paths.root")
            .iter()
            .any(|(scope, _)| *scope == ScopeKey::Global)
    );
}

#[test]
fn values_follow_a_profile_imported_as_a_copy() {
    let (store, work, _) = populated();
    let payload = export(&store, Some(&[work.id]), PROFILES_ONLY);
    let summary = apply(
        &store,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            conflicts: [(work.id, ConflictResolution::KeepBoth)].into(),
            ..ImportOptions::default()
        },
    );
    assert_eq!(summary.renamed.len(), 1);
    let copy = store.load_profile(summary.imported[0].0).unwrap().unwrap();
    assert_ne!(copy.id, work.id);
    let values = values_of(&store, "paths.root");
    assert!(values.contains(&(ScopeKey::Profile { profile: copy.id }, text("/work"))));
    assert!(values.contains(&(
        ScopeKey::Project {
            profile: copy.id,
            project: copy.projects[0].id
        },
        text("/work/shop")
    )));
    // The favorite of the copy is the copy's own CMR, not the original's.
    assert!(values_of(&store, "projects.favorite").contains(&(
        ScopeKey::Profile { profile: copy.id },
        VariableValue::ProjectId(copy.projects[1].id)
    )));
    // The original's values are untouched, and no definition was replaced.
    assert!(values.contains(&(ScopeKey::Profile { profile: work.id }, text("/work"))));
    assert_eq!(summary.variables, Vec::<String>::new());
    assert!(
        store
            .shared_variables()
            .unwrap()
            .iter()
            .all(|s| s.origin == VariableOrigin::User)
    );
}

#[test]
fn a_local_definition_wins_and_values_that_do_not_fit_are_reported() {
    let (source, work, _) = populated();
    let payload = export(&source, Some(&[work.id]), PROFILES_ONLY);
    let target = SqliteStore::open_in_memory().unwrap();
    let mut root = VariableDefinition::new(
        key("paths.root"),
        VariableType::Integer,
        &[VariableScope::Profile],
    );
    root.label = "mine".into();
    target.save_shared_variable(&root).unwrap();
    let shown = preview(&payload, &target, &modules(), Vec::new()).unwrap();
    assert_eq!(shown.variables.incompatible, [key("paths.root")]);
    let summary = apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            ..ImportOptions::default()
        },
    );
    assert_eq!(
        summary.skipped_values.len(),
        2,
        "{:?}",
        summary.skipped_values
    );
    assert_eq!(
        target
            .shared_variable(&key("paths.root"))
            .unwrap()
            .unwrap()
            .definition,
        root
    );
    assert_eq!(values_of(&target, "paths.root"), Vec::new());
}

#[test]
fn crafted_variables_are_refused_before_anything_is_written() {
    let (store, work, home) = populated();
    let good = export(&store, Some(&[work.id]), WITH_SETTINGS);
    let catalog = ModelCatalog::builtin();
    let mut cases = Vec::new();
    // A value of a profile that is not in the export.
    let mut payload = good.clone();
    payload.variables.values.push(ExportedValue {
        key: key("paths.root"),
        scope: ScopeKey::Profile { profile: home.id },
        value: text("x"),
    });
    cases.push(payload);
    // A project of another profile.
    let mut payload = good.clone();
    payload.variables.values.push(ExportedValue {
        key: key("paths.root"),
        scope: ScopeKey::Project {
            profile: work.id,
            project: home.projects[0].id,
        },
        value: text("x"),
    });
    cases.push(payload);
    // A run's value, a wrong type, a module's definition, a duplicate.
    let mut payload = good.clone();
    payload.variables.values.push(ExportedValue {
        key: key("paths.root"),
        scope: ScopeKey::Execution {
            execution: deckpilot_core::ExecutionId::new(),
        },
        value: text("x"),
    });
    cases.push(payload);
    let mut payload = good.clone();
    payload.variables.values[0].value = VariableValue::Boolean(true);
    cases.push(payload);
    let mut payload = good.clone();
    payload.variables.definitions[0].module = Some(ModuleId::new("projects").unwrap());
    cases.push(payload);
    let mut payload = good.clone();
    let first = payload.variables.definitions[0].clone();
    payload.variables.definitions.push(first);
    cases.push(payload);
    for payload in cases {
        assert!(validate(&payload, &catalog).is_err());
    }
    // Reserved namespaces cannot even be read.
    let mut json = serde_json::to_value(&good).unwrap();
    json["variables"]["definitions"][0]["key"] = "system.project".into();
    assert!(serde_json::from_value::<Payload>(json).is_err());
    validate(&good, &catalog).unwrap();
}

#[test]
fn exports_of_earlier_builds_still_import() {
    let (store, work, _) = populated();
    let mut payload = export(&store, Some(&[work.id]), PROFILES_ONLY);
    payload.version = 1;
    payload.variables = payload::VariablesExport::default();
    let mut json = serde_json::to_value(&payload).unwrap();
    json.as_object_mut().unwrap().remove("variables");
    let payload: Payload = serde_json::from_value(json).unwrap();
    let target = SqliteStore::open_in_memory().unwrap();
    apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            ..ImportOptions::default()
        },
    );
    assert_eq!(target.list_profiles().unwrap().len(), 1);
    assert_eq!(target.shared_variables().unwrap(), Vec::new());
}
