//! Key icons of a profile: imported images that keys can show.
//!
//! The frontend sends a picked file as base64. The file is decoded with
//! limits, scaled down and encoded again (`deckpilot_render::icons`); only
//! that PNG is stored, in the profile.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use tauri::State;

use deckpilot_core::{Icon, IconId, Profile, ProfileId};
use deckpilot_render::icons::{self, MAX_IMPORT_BYTES};

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile};

/// An icon as the frontend lists it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IconInfo {
    id: IconId,
    name: String,
    width: u32,
    height: u32,
    /// Size of the stored PNG.
    bytes: usize,
    /// The PNG as a `data:` URL for `<img>`.
    data_url: String,
    /// Keys that show it; an icon in use cannot be deleted.
    usages: usize,
}

fn info(profile: &Profile, icon: &Icon) -> IconInfo {
    IconInfo {
        id: icon.id,
        name: icon.name.clone(),
        width: icon.width,
        height: icon.height,
        bytes: icon.png.len(),
        data_url: icon.png.data_url(),
        usages: profile.icon_usages(icon.id).len(),
    }
}

/// The fonts a key title may use (`deckpilot_core::TITLE_FONTS`): the
/// fonts built into the renderer, so the editor offers only what every
/// device draws alike.
#[tauri::command]
pub fn title_fonts() -> Vec<deckpilot_core::TitleFont> {
    deckpilot_core::TITLE_FONTS
        .iter()
        .copied()
        .filter(|font| deckpilot_render::has_title_font(font.id))
        .collect()
}

/// The profile's icons, in import order.
#[tauri::command]
pub fn list_icons(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<Vec<IconInfo>> {
    let store = state.store()?;
    let profile = load_profile(&store, profile_id)?;
    Ok(profile
        .icons
        .iter()
        .map(|icon| info(&profile, icon))
        .collect())
}

/// A refused image, with the reason the frontend translates.
fn icon_error(error: &icons::IconError) -> CommandError {
    use icons::IconError as E;
    let message = error.to_string();
    match error {
        E::TooLarge => CommandError::invalid_with(
            "iconTooLarge",
            message,
            [("size", (MAX_IMPORT_BYTES / 1024 / 1024).into())],
        ),
        E::UnsupportedFormat => CommandError::invalid("iconFormat", message),
        E::TooManyPixels => CommandError::invalid_with(
            "iconTooManyPixels",
            message,
            [("size", icons::MAX_SOURCE_SIZE.into())],
        ),
        E::Malformed => CommandError::invalid("iconMalformed", message),
        E::Invalid(_) => CommandError::Invalid(message),
    }
}

/// Imports a PNG or JPEG file (`data`: its bytes as base64) as an icon.
/// Importing the same image again returns the existing icon.
#[tauri::command]
pub fn import_icon(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    name: String,
    data: String,
) -> CommandResult<IconInfo> {
    // Refuse large files before decoding the base64.
    if data.len() > MAX_IMPORT_BYTES.div_ceil(3) * 4 {
        return Err(icon_error(&icons::IconError::TooLarge));
    }
    let bytes = STANDARD
        .decode(data.trim())
        .map_err(|_| CommandError::invalid("fileUnreadable", "the file could not be read"))?;
    let icon = icons::import(&name, &bytes).map_err(|e| icon_error(&e))?;
    let imported = modify_profile(&state, profile_id, |profile| {
        let id = profile.add_icon(icon)?;
        let icon = profile.icon(id).ok_or(CommandError::NotFound("icon"))?;
        Ok(info(profile, icon))
    })?;
    // The agent decodes icons when it loads the configuration: so that a
    // key of the editor's live preview can show the new icon at once.
    crate::agent::notify_reload(&state.paths);
    Ok(imported)
}

/// Deletes an icon that no key shows.
#[tauri::command]
pub fn delete_icon(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    icon_id: IconId,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        profile.delete_icon(icon_id)?;
        Ok(())
    })?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}
