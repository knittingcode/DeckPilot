//! The shared visual asset library as the editor's Icon and Background
//! Libraries show it (`docs/key-text.md`, "Icons and backgrounds"):
//!
//! - DeckPilot's own icons and gradients, compiled into the renderer, with
//!   thumbnails drawn by the same renderer as the keys;
//! - the visual assets of installed plugins (plugin API 1.8), as the agent
//!   lists them, their files read through the agent, which checks them
//!   against the signed package's integrity record. They are kept here
//!   ([`VisualAssets`]) so the editor's previews draw them exactly as the
//!   device does; only an enabled plugin's assets are drawn.
//!
//! My Icons (the profile's imported images) come from `list_icons`. Nothing
//! here changes a key: the editor puts a chosen asset's id on its keys
//! through its draft, like any other change of their look.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use deckpilot_core::{AssetId, Background, Color, DefaultAppearance, Gradient, IconData};
use deckpilot_ipc::plugins::{PluginState, VisualAssetKind};
use deckpilot_render::{Icons, LibraryAsset};
use serde::Serialize;
use tauri::State;

use super::plugins::visual_asset_plugins;
use crate::state::AppState;

/// Side of a library tile's image, in pixels (two device pixels per point
/// at the tiles' size).
const THUMBNAIL: u32 = 64;

/// One of DeckPilot's own icons.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinIcon {
    /// `deckpilot.<name>`.
    id: String,
    /// The Lucide name (`house`), which the app shows as it is.
    name: &'static str,
    categories: Vec<&'static str>,
    tags: Vec<&'static str>,
    /// White on transparent, drawn from the renderer's font.
    thumbnail: String,
}

/// One of DeckPilot's own gradients.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinGradient {
    /// `deckpilot.gradient-<name>`.
    id: String,
    /// The name the app translates (`editor:assets.gradients.<name>`).
    name: &'static str,
    colors: Vec<String>,
    thumbnail: String,
}

/// A plugin's icon or background.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAsset {
    /// `<plugin>.<id>`: what a key stores.
    id: String,
    plugin: String,
    plugin_name: String,
    /// In the app's language (the agent localizes it).
    name: String,
    kind: VisualAssetKind,
    keywords: Vec<String>,
    /// The plugin is enabled: keys draw it. A disabled plugin's assets are
    /// listed (so a key that uses one says so), not offered.
    available: bool,
    /// The tile's image (`data:` URL); `None` when the file could not be
    /// read or checked.
    thumbnail: Option<String>,
}

/// What the libraries offer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualLibrary {
    categories: &'static [&'static str],
    icons: Arc<Vec<BuiltinIcon>>,
    gradients: Arc<Vec<BuiltinGradient>>,
    plugins: Vec<PluginAsset>,
    /// Why plugins' assets are not listed (`pluginsUnavailable`, `locked`,
    /// an older agent…), when they are not.
    #[serde(skip_serializing_if = "Option::is_none")]
    plugins_problem: Option<String>,
    /// Changes whenever the plugin assets the previews draw changed.
    version: u64,
}

fn png_url(png: Vec<u8>) -> String {
    IconData::new(png).data_url()
}

static BUILTIN_ICONS: LazyLock<Arc<Vec<BuiltinIcon>>> = LazyLock::new(|| {
    Arc::new(
        deckpilot_render::library_icons()
            .iter()
            .filter_map(|icon| {
                Some(BuiltinIcon {
                    id: AssetId::builtin(icon.name).ok()?.to_string(),
                    name: icon.name,
                    categories: icon.categories.clone(),
                    tags: icon.tags.clone(),
                    thumbnail: png_url(deckpilot_render::icon_thumbnail_png(icon.name, THUMBNAIL)?),
                })
            })
            .collect(),
    )
});

/// A background's tile: the background alone on a key, drawn by the
/// renderer.
fn background_thumbnail(background: &Background, icons: &Icons) -> Option<String> {
    let frame = deckpilot_render::background_preview(
        Some(background),
        THUMBNAIL,
        THUMBNAIL,
        icons,
        &DefaultAppearance::BUILTIN,
    );
    deckpilot_render::frame_png(&frame).map(png_url)
}

static BUILTIN_GRADIENTS: LazyLock<Arc<Vec<BuiltinGradient>>> = LazyLock::new(|| {
    let icons = Icons::default();
    Arc::new(
        deckpilot_core::BUILTIN_GRADIENTS
            .iter()
            .filter_map(|gradient| {
                let id = gradient.id();
                let background = Background::Asset(id.clone().into());
                Some(BuiltinGradient {
                    id: id.to_string(),
                    name: gradient.name,
                    colors: gradient.colors.iter().map(ToString::to_string).collect(),
                    thumbnail: background_thumbnail(&background, &icons)?,
                })
            })
            .collect(),
    )
});

/// A plugin asset as it was read: what the previews draw and its tile.
#[derive(Debug, Clone)]
struct Fetched {
    drawn: Option<LibraryAsset>,
    thumbnail: Option<String>,
}

/// The plugin assets the editor's previews draw, as last listed, and the
/// files already read (by plugin version and asset).
#[derive(Debug, Default)]
pub struct VisualAssets {
    drawable: Mutex<Vec<(AssetId, LibraryAsset)>>,
    fetched: Mutex<HashMap<(String, String, String), Fetched>>,
    version: AtomicU64,
}

impl VisualAssets {
    /// The assets of enabled plugins, for [`Icons::with_library`].
    pub fn drawable(&self) -> Vec<(AssetId, LibraryAsset)> {
        self.drawable
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The profile's icons with the plugin assets the previews draw.
    pub fn icons(&self, profile: &deckpilot_core::Profile) -> Icons {
        Icons::from_profile(profile).with_library(self.drawable())
    }

    fn replace(&self, drawable: Vec<(AssetId, LibraryAsset)>) -> u64 {
        let mut current = self.drawable.lock().unwrap_or_else(PoisonError::into_inner);
        if *current != drawable {
            *current = drawable;
            self.version.fetch_add(1, Ordering::Relaxed);
        }
        self.version.load(Ordering::Relaxed)
    }
}

/// Decodes a checked file (`data:` URL) for drawing.
fn decode(url: &str) -> Option<LibraryAsset> {
    let (_, data) = url.split_once(";base64,")?;
    let bytes = STANDARD.decode(data).ok()?;
    deckpilot_render::icons::decode_asset(&bytes)
        .ok()
        .map(|image| LibraryAsset::Image(Arc::new(image)))
}

fn gradient(colors: &[String], angle: Option<u16>) -> Option<Gradient> {
    let gradient = Gradient {
        colors: colors
            .iter()
            .map(|c| Color::parse(c).ok())
            .collect::<Option<_>>()?,
        angle: angle.unwrap_or(180),
    };
    gradient.check().ok().map(|()| gradient)
}

/// Reads what the previews draw of a plugin's asset `id` and its tile: a
/// gradient from its colours, an image from its file (checked by the agent).
async fn fetch(
    reader: &mut super::plugins::AssetReader,
    plugin: &str,
    asset: &deckpilot_ipc::plugins::PluginVisualAsset,
    id: &AssetId,
) -> Fetched {
    match asset.kind {
        VisualAssetKind::GradientBackground => {
            let drawn = gradient(&asset.colors, asset.angle).map(LibraryAsset::Gradient);
            let thumbnail = drawn.clone().and_then(|drawn| {
                let icons = Icons::default().with_library([(id.clone(), drawn)]);
                background_thumbnail(&Background::Asset(id.clone().into()), &icons)
            });
            Fetched { drawn, thumbnail }
        }
        VisualAssetKind::Icon | VisualAssetKind::ImageBackground => {
            let file = reader.file(plugin, &asset.asset).await.ok().flatten();
            Fetched {
                drawn: file.as_deref().and_then(decode),
                thumbnail: file,
            }
        }
    }
}

/// The libraries' contents: DeckPilot's icons and gradients always (also
/// with every plugin disabled, locked or without an agent), and the
/// installed plugins' assets when the agent lists them. Also refreshes
/// what the editor's previews draw of plugins.
#[tauri::command]
pub async fn visual_library(state: State<'_, AppState>) -> Result<VisualLibrary, String> {
    let assets = &state.visual_assets;
    let mut plugins = Vec::new();
    let mut drawable = Vec::new();
    let mut problem = None;
    match visual_asset_plugins(&state).await {
        Ok(Some((listed, mut reader))) => {
            for plugin in listed {
                let available = plugin.state == PluginState::Enabled;
                for asset in &plugin.visual_assets {
                    let Ok(id) = AssetId::parse(&asset.id) else {
                        continue;
                    };
                    let key = (plugin.id.clone(), plugin.version.clone(), asset.id.clone());
                    let known = assets
                        .fetched
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&key)
                        .cloned();
                    let fetched = if let Some(known) = known {
                        known
                    } else {
                        let fetched = fetch(&mut reader, &plugin.id, asset, &id).await;
                        assets
                            .fetched
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .insert(key, fetched.clone());
                        fetched
                    };
                    if available && let Some(drawn) = fetched.drawn.clone() {
                        drawable.push((id, drawn));
                    }
                    plugins.push(PluginAsset {
                        id: asset.id.clone(),
                        plugin: plugin.id.clone(),
                        plugin_name: plugin.name.clone(),
                        name: asset.name.clone(),
                        kind: asset.kind,
                        keywords: asset.keywords.clone(),
                        available,
                        thumbnail: fetched.thumbnail,
                    });
                }
            }
        }
        Ok(None) => problem = Some("agentTooOld".to_owned()),
        Err(error) => problem = Some(error.reason),
    }
    let version = assets.replace(drawable);
    Ok(VisualLibrary {
        categories: deckpilot_render::library_categories(),
        icons: BUILTIN_ICONS.clone(),
        gradients: BUILTIN_GRADIENTS.clone(),
        plugins,
        plugins_problem: problem,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_library_is_there_without_plugins() {
        assert!(BUILTIN_ICONS.len() >= 400);
        let house = BUILTIN_ICONS.iter().find(|i| i.name == "house").unwrap();
        assert_eq!(house.id, "deckpilot.house");
        assert!(house.thumbnail.starts_with("data:image/png;base64,"));
        assert!(house.tags.contains(&"home"));
        assert_eq!(
            BUILTIN_GRADIENTS.len(),
            deckpilot_core::BUILTIN_GRADIENTS.len()
        );
        assert!(BUILTIN_GRADIENTS[0].id.starts_with("deckpilot.gradient-"));
    }

    #[test]
    fn previews_draw_only_the_assets_they_were_given() {
        let assets = VisualAssets::default();
        let id = AssetId::parse("acme.tools.dusk").unwrap();
        let dusk = gradient(&["#ff8800".into(), "#220044".into()], None).unwrap();
        let first = assets.replace(vec![(id.clone(), LibraryAsset::Gradient(dusk.clone()))]);
        assert_eq!(
            assets.replace(vec![(id, LibraryAsset::Gradient(dusk))]),
            first
        );
        let profile = deckpilot_core::Profile::new("P");
        let reference = deckpilot_core::AssetRef::parse("acme.tools.dusk").unwrap();
        assert!(assets.icons(&profile).has(&reference));
        // The plugin was disabled: the next listing leaves it out.
        assert_ne!(assets.replace(Vec::new()), first);
        assert!(!assets.icons(&profile).has(&reference));
        assert!(gradient(&["#ff8800".into()], None).is_none());
        assert!(gradient(&["red".into(), "#000000".into()], None).is_none());
    }
}
