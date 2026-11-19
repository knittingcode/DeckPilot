use serde::Serialize;
use tauri::State;

use deckpilot_core::{
    ActiveProjectContext, DeviceLayout, KeyFace, LayoutId, LayoutIssue, PageDestination, PageId,
    ProfileId, ProjectId, page_faces,
};
use deckpilot_device::{ControlId, DeviceModelId};
use deckpilot_storage::ConfigStore;

use super::reload::Saved;
use super::{modify_profile, modify_profile_and_reload};
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
/// is saved. Returns once the agent has read it, so its keys work right
/// away.
#[tauri::command]
pub async fn save_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout: DeviceLayout,
) -> CommandResult<Saved<SavedLayout>> {
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
    modify_profile_and_reload(&state, profile_id, |profile| {
        let stored = profile.layouts.iter().find(|l| l.id == layout.id);
        let (issues, kept) = check_for_save(&layout, stored, &state.catalog, &registry, profile);
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
        Ok(SavedLayout {
            layout,
            incompatible: kept.iter().map(ToString::to_string).collect(),
        })
    })
    .await
}

/// A saved layout, with the bindings it keeps that cannot run their
/// action from their control. Serialized as the layout with an
/// `incompatible` field (left out when empty), so the frontend reads it as
/// before.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLayout {
    #[serde(flatten)]
    layout: DeviceLayout,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    incompatible: Vec<String>,
}

/// What saving `layout` over `stored` (the saved version of the same
/// layout, if any) must refuse, and the incompatible bindings it keeps.
///
/// Refused: problems of the layout (`validate`), of binding overrides,
/// and bindings whose action cannot run from their control that are new in
/// this version. An incompatible binding that is stored unchanged (same
/// page, control, trigger and action) is kept and reported: a profile
/// saved before compatibility existed, or imported, never becomes
/// unsavable, and nothing is removed or converted
/// (`docs/device-controls.md`).
fn check_for_save(
    layout: &DeviceLayout,
    stored: Option<&DeviceLayout>,
    catalog: &deckpilot_device::ModelCatalog,
    registry: &deckpilot_core::ActionRegistry,
    profile: &deckpilot_core::Profile,
) -> (Vec<LayoutIssue>, Vec<LayoutIssue>) {
    let mut issues = layout.validate(catalog, profile);
    issues.extend(layout.check_overrides(registry, profile));
    let (refused, kept) = layout.compatibility_for_save(stored, catalog, registry, profile);
    issues.extend(refused);
    (issues, kept)
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

/// Deletes a layout of this profile. A layout other profiles use too is
/// not deleted: it is removed in the Layouts page, which names them, or
/// removed from this profile only.
#[tauri::command]
pub fn delete_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout_id: LayoutId,
) -> CommandResult<()> {
    let store = state.store()?;
    let users = store.layout_users(layout_id)?;
    if !users.iter().any(|user| user.profile_id == profile_id) {
        return Err(deckpilot_core::ProfileIssue::NotFound("layout").into());
    }
    let others: Vec<String> = users
        .iter()
        .filter(|user| user.profile_id != profile_id)
        .map(|user| format!("“{}”", user.profile_name))
        .collect();
    if !others.is_empty() {
        return Err(CommandError::Invalid(format!(
            "the layout is also used by {}; delete it in Layouts, or remove it from this profile",
            others.join(", ")
        )));
    }
    store.delete_shared_layout(layout_id, &[profile_id])?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
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
    /// Why bindings of this control on the page cannot run their action
    /// from it (`docs/device-controls.md`); left out when they all can.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    incompatible: Vec<IncompatibleReason>,
}

/// One binding of a control that cannot run its action from it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncompatibleReason {
    trigger: deckpilot_core::Trigger,
    reason: deckpilot_core::IncompatibilityReason,
    /// The reason in words.
    message: String,
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
    let incompatible: Vec<deckpilot_core::IncompatibleBinding> = layout
        .incompatible_bindings(&model.capabilities, &actions, &profile)
        .into_iter()
        .filter(|found| found.page == page_id)
        .collect();
    Ok(page_faces(
        &actions,
        &layout,
        page_id,
        &profile,
        project,
        &controls,
        Some(&model.capabilities),
    )
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
            incompatible: incompatible
                .iter()
                .filter(|found| found.control == control)
                .map(|found| IncompatibleReason {
                    trigger: found.trigger,
                    message: found.reason.to_string(),
                    reason: found.reason.clone(),
                })
                .collect(),
        }
    })
    .collect())
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

#[cfg(test)]
mod tests {
    use deckpilot_core::{BindingTarget, ControlBinding, Os, Profile, Trigger};
    use deckpilot_device::{ModelCatalog, models};

    use super::*;

    /// A profile with a Stream Deck + layout whose dial rotation runs
    /// `general.log` (keys only): stored before compatibility existed.
    fn stored() -> (Profile, deckpilot_core::ActionRegistry) {
        let registry = deckpilot_modules::builtin(Os::current()).action_registry();
        let mut profile = Profile::new("Work");
        let log = registry
            .create_instance(
                "Log",
                deckpilot_modules::general::log_id(),
                &deckpilot_core::ActionParameters::new().with(
                    "message",
                    deckpilot_core::ParameterValue::Text("hello".into()),
                ),
            )
            .unwrap();
        let mut layout = DeviceLayout::new("Plus", models::stream_deck_plus().id);
        layout.pages[0].bindings = vec![
            ControlBinding::new(
                ControlId::Encoder { index: 0 },
                Trigger::RotateClockwise,
                BindingTarget::action(log.id),
            ),
            ControlBinding::new(
                ControlId::Key { index: 0 },
                Trigger::Press,
                BindingTarget::action(log.id),
            ),
        ];
        profile.actions.push(log);
        profile.layouts.push(layout);
        (profile, registry)
    }

    #[test]
    fn a_stored_incompatible_binding_is_kept_and_a_new_one_refused() {
        let (profile, registry) = stored();
        let catalog = ModelCatalog::builtin();
        let saved = &profile.layouts[0];

        // Saving the stored layout as it is (or with other changes) works:
        // the binding is reported, not removed.
        let mut edited = saved.clone();
        edited.name = "Plus (edited)".into();
        let (refused, kept) = check_for_save(&edited, Some(saved), &catalog, &registry, &profile);
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(kept.len(), 1);
        assert!(kept[0].to_string().contains("encoder 0"), "{}", kept[0]);

        // A new incompatible binding is refused with the reason.
        let action = saved.pages[0].bindings[0].target.clone();
        edited.pages[0].bindings.push(ControlBinding::new(
            ControlId::Encoder { index: 2 },
            Trigger::RotateCounterClockwise,
            action.clone(),
        ));
        let (refused, kept) = check_for_save(&edited, Some(saved), &catalog, &registry, &profile);
        assert_eq!(kept.len(), 1);
        assert_eq!(refused.len(), 1);
        assert!(
            refused[0]
                .to_string()
                .contains("the action cannot be bound to an encoder; it works on keys only"),
            "{}",
            refused[0]
        );

        // A new layout with the same binding: refused.
        let mut copy = saved.clone();
        copy.id = LayoutId::new();
        let (refused, _) = check_for_save(&copy, None, &catalog, &registry, &profile);
        assert_eq!(refused.len(), 1);

        // Keys accept the press action.
        let mut keys = DeviceLayout::new("Keys", models::stream_deck_mk2().id);
        keys.pages[0].bindings.push(ControlBinding::new(
            ControlId::Key { index: 3 },
            Trigger::LongPress,
            action,
        ));
        assert_eq!(
            check_for_save(&keys, None, &catalog, &registry, &profile),
            (Vec::new(), Vec::new())
        );
    }

    #[test]
    fn saved_layouts_serialize_as_before() {
        let (profile, _) = stored();
        let layout = profile.layouts[0].clone();
        let plain = serde_json::to_value(&layout).unwrap();
        let saved = serde_json::to_value(SavedLayout {
            layout: layout.clone(),
            incompatible: Vec::new(),
        })
        .unwrap();
        assert_eq!(saved, plain);
        let reported = serde_json::to_value(SavedLayout {
            layout,
            incompatible: vec!["why".into()],
        })
        .unwrap();
        assert_eq!(reported["incompatible"], serde_json::json!(["why"]));
        assert_eq!(reported["id"], plain["id"]);
    }
}
