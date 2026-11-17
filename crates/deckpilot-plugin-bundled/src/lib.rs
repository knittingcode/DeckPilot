//! **Experimental.** First-party plugins shipped inside the application
//! bundle (`docs/marketplace/module-migration.md` §4).
//!
//! A bundled plugin is an app component, not a package: unpacked files
//! `bundled/<id>/manifest.json`, `plugin.wasm`, `LICENSE` and the icons the
//! manifest names (`assets/<name>.png`) in the installer's resources, next
//! to `bundled/pins.json`, never a `.dpplugin`.
//! The release build writes the pins (`plugins/first-party/build.sh`): id,
//! version, class, and the size and sha256 of every file.
//!
//! # Trust (§4.4: option B, plus A on macOS)
//!
//! * **Pins compiled into the agent.** The build compiles the pins file
//!   into this crate (`build.rs`, `DECKPILOT_BUNDLED_PINS`, then
//!   `include_bytes!`: [`EMBEDDED_PINS`]). The files are checked against
//!   these digests, never against the `pins.json` next to them, before
//!   seeding ([`verify_bundled`]) and at every load ([`verify_installed`],
//!   [`check_module`]). Trust is that of the agent binary: whoever can
//!   replace the pins can replace the agent.
//! * **The app bundle's code seal on macOS** ([`seal`]), in addition: a
//!   signed release build refuses to seed when the seal check fails or
//!   errors; only a debug build tolerates an unsigned bundle.
//!
//! No key signs bundled plugins: a plugin signature made in this
//! repository is what the release separation forbids.
//!
//! Without `DECKPILOT_BUNDLED_PINS` (every build today) nothing is pinned
//! and nothing is seeded.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use deckpilot_plugin_manifest::{PackageDigest, PluginId};
use deckpilot_plugin_package::{
    ARTIFACT, ExpectedFiles, Files, LICENSE, Limits, MANIFEST, PackageError, VerifiedPackage,
    sha256, verify_bundled_files,
};
use semver::Version;
use serde::{Deserialize, Serialize};

pub mod seal;

/// The resources directory of the bundled plugins.
pub const BUNDLED_DIR: &str = "bundled";
/// The pins file in [`BUNDLED_DIR`]. Deliberately not `index.json` (and the
/// directory is not `index/`), so no registry pattern of the release
/// separation can match it.
pub const PINS_FILE: &str = "pins.json";
/// The only pins format.
pub const PINS_VERSION: u32 = 1;
/// The files every bundled plugin has. Besides them only its icons
/// ([`is_allowed_file`]); every file is pinned.
pub const FILES: [&str; 3] = [MANIFEST, ARTIFACT, LICENSE];
/// The directory of a bundled plugin's icons (the manifest's `icons`,
/// which the manifest contract requires).
pub const ICONS_DIR: &str = "assets";

/// Whether a bundled plugin may contain `path`: one of [`FILES`], or a PNG
/// icon directly in [`ICONS_DIR`] (`assets/icon-128.png`). Nothing else:
/// no archive, no signature, no script.
pub fn is_allowed_file(path: &str) -> bool {
    if FILES.contains(&path) {
        return true;
    }
    // Lowercase only: `.PNG` is not allowed.
    path.strip_prefix("assets/")
        .and_then(|name| name.strip_suffix(".png"))
        .is_some_and(|stem| {
            !stem.is_empty()
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}
/// The publisher of every bundled plugin.
pub const FIRST_PARTY_PUBLISHER: &str = deckpilot_plugin_manifest::FIRST_PARTY_PUBLISHER;

/// The ids of the first-party plugins the application bundles
/// (`plugins/first-party/bundled.json`, checked by this crate's tests).
///
/// They come only from the bundle, verified against [`EMBEDDED_PINS`]: a
/// registry never serves them, even one whose key set authorizes the
/// `deckpilot` namespace, so the pinned-digest trust stays their only
/// source. Fixed in the code, not read from the pins, because every build
/// without `DECKPILOT_BUNDLED_PINS` pins nothing and would refuse nothing;
/// [`is_bundled_id`] also covers whatever the pins of a build name.
pub const BUNDLED_PLUGIN_IDS: &[&str] = &[
    "deckpilot.github",
    "deckpilot.gitlab",
    "deckpilot.jira",
    "deckpilot.notifications",
];

/// Whether `id` is a bundled plugin: in [`BUNDLED_PLUGIN_IDS`] or pinned
/// by `pins` (whatever version).
pub fn is_bundled_id(id: &PluginId, pins: &Pins) -> bool {
    BUNDLED_PLUGIN_IDS.contains(&id.as_str()) || pins.get(id).is_some()
}

/// The pins compiled into this build (`DECKPILOT_BUNDLED_PINS` at build
/// time; empty without it). `deckpilot-agent --print-bundled-pins` prints
/// these bytes, so the release check can compare them with the bundle.
pub const EMBEDDED_PINS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pins.json"));

/// How a bundled plugin is seeded (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BundleClass {
    /// Seeded on a fresh install and on upgrades, unless tombstoned.
    Default,
    /// Seeded only when the user's data uses the legacy module (the
    /// migration engine, a later step); never by plain seeding.
    Migration,
}

impl BundleClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Migration => "migration",
        }
    }
}

impl fmt::Display for BundleClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One bundled plugin as pinned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub id: PluginId,
    pub version: Version,
    pub class: BundleClass,
    /// Size and sha256 of each of [`FILES`].
    pub files: BTreeMap<String, PackageDigest>,
}

impl Pin {
    /// What [`verify_bundled_files`] checks the files against.
    pub fn expected(&self) -> ExpectedFiles {
        ExpectedFiles {
            id: self.id.clone(),
            version: self.version.clone(),
            files: self.files.clone(),
        }
    }

    /// The pinned digest of `file`.
    pub fn file(&self, file: &str) -> Option<&PackageDigest> {
        self.files.get(file)
    }
}

/// `bundled/pins.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Pins {
    pub pins_version: u32,
    pub plugins: Vec<Pin>,
}

/// Why pins, or the files of a bundled plugin, were refused.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("the bundled plugin pins are invalid: {0}")]
    Pins(String),
    #[error("{plugin}: {path} {problem}")]
    File {
        plugin: PluginId,
        path: String,
        problem: String,
    },
    #[error("{plugin}: {error}")]
    Package {
        plugin: PluginId,
        #[source]
        error: PackageError,
    },
    #[error("{path}: {message}")]
    Io { path: String, message: String },
}

impl BundleError {
    fn file(pin: &Pin, path: impl Into<String>, problem: impl Into<String>) -> Self {
        Self::File {
            plugin: pin.id.clone(),
            path: path.into(),
            problem: problem.into(),
        }
    }

    fn io(path: &Path, error: &std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        }
    }
}

impl Pins {
    /// No plugin pinned.
    pub fn empty() -> Self {
        Self {
            pins_version: PINS_VERSION,
            plugins: Vec::new(),
        }
    }

    /// Reads and checks a pins file: format 1, unique first-party ids,
    /// exactly [`FILES`] per plugin.
    pub fn parse(bytes: &[u8]) -> Result<Self, BundleError> {
        let pins: Self =
            serde_json::from_slice(bytes).map_err(|e| BundleError::Pins(e.to_string()))?;
        if pins.pins_version != PINS_VERSION {
            return Err(BundleError::Pins(format!(
                "pinsVersion {} (this agent reads {PINS_VERSION})",
                pins.pins_version
            )));
        }
        let mut seen = BTreeSet::new();
        for pin in &pins.plugins {
            if pin.id.publisher() != FIRST_PARTY_PUBLISHER {
                return Err(BundleError::Pins(format!(
                    "{} is not a first-party plugin (only {FIRST_PARTY_PUBLISHER}.* is bundled)",
                    pin.id
                )));
            }
            if !seen.insert(pin.id.clone()) {
                return Err(BundleError::Pins(format!("{} is pinned twice", pin.id)));
            }
            if let Some(missing) = FILES.iter().find(|f| !pin.files.contains_key(**f)) {
                return Err(BundleError::Pins(format!(
                    "{} does not pin {missing}",
                    pin.id
                )));
            }
            if let Some(other) = pin.files.keys().find(|f| !is_allowed_file(f)) {
                return Err(BundleError::Pins(format!(
                    "{} pins {other}: a bundled plugin is {FILES:?} and its icons in {ICONS_DIR}/ only",
                    pin.id
                )));
            }
        }
        Ok(pins)
    }

    /// The pins compiled into this build ([`EMBEDDED_PINS`]).
    pub fn embedded() -> Result<Self, BundleError> {
        Self::parse(EMBEDDED_PINS)
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// The pin of `id`, whatever its version.
    pub fn get(&self, id: &PluginId) -> Option<&Pin> {
        self.plugins.iter().find(|pin| &pin.id == id)
    }

    /// The pin of `id` at exactly `version`.
    pub fn pinned(&self, id: &PluginId, version: &Version) -> Option<&Pin> {
        self.get(id).filter(|pin| &pin.version == version)
    }

    /// Pins of the files in `bundled/<id>/` (what the release build
    /// writes), sorted by id. For tools and tests; the agent only ever
    /// reads pins compiled into it.
    pub fn compute(
        bundled_dir: &Path,
        plugins: &[(PluginId, Version, BundleClass)],
    ) -> Result<Self, BundleError> {
        let mut pins = Self::empty();
        for (id, version, class) in plugins {
            let dir = bundled_dir.join(id.as_str());
            let mut names: Vec<String> = FILES.iter().map(|f| (*f).to_owned()).collect();
            let icons = dir.join(ICONS_DIR);
            if icons.is_dir() {
                for entry in std::fs::read_dir(&icons).map_err(|e| BundleError::io(&icons, &e))? {
                    let entry = entry.map_err(|e| BundleError::io(&icons, &e))?;
                    names.push(format!(
                        "{ICONS_DIR}/{}",
                        entry.file_name().to_string_lossy()
                    ));
                }
            }
            let mut files = BTreeMap::new();
            for name in names {
                let path = dir.join(&name);
                let bytes = std::fs::read(&path).map_err(|e| BundleError::io(&path, &e))?;
                files.insert(
                    name,
                    PackageDigest {
                        sha256: sha256(&bytes),
                        size: len(&bytes),
                    },
                );
            }
            pins.plugins.push(Pin {
                id: id.clone(),
                version: version.clone(),
                class: *class,
                files,
            });
        }
        pins.plugins.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(pins)
    }

    /// Two-space indented JSON with a final newline, as the release build
    /// writes it.
    ///
    /// # Panics
    ///
    /// Never: pins always serialize.
    pub fn to_json_pretty(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("pins serialize");
        text.push('\n');
        text
    }
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

/// Reads the files of `pin` from `dir` and checks them against the pin:
/// `dir` (and `assets/` in it) are directories, not links; `dir` holds
/// exactly the pinned files, each a regular file (no link) whose size is
/// checked before it is read and whose sha256 is checked after.
pub fn read_pinned(dir: &Path, pin: &Pin) -> Result<Files, BundleError> {
    let mut files = Files::new();
    read_level(dir, "", pin, &mut files)?;
    if let Some(missing) = pin.files.keys().find(|name| !files.contains_key(*name)) {
        return Err(BundleError::file(pin, missing.clone(), "is missing"));
    }
    Ok(files)
}

fn read_level(dir: &Path, prefix: &str, pin: &Pin, files: &mut Files) -> Result<(), BundleError> {
    let kind = std::fs::symlink_metadata(dir)
        .map_err(|e| BundleError::io(dir, &e))?
        .file_type();
    if !kind.is_dir() {
        let at = if prefix.is_empty() { "." } else { prefix };
        return Err(BundleError::file(pin, at, "is not a directory"));
    }
    for entry in std::fs::read_dir(dir).map_err(|e| BundleError::io(dir, &e))? {
        let entry = entry.map_err(|e| BundleError::io(dir, &e))?;
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(|n| format!("{prefix}{n}")) else {
            return Err(BundleError::file(
                pin,
                path.display().to_string(),
                "has a name that is not UTF-8",
            ));
        };
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| BundleError::io(&path, &e))?;
        if prefix.is_empty() && name == ICONS_DIR && metadata.file_type().is_dir() {
            read_level(&path, &format!("{ICONS_DIR}/"), pin, files)?;
            continue;
        }
        let Some(digest) = pin.file(&name) else {
            return Err(BundleError::file(pin, name, "is not pinned"));
        };
        if !metadata.file_type().is_file() {
            return Err(BundleError::file(pin, name, "is not a regular file"));
        }
        if metadata.len() != digest.size {
            return Err(BundleError::file(
                pin,
                name,
                format!("has {} bytes, {} are pinned", metadata.len(), digest.size),
            ));
        }
        let bytes = std::fs::read(&path).map_err(|e| BundleError::io(&path, &e))?;
        check_bytes(pin, &name, &bytes)?;
        files.insert(name, bytes);
    }
    Ok(())
}

/// `bytes` are the pinned contents of `file` of `pin`.
pub fn check_bytes(pin: &Pin, file: &str, bytes: &[u8]) -> Result<(), BundleError> {
    let digest = pin
        .file(file)
        .ok_or_else(|| BundleError::file(pin, file, "is not pinned"))?;
    let size = len(bytes);
    if size != digest.size {
        return Err(BundleError::file(
            pin,
            file,
            format!("has {size} bytes, {} are pinned", digest.size),
        ));
    }
    let actual = sha256(bytes);
    if actual != digest.sha256 {
        return Err(BundleError::file(
            pin,
            file,
            format!("has sha256 {actual}, {} is pinned", digest.sha256),
        ));
    }
    Ok(())
}

/// Before seeding: the files in `dir` (`bundled/<id>`) against `pin`, then
/// the package rules with the first-party manifest rules. Only the result
/// can be installed.
pub fn verify_bundled(dir: &Path, pin: &Pin) -> Result<VerifiedPackage, BundleError> {
    let files = read_pinned(dir, pin)?;
    verify_bundled_files(files, &pin.expected(), &Limits::default()).map_err(|error| {
        BundleError::Package {
            plugin: pin.id.clone(),
            error,
        }
    })
}

/// At every load: the installed copy of a bundled plugin (its version
/// directory in the plugin store) still has exactly the pinned files.
pub fn verify_installed(dir: &Path, pin: &Pin) -> Result<(), BundleError> {
    read_pinned(dir, pin).map(drop)
}

/// The module bytes handed to the plugin host are the pinned
/// `plugin.wasm` (checked on the very bytes, after they were read).
pub fn check_module(pin: &Pin, module: &[u8]) -> Result<(), BundleError> {
    check_bytes(pin, ARTIFACT, module)
}

/// The product name of the installers (`tauri.conf.json`), which names the
/// resources directory on Linux.
const PRODUCT_NAME: &str = "DeckPilot";

/// Where an installed application keeps its resources, for an agent at
/// `exe` (an external binary next to the application executable), most
/// specific first:
///
/// * macOS: `DeckPilot.app/Contents/MacOS/deckpilot-agent` →
///   `Contents/Resources`;
/// * Windows (MSI, NSIS): the installation directory itself;
/// * Linux (`.deb`, `.rpm`, AppImage): `/usr/bin/deckpilot-agent` →
///   `/usr/lib/DeckPilot` (inside the AppImage's mount for an AppImage).
pub fn resource_dirs(exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    if cfg!(target_os = "macos") {
        let in_app = dir.file_name().is_some_and(|n| n == "MacOS")
            && dir
                .parent()
                .is_some_and(|c| c.file_name().is_some_and(|n| n == "Contents"));
        if in_app {
            return dir
                .parent()
                .map(|contents| vec![contents.join("Resources")])
                .unwrap_or_default();
        }
        Vec::new()
    } else if cfg!(windows) {
        vec![dir.to_path_buf()]
    } else {
        dir.parent()
            .map(|prefix| vec![prefix.join("lib").join(PRODUCT_NAME)])
            .unwrap_or_default()
    }
}

/// The `bundled` directory of the installed application the agent at
/// `exe` belongs to, if there is one.
pub fn locate(exe: &Path) -> Option<PathBuf> {
    resource_dirs(exe)
        .into_iter()
        .map(|dir| dir.join(BUNDLED_DIR))
        .find(|dir| dir.is_dir())
}

/// What a `bundled` directory holds besides the pinned plugins: entries no
/// pin names (ignored, and reported). The release check refuses them.
pub fn unpinned_entries(bundled_dir: &Path, pins: &Pins) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(bundled_dir) else {
        return Vec::new();
    };
    let mut extra: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != PINS_FILE && !pins.plugins.iter().any(|p| p.id.as_str() == name))
        .collect();
    extra.sort();
    extra
}

#[cfg(test)]
mod tests;
