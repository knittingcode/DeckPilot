use serde::Serialize;
use tauri::State;

use deckpilot_core::{
    ActiveProjectContext, DeviceLayout, KeyFace, LayoutId, PageDestination, PageId, ProfileId,
    ProjectId, page_faces,
};
use deckpilot_device::{ControlId, DeviceModelId};
use deckpilot_storage::ConfigStore;

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile};

/// Creates a layout for `model`, empty or generated from the profile's
/// actions. It becomes the active layout if the model has none yet.
#[tauri::command]
pub fn create_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    model_id: DeviceModelId,
    name: String,
    generate: bool,
) -> CommandResult<DeviceLayout> {
    let model = state
        .catalog
        .get(&model_id)
        .ok_or(CommandError::NotFound("device model"))?;
    let name = match name.trim() {
        "" => format!("{} layout", model.name),
        name => name.to_owned(),
    };
    modify_profile(&state, profile_id, |profile| {
        let layout = if generate {
            DeviceLayout::generate(name, model, profile)
        } else {
            DeviceLayout::new(name, model_id.clone())
        };
        let first = profile.layout_for(&model_id).is_none();
        profile.layouts.push(layout.clone());
        if first {
            profile.activate_layout(layout.id)?;
        }
        Ok(layout)
    })
}

/// Saves an edited layout. Bindings are checked against the model's
/// capabilities and the profile's actions; problems are reported and nothing
/// is saved.
#[tauri::command]
pub fn save_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout: DeviceLayout,
) -> CommandResult<DeviceLayout> {
    let mut layout = layout;
    super::trim_in_place(&mut layout.name);
    if layout.name.is_empty() {
        return Err(CommandError::Invalid(
            "layout name must not be empty".into(),
        ));
    }
    for binding in layout.pages.iter_mut().flat_map(|page| &mut page.bindings) {
        binding.label = binding
            .label
            .take()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty());
        deckpilot_core::validate_presentation(None, binding.label.as_deref())
            .map_err(CommandError::Invalid)?;
        // Colours and sizes are checked when the layout is read; a default
        // appearance is not stored. Icons must exist (checked below).
        binding.appearance = binding
            .appearance
            .take()
            .and_then(deckpilot_core::KeyAppearance::normalized);
    }
    let registry = state.actions();
    modify_profile(&state, profile_id, |profile| {
        let mut issues = layout.validate(&state.catalog, profile);
        issues.extend(layout.check_overrides(&registry, profile));
        if !issues.is_empty() {
            let messages: Vec<String> = issues.iter().map(ToString::to_string).collect();
            return Err(CommandError::Invalid(messages.join("\n")));
        }
        match profile
            .layouts
            .iter_mut()
            .find(|existing| existing.id == layout.id)
        {
            Some(existing) => {
                if existing.model != layout.model {
                    return Err(deckpilot_core::ProfileIssue::ModelMismatch.into());
                }
                *existing = layout.clone();
            }
            None => profile.layouts.push(layout.clone()),
        }
        Ok(layout)
    })
}

#[tauri::command]
pub fn duplicate_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
    name: String,
) -> CommandResult<LayoutId> {
    modify_profile(&state, profile_id, |profile| {
        Ok(profile.duplicate_layout(layout_id, &name)?)
    })
}

#[tauri::command]
pub fn delete_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        profile.delete_layout(layout_id)?;
        Ok(())
    })
}

/// Makes a layout the one used for its device model.
#[tauri::command]
pub fn activate_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        Ok(profile.activate_layout(layout_id)?)
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlFace {
    control: ControlId,
    face: KeyFace,
    /// With `render`: the key as the device shows it (appearance, icon,
    /// title), as a PNG `data:` URL at the model's image size. `None` for
    /// controls without a display.
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

/// The image size of `control` on `capabilities`, if it has a display.
fn image_size(
    capabilities: &deckpilot_device::DeviceCapabilities,
    control: ControlId,
) -> Option<(u32, u32)> {
    let spec = match control {
        ControlId::Key { .. } => capabilities.keys.and_then(|k| k.image),
        ControlId::TouchZone { .. } => capabilities.touch.as_ref().and_then(|t| t.image),
        ControlId::Encoder { .. } => None,
    }?;
    Some((spec.width, spec.height))
}

/// What every control of a (possibly unsaved) layout page shows for a given
/// project, without a device and without running any action. With
/// `render`, each face also comes as the image the device would show,
/// drawn by the agent's renderer.
#[tauri::command]
pub fn preview_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout: DeviceLayout,
    page_id: PageId,
    project_id: Option<ProjectId>,
    render: Option<bool>,
) -> CommandResult<Vec<ControlFace>> {
    let store = state.store()?;
    let profile = load_profile(&store, profile_id)?;
    let model = state
        .catalog
        .get(&layout.model)
        .ok_or(CommandError::NotFound("device model"))?;
    let context =
        ActiveProjectContext::restore(&profile, project_id.or(store.active_project(profile_id)?));
    let project = context.project(&profile)?;
    let controls = model.capabilities.controls();
    let actions = state.actions();
    let icons = render
        .unwrap_or(false)
        .then(|| deckpilot_render::Icons::from_profile(&profile));
    Ok(
        page_faces(&actions, &layout, page_id, &profile, project, &controls)
            .into_iter()
            .map(|(control, face)| {
                let image = icons.as_ref().and_then(|icons| {
                    let (width, height) = image_size(&model.capabilities, control)?;
                    let frame = deckpilot_render::render_with(&face, width, height, icons);
                    let png = deckpilot_render::frame_png(&frame)?;
                    Some(deckpilot_core::IconData::new(png).data_url())
                });
                ControlFace {
                    control,
                    face,
                    image,
                }
            })
            .collect(),
    )
}

/// One page of a layout's page tree.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageNode {
    id: PageId,
    name: String,
    /// The folder it is in (stored or inferred); `None`: top level.
    parent: Option<PageId>,
    /// Whether `parent` is stored rather than inferred.
    stored: bool,
    /// 0 for top-level pages.
    depth: usize,
    /// The key the runtime adds as Back because the page has none.
    automatic_back: Option<ControlId>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTree {
    /// In layout order.
    pages: Vec<PageNode>,
    /// Problems that do not prevent saving, e.g. folders without Back.
    warnings: Vec<String>,
}

/// The page tree of a (possibly unsaved) layout, with inferred folders for
/// layouts that do not store them, and its warnings.
#[tauri::command]
pub fn page_tree(state: State<'_, AppState>, layout: DeviceLayout) -> PageTree {
    let controls: Vec<ControlId> = state
        .catalog
        .get(&layout.model)
        .map(|model| {
            model
                .capabilities
                .controls()
                .into_iter()
                .filter(|c| model.capabilities.has_display(*c))
                .collect()
        })
        .unwrap_or_default();
    let parents = layout.page_parents();
    PageTree {
        pages: layout
            .pages
            .iter()
            .map(|page| PageNode {
                id: page.id,
                name: page.name.clone(),
                parent: parents.get(&page.id).copied(),
                stored: page.parent.is_some() || layout.explicit_tree,
                depth: layout.ancestors(page.id).len(),
                automatic_back: layout.automatic_back(page.id, &controls),
            })
            .collect(),
        warnings: layout.warnings().iter().map(ToString::to_string).collect(),
    }
}

/// Puts the pages of a saved layout in `order` (every page once). The page
/// tree is stored explicitly.
#[tauri::command]
pub fn reorder_pages(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
    order: Vec<PageId>,
) -> CommandResult<DeviceLayout> {
    modify_profile(&state, profile_id, |profile| {
        let layout = profile
            .layouts
            .iter_mut()
            .find(|l| l.id == layout_id)
            .ok_or(CommandError::NotFound("layout"))?;
        layout.reorder_pages(&order)?;
        Ok(layout.clone())
    })
}

/// The result of moving or copying a page.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageChange {
    /// The page's id in the destination (new for copies).
    page: PageId,
    /// Keys removed from the old layout because they opened moved pages.
    removed_keys: usize,
    /// Every layout that changed, as saved.
    layouts: Vec<DeviceLayout>,
}

fn change_page(
    state: &AppState,
    profile_id: ProfileId,
    layout_id: LayoutId,
    target_layout_id: Option<LayoutId>,
    change: impl FnOnce(&mut deckpilot_core::Profile, PageDestination) -> CommandResult<(PageId, usize)>,
    parent_id: Option<PageId>,
) -> CommandResult<PageChange> {
    let destination = PageDestination {
        layout: target_layout_id.unwrap_or(layout_id),
        parent: parent_id,
    };
    modify_profile(state, profile_id, |profile| {
        let (page, removed_keys) = change(profile, destination)?;
        let layouts = profile
            .layouts
            .iter()
            .filter(|l| l.id == layout_id || l.id == destination.layout)
            .cloned()
            .collect();
        Ok(PageChange {
            page,
            removed_keys,
            layouts,
        })
    })
}

/// Moves a page with everything inside it: into another folder of the same
/// layout (`targetLayoutId` empty), or to another layout whose model
/// supports its keys. `parentId` empty: top level within a layout, the
/// home page of another layout.
#[tauri::command]
pub fn move_page(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
    page_id: PageId,
    target_layout_id: Option<LayoutId>,
    parent_id: Option<PageId>,
) -> CommandResult<PageChange> {
    let catalog = state.catalog.clone();
    change_page(
        &state,
        profile_id,
        layout_id,
        target_layout_id,
        |profile, destination| {
            let removed = profile.move_page(layout_id, page_id, destination, &catalog)?;
            Ok((page_id, removed))
        },
        parent_id,
    )
}

/// Copies a page with everything inside it (new identifiers). `parentId`
/// empty: next to the original within a layout, the home page of another
/// layout.
#[tauri::command]
pub fn copy_page(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
    page_id: PageId,
    target_layout_id: Option<LayoutId>,
    parent_id: Option<PageId>,
) -> CommandResult<PageChange> {
    let catalog = state.catalog.clone();
    change_page(
        &state,
        profile_id,
        layout_id,
        target_layout_id,
        |profile, destination| {
            let copy = profile.copy_page(layout_id, page_id, destination, &catalog)?;
            Ok((copy, 0))
        },
        parent_id,
    )
}
