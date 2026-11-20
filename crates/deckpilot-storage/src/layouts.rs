//! Shared layouts: layouts, the actions their keys run and the icons they
//! show are stored once, in tables of their own, and profiles refer to
//! them (schema 10, see `docs/data-model.md`, "Shared layouts").
//!
//! - `layouts`: one row per layout (stable id, name, model, document).
//! - `profile_layouts`: which profiles use a layout, in each profile's
//!   order, and the profile's default layout per model (`is_default`, at
//!   most one per profile and model). A profile's `activeLayouts` are its
//!   defaults.
//! - `library_actions` and `library_icons`: the Action Library's action
//!   instances and the key icons, one record each.
//! - `profile_actions`, `profile_icons`: the actions and icons a profile
//!   shows, in its order; `own` marks those it has of its own (its
//!   library), the others it has through a layout. `layout_actions`,
//!   `layout_icons`: those a layout's bindings run and show. A profile sees its own plus those of
//!   the layouts it uses, so a layout behaves the same in every profile;
//!   projects, variables and workflows stay the profile's.
//! - `device_layouts`: runtime state of this computer, the layout a device
//!   switched to in a profile (*Switch layout*). It references the
//!   profile's assignment, so it disappears with it.
//!
//! [`Profile`] keeps its shape: [`compose`] fills `layouts`,
//! `activeLayouts`, `actions` and `icons` when a profile is loaded, and
//! [`decompose`] stores them when it is saved. Editing a shared layout or
//! action through any profile edits the one record; removing a layout from
//! a profile only removes the assignment. Records nothing refers to any
//! more are removed ([`collect_garbage`]); layouts never are, they are
//! deleted explicitly ([`SqliteStore::delete_shared_layout`]).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use deckpilot_core::shared_layouts::action_closure;
use deckpilot_core::{ActionInstance, DeviceLayout, Icon, LayoutId, Profile, ProfileId};
use deckpilot_device::DeviceId;

use crate::sqlite::{SqliteStore, decode, encode, now, parse_id};
use crate::{Result, StorageError};

/// Version of the documents in `layouts`, `library_actions` and
/// `library_icons`: the JSON of a [`DeviceLayout`], [`ActionInstance`] and
/// [`Icon`] as profile documents of version 5 contained them.
pub(crate) const ITEM_DOCUMENT_VERSION: u32 = 1;

pub(crate) const SCHEMA: &str = r"
    CREATE TABLE layouts (
        id               TEXT PRIMARY KEY NOT NULL,
        name             TEXT NOT NULL,
        model            TEXT NOT NULL,
        document_version INTEGER NOT NULL,
        document         TEXT NOT NULL,
        created_at       INTEGER NOT NULL,
        updated_at       INTEGER NOT NULL,
        UNIQUE (id, model)
    );
    CREATE TABLE profile_layouts (
        profile_id TEXT NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        layout_id  TEXT NOT NULL,
        model      TEXT NOT NULL,
        position   INTEGER NOT NULL,
        is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
        PRIMARY KEY (profile_id, layout_id),
        FOREIGN KEY (layout_id, model) REFERENCES layouts (id, model) ON DELETE CASCADE
    );
    CREATE INDEX profile_layouts_layout ON profile_layouts (layout_id);
    CREATE UNIQUE INDEX profile_layouts_default ON profile_layouts (profile_id, model)
        WHERE is_default = 1;
    CREATE TABLE library_actions (
        id               TEXT PRIMARY KEY NOT NULL,
        document_version INTEGER NOT NULL,
        document         TEXT NOT NULL,
        updated_at       INTEGER NOT NULL
    );
    CREATE TABLE profile_actions (
        profile_id TEXT NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        action_id  TEXT NOT NULL REFERENCES library_actions (id) ON DELETE CASCADE,
        position   INTEGER NOT NULL,
        own        INTEGER NOT NULL CHECK (own IN (0, 1)),
        PRIMARY KEY (profile_id, action_id)
    );
    CREATE INDEX profile_actions_action ON profile_actions (action_id);
    CREATE TABLE layout_actions (
        layout_id TEXT NOT NULL REFERENCES layouts (id) ON DELETE CASCADE,
        action_id TEXT NOT NULL REFERENCES library_actions (id) ON DELETE RESTRICT,
        PRIMARY KEY (layout_id, action_id)
    );
    CREATE INDEX layout_actions_action ON layout_actions (action_id);
    CREATE TABLE library_icons (
        id               TEXT PRIMARY KEY NOT NULL,
        document_version INTEGER NOT NULL,
        document         TEXT NOT NULL,
        updated_at       INTEGER NOT NULL
    );
    CREATE TABLE profile_icons (
        profile_id TEXT NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        icon_id    TEXT NOT NULL REFERENCES library_icons (id) ON DELETE CASCADE,
        position   INTEGER NOT NULL,
        own        INTEGER NOT NULL CHECK (own IN (0, 1)),
        PRIMARY KEY (profile_id, icon_id)
    );
    CREATE INDEX profile_icons_icon ON profile_icons (icon_id);
    CREATE TABLE layout_icons (
        layout_id TEXT NOT NULL REFERENCES layouts (id) ON DELETE CASCADE,
        icon_id   TEXT NOT NULL REFERENCES library_icons (id) ON DELETE RESTRICT,
        PRIMARY KEY (layout_id, icon_id)
    );
    CREATE INDEX layout_icons_icon ON layout_icons (icon_id);
    CREATE TABLE device_layouts (
        profile_id TEXT NOT NULL,
        device     TEXT NOT NULL,
        layout_id  TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (profile_id, device),
        FOREIGN KEY (profile_id, layout_id)
            REFERENCES profile_layouts (profile_id, layout_id) ON DELETE CASCADE
    );
";

/// Namespace of identifiers derived in migration 10 for a layout, action
/// or icon whose identifier an earlier profile already used.
const MIGRATION_NAMESPACE: Uuid = Uuid::from_u128(0x5d7c_9a10_3e2f_4b8d_a1c6_70e4_2b9f_d351);

/// A profile that uses a layout ([`StoredLayout::profiles`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutAssignment {
    pub profile_id: ProfileId,
    pub profile_name: String,
    /// Position in the profile's list of layouts, from 0.
    pub position: u32,
    /// The profile's default layout for the layout's model.
    pub is_default: bool,
}

/// A layout with the profiles that use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredLayout {
    pub layout: DeviceLayout,
    /// Seconds since the Unix epoch.
    pub created_at: i64,
    pub updated_at: i64,
    pub profiles: Vec<LayoutAssignment>,
}

fn id_set(connection: &Connection, sql: &str, key: &str) -> Result<BTreeSet<String>> {
    Ok(connection
        .prepare(sql)?
        .query_map([key], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

fn decode_item<T: serde::de::DeserializeOwned>(version: u32, document: &str) -> Result<T> {
    if version != ITEM_DOCUMENT_VERSION {
        return Err(StorageError::UnsupportedDocument(version));
    }
    Ok(serde_json::from_str(document)?)
}

/// Every action of the shared Action Library.
fn library(connection: &Connection) -> Result<Vec<ActionInstance>> {
    let rows: Vec<(u32, String)> = connection
        .prepare("SELECT document_version, document FROM library_actions ORDER BY id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.iter()
        .map(|(version, document)| decode_item(*version, document))
        .collect()
}

fn icon(connection: &Connection, id: &str) -> Result<Option<Icon>> {
    let row: Option<(u32, String)> = connection
        .query_row(
            "SELECT document_version, document FROM library_icons WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(version, document)| decode_item(version, &document))
        .transpose()
}

fn layout_row(connection: &Connection, id: &str) -> Result<Option<DeviceLayout>> {
    let row: Option<(u32, String)> = connection
        .query_row(
            "SELECT document_version, document FROM layouts WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(version, document)| decode_item(version, &document))
        .transpose()
}

/// Fills the shared parts of `profile` (stored from schema 10 on): its
/// layouts in order, its default layout per model, the actions it has and
/// those its layouts run, the icons it has and those its layouts show.
pub(crate) fn compose(connection: &Connection, profile: &mut Profile) -> Result<()> {
    let key = profile.id.to_string();
    let rows: Vec<(u32, String, bool)> = connection
        .prepare(
            "SELECT l.document_version, l.document, pl.is_default
             FROM profile_layouts pl JOIN layouts l ON l.id = pl.layout_id
             WHERE pl.profile_id = ?1 ORDER BY pl.position, pl.layout_id",
        )?
        .query_map([&key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    profile.layouts.clear();
    profile.active_layouts.clear();
    for (version, document, is_default) in rows {
        let layout: DeviceLayout = decode_item(version, &document)?;
        if is_default {
            profile
                .active_layouts
                .insert(layout.model.clone(), layout.id);
        }
        profile.layouts.push(layout);
    }

    let library = library(connection)?;
    let by_id: BTreeMap<String, &ActionInstance> = library
        .iter()
        .map(|action| (action.id.to_string(), action))
        .collect();
    let inherited = action_closure(
        profile.layouts.iter().flat_map(DeviceLayout::bound_actions),
        &library,
    );
    let inherited_set: HashSet<String> = inherited.iter().map(ToString::to_string).collect();
    let rows: Vec<(String, bool)> = connection
        .prepare(
            "SELECT action_id, own FROM profile_actions WHERE profile_id = ?1
             ORDER BY position, action_id",
        )?
        .query_map([&key], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut seen = HashSet::new();
    profile.actions.clear();
    // In the profile's order: its own, and those of its layouts that are
    // still there; then what its layouts use since it was saved last.
    let ordered = rows
        .into_iter()
        .filter(|(id, own)| *own || inherited_set.contains(id))
        .map(|(id, _)| id)
        .chain(inherited.iter().map(ToString::to_string));
    for id in ordered {
        if let Some(action) = by_id.get(&id)
            && seen.insert(action.id)
        {
            profile.actions.push((*action).clone());
        }
    }

    let shown: HashSet<String> = profile
        .layouts
        .iter()
        .flat_map(DeviceLayout::shown_icons)
        .map(|id| id.to_string())
        .collect();
    let rows: Vec<(String, bool)> = connection
        .prepare(
            "SELECT icon_id, own FROM profile_icons WHERE profile_id = ?1
             ORDER BY position, icon_id",
        )?
        .query_map([&key], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let in_layout_order: Vec<String> = profile
        .layouts
        .iter()
        .flat_map(DeviceLayout::shown_icons)
        .map(|id| id.to_string())
        .collect();
    let mut icons_seen = HashSet::new();
    profile.icons.clear();
    let ordered = rows
        .into_iter()
        .filter(|(id, own)| *own || shown.contains(id))
        .map(|(id, _)| id)
        .chain(in_layout_order);
    for id in ordered {
        if icons_seen.insert(id.clone())
            && let Some(icon) = icon(connection, &id)?
        {
            profile.icons.push(icon);
        }
    }
    Ok(())
}

/// Stores the shared parts of `profile` inside the caller's transaction,
/// after its row was written. See the module documentation; the rules for
/// what a profile has of its own:
///
/// - an action or icon only there because of a layout the profile uses
///   now is the layout's;
/// - one only there because of a layout the profile stopped using (and
///   not the profile's before) goes with that layout;
/// - everything else is the profile's own, and so are the layouts'
///   actions that the profile's own actions and workflows name.
#[allow(clippy::too_many_lines)] // One pass over the shared parts, in order.
pub(crate) fn decompose(tx: &Connection, profile: &Profile) -> Result<()> {
    let key = profile.id.to_string();
    let old_members = id_set(
        tx,
        "SELECT action_id FROM profile_actions WHERE profile_id = ?1 AND own = 1",
        &key,
    )?;
    let old_icon_members = id_set(
        tx,
        "SELECT icon_id FROM profile_icons WHERE profile_id = ?1 AND own = 1",
        &key,
    )?;
    let old_assigned: Vec<(String, String, bool)> = tx
        .prepare("SELECT layout_id, model, is_default FROM profile_layouts WHERE profile_id = ?1")?
        .query_map([&key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let assigned: BTreeSet<String> = profile.layouts.iter().map(|l| l.id.to_string()).collect();
    let mut removed = Vec::new();
    for (id, _, _) in &old_assigned {
        if !assigned.contains(id)
            && let Some(layout) = layout_row(tx, id)?
        {
            removed.push(layout);
        }
    }

    // Actions.
    let ids = |list: Vec<deckpilot_core::ActionInstanceId>| -> BTreeSet<String> {
        list.into_iter().map(|id| id.to_string()).collect()
    };
    let current = ids(action_closure(
        profile.layouts.iter().flat_map(DeviceLayout::bound_actions),
        &profile.actions,
    ));
    let gone: BTreeSet<String> = ids(action_closure(
        removed.iter().flat_map(DeviceLayout::bound_actions),
        &profile.actions,
    ))
    .into_iter()
    .filter(|id| !old_members.contains(id) && !current.contains(id))
    .collect();
    let own: Vec<&ActionInstance> = profile
        .actions
        .iter()
        .filter(|a| {
            let id = a.id.to_string();
            !current.contains(&id) && !gone.contains(&id)
        })
        .collect();
    let workflows = serde_json::to_string(&profile.workflows)?;
    let named = profile
        .actions
        .iter()
        .filter(|a| deckpilot_core::shared_layouts::mentions(&workflows, &a.id.to_string()))
        .map(|a| a.id);
    let needed = ids(action_closure(
        named.chain(own.iter().map(|a| a.id)),
        &profile.actions,
    ));
    // Shown by the profile, in its order: everything but what goes with a
    // layout it stopped using; its own are those it needs.
    let shown_actions: Vec<(&ActionInstance, bool)> = profile
        .actions
        .iter()
        .map(|a| (a, needed.contains(&a.id.to_string())))
        .filter(|(a, own)| *own || !gone.contains(&a.id.to_string()))
        .collect();
    let stamp = now();
    for (action, _) in &shown_actions {
        upsert_action(tx, action, stamp)?;
    }

    // Icons.
    let shown = |layouts: &mut dyn Iterator<Item = &DeviceLayout>| -> BTreeSet<String> {
        layouts
            .flat_map(DeviceLayout::shown_icons)
            .map(|id| id.to_string())
            .collect()
    };
    let shown_now = shown(&mut profile.layouts.iter());
    let icons_gone: BTreeSet<String> = shown(&mut removed.iter())
        .into_iter()
        .filter(|id| !old_icon_members.contains(id) && !shown_now.contains(id))
        .collect();
    let mut shown_icons = Vec::new();
    for icon in &profile.icons {
        let id = icon.id.to_string();
        if icons_gone.contains(&id) {
            continue;
        }
        upsert_icon(tx, icon, stamp)?;
        let own = !shown_now.contains(&id);
        shown_icons.push((id, own));
    }

    // Layouts and their references.
    for layout in &profile.layouts {
        upsert_layout(tx, layout, Some(&profile.actions), stamp)?;
    }

    // Assignments, in order, with the defaults.
    let old_defaults: BTreeMap<String, String> = old_assigned
        .iter()
        .filter(|(_, _, is_default)| *is_default)
        .map(|(id, model, _)| (model.clone(), id.clone()))
        .collect();
    for (id, _, _) in &old_assigned {
        if !assigned.contains(id) {
            tx.execute(
                "DELETE FROM profile_layouts WHERE profile_id = ?1 AND layout_id = ?2",
                params![key, id],
            )?;
        }
    }
    tx.execute(
        "UPDATE profile_layouts SET is_default = 0 WHERE profile_id = ?1",
        [&key],
    )?;
    let mut new_defaults = BTreeMap::new();
    for (position, layout) in profile.layouts.iter().enumerate() {
        let is_default = profile.active_layouts.get(&layout.model) == Some(&layout.id);
        if is_default {
            new_defaults.insert(layout.model.to_string(), layout.id.to_string());
        }
        tx.execute(
            "INSERT INTO profile_layouts (profile_id, layout_id, model, position, is_default)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (profile_id, layout_id) DO UPDATE SET
                position = excluded.position, is_default = excluded.is_default",
            params![
                key,
                layout.id.to_string(),
                layout.model.as_str(),
                i64::try_from(position).unwrap_or(i64::MAX),
                is_default
            ],
        )?;
    }
    // A new default for a model: devices of the model that switched to
    // another layout of this profile show the new default.
    let models: BTreeSet<&String> = old_defaults.keys().chain(new_defaults.keys()).collect();
    for model in models {
        if old_defaults.get(model) != new_defaults.get(model) {
            tx.execute(
                "DELETE FROM device_layouts WHERE profile_id = ?1
                 AND layout_id IN (SELECT id FROM layouts WHERE model = ?2)",
                params![key, model],
            )?;
        }
    }

    // What the profile has of its own.
    tx.execute("DELETE FROM profile_actions WHERE profile_id = ?1", [&key])?;
    for (position, (action, own)) in shown_actions.iter().enumerate() {
        tx.execute(
            "INSERT INTO profile_actions (profile_id, action_id, position, own)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                key,
                action.id.to_string(),
                i64::try_from(position).unwrap_or(i64::MAX),
                own
            ],
        )?;
    }
    tx.execute("DELETE FROM profile_icons WHERE profile_id = ?1", [&key])?;
    for (position, (id, own)) in shown_icons.iter().enumerate() {
        tx.execute(
            "INSERT INTO profile_icons (profile_id, icon_id, position, own) VALUES (?1, ?2, ?3, ?4)",
            params![key, id, i64::try_from(position).unwrap_or(i64::MAX), own],
        )?;
    }
    collect_garbage(tx)
}

fn upsert_action(tx: &Connection, action: &ActionInstance, stamp: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO library_actions (id, document_version, document, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (id) DO UPDATE SET
            document_version = excluded.document_version,
            document = excluded.document,
            updated_at = excluded.updated_at
         WHERE library_actions.document <> excluded.document",
        params![
            action.id.to_string(),
            ITEM_DOCUMENT_VERSION,
            encode(action)?,
            stamp
        ],
    )?;
    Ok(())
}

fn upsert_icon(tx: &Connection, icon: &Icon, stamp: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO library_icons (id, document_version, document, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (id) DO UPDATE SET
            document_version = excluded.document_version,
            document = excluded.document,
            updated_at = excluded.updated_at
         WHERE library_icons.document <> excluded.document",
        params![
            icon.id.to_string(),
            ITEM_DOCUMENT_VERSION,
            encode(icon)?,
            stamp
        ],
    )?;
    Ok(())
}

/// Writes `layout`'s record (its model never changes) and the actions and
/// icons it uses: the actions its keys run and those these name (a Multi
/// Action Switch's states), found among `actions` (a profile's, when the
/// layout is saved with it) or else the whole library. References to
/// actions or icons that have no record are not stored (the layout
/// reports them as problems).
pub(crate) fn upsert_layout(
    tx: &Connection,
    layout: &DeviceLayout,
    actions: Option<&[ActionInstance]>,
    stamp: i64,
) -> Result<()> {
    let id = layout.id.to_string();
    let stored_model: Option<String> = tx
        .query_row("SELECT model FROM layouts WHERE id = ?1", [&id], |row| {
            row.get(0)
        })
        .optional()?;
    if stored_model
        .as_deref()
        .is_some_and(|m| m != layout.model.as_str())
    {
        return Err(StorageError::Invalid(format!(
            "the layout “{}” is made for another device model",
            layout.name
        )));
    }
    tx.execute(
        "INSERT INTO layouts (id, name, model, document_version, document, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
         ON CONFLICT (id) DO UPDATE SET
            name = excluded.name,
            document_version = excluded.document_version,
            document = excluded.document,
            updated_at = excluded.updated_at
         WHERE layouts.document <> excluded.document",
        params![
            id,
            layout.name,
            layout.model.as_str(),
            ITEM_DOCUMENT_VERSION,
            encode(layout)?,
            stamp
        ],
    )?;
    tx.execute("DELETE FROM layout_actions WHERE layout_id = ?1", [&id])?;
    let library_actions;
    let candidates = if let Some(actions) = actions {
        actions
    } else {
        library_actions = library(tx)?;
        &library_actions
    };
    for action in action_closure(layout.bound_actions(), candidates) {
        tx.execute(
            "INSERT OR IGNORE INTO layout_actions (layout_id, action_id)
             SELECT ?1, id FROM library_actions WHERE id = ?2",
            params![id, action.to_string()],
        )?;
    }
    tx.execute("DELETE FROM layout_icons WHERE layout_id = ?1", [&id])?;
    for icon in layout.shown_icons() {
        tx.execute(
            "INSERT OR IGNORE INTO layout_icons (layout_id, icon_id)
             SELECT ?1, id FROM library_icons WHERE id = ?2",
            params![id, icon.to_string()],
        )?;
    }
    Ok(())
}

/// Removes actions and icons that no profile has and no layout uses.
/// Layouts are never removed here.
pub(crate) fn collect_garbage(tx: &Connection) -> Result<()> {
    tx.execute(
        "DELETE FROM library_actions
         WHERE id NOT IN (SELECT action_id FROM profile_actions WHERE own = 1)
           AND id NOT IN (SELECT action_id FROM layout_actions)",
        [],
    )?;
    tx.execute(
        "DELETE FROM library_icons
         WHERE id NOT IN (SELECT icon_id FROM profile_icons WHERE own = 1)
           AND id NOT IN (SELECT icon_id FROM layout_icons)",
        [],
    )?;
    Ok(())
}

fn derived_id(profile: &str, old: &str, attempt: u32) -> String {
    Uuid::new_v5(
        &MIGRATION_NAMESPACE,
        format!("{profile}:{old}:{attempt}").as_bytes(),
    )
    .to_string()
}

/// Migration 10: moves every profile's layouts, actions and icons out of
/// its document into the shared tables, assigned to that profile with its
/// order and active layouts as defaults. Nothing becomes shared: each
/// layout stays with the one profile that had it.
///
/// Profiles are processed in id order. An identifier that an earlier
/// profile already used (profile documents copied by hand) gets a new one
/// derived from the profile and the old identifier, and every reference in
/// that profile's document and its provenance follows; the change is
/// recorded in `migration_notes`. The same identifier twice inside one
/// profile: the first keeps it with every reference, the later item gets a
/// derived one. Documents are rewritten in document version 6; each
/// profile's `updated_at` is kept.
pub(crate) fn migrate(tx: &Connection) -> Result<()> {
    let rows: Vec<(String, u32, String, i64)> = tx
        .prepare("SELECT id, document_version, document, updated_at FROM profiles ORDER BY id")?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut seen: HashSet<String> = HashSet::new();
    for (id, version, document, updated_at) in rows {
        let mut value: Value = serde_json::from_str(&document)?;
        let mut notes = Vec::new();
        let mut own: HashSet<String> = HashSet::new();
        let mut renames: Vec<(String, String)> = Vec::new();
        for field in ["layouts", "actions", "icons"] {
            let Some(items) = value.get_mut(field).and_then(Value::as_array_mut) else {
                continue;
            };
            for item in items {
                let Some(old) = item.get("id").and_then(Value::as_str).map(str::to_owned) else {
                    continue;
                };
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let fresh = |seen: &HashSet<String>, own: &HashSet<String>| {
                    (0..u32::MAX)
                        .map(|attempt| derived_id(&id, &old, attempt))
                        .find(|candidate| !seen.contains(candidate) && !own.contains(candidate))
                        .expect("a free identifier exists")
                };
                if own.contains(&old) {
                    let new = fresh(&seen, &own);
                    item["id"] = Value::String(new.clone());
                    notes.push(format!(
                        "“{name}” had the identifier of another item of this profile and got a \
                         new one; references to the identifier still lead to the first item."
                    ));
                    own.insert(new);
                } else if seen.contains(&old) {
                    let new = fresh(&seen, &own);
                    notes.push(format!(
                        "“{name}” had the identifier of an item of another profile and got a \
                         new one; this profile's references follow it. Nothing is shared."
                    ));
                    own.insert(new.clone());
                    renames.push((old, new));
                } else {
                    own.insert(old);
                }
            }
        }
        let mut text = serde_json::to_string(&value)?;
        for (old, new) in &renames {
            text = text.replace(old.as_str(), new);
            tx.execute(
                "UPDATE provenance SET entity_id = ?1 WHERE profile_id = ?2 AND entity_id = ?3",
                params![new, id, old],
            )?;
        }
        seen.extend(own);
        for message in notes {
            tx.execute(
                "INSERT INTO migration_notes (schema_version, profile_id, message, created_at)
                 VALUES (10, ?1, ?2, ?3)",
                params![id, message, now()],
            )?;
        }
        let mut profile: Profile = decode(version, &text)?;
        let profile_id: ProfileId = parse_id(&id)?;
        profile.projects = crate::sqlite::projects_of(tx, profile_id)?;
        profile.default_project = crate::sqlite::default_project_of(tx, profile_id)?;
        crate::sqlite::write_profile_at(tx, &profile, updated_at)?;
    }
    Ok(())
}

/// Migration 11: every folder page of every stored layout gets its Back
/// setting ([`DeviceLayout::assign_folder_backs`]), so the reserved Back
/// key is part of the stored layout and does not depend on later edits.
/// Nothing of the user's is lost: a folder whose reserved key binds nothing
/// or only Back gets the reserved Back (those Back bindings are replaced
/// by it); a folder whose reserved key runs something else keeps its Back
/// where it was (`legacy`), with a migration note that says how to move it.
/// Explicit Back bindings on other keys stay. Layouts of models this build
/// does not know are left as they are (they behave as before). The
/// encrypted backup before every migration holds the previous state;
/// `updated_at` is kept.
pub(crate) fn migrate_folder_backs(tx: &Connection) -> Result<()> {
    let catalog = deckpilot_device::ModelCatalog::builtin();
    let rows: Vec<(String, u32, String)> = tx
        .prepare("SELECT id, document_version, document FROM layouts ORDER BY id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, version, document) in rows {
        let mut layout: DeviceLayout = decode_item(version, &document)?;
        let Some(model) = catalog.get(&layout.model) else {
            continue;
        };
        let report = layout.assign_folder_backs(&model.capabilities);
        if !report.changed() {
            continue;
        }
        tx.execute(
            "UPDATE layouts SET document = ?1 WHERE id = ?2",
            params![encode(&layout)?, id],
        )?;
        let profile: Option<String> = tx
            .query_row(
                "SELECT profile_id FROM profile_layouts WHERE layout_id = ?1
                 ORDER BY profile_id LIMIT 1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        for page in report.legacy.iter().filter_map(|page| layout.page(*page)) {
            tx.execute(
                "INSERT INTO migration_notes (schema_version, profile_id, message, created_at)
                 VALUES (11, ?1, ?2, ?3)",
                params![
                    profile,
                    format!(
                        "Layout “{}”, folder “{}”: its first key already runs something, so its \
                         Back stays on the first free key. To use the reserved Back key, select \
                         the folder's Back key in the layout editor and choose “Move Back to the \
                         first key”.",
                        layout.name, page.name
                    ),
                    now()
                ],
            )?;
        }
    }
    Ok(())
}

/// Migration 12: every folder control of every stored layout
/// ([`deckpilot_core::folder_control`]) keeps only the binding that opens its
/// folder, on the press ([`DeviceLayout::normalize_folder_controls`]). Its
/// other bindings, with its gesture timings, move to the layout's
/// `recovered` list: stored, never run, offered in the folder key's
/// Inspector to be moved to a free key or discarded. Every such key gets a
/// migration note. Shared layouts are rewritten once (every profile that
/// uses them sees the same document); nested folders page by page; layouts
/// without such keys stay byte for byte the same, so the migration is
/// idempotent. Needs no device model. The encrypted backup before every
/// migration holds the previous state; `updated_at` is kept.
pub(crate) fn migrate_folder_controls(tx: &Connection) -> Result<()> {
    let rows: Vec<(String, u32, String)> = tx
        .prepare("SELECT id, document_version, document FROM layouts ORDER BY id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, version, document) in rows {
        let mut layout: DeviceLayout = decode_item(version, &document)?;
        let before = layout.clone();
        let report = layout.normalize_folder_controls();
        if layout == before {
            continue;
        }
        tx.execute(
            "UPDATE layouts SET document = ?1 WHERE id = ?2",
            params![encode(&layout)?, id],
        )?;
        let profile: Option<String> = tx
            .query_row(
                "SELECT profile_id FROM profile_layouts WHERE layout_id = ?1
                 ORDER BY profile_id LIMIT 1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        for normalized in &report {
            let name = |page| {
                layout
                    .page(page)
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            };
            tx.execute(
                "INSERT INTO migration_notes (schema_version, profile_id, message, created_at)
                 VALUES (12, ?1, ?2, ?3)",
                params![
                    profile,
                    format!(
                        "Layout “{}”, page “{}”: the key that opens the folder “{}” now only \
                         opens it. Its other {} gesture(s) were kept as “Recovered gestures” \
                         and do not run; select the folder key in the layout editor to move \
                         them to a free key or discard them.",
                        layout.name,
                        name(normalized.page),
                        name(normalized.folder),
                        normalized.recovered
                    ),
                    now()
                ],
            )?;
        }
    }
    Ok(())
}

/// Migration 13: the key that opens a folder shows the folder's name, its
/// one title ([`DeviceLayout::normalize_folder_labels`]). A title of the
/// key's own equal to the name is dropped; one that differs moves, with the
/// binding as it was, to the layout's `recovered` list (offered in the
/// folder key's Inspector: use it as the folder's name, or discard it), and
/// the key gets a migration note. Shared layouts are rewritten once; layouts
/// without such titles stay byte for byte the same, so the migration is
/// idempotent. The encrypted backup before every migration holds the
/// previous state; `updated_at` is kept.
pub(crate) fn migrate_folder_labels(tx: &Connection) -> Result<()> {
    let rows: Vec<(String, u32, String)> = tx
        .prepare("SELECT id, document_version, document FROM layouts ORDER BY id")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, version, document) in rows {
        let mut layout: DeviceLayout = decode_item(version, &document)?;
        let before = layout.clone();
        let report = layout.normalize_folder_labels();
        if layout == before {
            continue;
        }
        tx.execute(
            "UPDATE layouts SET document = ?1 WHERE id = ?2",
            params![encode(&layout)?, id],
        )?;
        let profile: Option<String> = tx
            .query_row(
                "SELECT profile_id FROM profile_layouts WHERE layout_id = ?1
                 ORDER BY profile_id LIMIT 1",
                [&id],
                |row| row.get(0),
            )
            .optional()?;
        for normalized in &report {
            let name = |page| {
                layout
                    .page(page)
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            };
            tx.execute(
                "INSERT INTO migration_notes (schema_version, profile_id, message, created_at)
                 VALUES (13, ?1, ?2, ?3)",
                params![
                    profile,
                    format!(
                        "Layout “{}”, page “{}”: the key that opens the folder “{}” now shows \
                         the folder's name. Its own title “{}” was kept; select the folder key \
                         in the layout editor to use it as the folder's name or discard it.",
                        layout.name,
                        name(normalized.page),
                        name(normalized.folder),
                        normalized.label
                    ),
                    now()
                ],
            )?;
        }
    }
    Ok(())
}

fn assignments(connection: &Connection, layout: &str) -> Result<Vec<LayoutAssignment>> {
    let rows: Vec<(String, String, i64, bool)> = connection
        .prepare(
            "SELECT pl.profile_id, p.name, pl.position, pl.is_default
             FROM profile_layouts pl JOIN profiles p ON p.id = pl.profile_id
             WHERE pl.layout_id = ?1 ORDER BY p.name COLLATE NOCASE, p.id",
        )?
        .query_map([layout], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(profile, name, position, is_default)| {
            Ok(LayoutAssignment {
                profile_id: parse_id(&profile)?,
                profile_name: name,
                position: u32::try_from(position).unwrap_or(u32::MAX),
                is_default,
            })
        })
        .collect()
}

impl SqliteStore {
    /// Every layout, by name, with the profiles that use it.
    pub fn shared_layouts(&self) -> Result<Vec<StoredLayout>> {
        let connection = self.connection();
        let rows: Vec<(String, u32, String, i64, i64)> = connection
            .prepare(
                "SELECT id, document_version, document, created_at, updated_at FROM layouts
                 ORDER BY name COLLATE NOCASE, id",
            )?
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(id, version, document, created_at, updated_at)| {
                Ok(StoredLayout {
                    layout: decode_item(version, &document)?,
                    created_at,
                    updated_at,
                    profiles: assignments(connection, &id)?,
                })
            })
            .collect()
    }

    /// One layout with the profiles that use it.
    pub fn shared_layout(&self, id: LayoutId) -> Result<Option<StoredLayout>> {
        let connection = self.connection();
        let row: Option<(u32, String, i64, i64)> = connection
            .query_row(
                "SELECT document_version, document, created_at, updated_at FROM layouts
                 WHERE id = ?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        row.map(|(version, document, created_at, updated_at)| {
            Ok(StoredLayout {
                layout: decode_item(version, &document)?,
                created_at,
                updated_at,
                profiles: assignments(connection, &id.to_string())?,
            })
        })
        .transpose()
    }

    /// Stores a new layout, used by no profile unless `assign_to` names
    /// one. Its keys may only run actions and show icons that are stored.
    pub fn create_shared_layout(
        &self,
        layout: &DeviceLayout,
        assign_to: Option<ProfileId>,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        if layout_row(&tx, &layout.id.to_string())?.is_some() {
            return Err(StorageError::Invalid("the layout exists already".into()));
        }
        upsert_layout(&tx, layout, None, now())?;
        if let Some(profile) = assign_to {
            assign(&tx, profile, layout)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Renames a layout, for every profile that uses it.
    pub fn rename_shared_layout(&self, id: LayoutId, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StorageError::Invalid(
                "layout name must not be empty".into(),
            ));
        }
        let tx = self.write_transaction()?;
        let mut layout = layout_row(&tx, &id.to_string())?.ok_or(StorageError::UnknownLayout)?;
        name.clone_into(&mut layout.name);
        upsert_layout(&tx, &layout, None, now())?;
        tx.commit()?;
        Ok(())
    }

    /// Stores an independent copy of a layout (new layout and page ids;
    /// see [`DeviceLayout::duplicated`]), used by no profile unless
    /// `assign_to` names one.
    pub fn duplicate_shared_layout(
        &self,
        id: LayoutId,
        name: &str,
        assign_to: Option<ProfileId>,
    ) -> Result<DeviceLayout> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StorageError::Invalid(
                "layout name must not be empty".into(),
            ));
        }
        let tx = self.write_transaction()?;
        let original = layout_row(&tx, &id.to_string())?.ok_or(StorageError::UnknownLayout)?;
        let copy = original.duplicated(name);
        upsert_layout(&tx, &copy, None, now())?;
        if let Some(profile) = assign_to {
            assign(&tx, profile, &copy)?;
        }
        tx.commit()?;
        Ok(copy)
    }

    /// Deletes a layout. `confirmed` must be exactly the profiles that use
    /// it, as the user saw them when confirming; otherwise nothing changes
    /// and [`StorageError::LayoutInUse`] names the profiles that use it
    /// now. The profiles stay; each uses its other layouts (a default the
    /// layout was falls back to the profile's first layout for the model),
    /// and devices that showed it show their profile's layout.
    pub fn delete_shared_layout(&self, id: LayoutId, confirmed: &[ProfileId]) -> Result<()> {
        let tx = self.write_transaction()?;
        let key = id.to_string();
        if layout_row(&tx, &key)?.is_none() {
            return Err(StorageError::UnknownLayout);
        }
        let users = assignments(&tx, &key)?;
        let using: BTreeSet<ProfileId> = users.iter().map(|a| a.profile_id).collect();
        let expected: BTreeSet<ProfileId> = confirmed.iter().copied().collect();
        if using != expected {
            return Err(StorageError::LayoutInUse(
                users
                    .into_iter()
                    .map(|a| (a.profile_id, a.profile_name))
                    .collect(),
            ));
        }
        // What the profiles that used it had only through it goes with it,
        // except what their own actions and workflows need: each profile is
        // stored again without the layout.
        for profile in &using {
            let mut document = crate::sqlite::load_profile_in(&tx, *profile)?
                .ok_or(StorageError::UnknownProfile)?;
            document.layouts.retain(|layout| layout.id != id);
            document.active_layouts.retain(|_, layout| *layout != id);
            crate::sqlite::write_profile_at(&tx, &document, now())?;
        }
        tx.execute("DELETE FROM layouts WHERE id = ?1", [&key])?;
        collect_garbage(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds a layout to the end of a profile's layouts (nothing when the
    /// profile uses it already). The profile sees the actions and icons
    /// the layout uses.
    pub fn assign_layout(&self, profile: ProfileId, layout: LayoutId) -> Result<()> {
        let tx = self.write_transaction()?;
        let stored = layout_row(&tx, &layout.to_string())?.ok_or(StorageError::UnknownLayout)?;
        assign(&tx, profile, &stored)?;
        tx.commit()?;
        Ok(())
    }

    /// Removes a layout from a profile; the layout stays. What the profile
    /// had only through it goes with it, except what the profile's own
    /// actions and workflows use. If it was the profile's default for its
    /// model, the profile's first layout for the model is used.
    pub fn unassign_layout(&self, profile: ProfileId, layout: LayoutId) -> Result<()> {
        let tx = self.write_transaction()?;
        let mut document =
            crate::sqlite::load_profile_in(&tx, profile)?.ok_or(StorageError::UnknownProfile)?;
        let before = document.layouts.len();
        document.layouts.retain(|l| l.id != layout);
        if document.layouts.len() == before {
            return Err(StorageError::UnknownLayout);
        }
        document.active_layouts.retain(|_, l| *l != layout);
        crate::sqlite::write_profile_at(&tx, &document, now())?;
        tx.commit()?;
        Ok(())
    }

    /// Puts a profile's layouts in `order`, which must list each of them
    /// once.
    pub fn reorder_profile_layouts(&self, profile: ProfileId, order: &[LayoutId]) -> Result<()> {
        let tx = self.write_transaction()?;
        let key = profile.to_string();
        let assigned = id_set(
            &tx,
            "SELECT layout_id FROM profile_layouts WHERE profile_id = ?1",
            &key,
        )?;
        let wanted: BTreeSet<String> = order.iter().map(ToString::to_string).collect();
        if wanted != assigned || wanted.len() != order.len() {
            return Err(StorageError::Invalid(
                "the new order must list every layout of the profile once".into(),
            ));
        }
        for (position, layout) in order.iter().enumerate() {
            tx.execute(
                "UPDATE profile_layouts SET position = ?1 WHERE profile_id = ?2 AND layout_id = ?3",
                params![
                    i64::try_from(position).unwrap_or(i64::MAX),
                    key,
                    layout.to_string()
                ],
            )?;
        }
        touch_profile(&tx, &key)?;
        tx.commit()?;
        Ok(())
    }

    /// Makes `layout` the profile's default layout for its model. Devices
    /// of the model that switched to another layout of the profile show it.
    pub fn set_default_layout(&self, profile: ProfileId, layout: LayoutId) -> Result<()> {
        let tx = self.write_transaction()?;
        let key = profile.to_string();
        let model: String = tx
            .query_row(
                "SELECT model FROM profile_layouts WHERE profile_id = ?1 AND layout_id = ?2",
                params![key, layout.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::UnknownLayout)?;
        tx.execute(
            "UPDATE profile_layouts SET is_default = 0 WHERE profile_id = ?1 AND model = ?2",
            params![key, model],
        )?;
        tx.execute(
            "UPDATE profile_layouts SET is_default = 1 WHERE profile_id = ?1 AND layout_id = ?2",
            params![key, layout.to_string()],
        )?;
        tx.execute(
            "DELETE FROM device_layouts WHERE profile_id = ?1
             AND layout_id IN (SELECT id FROM layouts WHERE model = ?2)",
            params![key, model],
        )?;
        touch_profile(&tx, &key)?;
        tx.commit()?;
        Ok(())
    }

    /// The layouts devices switched to, per profile (runtime state of this
    /// computer).
    pub fn device_layouts(&self) -> Result<Vec<(ProfileId, DeviceId, LayoutId)>> {
        let rows: Vec<(String, String, String)> = self
            .connection()
            .prepare("SELECT profile_id, device, layout_id FROM device_layouts ORDER BY profile_id, device")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(profile, device, layout)| {
                Some((
                    profile.parse().ok()?,
                    serde_json::from_str(&device).ok()?,
                    layout.parse().ok()?,
                ))
            })
            .collect())
    }

    /// Replaces the layouts devices switched to. Entries whose layout is
    /// not assigned to their profile, or not made for the device's model,
    /// are not stored.
    pub fn save_device_layouts(&self, layouts: &[(ProfileId, DeviceId, LayoutId)]) -> Result<()> {
        let tx = self.write_transaction()?;
        tx.execute("DELETE FROM device_layouts", [])?;
        let stamp = now();
        for (profile, device, layout) in layouts {
            tx.execute(
                "INSERT OR IGNORE INTO device_layouts (profile_id, device, layout_id, updated_at)
                 SELECT ?1, ?2, ?3, ?4 FROM profile_layouts
                 WHERE profile_id = ?1 AND layout_id = ?3 AND model = ?5",
                params![
                    profile.to_string(),
                    serde_json::to_string(device)?,
                    layout.to_string(),
                    stamp,
                    device.model.as_str()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes layouts no profile uses, except `keep`. For an import that
    /// replaces the whole configuration.
    pub(crate) fn delete_unused_layouts(tx: &Connection, keep: &[LayoutId]) -> Result<()> {
        let unused: Vec<String> = tx
            .prepare(
                "SELECT id FROM layouts WHERE id NOT IN (SELECT layout_id FROM profile_layouts)",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in unused {
            if !keep.iter().any(|k| k.to_string() == id) {
                tx.execute("DELETE FROM layouts WHERE id = ?1", [id])?;
            }
        }
        collect_garbage(tx)
    }

    /// Stores layouts used by no profile, with the actions and icons they
    /// use (from an import). An existing layout with the same id is
    /// replaced only when no profile uses it.
    pub(crate) fn save_unassigned_layouts(
        tx: &Connection,
        layouts: &[DeviceLayout],
        actions: &[ActionInstance],
        icons: &[Icon],
    ) -> Result<()> {
        let stamp = now();
        for action in actions {
            upsert_action(tx, action, stamp)?;
        }
        for icon in icons {
            upsert_icon(tx, icon, stamp)?;
        }
        for layout in layouts {
            let used: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM profile_layouts WHERE layout_id = ?1)",
                [layout.id.to_string()],
                |row| row.get(0),
            )?;
            if !used {
                upsert_layout(tx, layout, None, stamp)?;
            }
        }
        collect_garbage(tx)
    }

    /// The actions and icons `layout` uses, from the shared library (for
    /// exporting layouts no profile uses).
    pub fn layout_library(
        &self,
        layout: &DeviceLayout,
    ) -> Result<(Vec<ActionInstance>, Vec<Icon>)> {
        let connection = self.connection();
        let library = library(connection)?;
        let wanted: BTreeSet<_> = action_closure(layout.bound_actions(), &library)
            .into_iter()
            .collect();
        let actions = library
            .into_iter()
            .filter(|action| wanted.contains(&action.id))
            .collect();
        let mut icons = Vec::new();
        for id in layout.shown_icons() {
            if let Some(icon) = icon(connection, &id.to_string())? {
                icons.push(icon);
            }
        }
        Ok((actions, icons))
    }

    /// The stored document of a layout, action or icon record, if there is
    /// one: an import compares it with its own item of the same id.
    pub fn stored_item(&self, kind: ItemKind, id: &str) -> Result<Option<String>> {
        let table = match kind {
            ItemKind::Layout => "layouts",
            ItemKind::Action => "library_actions",
            ItemKind::Icon => "library_icons",
        };
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT document FROM {table} WHERE id = ?1"),
                [id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The profiles that use layout `id`.
    pub fn layout_users(&self, id: LayoutId) -> Result<Vec<LayoutAssignment>> {
        assignments(self.connection(), &id.to_string())
    }
}

/// A kind of shared record ([`SqliteStore::stored_item`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Layout,
    Action,
    Icon,
}

/// Adds `layout` at the end of `profile`'s layouts, inside the caller's
/// transaction. The profile is stored again so that it sees what the layout
/// uses and keeps what it has.
fn assign(tx: &Connection, profile: ProfileId, layout: &DeviceLayout) -> Result<()> {
    let mut document =
        crate::sqlite::load_profile_in(tx, profile)?.ok_or(StorageError::UnknownProfile)?;
    if document.layouts.iter().any(|l| l.id == layout.id) {
        return Ok(());
    }
    // The profile receives the stored layout, with what it uses.
    let mut with_layout = document.clone();
    with_layout.layouts.push(layout.clone());
    let library = library(tx)?;
    let have: BTreeSet<_> = with_layout.actions.iter().map(|a| a.id).collect();
    for id in action_closure(layout.bound_actions(), &library) {
        if !have.contains(&id)
            && let Some(action) = library.iter().find(|a| a.id == id)
        {
            with_layout.actions.push(action.clone());
        }
    }
    for id in layout.shown_icons() {
        if !with_layout.icons.iter().any(|icon| icon.id == id)
            && let Some(icon) = icon(tx, &id.to_string())?
        {
            with_layout.icons.push(icon);
        }
    }
    document = with_layout;
    crate::sqlite::write_profile_at(tx, &document, now())
}

fn touch_profile(tx: &Connection, key: &str) -> Result<()> {
    tx.execute(
        "UPDATE profiles SET updated_at = ?1 WHERE id = ?2",
        params![now(), key],
    )?;
    Ok(())
}
