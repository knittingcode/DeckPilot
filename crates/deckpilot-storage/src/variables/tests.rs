use deckpilot_core::variables::{
    DefinitionIssue, MissingReason, ProjectSource, Resolution, ResolutionContext, ScopeKey,
    ValueSource, VariableDefinition, VariableKey, VariableScope, VariableType, VariableValue,
};
use deckpilot_core::{CredentialId, ExecutionId, ModuleId, Profile, Project, ProjectKey, Secret};

use crate::provenance::{Provenance, ProvenanceEntity};
use crate::{ConfigStore, SqliteStore, StorageError};

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

fn definition(
    text: &str,
    value_type: VariableType,
    scopes: &[VariableScope],
) -> VariableDefinition {
    VariableDefinition::new(key(text), value_type, scopes)
}

const ALL: &[VariableScope] = &[
    VariableScope::Global,
    VariableScope::Profile,
    VariableScope::Project,
    VariableScope::Execution,
];

fn profile_with_projects(name: &str, keys: &[&str]) -> Profile {
    let mut profile = Profile::new(name);
    for project in keys {
        profile
            .projects
            .push(Project::new(ProjectKey::new(*project).unwrap(), *project));
    }
    profile
}

fn text(value: &str) -> VariableValue {
    VariableValue::String(value.to_owned())
}

#[test]
fn definitions_round_trip_and_keep_their_origin() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut favorite = definition("projects.favorite", VariableType::ProjectId, ALL);
    favorite.label = "Favorite project".into();
    favorite.description = "Opened by the favorite key".into();
    store
        .save_shared_variable_from(&favorite, crate::VariableOrigin::Seed)
        .unwrap();
    let mut root = definition("paths.root", VariableType::String, &[VariableScope::Global]);
    root.default = Some(text("/src"));
    root.sensitive = true;
    store.save_shared_variable(&root).unwrap();

    let stored = store.shared_variables().unwrap();
    let keys: Vec<_> = stored.iter().map(|s| s.definition.key.as_str()).collect();
    assert_eq!(keys, ["paths.root", "projects.favorite"]);
    assert_eq!(stored[0].definition, root);
    assert_eq!(stored[0].origin, crate::VariableOrigin::User);
    assert_eq!(stored[1].definition, favorite);
    assert_eq!(stored[1].origin, crate::VariableOrigin::Seed);

    // Changing a definition keeps its origin and creation time.
    favorite.label = "Favorite".into();
    store.save_shared_variable(&favorite).unwrap();
    let again = store.shared_variable(&favorite.key).unwrap().unwrap();
    assert_eq!(again.definition.label, "Favorite");
    assert_eq!(again.origin, crate::VariableOrigin::Seed);
    assert_eq!(again.created_at, stored[1].created_at);
}

#[test]
fn reserved_namespaces_never_reach_the_store() {
    for reserved in ["steps.a", "system.project", "workflow.id"] {
        assert!(VariableKey::new(reserved).is_err(), "{reserved}");
        // Also not through deserialization (an import, a stored row).
        let json = serde_json::json!({
            "key": reserved, "valueType": {"type": "string"}, "scopes": ["global"]
        });
        assert!(serde_json::from_value::<VariableDefinition>(json).is_err());
    }
    // A raw row with a reserved key (a damaged or crafted database) is
    // ignored instead of becoming a variable.
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .connection_for_tests()
        .execute(
            "INSERT INTO shared_variables (key, value_type, scopes, created_at, updated_at)
             VALUES ('steps.x', '{\"type\":\"string\"}', '[\"global\"]', 0, 0)",
            [],
        )
        .unwrap();
    assert_eq!(store.shared_variables().unwrap(), Vec::new());
    // `shared` is not reserved: it is never part of a key.
    store
        .save_shared_variable(&definition("shared.x", VariableType::String, ALL))
        .unwrap();
}

#[test]
fn invalid_definitions_are_refused() {
    let store = SqliteStore::open_in_memory().unwrap();
    let none = definition("a.b", VariableType::String, &[]);
    assert!(matches!(
        store.save_shared_variable(&none),
        Err(StorageError::Variable(DefinitionIssue::NoScopes(_)))
    ));
    let mut wrong_default = definition("a.b", VariableType::Integer, ALL);
    wrong_default.default = Some(text("x"));
    assert!(matches!(
        store.save_shared_variable(&wrong_default),
        Err(StorageError::Variable(DefinitionIssue::WrongType { .. }))
    ));
    // A user cannot claim to be a module.
    let mut claimed = definition("a.b", VariableType::String, ALL);
    claimed.module = Some(ModuleId::new("projects").unwrap());
    assert!(matches!(
        store.save_shared_variable(&claimed),
        Err(StorageError::Invalid(_))
    ));
    assert_eq!(store.shared_variables().unwrap(), Vec::new());
}

#[test]
fn user_variables_never_take_keys_a_module_declares() {
    let store = SqliteStore::open_in_memory().unwrap();
    let projects = ModuleId::new("projects").unwrap();
    let declared = definition("projects.current_root", VariableType::String, ALL);
    assert_eq!(
        store
            .declare_module_variables(&projects, std::slice::from_ref(&declared))
            .unwrap(),
        Vec::new()
    );
    let stored = store.shared_variable(&declared.key).unwrap().unwrap();
    assert_eq!(stored.definition.module, Some(projects.clone()));

    // The declared key is the module's…
    assert!(matches!(
        store.save_shared_variable(&declared),
        Err(StorageError::VariableKeyTaken { .. })
    ));
    assert!(matches!(
        store.delete_shared_variable(&declared.key),
        Err(StorageError::VariableKeyTaken { .. })
    ));
    // …but the module's group is open to the user (D4).
    store
        .save_shared_variable(&definition(
            "projects.favorite",
            VariableType::ProjectId,
            ALL,
        ))
        .unwrap();

    // A module declaring a key that a user variable or another module
    // already holds gets a conflict; the holder is left alone.
    let mut users = definition("projects.favorite", VariableType::String, ALL);
    users.label = "module version".into();
    let other = ModuleId::new("general").unwrap();
    let conflicts = store
        .declare_module_variables(&other, &[users, declared.clone()])
        .unwrap();
    assert_eq!(conflicts.len(), 2);
    assert_eq!(conflicts[0].holder, None);
    assert_eq!(conflicts[1].holder, Some(projects.clone()));
    let favorite = store
        .shared_variable(&key("projects.favorite"))
        .unwrap()
        .unwrap();
    assert_eq!(favorite.definition.module, None);
    assert_eq!(favorite.definition.value_type, VariableType::ProjectId);
    // Declaring again is idempotent for the module itself.
    assert_eq!(
        store
            .declare_module_variables(&projects, &[declared])
            .unwrap(),
        Vec::new()
    );
}

#[test]
fn values_are_checked_against_the_definition_and_their_scope() {
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile_with_projects("Work", &["SHOP", "CMR"]);
    let home = profile_with_projects("Home", &["BLOG"]);
    store.save_profile(&work).unwrap();
    store.save_profile(&home).unwrap();
    let root = definition(
        "paths.root",
        VariableType::String,
        &[VariableScope::Global, VariableScope::Profile],
    );
    store.save_shared_variable(&root).unwrap();
    let shop = work.projects[0].id;

    store
        .set_shared_variable_value(&root.key, &ScopeKey::Global, &text("/src"))
        .unwrap();
    let at_work = ScopeKey::Profile { profile: work.id };
    store
        .set_shared_variable_value(&root.key, &at_work, &text("/work"))
        .unwrap();
    // The same slot is replaced, not duplicated.
    store
        .set_shared_variable_value(&root.key, &at_work, &text("/office"))
        .unwrap();
    let values = store.shared_variable_values(&root.key).unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[1].value, text("/office"));

    // Not allowed at project scope, wrong type, execution never stored.
    let at_shop = ScopeKey::Project {
        profile: work.id,
        project: shop,
    };
    assert!(matches!(
        store.set_shared_variable_value(&root.key, &at_shop, &text("x")),
        Err(StorageError::Variable(
            DefinitionIssue::ScopeNotAllowed { .. }
        ))
    ));
    assert!(matches!(
        store.set_shared_variable_value(&root.key, &ScopeKey::Global, &VariableValue::Integer(1)),
        Err(StorageError::Variable(DefinitionIssue::WrongType { .. }))
    ));
    let everywhere = definition("paths.repo", VariableType::String, ALL);
    store.save_shared_variable(&everywhere).unwrap();
    assert!(matches!(
        store.set_shared_variable_value(
            &everywhere.key,
            &ScopeKey::Execution {
                execution: ExecutionId::new()
            },
            &text("x")
        ),
        Err(StorageError::Invalid(_))
    ));
    // A project of another profile, an unknown profile, an unknown key.
    let foreign = ScopeKey::Project {
        profile: home.id,
        project: shop,
    };
    assert!(matches!(
        store.set_shared_variable_value(&everywhere.key, &foreign, &text("x")),
        Err(StorageError::ProjectOfAnotherProfile(p)) if p == shop
    ));
    assert!(matches!(
        store.set_shared_variable_value(
            &everywhere.key,
            &ScopeKey::Profile {
                profile: deckpilot_core::ProfileId::new()
            },
            &text("x")
        ),
        Err(StorageError::UnknownProfile)
    ));
    assert!(matches!(
        store.set_shared_variable_value(&key("no.such"), &ScopeKey::Global, &text("x")),
        Err(StorageError::UnknownVariable(_))
    ));
    // Nothing of the refused writes is stored.
    assert_eq!(
        store.shared_variable_values(&everywhere.key).unwrap(),
        Vec::new()
    );

    assert!(
        store
            .clear_shared_variable_value(&root.key, &at_work)
            .unwrap()
    );
    assert!(
        !store
            .clear_shared_variable_value(&root.key, &at_work)
            .unwrap()
    );
    assert_eq!(store.shared_variable_values(&root.key).unwrap().len(), 1);
}

#[test]
fn ids_in_values_of_a_profile_stay_in_that_profile() {
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile_with_projects("Work", &["SHOP"]);
    let home = profile_with_projects("Home", &["BLOG"]);
    store.save_profile(&work).unwrap();
    store.save_profile(&home).unwrap();
    let favorite = definition("projects.favorite", VariableType::ProjectId, ALL);
    store.save_shared_variable(&favorite).unwrap();
    let at_work = ScopeKey::Profile { profile: work.id };
    assert!(matches!(
        store.set_shared_variable_value(
            &favorite.key,
            &at_work,
            &VariableValue::ProjectId(home.projects[0].id)
        ),
        Err(StorageError::ProjectOfAnotherProfile(_))
    ));
    store
        .set_shared_variable_value(
            &favorite.key,
            &at_work,
            &VariableValue::ProjectId(work.projects[0].id),
        )
        .unwrap();
    let owner = definition("profiles.main", VariableType::ProfileId, ALL);
    store.save_shared_variable(&owner).unwrap();
    assert!(matches!(
        store.set_shared_variable_value(&owner.key, &at_work, &VariableValue::ProfileId(home.id)),
        Err(StorageError::Invalid(_))
    ));
}

#[test]
fn values_disappear_with_their_project_profile_or_variable() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut work = profile_with_projects("Work", &["SHOP", "CMR"]);
    store.save_profile(&work).unwrap();
    let repo = definition("paths.repo", VariableType::String, ALL);
    store.save_shared_variable(&repo).unwrap();
    let (shop, cmr) = (work.projects[0].id, work.projects[1].id);
    for (project, value) in [(shop, "/shop"), (cmr, "/cmr")] {
        store
            .set_shared_variable_value(
                &repo.key,
                &ScopeKey::Project {
                    profile: work.id,
                    project,
                },
                &text(value),
            )
            .unwrap();
    }
    store
        .set_shared_variable_value(
            &repo.key,
            &ScopeKey::Profile { profile: work.id },
            &text("/w"),
        )
        .unwrap();
    store
        .set_shared_variable_value(&repo.key, &ScopeKey::Global, &text("/g"))
        .unwrap();

    // Removing a project from the profile removes its values only.
    work.projects.remove(1);
    store.save_profile(&work).unwrap();
    let left: Vec<_> = store
        .shared_variable_values(&repo.key)
        .unwrap()
        .into_iter()
        .map(|v| v.value)
        .collect();
    assert_eq!(left, [text("/g"), text("/w"), text("/shop")]);

    // Deleting the profile removes its values; the global one stays.
    assert!(store.delete_profile(work.id).unwrap());
    let left = store.shared_variable_values(&repo.key).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].scope, ScopeKey::Global);

    // Deleting the variable removes the rest.
    assert!(store.delete_shared_variable(&repo.key).unwrap());
    assert!(!store.delete_shared_variable(&repo.key).unwrap());
    let rows: i64 = store
        .connection_for_tests()
        .query_row("SELECT count(*) FROM shared_variable_values", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn a_definition_change_never_drops_values_silently() {
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile_with_projects("Work", &["SHOP"]);
    store.save_profile(&work).unwrap();
    let mut count = definition("limits.count", VariableType::Integer, ALL);
    store.save_shared_variable(&count).unwrap();
    let at_work = ScopeKey::Profile { profile: work.id };
    store
        .set_shared_variable_value(&count.key, &at_work, &VariableValue::Integer(3))
        .unwrap();

    count.value_type = VariableType::String;
    assert!(matches!(
        store.save_shared_variable(&count),
        Err(StorageError::VariableValuesConflict { count: 1, .. })
    ));
    count.value_type = VariableType::Integer;
    count.scopes = [VariableScope::Global].into();
    assert!(matches!(
        store.save_shared_variable(&count),
        Err(StorageError::VariableValuesConflict { .. })
    ));
    let stored = store.shared_variable(&count.key).unwrap().unwrap();
    assert_eq!(stored.definition.scopes.len(), ALL.len());
    assert_eq!(store.shared_variable_values(&count.key).unwrap().len(), 1);

    // Once the value is cleared, the change is accepted.
    store
        .clear_shared_variable_value(&count.key, &at_work)
        .unwrap();
    store.save_shared_variable(&count).unwrap();
}

#[test]
fn a_snapshot_resolves_with_precedence_and_profile_isolation() {
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile_with_projects("Work", &["SHOP", "CMR"]);
    let home = profile_with_projects("Home", &["BLOG"]);
    store.save_profile(&work).unwrap();
    store.save_profile(&home).unwrap();
    let mut repo = definition("paths.repo", VariableType::String, ALL);
    repo.default = Some(text("default"));
    store.save_shared_variable(&repo).unwrap();
    let shop = work.projects[0].id;
    let set = |scope: ScopeKey, value: &str| {
        store
            .set_shared_variable_value(&repo.key, &scope, &text(value))
            .unwrap();
    };
    set(ScopeKey::Global, "global");
    set(ScopeKey::Profile { profile: work.id }, "work");
    set(ScopeKey::Profile { profile: home.id }, "home");
    set(
        ScopeKey::Project {
            profile: work.id,
            project: shop,
        },
        "shop",
    );

    let variables = store.shared_variables_for(work.id).unwrap();
    // Values of other profiles are not even part of the snapshot.
    assert_eq!(variables.values(&repo.key).len(), 3);
    let value =
        |context: &ResolutionContext, execution: &[(ScopeKey, VariableValue)]| match variables
            .resolve(&repo.key, context, execution)
        {
            Resolution::Value { value, source, .. } => (value, source),
            other => panic!("{other:?}"),
        };
    let at_shop =
        ResolutionContext::preview(work.id).with_project(Some(shop), ProjectSource::Explicit);
    assert_eq!(value(&at_shop, &[]).0, text("shop"));
    let at_cmr = ResolutionContext::preview(work.id)
        .with_project(Some(work.projects[1].id), ProjectSource::Explicit);
    assert_eq!(value(&at_cmr, &[]).0, text("work"));
    // An execution value wins.
    let run = ExecutionId::new();
    let in_run = at_shop.clone().for_execution(run);
    let execution = [(ScopeKey::Execution { execution: run }, text("run"))];
    let (resolved, source) = value(&in_run, &execution);
    assert_eq!(resolved, text("run"));
    assert_eq!(
        source,
        ValueSource::Scope {
            scope: ScopeKey::Execution { execution: run }
        }
    );
    // Another profile's context sees nothing of this snapshot.
    assert_eq!(
        variables.resolve(&repo.key, &ResolutionContext::preview(home.id), &[]),
        Resolution::Missing(MissingReason::OtherProfile)
    );
    assert_eq!(
        variables.resolve(&key("no.such"), &at_shop, &[]),
        Resolution::Missing(MissingReason::UnknownVariable)
    );

    // A value changed afterwards is seen by the next snapshot.
    set(
        ScopeKey::Project {
            profile: work.id,
            project: shop,
        },
        "shop 2",
    );
    assert_eq!(value(&at_shop, &[]).0, text("shop"));
    let fresh = store.shared_variables_for(work.id).unwrap();
    assert!(matches!(
        fresh.resolve(&repo.key, &at_shop, &[]),
        Resolution::Value { value, .. } if value == text("shop 2")
    ));
    // Without values the default applies.
    store.delete_profile(home.id).unwrap();
    let mut bare = definition("paths.other", VariableType::String, ALL);
    bare.default = Some(text("fallback"));
    bare.sensitive = true;
    store.save_shared_variable(&bare).unwrap();
    let fresh = store.shared_variables_for(work.id).unwrap();
    assert_eq!(
        fresh.resolve(&bare.key, &at_shop, &[]),
        Resolution::Value {
            value: text("fallback"),
            source: ValueSource::Default,
            sensitive: true
        }
    );
}

#[test]
fn a_global_project_value_of_another_profile_resolves_as_missing() {
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile_with_projects("Work", &["SHOP"]);
    let home = profile_with_projects("Home", &["BLOG"]);
    store.save_profile(&work).unwrap();
    store.save_profile(&home).unwrap();
    let favorite = definition("projects.favorite", VariableType::ProjectId, ALL);
    store.save_shared_variable(&favorite).unwrap();
    store
        .set_shared_variable_value(
            &favorite.key,
            &ScopeKey::Global,
            &VariableValue::ProjectId(work.projects[0].id),
        )
        .unwrap();
    let at_home = store.shared_variables_for(home.id).unwrap();
    assert_eq!(
        at_home.resolve(&favorite.key, &ResolutionContext::preview(home.id), &[]),
        Resolution::Missing(MissingReason::OtherProfile)
    );
    let at_work = store.shared_variables_for(work.id).unwrap();
    assert!(matches!(
        at_work.resolve(&favorite.key, &ResolutionContext::preview(work.id), &[]),
        Resolution::Value { .. }
    ));
}

#[test]
fn secret_references_hold_the_name_never_the_secret() {
    let store = SqliteStore::open_in_memory().unwrap();
    let token = CredentialId::new("api.token").unwrap();
    store
        .set_credential(&token, "", Some(&Secret::new("very-secret-value")))
        .unwrap();
    let reference = definition(
        "api.token",
        VariableType::SecretRef,
        &[VariableScope::Global],
    );
    store.save_shared_variable(&reference).unwrap();
    store
        .set_shared_variable_value(
            &reference.key,
            &ScopeKey::Global,
            &VariableValue::SecretRef(token.clone()),
        )
        .unwrap();
    let rows: Vec<String> = store
        .connection_for_tests()
        .prepare("SELECT value FROM shared_variable_values")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(rows, [r#"{"type":"secretRef","value":"api.token"}"#]);
    assert!(rows.iter().all(|row| !row.contains("very-secret-value")));
}

#[test]
fn values_that_look_like_references_stay_plain_text() {
    // Values are literals: storing and reading never expands them, so a
    // value cannot form a cycle.
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile_with_projects("Work", &[]);
    store.save_profile(&work).unwrap();
    let a = definition("loop.a", VariableType::String, ALL);
    store.save_shared_variable(&a).unwrap();
    let looping = text("{{ shared.loop.a }}");
    store
        .set_shared_variable_value(&a.key, &ScopeKey::Global, &looping)
        .unwrap();
    let variables = store.shared_variables_for(work.id).unwrap();
    assert!(matches!(
        variables.resolve(&a.key, &ResolutionContext::preview(work.id), &[]),
        Resolution::Value { value, .. } if value == looping
    ));
}

#[test]
fn deleting_a_variable_removes_its_provenance() {
    let store = SqliteStore::open_in_memory().unwrap();
    let work = profile_with_projects("Work", &[]);
    store.save_profile(&work).unwrap();
    let a = definition("demo.a", VariableType::String, ALL);
    store
        .save_shared_variable_from(&a, crate::VariableOrigin::Seed)
        .unwrap();
    store
        .record_provenance(&[Provenance::seeded(
            work.id,
            ProvenanceEntity::Variable,
            a.key.as_str(),
            &a,
            "dev-seed",
            "demo.a",
            1,
        )
        .unwrap()])
        .unwrap();
    assert_eq!(store.provenance(work.id).unwrap().len(), 1);
    store.delete_shared_variable(&a.key).unwrap();
    assert_eq!(store.provenance(work.id).unwrap(), Vec::new());
}
