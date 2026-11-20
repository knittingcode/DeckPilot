use serde::{Deserialize, Serialize};
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
        return Err(CommandError::invalid(
            "layoutNameEmpty",
            "layout name must not be empty",
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
        if let Some(appearance) = &binding.appearance {
            appearance.check().map_err(CommandError::Invalid)?;
        }
    }
    // Folder pages without a Back setting (moved into a folder, or from
    // before the reserved Back key) get one as migration 11 gives it: the
    // reserved key when it is free or only goes back, else where it was.
    if let Some(model) = state.catalog.get(&layout.model) {
        layout.assign_folder_backs(&model.capabilities);
    }
    tidy_folder_controls(&mut layout);
    // A folder key shows its folder's name: no title of its own is stored
    // (one that differs is kept as recovered, as migration 13 keeps it).
    layout.normalize_folder_labels();
    let registry = state.actions();
    modify_profile_and_reload(&state, profile_id, |profile| {
        let stored = profile.layouts.iter().find(|l| l.id == layout.id);
        let (issues, kept) = check_for_save(&layout, stored, &state.catalog, &registry, profile);
        if !issues.is_empty() {
            return Err(refusal(&issues));
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

/// What saving drops without asking: gesture timings of folder keys
/// (they react to the press only, `deckpilot_core::folder_control`) and
/// recovered bindings of pages that are gone.
fn tidy_folder_controls(layout: &mut DeviceLayout) {
    let folder_keys: Vec<(PageId, Vec<ControlId>)> = layout
        .pages
        .iter()
        .map(|page| {
            let keys = layout.folder_controls(page.id);
            (
                page.id,
                keys.into_iter().map(|(control, _)| control).collect(),
            )
        })
        .collect();
    for (page, keys) in folder_keys {
        if let Some(page) = layout.page_mut(page) {
            page.gestures.retain(|g| !keys.contains(&g.control));
        }
    }
    layout.prune_recovered();
}

/// The refusal of a save with `issues`: every message, with the stable
/// code of the first folder-key refusal (`invalid.folderControl.*`, which
/// the editor translates) when there is one, else the generic `invalid`.
fn refusal(issues: &[LayoutIssue]) -> CommandError {
    let message = issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let Some((reason, issue)) = issues
        .iter()
        .find_map(|issue| issue.folder_control_reason().map(|reason| (reason, issue)))
    else {
        return CommandError::Invalid(message);
    };
    // Folder keys are keys: the key's number as the editor shows it.
    let (key, folder) = match issue {
        LayoutIssue::FolderControlBindings {
            control, folder, ..
        }
        | LayoutIssue::FolderControlTrigger {
            control, folder, ..
        }
        | LayoutIssue::FolderControlReplaced {
            control, folder, ..
        } => (
            match control {
                ControlId::Key { index } => u32::from(*index) + 1,
                _ => 0,
            },
            folder.clone(),
        ),
        _ => (0, String::new()),
    };
    CommandError::invalid_with(
        reason,
        message,
        [("key", key.into()), ("folder", folder.into())],
    )
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
    // Folder keys stay folder keys; recovered bindings are only kept,
    // moved or discarded (`deckpilot_core::folder_control`).
    issues.extend(layout.folder_control_edits(stored));
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
        let profiles = others.join(", ");
        return Err(CommandError::invalid_with(
            "layoutUsedElsewhere",
            format!(
                "the layout is also used by {profiles}; delete it in Layouts, or remove it from \
                 this profile"
            ),
            [("profiles", profiles.into())],
        ));
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
    /// title), as a PNG `data:` URL at the model's image size, or at the
    /// size the preview asked for (`key_pixels`). `None` for
    /// controls without a display.
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    /// Why bindings of this control on the page cannot run their action
    /// from it (`docs/device-controls.md`); left out when they all can.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    incompatible: Vec<IncompatibleReason>,
    /// With `render`: how the title comes out on the device
    /// (`docs/key-text.md`), measured at the device's own key size. Left
    /// out for controls without a display or a title.
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<TextLayoutReport>,
    /// What pressing the control does to the view, for the editor's
    /// interactive preview (`docs/editor-navigation-and-simulation.md`):
    /// open a folder, go back (also the automatic Back key, which is not a
    /// binding: `automatic`), turn the page, switch the layout. Read from
    /// the binding and the action type's `editorNavigation`; nothing runs.
    /// Left out for controls that do not navigate.
    #[serde(skip_serializing_if = "Option::is_none")]
    navigation: Option<deckpilot_core::ControlNavigation>,
}

/// How a key's title comes out on the device ([`deckpilot_render::TextReport`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLayoutReport {
    mode: deckpilot_core::TextMode,
    /// All of the title is shown (a scrolling title shows it in turn).
    fits: bool,
    /// Drawn cut: with `…`, clipped, or with lines left out.
    truncated: bool,
    /// The size it is drawn at in points (pixels on a 72 px key), to a
    /// tenth.
    font_size: f32,
    lines: usize,
    /// It scrolls on the device (a marquee title wider than the key).
    needs_scroll: bool,
}

impl From<deckpilot_render::TextReport> for TextLayoutReport {
    fn from(report: deckpilot_render::TextReport) -> Self {
        Self {
            mode: report.mode,
            fits: report.fits,
            truncated: report.truncated,
            font_size: (report.font_points * 10.0).round() / 10.0,
            lines: report.lines,
            needs_scroll: report.needs_scroll,
        }
    }
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

/// Smallest and largest key width a preview may ask for, in pixels.
pub const PREVIEW_KEY_PIXELS: (u32, u32) = (16, 512);

/// The size a preview draws `control` at. The device's own image size
/// (`native`) without `key_pixels`; with it, the native size scaled so that
/// a key is `key_pixels` wide (the width the app shows a key at, in device
/// pixels: CSS size times the display's pixel ratio), and every other
/// display (touch-strip zones) by the same factor. The face is laid out by
/// the same rules at every size, so the preview shows what the device shows,
/// only drawn for the screen instead of scaled up from the device's pixels.
fn preview_size(
    capabilities: &deckpilot_device::DeviceCapabilities,
    native: (u32, u32),
    key_pixels: Option<u32>,
) -> (u32, u32) {
    let Some(requested) = key_pixels else {
        return native;
    };
    let Some(key_width) = capabilities
        .keys
        .and_then(|k| k.image)
        .map(|spec| spec.width)
        .or_else(|| {
            capabilities
                .touch
                .as_ref()
                .and_then(|t| t.image)
                .map(|spec| spec.height)
        })
        .filter(|width| *width > 0)
    else {
        return native;
    };
    let requested = requested.clamp(PREVIEW_KEY_PIXELS.0, PREVIEW_KEY_PIXELS.1);
    let scale = |side: u32| {
        let scaled = u64::from(side) * u64::from(requested) / u64::from(key_width);
        u32::try_from(scaled)
            .unwrap_or(u32::MAX)
            .clamp(1, PREVIEW_KEY_PIXELS.1 * 4)
    };
    (scale(native.0), scale(native.1))
}

/// Live key states for a preview, as the agent reported them
/// (`watch_key_states`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePreview {
    /// Whether the agent is connected and the states are current; `false`:
    /// they are the last known ones (or none), shown as stale.
    pub connected: bool,
    pub states: Vec<deckpilot_ipc::KeyStateEntry>,
}

/// `face` of a key running `action` with what the agent reported: its
/// state as the device shows it; a stale one with a grey dot instead of
/// its status; a dash for a key whose live value is not known. Keys
/// without live state stay as they are.
fn with_live_state(
    face: KeyFace,
    action: Option<deckpilot_core::ActionInstanceId>,
    live: &LivePreview,
) -> KeyFace {
    let Some(entry) = action.and_then(|action| live.states.iter().find(|e| e.action == action))
    else {
        return face;
    };
    let face = face.with_snapshot(&entry.snapshot);
    if live.connected {
        return face;
    }
    let mut stale = face.with_indicator(deckpilot_core::Indicator::Unknown);
    if matches!(entry.snapshot, deckpilot_core::StateSnapshot::Loading) {
        stale.detail = Some("\u{2014}".into());
    }
    stale
}

/// The faces of a (possibly unsaved) layout page, with live states, and
/// the model and profile they were made for.
struct PageFaces {
    faces: Vec<(ControlId, KeyFace)>,
    capabilities: deckpilot_device::DeviceCapabilities,
    /// The model's colours for keys that set none, drawn as on the device.
    look: deckpilot_device::DefaultAppearance,
    profile: deckpilot_core::Profile,
    incompatible: Vec<deckpilot_core::IncompatibleBinding>,
    /// What each control that navigates does to the view.
    navigation: Vec<(ControlId, deckpilot_core::ControlNavigation)>,
}

fn page_preview(
    state: &AppState,
    profile_id: ProfileId,
    layout: &DeviceLayout,
    page_id: PageId,
    project_id: Option<ProjectId>,
    live: Option<&LivePreview>,
) -> CommandResult<PageFaces> {
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
    let incompatible: Vec<deckpilot_core::IncompatibleBinding> = layout
        .incompatible_bindings(&model.capabilities, &actions, &profile)
        .into_iter()
        .filter(|found| found.page == page_id)
        .collect();
    let faces = page_faces(
        &actions,
        layout,
        page_id,
        &profile,
        project,
        &controls,
        Some(&model.capabilities),
    )
    .into_iter()
    .map(|(control, face)| {
        let face = match live {
            Some(live) => {
                let action = layout
                    .page(page_id)
                    .and_then(|page| deckpilot_core::device_runtime::shown_binding(page, control))
                    .and_then(|binding| binding.target.action_id());
                with_live_state(face, action, live)
            }
            None => face,
        };
        (control, face)
    })
    .collect();
    let display = deckpilot_core::editor_navigation::display_controls(&model.capabilities);
    let navigation = controls
        .iter()
        .filter_map(|control| {
            deckpilot_core::control_navigation(
                &actions, layout, page_id, &profile, *control, &display,
            )
            .map(|navigation| (*control, navigation))
        })
        .collect();
    Ok(PageFaces {
        navigation,
        faces,
        capabilities: model.capabilities.clone(),
        look: model.effective_appearance(),
        profile,
        incompatible,
    })
}

/// What every control of a (possibly unsaved) layout page shows for a given
/// project (`None`: the profile's active project, as on the device),
/// without a device and without running any action. With `render`, each
/// face also comes as the image the device would show, drawn by the
/// agent's renderer: at the device's own size, or with `key_pixels` at the
/// size the app shows it ([`preview_size`]), and with how its title comes
/// out on the device (`text`). With `live`, keys show the live states the
/// agent reported for them (`watch_key_states`). A scrolling title is drawn
/// still, cut with `…`; [`preview_marquee`] has its frames.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri passes each argument by name.
pub fn preview_layout(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout: DeviceLayout,
    page_id: PageId,
    project_id: Option<ProjectId>,
    render: Option<bool>,
    live: Option<LivePreview>,
    key_pixels: Option<u32>,
) -> CommandResult<Vec<ControlFace>> {
    let preview = page_preview(
        &state,
        profile_id,
        &layout,
        page_id,
        project_id,
        live.as_ref(),
    )?;
    let capabilities = &preview.capabilities;
    let icons = render
        .unwrap_or(false)
        .then(|| state.visual_assets.icons(&preview.profile));
    Ok(preview
        .faces
        .into_iter()
        .map(|(control, face)| {
            let native = image_size(capabilities, control);
            let drawn = icons.as_ref().zip(native).and_then(|(icons, native)| {
                let (width, height) = preview_size(capabilities, native, key_pixels);
                let frame =
                    deckpilot_render::render_for(&face, width, height, icons, &preview.look);
                let png = deckpilot_render::frame_png(&frame)?;
                // Measured at the device's own size: what the device shows.
                let text = deckpilot_render::text_report(&face, native.0, native.1, icons);
                Some((deckpilot_core::IconData::new(png).data_url(), text))
            });
            let (image, text) = drawn.map_or((None, None), |(image, text)| (Some(image), text));
            ControlFace {
                control,
                face,
                image,
                incompatible: preview
                    .incompatible
                    .iter()
                    .filter(|found| found.control == control)
                    .map(|found| IncompatibleReason {
                        trigger: found.trigger,
                        message: found.reason.to_string(),
                        reason: found.reason.clone(),
                    })
                    .collect(),
                text: text.map(TextLayoutReport::from),
                navigation: preview
                    .navigation
                    .iter()
                    .find(|(c, _)| *c == control)
                    .map(|(_, navigation)| navigation.clone()),
            }
        })
        .collect())
}

/// Most frames of a scrolling preview.
const MARQUEE_PREVIEW_FRAMES: usize = 400;

/// One frame of a scrolling title's preview.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarqueeFrame {
    /// PNG `data:` URL.
    image: String,
    /// How long it is shown, in milliseconds.
    duration_ms: u32,
}

/// The frames of one cycle of a scrolling title, as the device shows them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarqueePreview {
    frames: Vec<MarqueeFrame>,
    cycle_ms: u32,
}

/// One cycle of the scrolling title of `control` (`docs/key-text.md`): the
/// frames the device shows, timed as there (at most
/// [`deckpilot_render::MARQUEE_FPS`] a second, the pauses as one frame
/// each), drawn at the size the app shows the key at ([`preview_size`]).
/// `None` when its title does not scroll. The app plays them unless the
/// system asks for reduced motion.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri passes each argument by name.
#[expect(
    clippy::cast_precision_loss,
    reason = "key sizes and offsets are a few hundred pixels"
)]
pub fn preview_marquee(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    layout: DeviceLayout,
    page_id: PageId,
    control: ControlId,
    project_id: Option<ProjectId>,
    live: Option<LivePreview>,
    key_pixels: Option<u32>,
) -> CommandResult<Option<MarqueePreview>> {
    let preview = page_preview(
        &state,
        profile_id,
        &layout,
        page_id,
        project_id,
        live.as_ref(),
    )?;
    let Some((_, face)) = preview.faces.iter().find(|(c, _)| *c == control) else {
        return Ok(None);
    };
    let Some(native) = image_size(&preview.capabilities, control) else {
        return Ok(None);
    };
    let icons = state.visual_assets.icons(&preview.profile);
    // Timed at the device's size, where its speed is set.
    let Some(marquee) = deckpilot_render::marquee(face, native.0, native.1, &icons) else {
        return Ok(None);
    };
    let (width, height) = preview_size(&preview.capabilities, native, key_pixels);
    let scale = width as f32 / native.0.max(1) as f32;
    let millis =
        |duration: std::time::Duration| u32::try_from(duration.as_millis()).unwrap_or(u32::MAX);
    let frames = deckpilot_render::marquee_timeline(&marquee, deckpilot_render::MARQUEE_FPS)
        .into_iter()
        .take(MARQUEE_PREVIEW_FRAMES)
        .map(|(offset, duration)| {
            let frame = deckpilot_render::render_scrolled(
                face,
                width,
                height,
                &icons,
                &preview.look,
                offset as f32 * scale,
            );
            let png = deckpilot_render::frame_png(&frame).ok_or(CommandError::invalid(
                "keyImageEncoding",
                "the key image could not be encoded",
            ))?;
            Ok(MarqueeFrame {
                image: deckpilot_core::IconData::new(png).data_url(),
                duration_ms: millis(duration),
            })
        })
        .collect::<CommandResult<Vec<_>>>()?;
    Ok(Some(MarqueePreview {
        frames,
        cycle_ms: millis(marquee.cycle()),
    }))
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
    /// The folder's protected system Back key (its reserved key), if it
    /// has one ([`deckpilot_core::folder`]).
    system_back: Option<ControlId>,
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
    let capabilities = state
        .catalog
        .get(&layout.model)
        .map(|model| model.capabilities.clone());
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
                system_back: capabilities
                    .as_ref()
                    .and_then(|capabilities| layout.system_back(page.id, capabilities)),
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

    /// Work: Main's home key 1 opens the folder Tools; key 2 runs Log.
    fn with_folder() -> (Profile, deckpilot_core::ActionRegistry, PageId) {
        let (mut profile, registry) = stored();
        let log = profile.actions[0].id;
        let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
        let home = layout.home_page;
        let tools = deckpilot_core::LayoutPage::folder("Tools", home);
        let tools_id = tools.id;
        layout.pages.push(tools);
        layout.pages[0].bindings = vec![
            ControlBinding::new(
                ControlId::Key { index: 1 },
                Trigger::Press,
                BindingTarget::OpenPage { page: tools_id },
            ),
            ControlBinding::new(
                ControlId::Key { index: 2 },
                Trigger::Press,
                BindingTarget::action(log),
            ),
        ];
        layout.explicit_tree = true;
        profile.layouts = vec![layout];
        (profile, registry, tools_id)
    }

    fn code_of(refused: &[LayoutIssue]) -> (String, serde_json::Value) {
        let error = serde_json::to_value(refusal(refused)).unwrap();
        (
            error["code"].as_str().unwrap().to_owned(),
            error["params"].clone(),
        )
    }

    #[test]
    fn folder_keys_refuse_other_bindings_with_stable_codes() {
        let (profile, registry, tools) = with_folder();
        let catalog = ModelCatalog::builtin();
        let saved = &profile.layouts[0];
        let log = profile.actions[0].id;
        let check = |edited: &DeviceLayout| {
            check_for_save(edited, Some(saved), &catalog, &registry, &profile).0
        };
        assert_eq!(check(saved), []);
        // A gesture added to the folder key (as a malformed or outdated
        // client would send it).
        let mut gesture = saved.clone();
        gesture.pages[0].bindings.push(ControlBinding::new(
            ControlId::Key { index: 1 },
            Trigger::LongPress,
            BindingTarget::action(log),
        ));
        let refused = check(&gesture);
        let (code, params) = code_of(&refused);
        assert_eq!(code, "invalid.folderControl.extraBinding");
        assert_eq!(params["key"], 2);
        assert_eq!(params["folder"], "Tools");
        // Its open moved to another trigger.
        let mut trigger = saved.clone();
        trigger.pages[0].bindings[0].trigger = Trigger::DoubleClick;
        assert_eq!(code_of(&check(&trigger)).0, "invalid.folderControl.trigger");
        // Replaced by an action.
        let mut replaced = saved.clone();
        replaced.pages[0].bindings[0].target = BindingTarget::action(log);
        assert_eq!(
            code_of(&check(&replaced)).0,
            "invalid.folderControl.replaced"
        );
        // Recovered bindings a client made up.
        let mut injected = saved.clone();
        injected.recovered.push(deckpilot_core::RecoveredBindings {
            page: injected.home_page,
            control: ControlId::Key { index: 1 },
            folder: tools,
            bindings: Vec::new(),
            gestures: None,
            reason: deckpilot_core::RecoveryReason::FolderControl,
        });
        assert_eq!(
            code_of(&check(&injected)).0,
            "invalid.folderControl.recovered"
        );
        // Clearing the key is allowed.
        let mut cleared = saved.clone();
        cleared.pages[0].bindings.remove(0);
        assert_eq!(check(&cleared), []);
        // Other refusals keep the generic code.
        assert_eq!(
            code_of(&[LayoutIssue::MissingHomePage]).0,
            "invalid".to_owned()
        );
    }

    #[test]
    fn saving_drops_the_timings_of_folder_keys_and_orphaned_recovered_bindings() {
        let (profile, _, tools) = with_folder();
        let mut layout = profile.layouts[0].clone();
        layout.pages[0]
            .gestures
            .push(deckpilot_core::gesture::KeyGestureSettings {
                control: ControlId::Key { index: 1 },
                double_click_ms: Some(400),
                long_press_ms: None,
            });
        layout.recovered.push(deckpilot_core::RecoveredBindings {
            page: PageId::new(),
            control: ControlId::Key { index: 1 },
            folder: tools,
            bindings: Vec::new(),
            gestures: None,
            reason: deckpilot_core::RecoveryReason::FolderControl,
        });
        tidy_folder_controls(&mut layout);
        assert_eq!(layout.pages[0].gestures, []);
        assert_eq!(layout.recovered, []);
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

#[cfg(test)]
mod preview_size_tests {
    use deckpilot_device::models;

    use super::*;

    #[test]
    fn previews_are_drawn_at_the_size_they_are_shown() {
        let plus = models::stream_deck_plus().capabilities;
        let key = image_size(&plus, ControlId::Key { index: 0 }).unwrap();
        let zone = image_size(&plus, ControlId::TouchZone { index: 0 }).unwrap();
        assert_eq!((key, zone), ((120, 120), (200, 100)));
        // The device's own size without a request: what the device gets.
        assert_eq!(preview_size(&plus, key, None), (120, 120));
        assert_eq!(preview_size(&plus, zone, None), (200, 100));
        // A key shown 96 CSS px wide on a 2x display; zones scale alike.
        assert_eq!(preview_size(&plus, key, Some(192)), (192, 192));
        assert_eq!(preview_size(&plus, zone, Some(192)), (320, 160));
        // Smaller than the device, e.g. a small window on a 1x display.
        assert_eq!(preview_size(&plus, key, Some(60)), (60, 60));
        // Clamped.
        assert_eq!(preview_size(&plus, key, Some(0)), (16, 16));
        assert_eq!(preview_size(&plus, key, Some(u32::MAX)), (512, 512));
        // A model without displays keeps its (absent) size.
        let mut none = plus;
        none.keys = None;
        none.touch = None;
        assert_eq!(preview_size(&none, (10, 10), Some(200)), (10, 10));
    }
}
