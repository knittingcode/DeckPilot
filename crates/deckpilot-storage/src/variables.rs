//! Shared variables (schema 8): definitions and the values set at global,
//! profile and project scope (see `docs/action-architecture.md` §3.1, §4).
//!
//! Definitions are database-wide; a value belongs to one scope key. Values
//! of the execution scope live for one run and are never stored. A value is
//! a literal of its variable's type: a `secretRef` value is the *name* of a
//! credential, never the secret, which stays in the credential store.
//!
//! Every write goes through [`VariableDefinition::check_value`] (type and
//! allowed scope). Project values must name a project of their own profile;
//! a composite foreign key keeps it that way and removes the value with the
//! project or profile.
//!
//! The agent's resolver reads a [`SharedVariables`] snapshot of one profile
//! and resolves `shared.` references through [`SharedVariables::resolve`],
//! which applies the contract's precedence ([`select_value`]) and profile
//! isolation. A snapshot is a copy: a value changed afterwards is seen by
//! the next snapshot, never by a cached one.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use deckpilot_core::variables::{
    MissingReason, Resolution, ResolutionContext, ScopeKey, VariableDefinition, VariableKey,
    VariableScope, VariableType, VariableValue, select_value,
};
use deckpilot_core::{ModuleId, ProfileId, ProjectId};

use crate::sqlite::{constraint_violation, now, parse_id};
use crate::{Result, StorageError};

pub(crate) const SCHEMA: &str = r"
    CREATE TABLE shared_variables (
        key           TEXT PRIMARY KEY NOT NULL,
        label         TEXT NOT NULL DEFAULT '',
        description   TEXT NOT NULL DEFAULT '',
        value_type    TEXT NOT NULL,
        scopes        TEXT NOT NULL,
        default_value TEXT,
        sensitive     INTEGER NOT NULL DEFAULT 0 CHECK (sensitive IN (0, 1)),
        module_id     TEXT,
        origin        TEXT NOT NULL DEFAULT 'user'
                      CHECK (origin IN ('user', 'seed', 'import', 'migration')),
        created_at    INTEGER NOT NULL,
        updated_at    INTEGER NOT NULL
    );
    CREATE TABLE shared_variable_values (
        key        TEXT NOT NULL REFERENCES shared_variables (key) ON DELETE CASCADE,
        scope      TEXT NOT NULL CHECK (scope IN ('global', 'profile', 'project')),
        profile_id TEXT REFERENCES profiles (id) ON DELETE CASCADE,
        project_id TEXT,
        value      TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        CHECK ((scope = 'global'  AND profile_id IS NULL     AND project_id IS NULL)
            OR (scope = 'profile' AND profile_id IS NOT NULL AND project_id IS NULL)
            OR (scope = 'project' AND profile_id IS NOT NULL AND project_id IS NOT NULL)),
        FOREIGN KEY (profile_id, project_id) REFERENCES projects (profile_id, id) ON DELETE CASCADE
    );
    CREATE UNIQUE INDEX shared_variable_values_slot
        ON shared_variable_values (key, scope, IFNULL(profile_id, ''), IFNULL(project_id, ''));
    CREATE INDEX shared_variable_values_owner
        ON shared_variable_values (profile_id, project_id);
";

/// How a variable definition came into the database. Values have no origin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableOrigin {
    /// Created by the user (or declared by a module).
    #[default]
    User,
    /// Created by the development seed.
    Seed,
    /// Brought in by an import.
    Import,
    /// Created by a schema migration.
    Migration,
}

impl VariableOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Seed => "seed",
            Self::Import => "import",
            Self::Migration => "migration",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "seed" => Self::Seed,
            "import" => Self::Import,
            "migration" => Self::Migration,
            _ => Self::User,
        }
    }
}

/// A stored definition with its bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredVariable {
    pub definition: VariableDefinition,
    pub origin: VariableOrigin,
    /// Seconds since the Unix epoch.
    pub created_at: i64,
    pub updated_at: i64,
}

/// A stored value of a variable at one scope key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredValue {
    pub key: VariableKey,
    pub scope: ScopeKey,
    pub value: VariableValue,
    /// Seconds since the Unix epoch.
    pub updated_at: i64,
}

/// What [`crate::SqliteStore::declare_module_variables`] did not declare,
/// because the key is already taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclarationConflict {
    pub key: VariableKey,
    /// The module that holds the key; `None`: a user variable.
    pub holder: Option<ModuleId>,
}

fn encode<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// One `shared_variables` row as text, decoded by [`decode_definition`].
type DefinitionRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    bool,
    Option<String>,
    String,
    i64,
    i64,
);

const DEFINITION_COLUMNS: &str = "key, label, description, value_type, scopes, default_value, \
     sensitive, module_id, origin, created_at, updated_at";

fn definition_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DefinitionRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
    ))
}

/// `None` for a row this build cannot read (a key or type from a newer
/// build, damaged JSON): such rows stay in the table but are ignored, so
/// they never prevent DeckPilot from starting.
fn decode_definition(row: DefinitionRow) -> Option<StoredVariable> {
    let (
        key,
        label,
        description,
        value_type,
        scopes,
        default,
        sensitive,
        module,
        origin,
        created_at,
        updated_at,
    ) = row;
    let definition = VariableDefinition {
        key: VariableKey::new(key).ok()?,
        label,
        description,
        value_type: serde_json::from_str::<VariableType>(&value_type).ok()?,
        scopes: serde_json::from_str::<BTreeSet<VariableScope>>(&scopes).ok()?,
        default: match default {
            Some(text) => Some(serde_json::from_str(&text).ok()?),
            None => None,
        },
        sensitive,
        module: match module {
            Some(id) => Some(ModuleId::new(id).ok()?),
            None => None,
        },
    };
    Some(StoredVariable {
        definition,
        origin: VariableOrigin::parse(&origin),
        created_at,
        updated_at,
    })
}

pub(crate) fn list(connection: &Connection) -> Result<Vec<StoredVariable>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {DEFINITION_COLUMNS} FROM shared_variables ORDER BY key"
    ))?;
    let rows = statement
        .query_map([], definition_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().filter_map(decode_definition).collect())
}

pub(crate) fn get(connection: &Connection, key: &VariableKey) -> Result<Option<StoredVariable>> {
    let row = connection
        .query_row(
            &format!("SELECT {DEFINITION_COLUMNS} FROM shared_variables WHERE key = ?1"),
            [key.as_str()],
            definition_row,
        )
        .optional()?;
    Ok(row.and_then(decode_definition))
}

fn scope_of(scope: &str, profile: Option<&str>, project: Option<&str>) -> Option<ScopeKey> {
    let profile = || -> Option<ProfileId> { parse_id(profile?).ok() };
    match scope {
        "global" => Some(ScopeKey::Global),
        "profile" => Some(ScopeKey::Profile {
            profile: profile()?,
        }),
        "project" => Some(ScopeKey::Project {
            profile: profile()?,
            project: parse_id::<ProjectId>(project?).ok()?,
        }),
        _ => None,
    }
}

/// The columns of a stored scope key; `None` for the execution scope.
fn scope_columns(scope: &ScopeKey) -> Option<(&'static str, Option<String>, Option<String>)> {
    match scope {
        ScopeKey::Global => Some(("global", None, None)),
        ScopeKey::Profile { profile } => Some(("profile", Some(profile.to_string()), None)),
        ScopeKey::Project { profile, project } => Some((
            "project",
            Some(profile.to_string()),
            Some(project.to_string()),
        )),
        ScopeKey::Execution { .. } => None,
    }
}

/// Values matching `filter` (an SQL condition on `shared_variable_values`)
/// with `arguments`. Rows this build cannot read are ignored.
fn values_where(
    connection: &Connection,
    filter: &str,
    arguments: impl rusqlite::Params,
) -> Result<Vec<StoredValue>> {
    let mut statement = connection.prepare(&format!(
        "SELECT key, scope, profile_id, project_id, value, updated_at
         FROM shared_variable_values WHERE {filter}
         ORDER BY key, scope, profile_id, project_id"
    ))?;
    let rows = statement
        .query_map(arguments, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(key, scope, profile, project, value, updated_at)| {
            Some(StoredValue {
                key: VariableKey::new(key).ok()?,
                scope: scope_of(&scope, profile.as_deref(), project.as_deref())?,
                value: serde_json::from_str(&value).ok()?,
                updated_at,
            })
        })
        .collect())
}

pub(crate) fn values_of(connection: &Connection, key: &VariableKey) -> Result<Vec<StoredValue>> {
    values_where(connection, "key = ?1", [key.as_str()])
}

/// Global values, and the values of one profile and its projects.
pub(crate) fn values_for_profile(
    connection: &Connection,
    profile: ProfileId,
) -> Result<Vec<StoredValue>> {
    values_where(
        connection,
        "profile_id IS NULL OR profile_id = ?1",
        [profile.to_string()],
    )
}

/// Values of one profile and its projects, without global values.
pub(crate) fn values_of_profile(
    connection: &Connection,
    profile: ProfileId,
) -> Result<Vec<StoredValue>> {
    values_where(connection, "profile_id = ?1", [profile.to_string()])
}

pub(crate) fn global_values(connection: &Connection) -> Result<Vec<StoredValue>> {
    values_where(connection, "scope = 'global'", [])
}

/// Inserts or updates a user definition inside the caller's transaction.
/// `origin` applies to a new definition only; an existing one keeps its
/// origin and creation time.
pub(crate) fn save(
    tx: &Connection,
    definition: &VariableDefinition,
    origin: VariableOrigin,
) -> Result<()> {
    if let Some(module) = &definition.module {
        return Err(StorageError::Invalid(format!(
            "`{}` would be a variable of the module `{module}`; modules declare their own variables",
            definition.key
        )));
    }
    if let Some(issue) = definition.issues().into_iter().next() {
        return Err(issue.into());
    }
    if let Some(existing) = get(tx, &definition.key)? {
        if let Some(module) = existing.definition.module {
            return Err(StorageError::VariableKeyTaken {
                key: definition.key.to_string(),
                module: module.to_string(),
            });
        }
        // Never drop values silently: values that the new definition does
        // not accept (another type, a scope no longer allowed) must be
        // cleared by the user first.
        let misfits = values_of(tx, &definition.key)?
            .iter()
            .filter(|stored| {
                definition
                    .check_value(&stored.scope, &stored.value)
                    .is_err()
            })
            .count();
        if misfits > 0 {
            return Err(StorageError::VariableValuesConflict {
                key: definition.key.to_string(),
                count: misfits,
            });
        }
    } else if reserved_by_unreadable_row(tx, &definition.key)? {
        return Err(StorageError::Invalid(format!(
            "`{}` is stored in a form this DeckPilot cannot read; it is left unchanged",
            definition.key
        )));
    }
    write_definition(tx, definition, None, origin)
}

/// A row with this key exists but could not be decoded.
fn reserved_by_unreadable_row(tx: &Connection, key: &VariableKey) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM shared_variables WHERE key = ?1)",
        [key.as_str()],
        |row| row.get(0),
    )?)
}

fn write_definition(
    tx: &Connection,
    definition: &VariableDefinition,
    module: Option<&ModuleId>,
    origin: VariableOrigin,
) -> Result<()> {
    let time = now();
    tx.execute(
        "INSERT INTO shared_variables (key, label, description, value_type, scopes,
            default_value, sensitive, module_id, origin, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
         ON CONFLICT (key) DO UPDATE SET
            label = excluded.label,
            description = excluded.description,
            value_type = excluded.value_type,
            scopes = excluded.scopes,
            default_value = excluded.default_value,
            sensitive = excluded.sensitive,
            module_id = excluded.module_id,
            updated_at = excluded.updated_at",
        params![
            definition.key.as_str(),
            definition.label,
            definition.description,
            encode(&definition.value_type)?,
            encode(&definition.scopes)?,
            definition.default.as_ref().map(encode).transpose()?,
            definition.sensitive,
            module.map(ModuleId::as_str),
            origin.as_str(),
            time,
        ],
    )?;
    Ok(())
}

/// Declares the variables of `module`. A key already held by a user
/// variable or another module is not declared and is reported instead; the
/// existing definition and its values stay as they are.
pub(crate) fn declare(
    tx: &Connection,
    module: &ModuleId,
    definitions: &[VariableDefinition],
) -> Result<Vec<DeclarationConflict>> {
    let mut conflicts = Vec::new();
    for definition in definitions {
        if let Some(issue) = definition.issues().into_iter().next() {
            return Err(issue.into());
        }
        let existing = get(tx, &definition.key)?;
        match existing.as_ref().map(|e| e.definition.module.as_ref()) {
            Some(holder) if holder != Some(module) => {
                conflicts.push(DeclarationConflict {
                    key: definition.key.clone(),
                    holder: holder.cloned(),
                });
                continue;
            }
            None if reserved_by_unreadable_row(tx, &definition.key)? => {
                conflicts.push(DeclarationConflict {
                    key: definition.key.clone(),
                    holder: None,
                });
                continue;
            }
            _ => {}
        }
        let mut declared = definition.clone();
        declared.module = Some(module.clone());
        write_definition(tx, &declared, Some(module), VariableOrigin::User)?;
    }
    Ok(conflicts)
}

/// Deletes a user variable and its values. Module variables are removed
/// with their module only.
pub(crate) fn delete(tx: &Connection, key: &VariableKey) -> Result<bool> {
    if let Some(existing) = get(tx, key)?
        && let Some(module) = existing.definition.module
    {
        return Err(StorageError::VariableKeyTaken {
            key: key.to_string(),
            module: module.to_string(),
        });
    }
    tx.execute(
        "DELETE FROM provenance WHERE entity = 'variable' AND entity_id = ?1",
        [key.as_str()],
    )?;
    Ok(tx.execute(
        "DELETE FROM shared_variables WHERE key = ?1",
        [key.as_str()],
    )? > 0)
}

fn profile_exists(tx: &Connection, profile: ProfileId) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM profiles WHERE id = ?1)",
        [profile.to_string()],
        |row| row.get(0),
    )?)
}

fn project_profile(tx: &Connection, project: ProjectId) -> Result<Option<ProfileId>> {
    let owner: Option<String> = tx
        .query_row(
            "SELECT profile_id FROM projects WHERE id = ?1",
            [project.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(owner.map(|owner| parse_id(&owner)).transpose()?)
}

/// Sets the value at `scope` inside the caller's transaction, replacing an
/// earlier value there.
pub(crate) fn set_value(
    tx: &Connection,
    key: &VariableKey,
    scope: &ScopeKey,
    value: &VariableValue,
) -> Result<()> {
    let Some(stored) = get(tx, key)? else {
        return Err(StorageError::UnknownVariable(key.to_string()));
    };
    let Some((scope_name, profile_id, project_id)) = scope_columns(scope) else {
        return Err(StorageError::Invalid(
            "values of the execution scope exist only during a run and are never stored".into(),
        ));
    };
    stored.definition.check_value(scope, value)?;
    let owner = match scope {
        ScopeKey::Profile { profile } | ScopeKey::Project { profile, .. } => Some(*profile),
        ScopeKey::Global | ScopeKey::Execution { .. } => None,
    };
    if let Some(owner) = owner {
        if !profile_exists(tx, owner)? {
            return Err(StorageError::UnknownProfile);
        }
        // A value of a profile or project names only that profile's data.
        match value {
            VariableValue::ProjectId(project) if project_profile(tx, *project)? != Some(owner) => {
                return Err(StorageError::ProjectOfAnotherProfile(*project));
            }
            VariableValue::ProfileId(profile) if *profile != owner => {
                return Err(StorageError::Invalid(format!(
                    "`{key}`: a value of one profile cannot name another profile"
                )));
            }
            _ => {}
        }
    }
    clear_value(tx, key, scope)?;
    tx.execute(
        "INSERT INTO shared_variable_values (key, scope, profile_id, project_id, value, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            key.as_str(),
            scope_name,
            profile_id,
            project_id,
            encode(value)?,
            now()
        ],
    )
    .map_err(|error| match scope {
        // The composite foreign key: the project is not one of the profile.
        ScopeKey::Project { project, .. } if constraint_violation(&error) => {
            StorageError::ProjectOfAnotherProfile(*project)
        }
        _ => error.into(),
    })?;
    Ok(())
}

pub(crate) fn clear_value(tx: &Connection, key: &VariableKey, scope: &ScopeKey) -> Result<bool> {
    let Some((scope_name, profile_id, project_id)) = scope_columns(scope) else {
        return Ok(false);
    };
    Ok(tx.execute(
        "DELETE FROM shared_variable_values WHERE key = ?1 AND scope = ?2
            AND IFNULL(profile_id, '') = IFNULL(?3, '') AND IFNULL(project_id, '') = IFNULL(?4, '')",
        params![key.as_str(), scope_name, profile_id, project_id],
    )? > 0)
}

/// The shared variables as seen from one profile: every definition, the
/// global values and the values of the profile and its projects. Values of
/// other profiles are not part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedVariables {
    profile: ProfileId,
    projects: BTreeSet<ProjectId>,
    definitions: BTreeMap<VariableKey, VariableDefinition>,
    values: BTreeMap<VariableKey, Vec<(ScopeKey, VariableValue)>>,
}

impl SharedVariables {
    pub(crate) fn load(connection: &Connection, profile: ProfileId) -> Result<Self> {
        let projects = connection
            .prepare("SELECT id FROM projects WHERE profile_id = ?1")?
            .query_map([profile.to_string()], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .map(|id| parse_id(id))
            .collect::<rusqlite::Result<_>>()?;
        let definitions = list(connection)?
            .into_iter()
            .map(|stored| (stored.definition.key.clone(), stored.definition))
            .collect();
        let mut values: BTreeMap<VariableKey, Vec<(ScopeKey, VariableValue)>> = BTreeMap::new();
        for stored in values_for_profile(connection, profile)? {
            values
                .entry(stored.key)
                .or_default()
                .push((stored.scope, stored.value));
        }
        Ok(Self {
            profile,
            projects,
            definitions,
            values,
        })
    }

    pub fn profile(&self) -> ProfileId {
        self.profile
    }

    pub fn definitions(&self) -> impl Iterator<Item = &VariableDefinition> {
        self.definitions.values()
    }

    pub fn definition(&self, key: &VariableKey) -> Option<&VariableDefinition> {
        self.definitions.get(key)
    }

    /// Stored values of `key` visible to the profile.
    pub fn values(&self, key: &VariableKey) -> &[(ScopeKey, VariableValue)] {
        self.values.get(key).map_or(&[], Vec::as_slice)
    }

    /// Resolves `shared.<key>` in `context`. `execution`: values of the
    /// run, which are never stored; the agent passes them. Permissions are
    /// not checked here (the caller's capabilities decide).
    pub fn resolve(
        &self,
        key: &VariableKey,
        context: &ResolutionContext,
        execution: &[(ScopeKey, VariableValue)],
    ) -> Resolution {
        if context.profile != self.profile {
            return Resolution::Missing(MissingReason::OtherProfile);
        }
        let Some(definition) = self.definitions.get(key) else {
            return Resolution::Missing(MissingReason::UnknownVariable);
        };
        let candidates = self
            .values(key)
            .iter()
            .chain(execution)
            .map(|(scope, value)| (scope, value));
        let Some((value, source)) = select_value(definition, candidates, context) else {
            return Resolution::Missing(MissingReason::NoValue);
        };
        let foreign = match value {
            VariableValue::ProjectId(id) => !self.projects.contains(id),
            VariableValue::ProfileId(id) => *id != self.profile,
            _ => false,
        };
        if foreign {
            return Resolution::Missing(MissingReason::OtherProfile);
        }
        Resolution::Value {
            value: value.clone(),
            source,
            sensitive: definition.sensitive,
        }
    }
}

#[cfg(test)]
mod tests;
