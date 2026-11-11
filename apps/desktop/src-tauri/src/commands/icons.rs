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
        return Err(CommandError::Invalid(
            icons::IconError::TooLarge.to_string(),
        ));
    }
    let bytes = STANDARD
        .decode(data.trim())
        .map_err(|_| CommandError::Invalid("the file could not be read".into()))?;
    let icon = icons::import(&name, &bytes).map_err(|e| CommandError::Invalid(e.to_string()))?;
    modify_profile(&state, profile_id, |profile| {
        let id = profile.add_icon(icon)?;
        let icon = profile.icon(id).ok_or(CommandError::NotFound("icon"))?;
        Ok(info(profile, icon))
    })
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
    })
}
