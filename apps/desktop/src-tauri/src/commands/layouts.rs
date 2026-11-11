use serde::Serialize;
use tauri::State;

use deckpilot_core::{
    ActiveProjectContext, DeviceLayout, KeyFace, LayoutId, PageId, ProfileId, ProjectId, page_faces,
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
    modify_profile(&state, profile_id, |profile| {
        let issues = layout.validate(&state.catalog, profile);
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
