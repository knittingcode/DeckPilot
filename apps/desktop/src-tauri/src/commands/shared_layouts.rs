//! Layouts as resources of their own, shared by profiles: the Layouts page
//! and a profile's assigned layouts (`docs/data-model.md`, "Shared
//! layouts"). Every change is one store transaction; the agent reloads
//! afterwards.

use serde::Serialize;
use tauri::State;

use deckpilot_core::shared_layouts::{availability, compatibility_report};
use deckpilot_core::{DeviceLayout, LayoutCompatibility, LayoutId, ProfileId};
use deckpilot_device::DeviceModelId;
use deckpilot_storage::{LayoutAssignment, StoredLayout};

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile};

/// A layout as the Layouts page lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedLayoutSummary {
    id: LayoutId,
    name: String,
    model: DeviceModelId,
    /// `None`: this build does not know the model.
    model_name: Option<String>,
    pages: usize,
    bindings: usize,
    /// Seconds since the Unix epoch.
    updated_at: i64,
    /// The profiles that use it.
    profiles: Vec<LayoutAssignment>,
    /// The fit on every model of the catalog, its own model first.
    compatibility: Vec<LayoutCompatibility>,
}

/// One of a profile's layouts, in the profile's order.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileLayoutEntry {
    layout: SharedLayoutSummary,
    position: usize,
    /// The profile's default layout for the layout's model.
    is_default: bool,
    /// Keys that cannot run in this profile (a workflow or project of
    /// another profile), in words.
    unavailable: Vec<String>,
}

fn summary(state: &AppState, stored: StoredLayout) -> SharedLayoutSummary {
    let layout = stored.layout;
    SharedLayoutSummary {
        model_name: state.catalog.get(&layout.model).map(|m| m.name.clone()),
        pages: layout.pages.len(),
        bindings: layout.pages.iter().map(|p| p.bindings.len()).sum(),
        compatibility: compatibility_report(&layout, &state.catalog),
        updated_at: stored.updated_at,
        profiles: stored.profiles,
        id: layout.id,
        name: layout.name,
        model: layout.model,
    }
}

fn stored(state: &AppState, id: LayoutId) -> CommandResult<StoredLayout> {
    state
        .store()?
        .shared_layout(id)?
        .ok_or(CommandError::NotFound("layout"))
}

fn checked_name(name: &str) -> CommandResult<String> {
    match name.trim() {
        "" => Err(CommandError::Invalid(
            "layout name must not be empty".into(),
        )),
        name => Ok(name.to_owned()),
    }
}

/// A layout may only be assigned when this build knows its model, and when
/// the profile's icons stay within the limit.
fn check_assignable(
    state: &AppState,
    profile: ProfileId,
    layout: &DeviceLayout,
) -> CommandResult<()> {
    if state.catalog.get(&layout.model).is_none() {
        return Err(CommandError::Invalid(format!(
            "this DeckPilot does not know the device model of “{}” ({}); it is kept, but cannot \
             be used",
            layout.name, layout.model
        )));
    }
    let store = state.store()?;
    let current = load_profile(&store, profile)?;
    let mut icons: std::collections::BTreeSet<_> = current.icons.iter().map(|i| i.id).collect();
    icons.extend(layout.shown_icons());
    if icons.len() > deckpilot_core::MAX_ICONS {
        return Err(CommandError::Invalid(format!(
            "with “{}”, the profile would show more than {} icons",
            layout.name,
            deckpilot_core::MAX_ICONS
        )));
    }
    Ok(())
}

#[tauri::command]
pub fn list_shared_layouts(state: State<'_, AppState>) -> CommandResult<Vec<SharedLayoutSummary>> {
    let layouts = state.store()?.shared_layouts()?;
    Ok(layouts
        .into_iter()
        .map(|stored| summary(&state, stored))
        .collect())
}

/// Creates an empty layout for `model_id`, used by no profile unless
/// `assign_to` names one.
#[tauri::command]
pub fn create_shared_layout(
    state: State<'_, AppState>,
    model_id: DeviceModelId,
    name: String,
    assign_to: Option<ProfileId>,
) -> CommandResult<SharedLayoutSummary> {
    let model = state
        .catalog
        .get(&model_id)
        .ok_or(CommandError::NotFound("device model"))?;
    let name = match name.trim() {
        "" => format!("{} layout", model.name),
        name => name.to_owned(),
    };
    let layout = DeviceLayout::new(name, model_id);
    if let Some(profile) = assign_to {
        check_assignable(&state, profile, &layout)?;
    }
    state.store()?.create_shared_layout(&layout, assign_to)?;
    crate::agent::notify_reload(&state.paths);
    Ok(summary(&state, stored(&state, layout.id)?))
}

#[tauri::command]
pub fn rename_shared_layout(
    state: State<'_, AppState>,
    layout_id: LayoutId,
    name: String,
) -> CommandResult<()> {
    state
        .store()?
        .rename_shared_layout(layout_id, &checked_name(&name)?)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

/// An independent copy: new layout and page ids; its keys run the same
/// actions of the Action Library.
#[tauri::command]
pub fn duplicate_shared_layout(
    state: State<'_, AppState>,
    layout_id: LayoutId,
    name: String,
    assign_to: Option<ProfileId>,
) -> CommandResult<SharedLayoutSummary> {
    let name = checked_name(&name)?;
    if let Some(profile) = assign_to {
        check_assignable(&state, profile, &stored(&state, layout_id)?.layout)?;
    }
    let copy = state
        .store()?
        .duplicate_shared_layout(layout_id, &name, assign_to)?;
    crate::agent::notify_reload(&state.paths);
    Ok(summary(&state, stored(&state, copy.id)?))
}

/// Deletes a layout. `confirmed_profiles` are the profiles the user saw in
/// the confirmation; if others use it by now, nothing is deleted and the
/// error names them.
#[tauri::command]
pub fn delete_shared_layout(
    state: State<'_, AppState>,
    layout_id: LayoutId,
    confirmed_profiles: Vec<ProfileId>,
) -> CommandResult<()> {
    state
        .store()?
        .delete_shared_layout(layout_id, &confirmed_profiles)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

#[tauri::command]
pub fn assign_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
) -> CommandResult<()> {
    check_assignable(&state, profile_id, &stored(&state, layout_id)?.layout)?;
    state.store()?.assign_layout(profile_id, layout_id)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

/// Removes a layout from a profile; the layout is kept.
#[tauri::command]
pub fn unassign_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
) -> CommandResult<()> {
    state.store()?.unassign_layout(profile_id, layout_id)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

#[tauri::command]
pub fn reorder_profile_layouts(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    order: Vec<LayoutId>,
) -> CommandResult<()> {
    state.store()?.reorder_profile_layouts(profile_id, &order)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

/// Makes a layout the profile's default for its model; devices of the
/// model show it.
#[tauri::command]
pub fn set_default_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
) -> CommandResult<()> {
    state.store()?.set_default_layout(profile_id, layout_id)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

/// A profile's layouts in its order, with what of each cannot run in it.
#[tauri::command]
pub fn profile_layouts(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<Vec<ProfileLayoutEntry>> {
    let store = state.store()?;
    let profile = load_profile(&store, profile_id)?;
    let mut entries = Vec::new();
    for (position, layout) in profile.layouts.iter().enumerate() {
        let Some(stored) = store.shared_layout(layout.id)? else {
            continue;
        };
        entries.push(ProfileLayoutEntry {
            is_default: profile.active_layouts.get(&layout.model) == Some(&layout.id),
            unavailable: availability(layout, &profile)
                .iter()
                .map(ToString::to_string)
                .collect(),
            position,
            layout: summary(&state, stored),
        });
    }
    Ok(entries)
}
