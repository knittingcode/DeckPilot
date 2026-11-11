//! How a key looks: colours, title placement and size, and custom icons.
//!
//! A [`KeyAppearance`] is optional per layout binding; every field is
//! optional and falls back to the default look of the key's style, so
//! layouts written before appearances existed are unchanged.
//!
//! Icons are images the user imported into a profile ([`Profile::icons`]).
//! They are stored as small PNGs that DeckPilot encoded itself (imported
//! PNG and JPEG files are decoded, scaled down and encoded again, which
//! drops metadata), travel with the profile in exports, and are checked
//! here without decoding them: size in bytes, PNG signature and declared
//! dimensions. Decoding (with limits) is left to the renderer.
//!
//! [`Profile::icons`]: crate::Profile::icons

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::IconId;

/// Largest width and height of a stored icon, in pixels.
pub const MAX_ICON_SIZE: u32 = 256;
/// Largest stored icon (encoded PNG), in bytes.
pub const MAX_ICON_BYTES: usize = 192 * 1024;
/// Most icons in one profile.
pub const MAX_ICONS: usize = 64;
/// Longest icon name, in characters.
pub const MAX_ICON_NAME: usize = 64;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// An RGB colour, written as `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color(pub [u8; 3]);

impl Color {
    pub fn rgb(self) -> [u8; 3] {
        self.0
    }

    /// Parses `#rrggbb` (either case).
    pub fn parse(value: &str) -> Result<Self, String> {
        let invalid = || format!("“{value}” is not a colour; use #rrggbb");
        let hex = value.trim().strip_prefix('#').ok_or_else(invalid)?;
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let channel =
            |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).map_err(|_| invalid());
        Ok(Self([channel(0)?, channel(2)?, channel(4)?]))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "#{r:02x}{g:02x}{b:02x}")
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// Size of a key's title text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// Where the title sits on the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TitleAlign {
    Top,
    Middle,
    Bottom,
}

/// How an icon is scaled to the key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IconFit {
    /// Whole icon visible, aspect ratio kept (may leave borders).
    #[default]
    Fit,
    /// Stretched to the whole key (aspect ratio not kept).
    Fill,
    /// Covers the whole key, aspect ratio kept, the overflow cut off.
    Crop,
}

/// A key's icon: one of the profile's [`Icon`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconRef {
    pub id: IconId,
    #[serde(default)]
    pub fit: IconFit,
}

/// How a key looks. Every field is optional; unset fields keep the look of
/// the key's style.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAppearance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<FontSize>,
    /// Default: middle, or bottom when the key has an icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_align: Option<TitleAlign>,
    /// `false` hides the title (and the secondary line). Default: shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_title: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<IconRef>,
}

impl KeyAppearance {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// `None` when nothing is set, so default appearances are not stored.
    pub fn normalized(self) -> Option<Self> {
        (!self.is_default()).then_some(self)
    }

    pub fn shows_title(&self) -> bool {
        self.show_title.unwrap_or(true)
    }

    /// The title position, with the default for keys with and without an
    /// icon.
    pub fn title_align(&self) -> TitleAlign {
        self.title_align.unwrap_or(if self.icon.is_some() {
            TitleAlign::Bottom
        } else {
            TitleAlign::Middle
        })
    }
}

/// PNG bytes, stored as base64 in profile documents and exports.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct IconData(Vec<u8>);

impl IconData {
    pub fn new(png: Vec<u8>) -> Self {
        Self(png)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The PNG as a `data:` URL, e.g. for an `<img>` in the app.
    pub fn data_url(&self) -> String {
        format!("data:image/png;base64,{}", STANDARD.encode(&self.0))
    }
}

impl fmt::Debug for IconData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IconData({} bytes)", self.0.len())
    }
}

impl Serialize for IconData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for IconData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        // Base64 is 4/3 of the bytes; refuse far larger strings before
        // decoding them.
        if value.len() > MAX_ICON_BYTES / 3 * 4 + 8 {
            return Err(serde::de::Error::custom("icon data is too large"));
        }
        STANDARD
            .decode(value)
            .map(Self)
            .map_err(|_| serde::de::Error::custom("icon data is not valid base64"))
    }
}

/// An image imported into a profile, for key icons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Icon {
    pub id: IconId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// A PNG encoded by DeckPilot, at most [`MAX_ICON_SIZE`] pixels wide and
    /// high and [`MAX_ICON_BYTES`] long.
    pub png: IconData,
}

impl Icon {
    /// Checks the icon without decoding it: name, byte size, PNG signature
    /// and the dimensions in its header against the declared ones and the
    /// limits.
    pub fn check(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > MAX_ICON_NAME {
            return Err(format!(
                "an icon name must have 1 to {MAX_ICON_NAME} characters"
            ));
        }
        if self.png.len() > MAX_ICON_BYTES {
            return Err(format!(
                "icon “{name}” is larger than {} KiB",
                MAX_ICON_BYTES / 1024
            ));
        }
        let (width, height) = png_dimensions(self.png.bytes())
            .ok_or_else(|| format!("icon “{name}” is not a PNG image"))?;
        if (width, height) != (self.width, self.height) {
            return Err(format!("icon “{name}” has other dimensions than declared"));
        }
        if width == 0 || height == 0 || width > MAX_ICON_SIZE || height > MAX_ICON_SIZE {
            return Err(format!(
                "icon “{name}” must be at most {MAX_ICON_SIZE} x {MAX_ICON_SIZE} pixels"
            ));
        }
        Ok(())
    }
}

/// Width and height from a PNG's header (`IHDR`, the first chunk).
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(PNG_SIGNATURE) || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let read =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Some((read(16), read(20)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PNG header claiming `width` x `height` (not a complete image).
    pub(crate) fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend(13u32.to_be_bytes());
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0]);
        bytes
    }

    fn icon(width: u32, height: u32, png: Vec<u8>) -> Icon {
        Icon {
            id: IconId::new(),
            name: "Logo".into(),
            width,
            height,
            png: IconData::new(png),
        }
    }

    #[test]
    fn colours_are_validated() {
        assert_eq!(Color::parse("#FF8000").unwrap(), Color([255, 128, 0]));
        assert_eq!(Color([255, 128, 0]).to_string(), "#ff8000");
        for bad in ["ff8000", "#ff80", "#gg0000", "#ff800000", "red", ""] {
            assert!(Color::parse(bad).is_err(), "{bad}");
        }
        let json: Result<KeyAppearance, _> =
            serde_json::from_value(serde_json::json!({"background": "blue"}));
        assert!(json.is_err());
    }

    #[test]
    fn appearances_serialize_compactly_and_default_to_nothing() {
        let appearance = KeyAppearance {
            background: Some(Color([0, 0, 255])),
            font_size: Some(FontSize::Large),
            show_title: Some(false),
            icon: Some(IconRef {
                id: IconId::new(),
                fit: IconFit::Crop,
            }),
            ..KeyAppearance::default()
        };
        let value = serde_json::to_value(&appearance).unwrap();
        assert_eq!(value["background"], "#0000ff");
        assert_eq!(value["fontSize"], "large");
        assert_eq!(value["icon"]["fit"], "crop");
        assert!(value.get("textColor").is_none());
        let back: KeyAppearance = serde_json::from_value(value).unwrap();
        assert_eq!(back, appearance);
        assert!(!back.shows_title());
        assert_eq!(back.title_align(), TitleAlign::Bottom);
        assert_eq!(KeyAppearance::default().normalized(), None);
        assert_eq!(KeyAppearance::default().title_align(), TitleAlign::Middle);
        // An icon without a fit fits.
        let id = IconId::new();
        let reference: IconRef = serde_json::from_value(serde_json::json!({"id": id})).unwrap();
        assert_eq!(reference.fit, IconFit::Fit);
    }

    #[test]
    fn icons_are_checked_without_decoding() {
        assert!(icon(64, 32, png_header(64, 32)).check().is_ok());
        assert!(icon(64, 64, png_header(64, 32)).check().is_err());
        assert!(icon(512, 512, png_header(512, 512)).check().is_err());
        assert!(icon(0, 1, png_header(0, 1)).check().is_err());
        assert!(
            icon(1, 1, b"\xff\xd8\xff\xe0 jpeg".to_vec())
                .check()
                .is_err()
        );
        let mut large = png_header(16, 16);
        large.resize(MAX_ICON_BYTES + 1, 0);
        assert!(icon(16, 16, large).check().is_err());
        let mut unnamed = icon(16, 16, png_header(16, 16));
        unnamed.name = "  ".into();
        assert!(unnamed.check().is_err());
    }

    #[test]
    fn icon_data_travels_as_base64() {
        let icon = icon(16, 16, png_header(16, 16));
        let value = serde_json::to_value(&icon).unwrap();
        assert!(value["png"].as_str().unwrap().starts_with("iVBORw0KGgo"));
        assert_eq!(serde_json::from_value::<Icon>(value).unwrap(), icon);
        let bad = serde_json::json!({
            "id": IconId::new(), "name": "x", "width": 1, "height": 1, "png": "***"
        });
        assert!(serde_json::from_value::<Icon>(bad).is_err());
        let huge = serde_json::json!({
            "id": IconId::new(), "name": "x", "width": 1, "height": 1,
            "png": "A".repeat(MAX_ICON_BYTES * 2)
        });
        assert!(serde_json::from_value::<Icon>(huge).is_err());
        assert!(icon.png.data_url().starts_with("data:image/png;base64,"));
    }
}
