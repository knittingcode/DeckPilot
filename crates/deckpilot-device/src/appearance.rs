//! Colours, and how a device model's keys look when a key sets no colours.
//!
//! A [`DeviceModel`](crate::DeviceModel) may carry a [`DefaultAppearance`]:
//! the background a key has when it sets none (which may be
//! [`KeyBackground::Transparent`]), the default text colour and the colour
//! of the display behind the keys. A transparent key shows the display
//! background; the device gets opaque RGB images, so the renderer resolves
//! it to that colour before drawing ([`DefaultAppearance::effective_background`]).
//! Models without one use [`DefaultAppearance::BUILTIN`].

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An RGB colour, written as `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color(pub [u8; 3]);

impl Color {
    pub const BLACK: Self = Self([0, 0, 0]);
    pub const WHITE: Self = Self([255, 255, 255]);

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

/// A key's background: a colour, or transparent (the display background
/// shows through). Written as `#rrggbb` or `"transparent"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyBackground {
    Transparent,
    Color(Color),
}

impl KeyBackground {
    const TRANSPARENT: &'static str = "transparent";

    /// Parses `transparent` or `#rrggbb` (either case).
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.trim().eq_ignore_ascii_case(Self::TRANSPARENT) {
            return Ok(Self::Transparent);
        }
        Color::parse(value)
            .map(Self::Color)
            .map_err(|_| format!("“{value}” is not a background; use #rrggbb or transparent"))
    }

    /// The colour, `None` when transparent.
    pub fn color(self) -> Option<Color> {
        match self {
            Self::Transparent => None,
            Self::Color(color) => Some(color),
        }
    }

    /// The colour drawn on a display whose background is `display`: a
    /// transparent key shows the display.
    pub fn over(self, display: Color) -> Color {
        self.color().unwrap_or(display)
    }
}

impl From<Color> for KeyBackground {
    fn from(color: Color) -> Self {
        Self::Color(color)
    }
}

impl fmt::Display for KeyBackground {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transparent => f.write_str(Self::TRANSPARENT),
            Self::Color(color) => color.fmt(f),
        }
    }
}

impl Serialize for KeyBackground {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for KeyBackground {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// How a model's keys look when a key sets no colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultAppearance {
    /// A key's background when it sets none.
    pub key_background: KeyBackground,
    /// A key's text colour when it sets none.
    pub text_color: Color,
    /// The display behind the keys: what a transparent key shows.
    pub display_background: Color,
}

impl DefaultAppearance {
    /// The defaults of models that do not set their own (and of the
    /// Stream Deck models): transparent keys on a black display, white
    /// text.
    pub const BUILTIN: Self = Self {
        key_background: KeyBackground::Transparent,
        text_color: Color::WHITE,
        display_background: Color::BLACK,
    };

    /// The colour a key with `background` (its own, `None`: the default)
    /// is filled with: a transparent one shows the display background.
    pub fn effective_background(&self, background: Option<KeyBackground>) -> Color {
        background
            .unwrap_or(self.key_background)
            .over(self.display_background)
    }
}

impl Default for DefaultAppearance {
    fn default() -> Self {
        Self::BUILTIN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_and_print() {
        assert_eq!(Color::parse("#FF8000").unwrap(), Color([255, 128, 0]));
        assert_eq!(Color([255, 128, 0]).to_string(), "#ff8000");
        for bad in [
            "ff8000",
            "#ff80",
            "#gg0000",
            "#ff800000",
            "red",
            "",
            "transparent",
        ] {
            assert!(Color::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn backgrounds_are_a_colour_or_transparent() {
        for (text, background) in [
            ("transparent", KeyBackground::Transparent),
            ("Transparent", KeyBackground::Transparent),
            ("#00FF00", KeyBackground::Color(Color([0, 255, 0]))),
        ] {
            assert_eq!(KeyBackground::parse(text).unwrap(), background, "{text}");
        }
        for bad in ["", "none", "#00ff0", "green"] {
            assert!(KeyBackground::parse(bad).is_err(), "{bad}");
        }
        let json = serde_json::to_value([
            KeyBackground::Transparent,
            KeyBackground::Color(Color([1, 2, 3])),
        ])
        .unwrap();
        assert_eq!(json, serde_json::json!(["transparent", "#010203"]));
        let back: Vec<KeyBackground> = serde_json::from_value(json).unwrap();
        assert_eq!(back[1].color(), Some(Color([1, 2, 3])));
        assert!(serde_json::from_value::<KeyBackground>(serde_json::json!("clear")).is_err());
    }

    #[test]
    fn a_transparent_key_shows_the_display_background() {
        let builtin = DefaultAppearance::BUILTIN;
        assert_eq!(builtin.effective_background(None), Color::BLACK);
        let red = KeyBackground::Color(Color([200, 0, 0]));
        assert_eq!(builtin.effective_background(Some(red)), Color([200, 0, 0]));
        let blue_display = DefaultAppearance {
            display_background: Color([0, 0, 90]),
            ..builtin
        };
        assert_eq!(blue_display.effective_background(None), Color([0, 0, 90]));
        assert_eq!(
            blue_display.effective_background(Some(KeyBackground::Transparent)),
            Color([0, 0, 90])
        );
        let opaque = DefaultAppearance {
            key_background: KeyBackground::Color(Color([9, 9, 9])),
            ..blue_display
        };
        assert_eq!(opaque.effective_background(None), Color([9, 9, 9]));
        assert_eq!(
            opaque.effective_background(Some(KeyBackground::Transparent)),
            Color([0, 0, 90])
        );
        assert_eq!(
            serde_json::to_value(builtin).unwrap(),
            serde_json::json!({
                "keyBackground": "transparent",
                "textColor": "#ffffff",
                "displayBackground": "#000000",
            })
        );
    }
}
