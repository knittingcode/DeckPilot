use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, ParameterValue, Profile,
    ProfileId, Project, ProjectKey,
};

use super::{Provenance, ProvenanceEntity, ProvenanceOrigin, snapshot};
use crate::{ConfigStore, SqliteStore, StorageError};

fn action(name: &str) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: ActionTypeId::new("general.log").unwrap(),
        parameters: ActionParameters::new().with("message", ParameterValue::Text("hi".into())),
        description: String::new(),
        icon: None,
        label: None,
    }
}

fn seeded(profile: &Profile) -> Vec<Provenance> {
    let mut rows = Vec::new();
    for item in &profile.actions {
        rows.push(
            Provenance::seeded(
                profile.id,
                ProvenanceEntity::Action,
                item.id.to_string(),
                item,
                "dev-seed",
                format!("test.{}", item.name),
                1,
            )
            .unwrap(),
        );
    }
    for item in &profile.projects {
        rows.push(
            Provenance::seeded(
                profile.id,
                ProvenanceEntity::Project,
                item.id.to_string(),
                item,
                "dev-seed",
                format!("test.{}", item.key),
                1,
            )
            .unwrap(),
        );
    }
    rows
}

#[test]
fn snapshots_are_canonical_and_ignore_the_id() {
    let a = action("Hello");
    let mut b = a.clone();
    b.id = ActionInstanceId::new();
    assert_eq!(snapshot(&a).unwrap(), snapshot(&b).unwrap());
    assert!(!snapshot(&a).unwrap().contains(&a.id.to_string()));
    // Field order does not matter.
    let one = serde_json::json!({"b": 1, "a": {"d": [1, {"z": 0, "y": 1}], "c": null}});
    let two: serde_json::Value =
        serde_json::from_str(r#"{"a":{"c":null,"d":[1,{"y":1,"z":0}]},"b":1}"#).unwrap();
    assert_eq!(snapshot(&one).unwrap(), snapshot(&two).unwrap());
    assert_eq!(
        snapshot(&one).unwrap(),
        r#"{"a":{"c":null,"d":[1,{"y":1,"z":0}]},"b":1}"#
    );
}

#[test]
fn edited_items_are_told_apart_from_untouched_ones() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut profile = Profile::new("Demo");
    profile.actions.push(action("Hello"));
    profile.actions.push(action("Bye"));
    profile
        .projects
        .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
    store.save_profile(&profile).unwrap();
    store.record_provenance(&seeded(&profile)).unwrap();
    let statuses = store.provenance_status(profile.id).unwrap();
    assert_eq!(statuses.len(), 3);
    assert!(statuses.iter().all(|s| s.exists && !s.edited));
    assert!(
        statuses
            .iter()
            .all(|s| s.provenance.origin == ProvenanceOrigin::Seed)
    );

    profile.actions[0].label = Some("Changed".into());
    profile.projects[0].name = "Online Shop".into();
    store.save_profile(&profile).unwrap();
    let edited: Vec<_> = store
        .provenance_status(profile.id)
        .unwrap()
        .into_iter()
        .filter(|s| s.edited)
        .map(|s| s.provenance.seed_key.unwrap())
        .collect();
    assert_eq!(edited, ["test.Hello", "test.SHOP"]);
}

#[test]
fn rows_of_deleted_items_and_profiles_are_removed() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut profile = Profile::new("Demo");
    profile.actions.push(action("Hello"));
    profile.actions.push(action("Bye"));
    store.save_profile(&profile).unwrap();
    store.record_provenance(&seeded(&profile)).unwrap();

    // A deleted action takes its row along; a new action with the same
    // content is the user's.
    let removed = profile.actions.remove(0);
    store.save_profile(&profile).unwrap();
    let rows = store.provenance(profile.id).unwrap();
    assert_eq!(rows.len(), 1);
    assert_ne!(rows[0].entity_id, removed.id.to_string());
    profile.actions.push(action("Hello"));
    store.save_profile(&profile).unwrap();
    assert_eq!(store.provenance(profile.id).unwrap().len(), 1);

    store.delete_profile(profile.id).unwrap();
    let count: i64 = store
        .connection_for_tests()
        .query_row("SELECT count(*) FROM provenance", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn provenance_needs_an_existing_profile_and_is_all_or_nothing() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut profile = Profile::new("Demo");
    profile.actions.push(action("Hello"));
    store.save_profile(&profile).unwrap();
    let mut rows = seeded(&profile);
    let mut stray = rows[0].clone();
    stray.profile = ProfileId::new();
    rows.push(stray);
    assert!(matches!(
        store.record_provenance(&rows),
        Err(StorageError::UnknownProfile)
    ));
    assert_eq!(store.provenance(profile.id).unwrap(), Vec::new());
    assert!(matches!(
        store.provenance_status(ProfileId::new()),
        Err(StorageError::UnknownProfile)
    ));
}

#[test]
fn items_without_a_row_are_the_users() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut profile = Profile::new("Mine");
    profile.actions.push(action("Hello"));
    store.save_profile(&profile).unwrap();
    assert_eq!(store.provenance_status(profile.id).unwrap(), Vec::new());
}
