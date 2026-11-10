//! Schema migration 4: the projects module's generic action parameters.
//!
//! * `projects.select` and step overrides of it: the parameter `project`
//!   becomes `project_id`. A select action without a project (its legacy key
//!   matched no project in migration 3) now means "the current project"; a
//!   note asks the user to review it.
//! * `projects.resolve-resource`: the `resource` parameter was a resource
//!   reference and becomes a resource name of the project.
//!   - "role X of the active project" becomes resource `X` without a
//!     project, which means the same.
//!   - "fixed resource key K" becomes project P and resource X when exactly
//!     one project of the same profile maps a resource X to K. Otherwise the
//!     resource is left empty and a note names the old key; the action is
//!     kept and asks for a resource when edited.
//!
//! Action ids, names, workflows and layout bindings are not changed.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::Result;

const DOCUMENT_VERSION: u32 = 4;

fn note(connection: &Connection, profile: &str, message: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO migration_notes (schema_version, profile_id, message, created_at)
         VALUES (4, ?1, ?2, ?3)",
        params![profile, message, crate::sqlite::now()],
    )?;
    Ok(())
}

/// `(project id, resource name)` for every resource key a project of the
/// profile maps.
fn resource_owners(
    connection: &Connection,
    profile: &str,
) -> Result<HashMap<String, Vec<(String, String)>>> {
    let rows: Vec<(String, String)> = connection
        .prepare("SELECT id, resources FROM projects WHERE profile_id = ?1 ORDER BY position")?
        .query_map([profile], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut owners: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (project, resources) in rows {
        let resources: BTreeMap<String, String> = serde_json::from_str(&resources)?;
        for (role, key) in resources {
            owners.entry(key).or_default().push((project.clone(), role));
        }
    }
    Ok(owners)
}

/// The new parameters for an old `resource` value, or `None` with a reason.
fn convert_resource(
    reference: &Value,
    owners: &HashMap<String, Vec<(String, String)>>,
) -> std::result::Result<(Option<String>, String), String> {
    let inner = &reference["value"];
    match inner["type"].as_str() {
        Some("projectRole") => Ok((None, inner["role"].as_str().unwrap_or_default().to_owned())),
        Some("fixed") => {
            let key = inner["key"].as_str().unwrap_or_default();
            match owners.get(key).map(Vec::as_slice) {
                Some([(project, role)]) => Ok((Some(project.clone()), role.clone())),
                Some([_, _, ..]) => Err(format!(
                    "fixed resource \"{key}\" is used by several projects"
                )),
                _ => Err(format!(
                    "fixed resource \"{key}\" belongs to no project of this profile"
                )),
            }
        }
        _ => Err("its resource could not be read".into()),
    }
}

fn set_resource_parameters(
    parameters: &mut serde_json::Map<String, Value>,
    project: Option<String>,
    role: &str,
) {
    if let Some(project) = project {
        parameters.insert(
            "project_id".into(),
            json!({"type": "project", "value": project}),
        );
    }
    if !role.is_empty() {
        parameters.insert("resource".into(), json!({"type": "text", "value": role}));
    }
}

fn migrate_document(connection: &Connection, profile: &str, document: &mut Value) -> Result<()> {
    let owners = resource_owners(connection, profile)?;
    let mut kinds: HashMap<String, String> = HashMap::new();
    if let Some(actions) = document["actions"].as_array_mut() {
        for action in actions {
            let id = action["id"].as_str().unwrap_or_default().to_owned();
            let name = action["name"].as_str().unwrap_or_default().to_owned();
            let kind = action["actionType"].as_str().unwrap_or_default().to_owned();
            let Some(parameters) = action.as_object_mut().and_then(|a| {
                a.entry("parameters")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
            }) else {
                continue;
            };
            match kind.as_str() {
                "projects.select" => match parameters.remove("project") {
                    Some(project) => {
                        parameters.insert("project_id".into(), project);
                    }
                    None => note(
                        connection,
                        profile,
                        &format!(
                            "Action \"{name}\" has no project, so it now reports the current \
                             project instead of selecting one. Choose a project if it should \
                             switch."
                        ),
                    )?,
                },
                "projects.resolve-resource" => {
                    if let Some(reference) = parameters.remove("resource") {
                        match convert_resource(&reference, &owners) {
                            Ok((project, role)) => {
                                set_resource_parameters(parameters, project, &role);
                            }
                            Err(reason) => note(
                                connection,
                                profile,
                                &format!(
                                    "Action \"{name}\": {reason}. Choose a project and a \
                                     resource for it."
                                ),
                            )?,
                        }
                    }
                }
                _ => continue,
            }
            kinds.insert(id, kind);
        }
    }
    let Some(workflows) = document["workflows"].as_array_mut() else {
        return Ok(());
    };
    for workflow in workflows {
        let workflow_name = workflow["name"].as_str().unwrap_or_default().to_owned();
        let Some(steps) = workflow["steps"].as_array_mut() else {
            continue;
        };
        for (index, step) in steps.iter_mut().enumerate() {
            let action = step["action"].as_str().unwrap_or_default();
            let kind = kinds.get(action).cloned();
            let Some(overrides) = step.get_mut("overrides").and_then(Value::as_object_mut) else {
                continue;
            };
            match kind.as_deref() {
                Some("projects.select") => {
                    if let Some(project) = overrides.remove("project") {
                        overrides.insert("project_id".into(), project);
                    }
                }
                Some("projects.resolve-resource") => {
                    if let Some(reference) = overrides.remove("resource") {
                        match convert_resource(&reference, &owners) {
                            Ok((project, role)) => {
                                set_resource_parameters(overrides, project, &role);
                            }
                            Err(reason) => note(
                                connection,
                                profile,
                                &format!(
                                    "Workflow \"{workflow_name}\", step {}: the resource \
                                     override was removed because the {reason}.",
                                    index + 1
                                ),
                            )?,
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let profiles: Vec<(String, String)> = connection
        .prepare("SELECT id, document FROM profiles ORDER BY id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (profile, document) in profiles {
        let mut document: Value = serde_json::from_str(&document)?;
        migrate_document(connection, &profile, &mut document)?;
        connection.execute(
            "UPDATE profiles SET document_version = ?1, document = ?2 WHERE id = ?3",
            params![DOCUMENT_VERSION, document.to_string(), profile],
        )?;
    }
    connection.execute(
        "UPDATE machine_config SET document_version = ?1",
        [DOCUMENT_VERSION],
    )?;
    Ok(())
}
