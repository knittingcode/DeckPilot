//! Provenance (schema 8): where an item that the user did not create came
//! from (see `docs/action-architecture.md` §3.3).
//!
//! No row means *user-created*. Profile documents are not changed: the row
//! names the item by profile, kind and id, and keeps a canonical snapshot of
//! the item as it was created. An item is *edited* when its canonical JSON
//! now differs from the snapshot.
//!
//! Only newly created data gets provenance (decision D2): the schema 8
//! migration writes no rows, so data from before it — including seeded data
//! of development databases — stays exactly as it was and counts as the
//! user's.

use std::collections::BTreeSet;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use deckpilot_core::{Profile, ProfileId, variables::VariableDefinition};

use crate::sqlite::now;
use crate::{Result, StorageError};

pub(crate) const SCHEMA: &str = r"
    CREATE TABLE provenance (
        profile_id   TEXT NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        entity       TEXT NOT NULL
                     CHECK (entity IN ('action', 'workflow', 'layout', 'project', 'variable')),
        entity_id    TEXT NOT NULL,
        origin       TEXT NOT NULL CHECK (origin IN ('seed', 'import', 'migration')),
        catalog      TEXT,
        seed_key     TEXT,
        seed_version INTEGER,
        snapshot     TEXT NOT NULL,
        created_at   INTEGER NOT NULL,
        PRIMARY KEY (profile_id, entity, entity_id)
    );
";

/// The kind of item a provenance row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProvenanceEntity {
    Action,
    Workflow,
    Layout,
    Project,
    /// A shared variable definition; its id is the variable key.
    Variable,
}

impl ProvenanceEntity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Workflow => "workflow",
            Self::Layout => "layout",
            Self::Project => "project",
            Self::Variable => "variable",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "action" => Self::Action,
            "workflow" => Self::Workflow,
            "layout" => Self::Layout,
            "project" => Self::Project,
            "variable" => Self::Variable,
            _ => return None,
        })
    }
}

/// Where an item came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProvenanceOrigin {
    /// Demonstration data of the development seed.
    Seed,
    Import,
    Migration,
}

impl ProvenanceOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Import => "import",
            Self::Migration => "migration",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "seed" => Self::Seed,
            "import" => Self::Import,
            "migration" => Self::Migration,
            _ => return None,
        })
    }
}

/// One provenance row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub profile: ProfileId,
    pub entity: ProvenanceEntity,
    /// The item's id (the key for a variable).
    pub entity_id: String,
    pub origin: ProvenanceOrigin,
    /// The catalog that created it, e.g. `dev-seed`.
    pub catalog: Option<String>,
    /// Stable name of the item in its catalog, e.g. `work.next-project`.
    pub seed_key: Option<String>,
    pub seed_version: Option<u32>,
    /// Canonical JSON of the item without its id, as created.
    pub snapshot: String,
    /// Seconds since the Unix epoch.
    pub created_at: i64,
}

/// The state of an item with provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceStatus {
    pub provenance: Provenance,
    /// The item still exists.
    pub exists: bool,
    /// The item differs from its snapshot. `false` for an item that no
    /// longer exists.
    pub edited: bool,
}

/// Canonical JSON of `item` without its top-level `id`: object keys sorted,
/// no whitespace. Two values with the same content always give the same
/// text, whatever order their fields were written in.
pub fn snapshot<T: Serialize>(item: &T) -> Result<String> {
    let mut value = serde_json::to_value(item)?;
    if let Value::Object(fields) = &mut value {
        fields.remove("id");
    }
    let mut text = String::new();
    write_canonical(&value, &mut text);
    Ok(text)
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(fields) => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(&fields[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// The current snapshot of a profile item, `None` if it does not exist.
/// Variables are not profile items; see [`variable_snapshot`].
pub fn item_snapshot(
    profile: &Profile,
    entity: ProvenanceEntity,
    id: &str,
) -> Result<Option<String>> {
    match entity {
        ProvenanceEntity::Action => profile
            .actions
            .iter()
            .find(|item| item.id.to_string() == id)
            .map(snapshot)
            .transpose(),
        ProvenanceEntity::Workflow => profile
            .workflows
            .iter()
            .find(|item| item.id.to_string() == id)
            .map(snapshot)
            .transpose(),
        ProvenanceEntity::Layout => profile
            .layouts
            .iter()
            .find(|item| item.id.to_string() == id)
            .map(snapshot)
            .transpose(),
        ProvenanceEntity::Project => profile
            .projects
            .iter()
            .find(|item| item.id.to_string() == id)
            .map(snapshot)
            .transpose(),
        ProvenanceEntity::Variable => Ok(None),
    }
}

/// The snapshot of a variable definition.
pub fn variable_snapshot(definition: &VariableDefinition) -> Result<String> {
    snapshot(definition)
}

pub(crate) fn record(tx: &Connection, rows: &[Provenance]) -> Result<()> {
    for row in rows {
        let changed = tx.execute(
            "INSERT INTO provenance (profile_id, entity, entity_id, origin, catalog, seed_key,
                seed_version, snapshot, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9
             WHERE EXISTS (SELECT 1 FROM profiles WHERE id = ?1)
             ON CONFLICT (profile_id, entity, entity_id) DO UPDATE SET
                origin = excluded.origin,
                catalog = excluded.catalog,
                seed_key = excluded.seed_key,
                seed_version = excluded.seed_version,
                snapshot = excluded.snapshot,
                created_at = excluded.created_at",
            params![
                row.profile.to_string(),
                row.entity.as_str(),
                row.entity_id,
                row.origin.as_str(),
                row.catalog,
                row.seed_key,
                row.seed_version,
                row.snapshot,
                row.created_at,
            ],
        )?;
        if changed == 0 {
            return Err(StorageError::UnknownProfile);
        }
    }
    Ok(())
}

pub(crate) fn list(connection: &Connection, profile: ProfileId) -> Result<Vec<Provenance>> {
    let mut statement = connection.prepare(
        "SELECT entity, entity_id, origin, catalog, seed_key, seed_version, snapshot, created_at
         FROM provenance WHERE profile_id = ?1 ORDER BY entity, seed_key, entity_id",
    )?;
    let rows = statement
        .query_map([profile.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<u32>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(entity, entity_id, origin, catalog, seed_key, seed_version, snapshot, created_at)| {
                Some(Provenance {
                    profile,
                    entity: ProvenanceEntity::parse(&entity)?,
                    entity_id,
                    origin: ProvenanceOrigin::parse(&origin)?,
                    catalog,
                    seed_key,
                    seed_version,
                    snapshot,
                    created_at,
                })
            },
        )
        .collect())
}

/// Removes the rows of profile items that `profile` no longer contains,
/// inside the caller's transaction, so that an id never carries the
/// provenance of a deleted item. Variable rows are kept (variables are not
/// part of the profile document).
pub(crate) fn prune(tx: &Connection, profile: &Profile) -> Result<()> {
    let present = |entity: ProvenanceEntity| -> BTreeSet<String> {
        match entity {
            ProvenanceEntity::Action => profile.actions.iter().map(|i| i.id.to_string()).collect(),
            ProvenanceEntity::Workflow => {
                profile.workflows.iter().map(|i| i.id.to_string()).collect()
            }
            ProvenanceEntity::Layout => profile.layouts.iter().map(|i| i.id.to_string()).collect(),
            ProvenanceEntity::Project => {
                profile.projects.iter().map(|i| i.id.to_string()).collect()
            }
            ProvenanceEntity::Variable => BTreeSet::new(),
        }
    };
    for row in list(tx, profile.id)? {
        if row.entity != ProvenanceEntity::Variable && !present(row.entity).contains(&row.entity_id)
        {
            tx.execute(
                "DELETE FROM provenance WHERE profile_id = ?1 AND entity = ?2 AND entity_id = ?3",
                params![profile.id.to_string(), row.entity.as_str(), row.entity_id],
            )?;
        }
    }
    Ok(())
}

impl Provenance {
    /// A row for an item created by the seed catalog `catalog`.
    pub fn seeded<T: Serialize>(
        profile: ProfileId,
        entity: ProvenanceEntity,
        entity_id: impl Into<String>,
        item: &T,
        catalog: &str,
        seed_key: impl Into<String>,
        seed_version: u32,
    ) -> Result<Self> {
        Ok(Self {
            profile,
            entity,
            entity_id: entity_id.into(),
            origin: ProvenanceOrigin::Seed,
            catalog: Some(catalog.to_owned()),
            seed_key: Some(seed_key.into()),
            seed_version: Some(seed_version),
            snapshot: snapshot(item)?,
            created_at: now(),
        })
    }
}

#[cfg(test)]
mod tests;
