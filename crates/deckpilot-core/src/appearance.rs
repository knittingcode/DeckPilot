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
use crate::assets::{AssetRef, Background};

/// Largest width and height of a stored icon, in pixels.
pub const MAX_ICON_SIZE: u32 = 256;
/// Largest stored icon (encoded PNG), in bytes.
pub const MAX_ICON_BYTES: usize = 192 * 1024;
/// Most icons in one profile.
pub const MAX_ICONS: usize = 64;
/// Longest icon name, in characters.
pub const MAX_ICON_NAME: usize = 64;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Colours live with the device descriptions (a model's
/// [`DefaultAppearance`] uses them); re-exported here, where they always
/// were.
pub use deckpilot_device::{Color, DefaultAppearance, KeyBackground};

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

/// A key's icon: one of the profile's [`Icon`]s (My Icons, by its UUID,
/// as stored before the library existed) or an icon of the shared library
/// (`deckpilot.<name>`, `<plugin>.<name>`; [`crate::assets`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconRef {
    pub id: AssetRef,
    #[serde(default)]
    pub fit: IconFit,
}

/// How a key's title is laid out (`docs/key-text.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextMode {
    /// Wrapped at word boundaries (inside a word only when the word alone
    /// is wider than the line), explicit line breaks kept, at most
    /// `maxLines` lines; the last one ends with `…` when text is left.
    /// The default: what keys did before text modes existed.
    #[default]
    Wrap,
    /// One line at the preferred size, never smaller; what does not fit
    /// is cut with `…` or clipped ([`TextOverflow`]).
    SingleLine,
    /// The preferred size, made smaller only as far as needed (never below
    /// `minSize`, never larger), on one line or several ([`FitLines`]).
    AutoFit,
    /// One line that scrolls sideways when it is wider than the key, with
    /// a pause at the start and at the end.
    Marquee,
}

/// What a single line does with text wider than the key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextOverflow {
    /// Cut with `…`.
    #[default]
    Ellipsis,
    /// Cut at the edge of the text area.
    Clip,
}

/// Whether auto fit keeps the text on one line or may wrap it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FitLines {
    #[default]
    SingleLine,
    Multiline,
}

/// Horizontal alignment of the title's lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// Smallest and largest text size, in points (pixels on a 72 px key; see
/// [`TextAppearance::size`]).
pub const TEXT_SIZE_POINTS: (u8, u8) = (6, 32);
/// Smallest auto-fit size when none is set, in points.
pub const DEFAULT_MIN_TEXT_POINTS: u8 = 7;
/// Most lines a wrapped or fitted title may have.
pub const MAX_TEXT_LINES: u8 = 6;
/// Slowest and fastest marquee speed, in device pixels per second.
pub const MARQUEE_SPEED: (u16, u16) = (5, 240);
/// Marquee speed when none is set, in device pixels per second.
pub const DEFAULT_MARQUEE_SPEED: u16 = 30;
/// Longest marquee pause, in milliseconds.
pub const MAX_MARQUEE_PAUSE_MS: u16 = 10_000;
/// Marquee pause at the start and the end when none is set, in
/// milliseconds.
pub const DEFAULT_MARQUEE_PAUSE_MS: u16 = 1_000;

/// How a key's title is laid out. Every field is optional; fields that do
/// not belong to the mode are kept (so switching modes back and forth keeps
/// them) but not used. See `docs/key-text.md` for the rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextAppearance {
    #[serde(default)]
    pub mode: TextMode,
    /// Preferred size in points: pixels on a 72 px key, scaled with the
    /// key's shorter side on other keys. `None`: [`KeyAppearance::font_size`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u8>,
    /// Horizontal alignment. Default: centred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<TextAlign>,
    /// Wrap and multiline auto fit: most lines. `None`: as many as the key
    /// has room for next to its icon (two with an icon, three without).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u8>,
    /// Single line: what happens to text that is too wide. Default: `…`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow: Option<TextOverflow>,
    /// Auto fit: the smallest size in points. Default:
    /// [`DEFAULT_MIN_TEXT_POINTS`] (never above the preferred size).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size: Option<u8>,
    /// Auto fit: one line or several. Default: one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<FitLines>,
    /// Marquee: device pixels per second. Default:
    /// [`DEFAULT_MARQUEE_SPEED`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<u16>,
    /// Marquee: pause at the start and at the end, in milliseconds.
    /// Default: [`DEFAULT_MARQUEE_PAUSE_MS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause: Option<u16>,
}

impl TextAppearance {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// `None` when nothing is set.
    pub fn normalized(self) -> Option<Self> {
        (!self.is_default()).then_some(self)
    }

    /// The preferred size in points, if one is set (clamped to
    /// [`TEXT_SIZE_POINTS`]).
    pub fn size_points(&self) -> Option<u8> {
        self.size
            .map(|size| size.clamp(TEXT_SIZE_POINTS.0, TEXT_SIZE_POINTS.1))
    }

    /// The auto-fit minimum in points (clamped to [`TEXT_SIZE_POINTS`]).
    pub fn min_points(&self) -> u8 {
        self.min_size
            .unwrap_or(DEFAULT_MIN_TEXT_POINTS)
            .clamp(TEXT_SIZE_POINTS.0, TEXT_SIZE_POINTS.1)
    }

    /// The most lines, if set (clamped to 1–[`MAX_TEXT_LINES`]).
    pub fn max_lines(&self) -> Option<usize> {
        self.max_lines
            .map(|lines| usize::from(lines.clamp(1, MAX_TEXT_LINES)))
    }

    pub fn align(&self) -> TextAlign {
        self.align.unwrap_or_default()
    }

    pub fn overflow(&self) -> TextOverflow {
        self.overflow.unwrap_or_default()
    }

    pub fn fit(&self) -> FitLines {
        self.fit.unwrap_or_default()
    }

    /// The marquee speed in device pixels per second (clamped).
    pub fn speed(&self) -> u16 {
        self.speed
            .unwrap_or(DEFAULT_MARQUEE_SPEED)
            .clamp(MARQUEE_SPEED.0, MARQUEE_SPEED.1)
    }

    /// The marquee pause in milliseconds (clamped).
    pub fn pause_ms(&self) -> u16 {
        self.pause
            .unwrap_or(DEFAULT_MARQUEE_PAUSE_MS)
            .min(MAX_MARQUEE_PAUSE_MS)
    }

    /// Refuses values out of range, so the app reports them when saving.
    /// (The renderer clamps them anyway, e.g. in a hand-edited import.)
    pub fn check(&self) -> Result<(), String> {
        let (least, most) = TEXT_SIZE_POINTS;
        for (name, value) in [
            ("text size", self.size),
            ("minimum text size", self.min_size),
        ] {
            if let Some(value) = value
                && !(least..=most).contains(&value)
            {
                return Err(format!("{name} must be {least} to {most} points"));
            }
        }
        if let Some(lines) = self.max_lines
            && !(1..=MAX_TEXT_LINES).contains(&lines)
        {
            return Err(format!("a title has 1 to {MAX_TEXT_LINES} lines"));
        }
        if let Some(speed) = self.speed
            && !(MARQUEE_SPEED.0..=MARQUEE_SPEED.1).contains(&speed)
        {
            return Err(format!(
                "scrolling speed must be {} to {} pixels per second",
                MARQUEE_SPEED.0, MARQUEE_SPEED.1
            ));
        }
        if let Some(pause) = self.pause
            && pause > MAX_MARQUEE_PAUSE_MS
        {
            return Err(format!(
                "a scrolling pause is at most {} seconds",
                MAX_MARQUEE_PAUSE_MS / 1000
            ));
        }
        Ok(())
    }
}

/// A title font, by the id stored in [`KeyAppearance::font_family`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleFont {
    pub id: &'static str,
    /// Its name as the app shows it (a font's name is not translated).
    pub name: &'static str,
}

/// The fonts a key title may use: only fonts built into the renderer, so
/// every device and the app's previews draw them alike. The one source of
/// the list (the app asks for it; the renderer has faces for each).
pub const TITLE_FONTS: &[TitleFont] = &[TitleFont {
    id: "inter",
    name: "Inter",
}];

/// The font of titles that set none.
pub const DEFAULT_TITLE_FONT: &str = "inter";

/// A title font id (see [`TITLE_FONTS`]). Any string loads and is kept as
/// it is (a layout from a later version may name a font this one does not
/// have; it is drawn with the default font); [`KeyAppearance::check`]
/// refuses ids that are not in the list.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FontFamilyId(pub String);

impl FontFamilyId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The font, when it is one of [`TITLE_FONTS`].
    pub fn font(&self) -> Option<&'static TitleFont> {
        TITLE_FONTS.iter().find(|font| font.id == self.0)
    }
}

/// Parts of a key title's look that the editor offers (a plugin action may
/// support only some, [`ActionTitle::supports`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TitleFeature {
    /// Show or hide the title.
    Visibility,
    /// Top, middle, bottom.
    Position,
    /// Font family and size.
    Font,
    /// Bold, italic, underline.
    Style,
    /// Text colour.
    Color,
    /// Wrap, single line, auto fit, scrolling, alignment.
    Layout,
}

/// What an action type says about its keys' titles (plugin API 1.8
/// `contributes.actions[].title`): suggestions only. The user's own title
/// and look always win; nothing here is stored in a layout.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionTitle {
    /// The title a key shows while neither the key nor the action has a
    /// label of its own and the action still has its type's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// The action sets the title while it runs (its state's label).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dynamic: bool,
    /// Suggested position, used while the key sets none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<TitleAlign>,
    /// The parts of the look the editor offers; `None`: all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports: Option<Vec<TitleFeature>>,
}

impl ActionTitle {
    /// Whether the editor offers `feature` for keys of this action.
    pub fn supports(&self, feature: TitleFeature) -> bool {
        self.supports
            .as_ref()
            .is_none_or(|features| features.contains(&feature))
    }
}

/// How a key looks. Every field is optional; unset colours are the device
/// model's defaults ([`DefaultAppearance`]), whatever the kind of key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAppearance {
    /// A colour, `transparent` (the display shows through) or an asset of
    /// the library (`asset:<ref>`: a gradient or an image). `None`: the
    /// model's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
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
    /// How the title is laid out: wrapped, on one line, fitted or
    /// scrolling (`docs/key-text.md`). `None`: wrapped, as before text
    /// modes existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextAppearance>,
    /// The title's font ([`TITLE_FONTS`]). `None`: [`DEFAULT_TITLE_FONT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<FontFamilyId>,
    /// Bold title (`Some(false)`: explicitly not bold). `None`: not bold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// Italic title. `None`: upright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Underlined title. `None`: not underlined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
}

impl KeyAppearance {
    pub fn is_default(&self) -> bool {
        let mut plain = self.clone();
        plain.text = plain.text.filter(|text| !text.is_default());
        plain == Self::default()
    }

    /// `None` when nothing is set, so default appearances are not stored;
    /// a text appearance that sets nothing is dropped too.
    pub fn normalized(mut self) -> Option<Self> {
        self.text = self.text.and_then(TextAppearance::normalized);
        (!self.is_default()).then_some(self)
    }

    /// The text appearance, the default (wrapped) when none is set.
    pub fn text(&self) -> TextAppearance {
        self.text.unwrap_or_default()
    }

    /// Checks the text appearance's values (see [`TextAppearance::check`])
    /// and the font.
    pub fn check(&self) -> Result<(), String> {
        if let Some(family) = &self.font_family
            && family.font().is_none()
        {
            return Err(format!("`{}` is not a title font", family.as_str()));
        }
        self.text.as_ref().map_or(Ok(()), TextAppearance::check)
    }

    pub fn is_bold(&self) -> bool {
        self.bold.unwrap_or(false)
    }

    pub fn is_italic(&self) -> bool {
        self.italic.unwrap_or(false)
    }

    pub fn is_underlined(&self) -> bool {
        self.underline.unwrap_or(false)
    }

    /// The title font: the key's when it is a known one, else the default.
    pub fn font(&self) -> &'static TitleFont {
        self.font_family
            .as_ref()
            .and_then(FontFamilyId::font)
            .unwrap_or(&TITLE_FONTS[0])
    }

    /// This look with an action type's title suggestions filled in where
    /// the key sets nothing (the key's own values win). For the face only;
    /// never stored.
    #[must_use]
    pub fn with_suggestions(mut self, title: &ActionTitle) -> Self {
        if self.title_align.is_none() {
            self.title_align = title.position;
        }
        self
    }

    /// The profile images (My Icons) the look shows: its icon and its
    /// background image, each once.
    pub fn profile_icons(&self) -> impl Iterator<Item = IconId> + '_ {
        let icon = self.icon.as_ref().and_then(|icon| icon.id.profile_icon());
        let background = self
            .background
            .as_ref()
            .and_then(Background::asset)
            .and_then(AssetRef::profile_icon)
            .filter(|background| Some(*background) != icon);
        icon.into_iter().chain(background)
    }

    /// Whether the look shows the profile image `id` (as icon or
    /// background).
    pub fn shows_icon(&self, id: IconId) -> bool {
        self.profile_icons().any(|shown| shown == id)
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
    fn backgrounds_unset_transparent_and_coloured_stay_apart() {
        let unset: KeyAppearance = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(unset.background, None);
        let transparent: KeyAppearance =
            serde_json::from_value(serde_json::json!({"background": "transparent"})).unwrap();
        assert_eq!(transparent.background, Some(Background::TRANSPARENT));
        assert_eq!(
            serde_json::to_value(&transparent).unwrap(),
            serde_json::json!({"background": "transparent"})
        );
        assert!(!transparent.is_default(), "an explicit transparent is kept");
        // An explicit black or white is stored as such, never as "unset".
        for colour in ["#000000", "#ffffff"] {
            let look: KeyAppearance =
                serde_json::from_value(serde_json::json!({"background": colour})).unwrap();
            assert_eq!(serde_json::to_value(&look).unwrap()["background"], colour);
            assert!(look.clone().normalized().is_some());
        }
    }

    #[test]
    fn appearances_serialize_compactly_and_default_to_nothing() {
        let appearance = KeyAppearance {
            background: Some(Color([0, 0, 255]).into()),
            font_size: Some(FontSize::Large),
            show_title: Some(false),
            icon: Some(IconRef {
                id: IconId::new().into(),
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
    fn title_typography_is_optional_and_round_trips() {
        // A look stored before typography existed loads unchanged.
        let old: KeyAppearance = serde_json::from_value(serde_json::json!({
            "textColor": "#ffcc00", "titleAlign": "top", "showTitle": false
        }))
        .unwrap();
        assert_eq!(old.font_family, None);
        assert_eq!((old.bold, old.italic, old.underline), (None, None, None));
        assert!(!old.is_bold() && !old.is_italic() && !old.is_underlined());
        assert_eq!(old.font().id, DEFAULT_TITLE_FONT);
        let value = serde_json::to_value(&old).unwrap();
        for field in ["fontFamily", "bold", "italic", "underline"] {
            assert!(value.get(field).is_none(), "{field} is not written");
        }

        let look = KeyAppearance {
            font_family: Some(FontFamilyId::new("inter")),
            bold: Some(true),
            italic: Some(false),
            underline: Some(true),
            ..KeyAppearance::default()
        };
        let value = serde_json::to_value(&look).unwrap();
        assert_eq!(value["fontFamily"], "inter");
        assert_eq!(value["bold"], true);
        // An explicit "not italic" is kept: it can beat a suggestion.
        assert_eq!(value["italic"], false);
        let back: KeyAppearance = serde_json::from_value(value).unwrap();
        assert_eq!(back, look);
        assert!(back.check().is_ok());
        assert!(back.normalized().is_some());
    }

    #[test]
    fn unknown_title_fonts_are_kept_but_refused_when_saved() {
        let later: KeyAppearance =
            serde_json::from_value(serde_json::json!({"fontFamily": "someday-sans"})).unwrap();
        // Kept as it is (a later version's font) and drawn with Inter.
        assert_eq!(
            serde_json::to_value(&later).unwrap()["fontFamily"],
            "someday-sans"
        );
        assert_eq!(later.font().id, "inter");
        assert!(later.check().unwrap_err().contains("someday-sans"));
        // Every listed font is known.
        for font in TITLE_FONTS {
            assert_eq!(FontFamilyId::new(font.id).font(), Some(font));
        }
    }

    #[test]
    fn suggestions_fill_in_only_unset_fields() {
        let title = ActionTitle {
            position: Some(TitleAlign::Top),
            ..ActionTitle::default()
        };
        assert_eq!(
            KeyAppearance::default()
                .with_suggestions(&title)
                .title_align,
            Some(TitleAlign::Top)
        );
        let own = KeyAppearance {
            title_align: Some(TitleAlign::Bottom),
            ..KeyAppearance::default()
        };
        assert_eq!(
            own.with_suggestions(&title).title_align,
            Some(TitleAlign::Bottom)
        );
        assert!(title.supports(TitleFeature::Color));
        let limited = ActionTitle {
            supports: Some(vec![TitleFeature::Position]),
            ..ActionTitle::default()
        };
        assert!(limited.supports(TitleFeature::Position));
        assert!(!limited.supports(TitleFeature::Color));
        // Sent without empty fields.
        assert_eq!(
            serde_json::to_value(ActionTitle::default()).unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn text_appearances_default_to_wrapping_and_are_not_stored_empty() {
        // Layouts written before text modes: no `text`, wrapped.
        let old: KeyAppearance =
            serde_json::from_value(serde_json::json!({"fontSize": "large"})).unwrap();
        assert_eq!(old.text, None);
        assert_eq!(old.text().mode, TextMode::Wrap);
        assert!(serde_json::to_value(&old).unwrap().get("text").is_none());
        // A text appearance that sets nothing is not stored.
        let empty = KeyAppearance {
            text: Some(TextAppearance::default()),
            ..KeyAppearance::default()
        };
        assert!(empty.is_default());
        assert_eq!(empty.normalized(), None);
        let only_mode: TextAppearance = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(only_mode, TextAppearance::default());

        let text = TextAppearance {
            mode: TextMode::Marquee,
            size: Some(14),
            align: Some(TextAlign::Left),
            speed: Some(60),
            pause: Some(500),
            ..TextAppearance::default()
        };
        let look = KeyAppearance {
            text: Some(text),
            ..KeyAppearance::default()
        };
        let value = serde_json::to_value(&look).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"text": {
                "mode": "marquee", "size": 14, "align": "left", "speed": 60, "pause": 500
            }})
        );
        let back: KeyAppearance = serde_json::from_value(value).unwrap();
        assert_eq!(back, look);
        assert_eq!(back.clone().normalized(), Some(look));
        for (json, mode) in [
            ("wrap", TextMode::Wrap),
            ("singleLine", TextMode::SingleLine),
            ("autoFit", TextMode::AutoFit),
            ("marquee", TextMode::Marquee),
        ] {
            let text: TextAppearance =
                serde_json::from_value(serde_json::json!({"mode": json})).unwrap();
            assert_eq!(text.mode, mode);
        }
        assert!(
            serde_json::from_value::<TextAppearance>(serde_json::json!({"mode": "spin"})).is_err()
        );
    }

    #[test]
    fn text_values_are_checked_and_clamped() {
        let defaults = TextAppearance::default();
        assert_eq!(defaults.min_points(), DEFAULT_MIN_TEXT_POINTS);
        assert_eq!(defaults.speed(), DEFAULT_MARQUEE_SPEED);
        assert_eq!(defaults.pause_ms(), DEFAULT_MARQUEE_PAUSE_MS);
        assert_eq!(defaults.align(), TextAlign::Center);
        assert_eq!(defaults.overflow(), TextOverflow::Ellipsis);
        assert_eq!(defaults.fit(), FitLines::SingleLine);
        assert!(defaults.check().is_ok());
        for bad in [
            TextAppearance {
                size: Some(2),
                ..defaults
            },
            TextAppearance {
                min_size: Some(99),
                ..defaults
            },
            TextAppearance {
                max_lines: Some(0),
                ..defaults
            },
            TextAppearance {
                max_lines: Some(9),
                ..defaults
            },
            TextAppearance {
                speed: Some(1),
                ..defaults
            },
            TextAppearance {
                pause: Some(60_000),
                ..defaults
            },
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
            assert!(
                KeyAppearance {
                    text: Some(bad),
                    ..KeyAppearance::default()
                }
                .check()
                .is_err()
            );
        }
        let wild = TextAppearance {
            size: Some(200),
            min_size: Some(1),
            max_lines: Some(40),
            speed: Some(9999),
            pause: Some(u16::MAX),
            ..defaults
        };
        assert_eq!(wild.size_points(), Some(TEXT_SIZE_POINTS.1));
        assert_eq!(wild.min_points(), TEXT_SIZE_POINTS.0);
        assert_eq!(wild.max_lines(), Some(usize::from(MAX_TEXT_LINES)));
        assert_eq!(wild.speed(), MARQUEE_SPEED.1);
        assert_eq!(wild.pause_ms(), MAX_MARQUEE_PAUSE_MS);
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

    /// Icon data is stored in profile documents and exports, so its text
    /// must never change: standard alphabet (`+` and `/`), `=` padding, and
    /// strict decoding. The vectors are those of RFC 4648 section 10 and all
    /// 256 byte values, as `base64` 0.22 wrote them.
    #[test]
    fn icon_data_encoding_is_fixed() {
        const ALL_BYTES: &str = concat!(
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BB",
            "QkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn+AgYKD",
            "hIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TF",
            "xsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w==",
        );
        let all_bytes: Vec<u8> = (0..=255).collect();
        let vectors: [(&[u8], &str); 9] = [
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
            (&[0xfb, 0xff], "+/8="),
            (&all_bytes, ALL_BYTES),
        ];
        for (bytes, text) in vectors {
            let data = IconData::new(bytes.to_vec());
            assert_eq!(serde_json::to_value(&data).unwrap(), text);
            assert_eq!(
                data.data_url(),
                format!("data:image/png;base64,{text}"),
                "{text}"
            );
            let decoded: IconData = serde_json::from_value(text.into()).unwrap();
            assert_eq!(decoded.bytes(), bytes, "{text}");
        }
        // Strict decoding: missing padding, non-zero trailing bits, the
        // URL-safe alphabet and whitespace are all refused.
        for text in ["Zg", "Zm8", "Zh==", "Zm9=", "-_8=", "Zm9v\n", " Zm9v"] {
            assert!(
                serde_json::from_value::<IconData>(text.into()).is_err(),
                "{text:?} was accepted"
            );
        }
    }
}
