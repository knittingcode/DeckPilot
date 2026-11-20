//! The shared visual asset library: icons and backgrounds a key refers to
//! by a stable id (`docs/key-text.md`, "Icons and backgrounds").
//!
//! Three sources share one id space:
//!
//! - **DeckPilot** (built in, always there): the Lucide icons of the
//!   renderer (`deckpilot.<lucide-name>`, e.g. `deckpilot.house`) and the
//!   gradients of [`BUILTIN_GRADIENTS`] (`deckpilot.gradient-<name>`).
//! - **My Icons**: images the user imported into the profile
//!   ([`Profile::icons`](crate::Profile::icons)), referred to by their
//!   [`IconId`] (a UUID), as before.
//! - **Plugins**: icons and backgrounds a plugin's manifest declares
//!   (`contributes.visualAssets`, plugin API 1.8), namespaced by the
//!   plugin id: `<publisher>.<plugin>.<name>`.
//!
//! An [`AssetId`] is `<namespace>.<name>`: the name is the part after the
//! last dot, so a built-in id has one dot and a plugin's two (plugin ids
//! are `<publisher>.<name>`). Keys store only the reference, never image
//! data. A reference to an asset that is not there (a plugin disabled or
//! uninstalled, an id of a later version) is kept as it is; the key is
//! drawn as if it set none (no icon, the device's default background)
//! until the asset is back.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Color, IconId, KeyBackground};

/// The namespace of DeckPilot's own assets.
pub const BUILTIN_NAMESPACE: &str = "deckpilot";
/// Longest name of an asset (the part after the namespace).
pub const MAX_ASSET_NAME: usize = 48;
/// Longest asset id.
pub const MAX_ASSET_ID: usize = 128;
/// The prefix of a background that is an asset (`asset:<ref>`).
pub const BACKGROUND_ASSET_PREFIX: &str = "asset:";
/// Most colours of a gradient.
pub const MAX_GRADIENT_COLORS: usize = 4;

/// `[a-z0-9]+(-[a-z0-9]+)*`, 1 to `max` characters.
fn is_name(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.len() <= max
        && text.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// A library asset's id: `<namespace>.<name>` (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(String);

impl AssetId {
    /// Parses an id: a namespace of one or two dot-separated words (the
    /// built-in one, or a plugin id) and a name.
    pub fn parse(text: &str) -> Result<Self, String> {
        let invalid = || format!("“{text}” is not an asset id");
        if text.len() > MAX_ASSET_ID {
            return Err(invalid());
        }
        let (namespace, name) = text.rsplit_once('.').ok_or_else(invalid)?;
        let words: Vec<&str> = namespace.split('.').collect();
        if !is_name(name, MAX_ASSET_NAME)
            || words.len() > 2
            || !words.iter().all(|word| is_name(word, 32))
        {
            return Err(invalid());
        }
        Ok(Self(text.to_owned()))
    }

    /// A built-in asset (`deckpilot.<name>`).
    pub fn builtin(name: &str) -> Result<Self, String> {
        Self::parse(&format!("{BUILTIN_NAMESPACE}.{name}"))
    }

    /// An asset of the plugin `plugin` (`<plugin>.<name>`).
    pub fn of_plugin(plugin: &str, name: &str) -> Result<Self, String> {
        if !plugin.contains('.') {
            return Err(format!("“{plugin}” is not a plugin id"));
        }
        Self::parse(&format!("{plugin}.{name}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The namespace: [`BUILTIN_NAMESPACE`] or a plugin id.
    pub fn namespace(&self) -> &str {
        self.0
            .rsplit_once('.')
            .map_or("", |(namespace, _)| namespace)
    }

    /// The name inside the namespace.
    pub fn name(&self) -> &str {
        self.0.rsplit_once('.').map_or("", |(_, name)| name)
    }

    pub fn is_builtin(&self) -> bool {
        self.namespace() == BUILTIN_NAMESPACE
    }

    /// The plugin that contributes it, `None` for a built-in asset.
    pub fn plugin(&self) -> Option<&str> {
        let namespace = self.namespace();
        namespace.contains('.').then_some(namespace)
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for AssetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AssetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// What a key's icon or background image refers to: one of the profile's
/// imported images (My Icons, by its UUID, as stored before the library
/// existed) or a library asset (built in or a plugin's).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetRef {
    /// An image of the profile ([`crate::Icon`]).
    Profile(IconId),
    Library(AssetId),
}

impl AssetRef {
    /// A UUID is a profile image; anything else must be an [`AssetId`].
    pub fn parse(text: &str) -> Result<Self, String> {
        if let Ok(id) = text.parse::<IconId>() {
            return Ok(Self::Profile(id));
        }
        AssetId::parse(text).map(Self::Library)
    }

    /// The profile image it refers to, if it is one.
    pub fn profile_icon(&self) -> Option<IconId> {
        match self {
            Self::Profile(id) => Some(*id),
            Self::Library(_) => None,
        }
    }

    /// The library asset it refers to, if it is one.
    pub fn asset(&self) -> Option<&AssetId> {
        match self {
            Self::Profile(_) => None,
            Self::Library(id) => Some(id),
        }
    }
}

impl From<IconId> for AssetRef {
    fn from(id: IconId) -> Self {
        Self::Profile(id)
    }
}

impl From<AssetId> for AssetRef {
    fn from(id: AssetId) -> Self {
        Self::Library(id)
    }
}

impl PartialEq<IconId> for AssetRef {
    fn eq(&self, other: &IconId) -> bool {
        self.profile_icon() == Some(*other)
    }
}

impl fmt::Display for AssetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(id) => id.fmt(f),
            Self::Library(id) => id.fmt(f),
        }
    }
}

impl Serialize for AssetRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AssetRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// A key's background as stored: a colour or `transparent` (written as
/// before: `#rrggbb`, `"transparent"`), or an asset of the library
/// (`"asset:<ref>"`: a gradient or an image). Unset (`None` in
/// [`KeyAppearance::background`](crate::KeyAppearance::background)) is the
/// device's default, never one of these.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Background {
    Plain(KeyBackground),
    Asset(AssetRef),
}

impl Background {
    pub const TRANSPARENT: Self = Self::Plain(KeyBackground::Transparent);

    pub fn parse(text: &str) -> Result<Self, String> {
        if let Some(reference) = text.trim().strip_prefix(BACKGROUND_ASSET_PREFIX) {
            return AssetRef::parse(reference).map(Self::Asset);
        }
        KeyBackground::parse(text).map(Self::Plain).map_err(|_| {
            format!("“{text}” is not a background; use #rrggbb, transparent or asset:<id>")
        })
    }

    /// The colour or transparency, `None` for an asset.
    pub fn plain(&self) -> Option<KeyBackground> {
        match self {
            Self::Plain(plain) => Some(*plain),
            Self::Asset(_) => None,
        }
    }

    pub fn asset(&self) -> Option<&AssetRef> {
        match self {
            Self::Plain(_) => None,
            Self::Asset(reference) => Some(reference),
        }
    }
}

impl From<KeyBackground> for Background {
    fn from(plain: KeyBackground) -> Self {
        Self::Plain(plain)
    }
}

impl From<Color> for Background {
    fn from(color: Color) -> Self {
        Self::Plain(KeyBackground::Color(color))
    }
}

impl fmt::Display for Background {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain(plain) => plain.fmt(f),
            Self::Asset(reference) => write!(f, "{BACKGROUND_ASSET_PREFIX}{reference}"),
        }
    }
}

impl Serialize for Background {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Background {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// A linear gradient: two to [`MAX_GRADIENT_COLORS`] colours spaced
/// evenly along `angle` (degrees as in CSS: 0 runs to the top, 90 to the
/// right, 180 to the bottom). Rasterised by the renderer; never animated.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gradient {
    pub colors: Vec<Color>,
    #[serde(default = "Gradient::default_angle")]
    pub angle: u16,
}

impl Gradient {
    const fn default_angle() -> u16 {
        180
    }

    /// Refuses fewer than two or more than [`MAX_GRADIENT_COLORS`] colours
    /// and angles of a full turn or more.
    pub fn check(&self) -> Result<(), String> {
        if !(2..=MAX_GRADIENT_COLORS).contains(&self.colors.len()) {
            return Err(format!("a gradient has 2 to {MAX_GRADIENT_COLORS} colours"));
        }
        if self.angle >= 360 {
            return Err("a gradient's angle is 0 to 359 degrees".to_owned());
        }
        Ok(())
    }
}

/// A gradient DeckPilot ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinGradient {
    /// The name in [`AssetId`] terms (`gradient-<…>`).
    pub name: &'static str,
    pub colors: &'static [Color],
    pub angle: u16,
}

impl BuiltinGradient {
    pub fn id(&self) -> AssetId {
        AssetId(format!("{BUILTIN_NAMESPACE}.{}", self.name))
    }

    pub fn gradient(&self) -> Gradient {
        Gradient {
            colors: self.colors.to_vec(),
            angle: self.angle,
        }
    }
}

const fn rgb(hex: u32) -> Color {
    let [_, r, g, b] = hex.to_be_bytes();
    Color([r, g, b])
}

/// DeckPilot's gradients, in the order the Background Library shows them.
/// The app names them by `editor:assets.gradients.<name>`.
pub const BUILTIN_GRADIENTS: &[BuiltinGradient] = &[
    BuiltinGradient {
        name: "gradient-midnight",
        colors: &[rgb(0x1e_293b), rgb(0x02_0617)],
        angle: 180,
    },
    BuiltinGradient {
        name: "gradient-graphite",
        colors: &[rgb(0x52_525b), rgb(0x18_181b)],
        angle: 180,
    },
    BuiltinGradient {
        name: "gradient-ocean",
        colors: &[rgb(0x0e_a5e9), rgb(0x1e_3a8a)],
        angle: 135,
    },
    BuiltinGradient {
        name: "gradient-forest",
        colors: &[rgb(0x22_c55e), rgb(0x14_532d)],
        angle: 135,
    },
    BuiltinGradient {
        name: "gradient-sunset",
        colors: &[rgb(0xf9_7316), rgb(0xdb_2777), rgb(0x7c_3aed)],
        angle: 135,
    },
    BuiltinGradient {
        name: "gradient-ember",
        colors: &[rgb(0xfa_cc15), rgb(0xdc_2626)],
        angle: 180,
    },
    BuiltinGradient {
        name: "gradient-berry",
        colors: &[rgb(0xec_4899), rgb(0x58_1c87)],
        angle: 135,
    },
    BuiltinGradient {
        name: "gradient-aurora",
        colors: &[rgb(0x2d_d4bf), rgb(0x63_66f1), rgb(0xa8_55f7)],
        angle: 90,
    },
    BuiltinGradient {
        name: "gradient-steel",
        colors: &[rgb(0x94_a3b8), rgb(0x33_4155)],
        angle: 180,
    },
    BuiltinGradient {
        name: "gradient-lime",
        colors: &[rgb(0xa3_e635), rgb(0x0d_9488)],
        angle: 135,
    },
];

/// The built-in gradient `id` names, if it is one.
pub fn builtin_gradient(id: &AssetId) -> Option<&'static BuiltinGradient> {
    if !id.is_builtin() {
        return None;
    }
    BUILTIN_GRADIENTS.iter().find(|g| g.name == id.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_ids_are_namespaced() {
        let builtin = AssetId::parse("deckpilot.house").unwrap();
        assert!(builtin.is_builtin());
        assert_eq!(
            (builtin.namespace(), builtin.name()),
            ("deckpilot", "house")
        );
        assert_eq!(builtin.plugin(), None);
        let plugin = AssetId::parse("acme.weather.sunny-sky").unwrap();
        assert!(!plugin.is_builtin());
        assert_eq!(plugin.plugin(), Some("acme.weather"));
        assert_eq!(plugin.name(), "sunny-sky");
        // A first-party plugin's assets never collide with DeckPilot's own.
        let first_party = AssetId::of_plugin("deckpilot.notifications", "bell").unwrap();
        assert!(!first_party.is_builtin());
        assert_ne!(first_party, AssetId::builtin("bell").unwrap());
        assert_eq!(
            AssetId::builtin("trash-2").unwrap().as_str(),
            "deckpilot.trash-2"
        );
        for bad in [
            "",
            "house",
            ".house",
            "deckpilot.",
            "Deckpilot.house",
            "a.b.c.d",
            "deckpilot.house_1",
            "deckpilot.--",
            "deckpilot../x",
            "deckpilot.a b",
        ] {
            assert!(AssetId::parse(bad).is_err(), "{bad}");
        }
        assert!(AssetId::of_plugin("acme", "x").is_err());
        assert!(AssetId::parse(&format!("deckpilot.{}", "a".repeat(49))).is_err());
    }

    #[test]
    fn references_keep_profile_icons_and_library_assets_apart() {
        let id = IconId::new();
        let profile: AssetRef = serde_json::from_value(serde_json::json!(id)).unwrap();
        assert_eq!(profile, AssetRef::Profile(id));
        assert_eq!(profile, id);
        assert_eq!(
            serde_json::to_value(&profile).unwrap(),
            serde_json::json!(id)
        );
        let library: AssetRef =
            serde_json::from_value(serde_json::json!("deckpilot.star")).unwrap();
        assert_eq!(library.asset().map(AssetId::name), Some("star"));
        assert_eq!(library.profile_icon(), None);
        assert!(serde_json::from_value::<AssetRef>(serde_json::json!("../etc/passwd")).is_err());
    }

    #[test]
    fn backgrounds_read_old_values_and_assets() {
        for (text, expected) in [
            ("transparent", Background::TRANSPARENT),
            ("#ffcc00", Color([255, 204, 0]).into()),
            (
                "asset:deckpilot.gradient-ocean",
                Background::Asset(AssetRef::Library(
                    AssetId::parse("deckpilot.gradient-ocean").unwrap(),
                )),
            ),
        ] {
            let read: Background = serde_json::from_value(serde_json::json!(text)).unwrap();
            assert_eq!(read, expected, "{text}");
            assert_eq!(
                serde_json::to_value(&read).unwrap(),
                serde_json::json!(text)
            );
        }
        let icon = IconId::new();
        let image = Background::parse(&format!("asset:{icon}")).unwrap();
        assert_eq!(image.asset().and_then(AssetRef::profile_icon), Some(icon));
        assert_eq!(image.plain(), None);
        for bad in ["asset:", "asset:nothing", "blue", "#12345"] {
            assert!(Background::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn builtin_gradients_are_valid_and_unique() {
        let mut names = std::collections::BTreeSet::new();
        for gradient in BUILTIN_GRADIENTS {
            assert!(gradient.gradient().check().is_ok(), "{}", gradient.name);
            assert!(names.insert(gradient.name), "{}", gradient.name);
            assert_eq!(builtin_gradient(&gradient.id()), Some(gradient));
        }
        assert!(BUILTIN_GRADIENTS.len() >= 8);
        let none = AssetId::parse("acme.tools.gradient-ocean").unwrap();
        assert_eq!(builtin_gradient(&none), None);
        assert!(
            Gradient {
                colors: vec![Color::BLACK],
                angle: 0
            }
            .check()
            .is_err()
        );
        assert!(
            Gradient {
                colors: vec![Color::BLACK; 2],
                angle: 360
            }
            .check()
            .is_err()
        );
    }
}
