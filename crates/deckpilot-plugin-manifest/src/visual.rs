//! `contributes.visualAssets` (plugin API 1.8): icons and backgrounds a
//! plugin adds to DeckPilot's shared Icon and Background Libraries.
//!
//! The contribution is declarative: files of the package and colours,
//! never code. Ids are local to the plugin; DeckPilot names an asset
//! `<plugin id>.<id>` (e.g. `acme.weather.sunny`), so two plugins never
//! collide and neither collides with DeckPilot's own (`deckpilot.<name>`).
//! Icons and backgrounds share the plugin's id space.
//!
//! Files are package-relative paths under `assets/` (no scheme, no
//! absolute path, no `..`, no backslash): they are hashed into the
//! integrity record like every other file the manifest names, refused at
//! build and install time when their content does not match
//! ([`check_visual_asset`]), and checked against that record again every
//! time the agent reads them. Icons are PNG (at most
//! [`MAX_VISUAL_ICON_BYTES`] and [`MAX_VISUAL_ICON_SIDE`] pixels a side),
//! image backgrounds PNG or JPEG (at most [`MAX_BACKGROUND_BYTES`] and
//! [`MAX_BACKGROUND_SIDE`]); gradients are two to four `#rrggbb` colours
//! and an angle. Nothing is animated.

use serde::{Deserialize, Serialize};

use crate::compat::ApiLevel;

/// The plugin API level that introduced `contributes.visualAssets`.
pub const VISUAL_ASSETS_API: ApiLevel = ApiLevel::new(1, 8);
/// Most icons one plugin contributes.
pub const MAX_VISUAL_ICONS: usize = 128;
/// Most backgrounds one plugin contributes.
pub const MAX_VISUAL_BACKGROUNDS: usize = 32;
/// Longest asset id (local to the plugin).
pub const MAX_VISUAL_ASSET_ID: usize = 48;
/// Longest asset name, in characters.
pub const MAX_VISUAL_ASSET_NAME: usize = 48;
/// Largest icon file.
pub const MAX_VISUAL_ICON_BYTES: usize = 256 * 1024;
/// Largest width and height of an icon, in pixels.
pub const MAX_VISUAL_ICON_SIDE: u32 = 512;
/// Largest image background file.
pub const MAX_BACKGROUND_BYTES: usize = 1024 * 1024;
/// Largest width and height of an image background, in pixels.
pub const MAX_BACKGROUND_SIDE: u32 = 1024;
/// Fewest and most colours of a gradient.
pub const GRADIENT_COLORS: (usize, usize) = (2, 4);

/// `contributes.visualAssets`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualAssets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icons: Option<Vec<VisualIcon>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backgrounds: Option<Vec<VisualBackground>>,
}

impl VisualAssets {
    pub fn icons(&self) -> &[VisualIcon] {
        self.icons.as_deref().unwrap_or_default()
    }

    pub fn backgrounds(&self) -> &[VisualBackground] {
        self.backgrounds.as_deref().unwrap_or_default()
    }
}

/// An icon for keys: a PNG of the package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualIcon {
    pub id: String,
    /// Shown in the library (may be a `t:` translation).
    pub name: String,
    pub asset: String,
    /// Search words (not translated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
}

/// What a background is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VisualBackgroundKind {
    /// A PNG or JPEG of the package, covering the key.
    Image,
    /// A linear gradient of `colors` along `angle`.
    Gradient,
}

/// A background for keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualBackground {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: VisualBackgroundKind,
    /// `image`: the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// `gradient`: two to four `#rrggbb` colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<Vec<String>>,
    /// `gradient`: degrees as in CSS (0 to the top, 90 to the right),
    /// default 180 (top to bottom).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<u16>,
}

/// The formats of a visual asset file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisualFormat {
    Png,
    Jpeg,
}

impl VisualFormat {
    /// The format a path names by its (lowercase) extension.
    pub fn of_path(path: &str) -> Option<Self> {
        let (stem, extension) = path.rsplit_once('.')?;
        if stem.is_empty() || stem.ends_with('/') {
            return None;
        }
        match extension {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }

    pub fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
}

/// What a file is used as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisualUse {
    Icon,
    Background,
}

impl VisualUse {
    fn limits(self) -> (usize, u32) {
        match self {
            Self::Icon => (MAX_VISUAL_ICON_BYTES, MAX_VISUAL_ICON_SIDE),
            Self::Background => (MAX_BACKGROUND_BYTES, MAX_BACKGROUND_SIDE),
        }
    }

    fn allows(self, format: VisualFormat) -> bool {
        match self {
            Self::Icon => format == VisualFormat::Png,
            Self::Background => true,
        }
    }
}

/// Why a visual asset file cannot be shipped or shown. Completes a
/// sentence that starts with the file's path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VisualAssetError {
    #[error("is not a .png file (icons) or a .png or .jpg file (backgrounds)")]
    UnknownFormat,
    #[error("is {size} bytes; the limit is {limit} bytes")]
    TooLarge { size: usize, limit: usize },
    #[error("is not a valid {0} file")]
    Malformed(&'static str),
    #[error("is {width} x {height} pixels; the limit is {limit} x {limit}")]
    Dimensions { width: u32, height: u32, limit: u32 },
}

fn u16_be(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((u32_be(bytes, 16)?, u32_be(bytes, 20)?))
}

/// Width and height from a JPEG's first frame header (SOF0–SOF15 but DHT,
/// JPG and DAC), walking the marker segments; nothing is decoded.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xff {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        if marker == 0xff {
            at += 1;
            continue;
        }
        let length = usize::from(u16_be(bytes, at + 2)?);
        if length < 2 {
            return None;
        }
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            let height = u16_be(bytes, at + 5)?;
            let width = u16_be(bytes, at + 7)?;
            return Some((u32::from(width), u32::from(height)));
        }
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        at += 2 + length;
    }
}

/// Checks a visual asset file of the package: its format (by extension)
/// matches its content and its use, its size, its dimensions (from the
/// header; nothing is decoded). Gives the format and the dimensions.
pub fn check_visual_asset(
    path: &str,
    used: VisualUse,
    bytes: &[u8],
) -> Result<(VisualFormat, u32, u32), VisualAssetError> {
    let format = VisualFormat::of_path(path)
        .filter(|format| used.allows(*format))
        .ok_or(VisualAssetError::UnknownFormat)?;
    let (max_bytes, max_side) = used.limits();
    if bytes.len() > max_bytes {
        return Err(VisualAssetError::TooLarge {
            size: bytes.len(),
            limit: max_bytes,
        });
    }
    let (width, height) = match format {
        VisualFormat::Png => png_size(bytes).ok_or(VisualAssetError::Malformed("PNG"))?,
        VisualFormat::Jpeg => jpeg_size(bytes).ok_or(VisualAssetError::Malformed("JPEG"))?,
    };
    if width == 0 || height == 0 || width > max_side || height > max_side {
        return Err(VisualAssetError::Dimensions {
            width,
            height,
            limit: max_side,
        });
    }
    Ok((format, width, height))
}

/// Whether `text` is `#rrggbb`.
pub(crate) fn is_hex_color(text: &str) -> bool {
    text.strip_prefix('#')
        .is_some_and(|hex| hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend(13u32.to_be_bytes());
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        // An APP0 segment first, then the frame header.
        bytes.extend([0xff, 0xe0, 0x00, 0x04, 0x00, 0x00]);
        bytes.extend([0xff, 0xc0, 0x00, 0x0b, 0x08]);
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([0x01, 0x01, 0x11, 0x00]);
        bytes
    }

    #[test]
    fn formats_sizes_and_dimensions_are_checked() {
        use VisualUse::{Background, Icon};
        assert_eq!(
            check_visual_asset("assets/a.png", Icon, &png(64, 64)),
            Ok((VisualFormat::Png, 64, 64))
        );
        assert_eq!(
            check_visual_asset("assets/bg.jpg", Background, &jpeg(400, 200)),
            Ok((VisualFormat::Jpeg, 400, 200))
        );
        // Icons are PNG only; the extension must match the content.
        assert_eq!(
            check_visual_asset("assets/a.jpg", Icon, &jpeg(10, 10)),
            Err(VisualAssetError::UnknownFormat)
        );
        assert_eq!(
            check_visual_asset("assets/a.svg", Background, b"<svg/>"),
            Err(VisualAssetError::UnknownFormat)
        );
        assert_eq!(
            check_visual_asset("assets/a.png", Background, &jpeg(10, 10)),
            Err(VisualAssetError::Malformed("PNG"))
        );
        assert_eq!(
            check_visual_asset("assets/a.jpeg", Background, &png(10, 10)),
            Err(VisualAssetError::Malformed("JPEG"))
        );
        assert!(matches!(
            check_visual_asset("assets/a.png", Icon, &png(513, 16)),
            Err(VisualAssetError::Dimensions { limit: 512, .. })
        ));
        assert!(matches!(
            check_visual_asset("assets/a.png", Background, &png(1025, 16)),
            Err(VisualAssetError::Dimensions { limit: 1024, .. })
        ));
        assert!(matches!(
            check_visual_asset("assets/a.png", Icon, &png(0, 16)),
            Err(VisualAssetError::Dimensions { .. })
        ));
        let mut large = png(16, 16);
        large.resize(MAX_VISUAL_ICON_BYTES + 1, 0);
        assert!(matches!(
            check_visual_asset("assets/a.png", Icon, &large),
            Err(VisualAssetError::TooLarge { .. })
        ));
        assert!(is_hex_color("#a0B1c2") && !is_hex_color("a0b1c2") && !is_hex_color("#abc"));
    }
}
