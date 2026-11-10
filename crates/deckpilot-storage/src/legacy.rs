//! Schema migration 3: projects move out of the profile documents into the
//! `projects` table, and action types move to module namespaces.
//!
//! The migration is deterministic: profiles are processed in id order and
//! every decision depends only on the stored data. Nothing is assigned to a
//! profile it was not already stored in. Every change that is more than a
//! rename is written to `migration_notes`, so it can be shown to the user.
//!
//! Rules:
//!
//! * A project stays in the profile whose document contains it.
//! * A project id that an earlier profile already uses (an earlier release
//!   allowed copying a profile document by hand) gets a new id derived from
//!   the profile id and the old id. All references inside that profile's
//!   document follow the new id; other profiles are not touched.
//! * The same id twice inside one profile: the first entry keeps it together
//!   with all references, the later entry gets a derived id.
//! * A project key used twice inside one profile (keys are unique per
//!   profile and case-insensitive): the later project gets a `-2`, `-3`, ...
//!   suffix.
//! * The default and active project are kept only if they are a project of
//!   the same profile.
//! * `deckpilot.core.log` becomes `general.log`,
//!   `deckpilot.project.cycle` becomes `projects.next` or `projects.previous`
//!   (depending on its direction), `deckpilot.project.select` becomes
//!   `projects.select` and refers to its project by id instead of by key, and
//!   `deckpilot.resource.resolve` becomes `projects.resolve-resource`. A
//!   select action whose key matches no project of its profile is kept
//!   without a project, so the user can pick one.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{Result, StorageError};

/// Document format written by this migration.
const DOCUMENT_VERSION: u32 = 3;

/// Namespace for project ids derived during the migration.
const PROJECT_NAMESPACE: Uuid = Uuid::from_u128(0x8f0e_3b5c_2d41_4c7e_9a63_51f2_d7b8_0c19);

pub(crate) const SCHEMA: &str = r"
    CREATE TABLE projects (
        id         TEXT PRIMARY KEY NOT NULL,
        profile_id TEXT NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        key        TEXT NOT NULL,
        name       TEXT NOT NULL,
        position   INTEGER NOT NULL CHECK (position >= 0),
        resources  TEXT NOT NULL DEFAULT '{}',
        UNIQUE (profile_id, id)
    );
    CREATE UNIQUE INDEX projects_profile_key ON projects (profile_id, key COLLATE NOCASE);
    CREATE INDEX projects_profile_position ON projects (profile_id, position);
    CREATE TABLE default_projects (
        profile_id TEXT PRIMARY KEY NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        project_id TEXT NOT NULL,
        FOREIGN KEY (profile_id, project_id) REFERENCES projects (profile_id, id) ON DELETE CASCADE
    );
    CREATE TABLE active_projects (
        profile_id TEXT PRIMARY KEY NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        project_id TEXT NOT NULL,
        FOREIGN KEY (profile_id, project_id) REFERENCES projects (profile_id, id) ON DELETE CASCADE
    );
    CREATE TABLE modules (
        id      TEXT PRIMARY KEY NOT NULL,
        enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
        config  TEXT NOT NULL DEFAULT '{}'
    );
    CREATE TABLE migration_notes (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        schema_version INTEGER NOT NULL,
        profile_id     TEXT,
        message        TEXT NOT NULL,
        created_at     INTEGER NOT NULL
    );
";

struct Notes<'a> {
    connection: &'a Connection,
    profile: String,
}

impl Notes<'_> {
    fn add(&self, message: impl AsRef<str>) -> Result<()> {
        self.connection.execute(
            "INSERT INTO migration_notes (schema_version, profile_id, message, created_at)
             VALUES (3, ?1, ?2, ?3)",
            params![self.profile, message.as_ref(), crate::sqlite::now()],
        )?;
        Ok(())
    }
}

fn derived_id(profile: &str, old: &str, attempt: u32) -> String {
    Uuid::new_v5(
        &PROJECT_NAMESPACE,
        format!("{profile}/{old}/{attempt}").as_bytes(),
    )
    .to_string()
}

fn text_of(value: &Value) -> Option<&str> {
    value.get("value").and_then(Value::as_str)
}

/// The id of the project with `key` (case-insensitive) in `keys`.
fn project_for_key(keys: &[(String, String)], key: &str) -> Option<String> {
    keys.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, id)| id.clone())
}

/// Rewrites legacy action types of one profile document.
fn upgrade_actions(document: &mut Value, notes: &Notes<'_>) -> Result<()> {
    let keys: Vec<(String, String)> = document["projects"]
        .as_array()
        .map(|projects| {
            projects
                .iter()
                .filter_map(|p| Some((p["key"].as_str()?.to_owned(), p["id"].as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let project_for_key = |key: &str| project_for_key(&keys, key);

    // Action ids whose parameters changed shape, for step overrides.
    let mut converted: HashMap<String, &'static str> = HashMap::new();
    let Some(actions) = document["actions"].as_array_mut() else {
        return Ok(());
    };
    for action in actions {
        let name = action["name"].as_str().unwrap_or_default().to_owned();
        let id = action["id"].as_str().unwrap_or_default().to_owned();
        let Some(kind) = action["actionType"].as_str().map(str::to_owned) else {
            continue;
        };
        let parameters = action
            .as_object_mut()
            .expect("actions are objects")
            .entry("parameters")
            .or_insert_with(|| json!({}));
        let new_type = match kind.as_str() {
            "deckpilot.core.log" => "general.log",
            "deckpilot.resource.resolve" => "projects.resolve-resource",
            "deckpilot.project.cycle" => {
                let direction = parameters
                    .as_object_mut()
                    .and_then(|p| p.remove("direction"));
                converted.insert(id, "cycle");
                match direction.as_ref().and_then(text_of) {
                    Some("previous") => "projects.previous",
                    _ => "projects.next",
                }
            }
            "deckpilot.project.select" => {
                converted.insert(id, "select");
                let key = parameters.as_object_mut().and_then(|p| p.remove("project"));
                let key = key
                    .as_ref()
                    .and_then(text_of)
                    .unwrap_or_default()
                    .to_owned();
                match project_for_key(&key) {
                    Some(project) => {
                        parameters["project"] = json!({"type": "project", "value": project});
                    }
                    None => notes.add(format!(
                        "Action \"{name}\" selected project \"{key}\", which this profile does \
                         not contain. Choose a project for it."
                    ))?,
                }
                "projects.select"
            }
            _ => continue,
        };
        action["actionType"] = json!(new_type);
    }
    upgrade_overrides(document, &converted, &keys, notes)
}

/// Step overrides follow the parameter changes of their action.
fn upgrade_overrides(
    document: &mut Value,
    converted: &HashMap<String, &'static str>,
    keys: &[(String, String)],
    notes: &Notes<'_>,
) -> Result<()> {
    let project_for_key = |key: &str| project_for_key(keys, key);
    let Some(workflows) = document["workflows"].as_array_mut() else {
        return Ok(());
    };
    for workflow in workflows {
        let workflow_name = workflow["name"].as_str().unwrap_or_default().to_owned();
        let Some(steps) = workflow["steps"].as_array_mut() else {
            continue;
        };
        for (index, step) in steps.iter_mut().enumerate() {
            let action = step["action"].as_str().unwrap_or_default().to_owned();
            let Some(overrides) = step.get_mut("overrides").and_then(Value::as_object_mut) else {
                continue;
            };
            match converted.get(&action) {
                Some(&"cycle") => {
                    if overrides.remove("direction").is_some() {
                        notes.add(format!(
                            "Workflow \"{workflow_name}\", step {}: the direction override was \
                             removed; next and previous are now separate actions.",
                            index + 1
                        ))?;
                    }
                }
                Some(&"select") => {
                    if let Some(old) = overrides.remove("project") {
                        let key = text_of(&old).unwrap_or_default();
                        match project_for_key(key) {
                            Some(project) => {
                                overrides.insert(
                                    "project".into(),
                                    json!({"type": "project", "value": project}),
                                );
                            }
                            None => notes.add(format!(
                                "Workflow \"{workflow_name}\", step {}: the project override \
                                 \"{key}\" matches no project of this profile and was removed.",
                                index + 1
                            ))?,
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Replaces every occurrence of a project id string inside `value`.
fn replace_id(value: &mut Value, old: &str, new: &str) {
    match value {
        Value::String(text) if text == old => new.clone_into(text),
        Value::Array(items) => items.iter_mut().for_each(|v| replace_id(v, old, new)),
        Value::Object(map) => map.values_mut().for_each(|v| replace_id(v, old, new)),
        _ => {}
    }
}

fn unique_key(key: &str, taken: &BTreeSet<String>) -> String {
    // A profile has far fewer projects than this, so a free suffix exists.
    for n in 2..=taken.len() + 2 {
        let suffix = format!("-{n}");
        let stem: String = key.chars().take(32 - suffix.len()).collect();
        let candidate = format!("{stem}{suffix}");
        if !taken.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
    unreachable!("one of len + 1 candidates is free")
}

/// Inserts the projects of one profile, resolving id and key conflicts.
/// Returns the ids that changed for references and the profile's own ids.
fn insert_projects(
    connection: &Connection,
    notes: &Notes<'_>,
    owners: &mut BTreeMap<String, String>,
    profile_id: &str,
    projects: Vec<Value>,
) -> Result<(HashMap<String, String>, BTreeSet<String>)> {
    let mut remapped: HashMap<String, String> = HashMap::new();
    let mut own_ids: BTreeSet<String> = BTreeSet::new();
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for (position, project) in projects.into_iter().enumerate() {
        let Value::Object(mut project) = project else {
            continue;
        };
        let old_id = project
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut key = project
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut id = old_id.clone();
        let mut attempt = 0;
        while owners.contains_key(&id) || own_ids.contains(&id) {
            id = derived_id(profile_id, &old_id, attempt);
            attempt += 1;
        }
        if id != old_id {
            if let Some(owner) = owners.get(&old_id).filter(|o| **o != profile_id) {
                notes.add(format!(
                    "Project \"{key}\" received a new id because profile {owner} uses the \
                     same id; references in this profile were updated."
                ))?;
                remapped.insert(old_id.clone(), id.clone());
            } else {
                notes.add(format!(
                    "Project \"{key}\" received a new id because its id appeared twice in \
                     this profile; references keep pointing to the first project."
                ))?;
            }
        }
        if keys.contains(&key.to_ascii_lowercase()) {
            let renamed = unique_key(&key, &keys);
            notes.add(format!(
                "Project key \"{key}\" was used twice in this profile; the second project \
                 is now \"{renamed}\"."
            ))?;
            key = renamed;
        }
        keys.insert(key.to_ascii_lowercase());
        own_ids.insert(id.clone());
        owners.insert(id.clone(), profile_id.to_owned());
        let name = project
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let resources = project
            .remove("resources")
            .unwrap_or_else(|| Value::Object(Map::new()));
        connection.execute(
            "INSERT INTO projects (id, profile_id, key, name, position, resources)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                profile_id,
                key,
                name,
                i64::try_from(position).unwrap_or(i64::MAX),
                resources.to_string()
            ],
        )?;
    }
    Ok((remapped, own_ids))
}

/// Migrates one profile document. Profiles are processed in id order, and
/// `owners` records which profile kept which project id so far.
fn migrate_profile(
    connection: &Connection,
    owners: &mut BTreeMap<String, String>,
    profile_id: &str,
    document: &str,
    selected: Option<&str>,
) -> Result<()> {
    let notes = Notes {
        connection,
        profile: profile_id.to_owned(),
    };
    let mut document: Value = serde_json::from_str(document)?;
    upgrade_actions(&mut document, &notes)?;

    let projects = document
        .as_object_mut()
        .and_then(|d| d.remove("projects"))
        .and_then(|p| match p {
            Value::Array(items) => Some(items),
            _ => None,
        })
        .unwrap_or_default();
    let (remapped, own_ids) = insert_projects(connection, &notes, owners, profile_id, projects)?;
    for (old, new) in &remapped {
        replace_id(&mut document, old, new);
    }

    let default = document
        .as_object_mut()
        .and_then(|d| d.remove("defaultProject"))
        .and_then(|v| v.as_str().map(str::to_owned));
    if let Some(default) = default {
        let default = remapped.get(&default).cloned().unwrap_or(default);
        if own_ids.contains(&default) {
            connection.execute(
                "INSERT INTO default_projects (profile_id, project_id) VALUES (?1, ?2)",
                params![profile_id, default],
            )?;
        } else {
            notes.add(
                "The default project was cleared because it is not a project of this profile.",
            )?;
        }
    }
    if let Some(selected) = selected {
        let selected = remapped.get(selected).map_or(selected, String::as_str);
        if own_ids.contains(selected) {
            connection.execute(
                "INSERT INTO active_projects (profile_id, project_id) VALUES (?1, ?2)",
                params![profile_id, selected],
            )?;
        } else {
            notes.add("The stored active project was cleared because it is not a project of this profile.")?;
        }
    }
    connection.execute(
        "UPDATE profiles SET document_version = ?1, document = ?2 WHERE id = ?3",
        params![DOCUMENT_VERSION, document.to_string(), profile_id],
    )?;
    Ok(())
}

pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let active: HashMap<String, String> = connection
        .prepare(
            "SELECT profile_id, active_project FROM profile_state WHERE active_project IS NOT NULL",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let profiles: Vec<(String, u32, String)> = connection
        .prepare("SELECT id, document_version, document FROM profiles ORDER BY id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;

    // Project id -> profile that keeps it.
    let mut owners: BTreeMap<String, String> = BTreeMap::new();
    for (profile_id, version, document) in profiles {
        if !(1..=2).contains(&version) {
            return Err(StorageError::UnsupportedDocument(version));
        }
        let selected = active.get(&profile_id).map(String::as_str);
        migrate_profile(connection, &mut owners, &profile_id, &document, selected)?;
    }
    connection.execute_batch("DROP TABLE profile_state;")?;
    // The machine document did not change shape.
    connection.execute(
        "UPDATE machine_config SET document_version = ?1",
        [DOCUMENT_VERSION],
    )?;
    Ok(())
}
