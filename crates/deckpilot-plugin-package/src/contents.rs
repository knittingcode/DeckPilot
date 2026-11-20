//! Rules on the files of a package, applied by the builder before writing
//! and by the reader after reading (architecture sections 7 and 8).

use std::collections::BTreeMap;

use deckpilot_plugin_manifest::{
    LEGACY_JS_ARTIFACT, Manifest, PackageDigest, PackageIntegrity, RuntimeContract, Sha256Digest,
    WASM_ABI_VERSION, WASM_ARTIFACT,
};

use crate::error::PackageError;
use crate::limits::Limits;
use crate::paths::check_entry_set;
use crate::sha256;

/// The files of a package by path, sorted.
pub type Files = BTreeMap<String, Vec<u8>>;

/// The executable artifact every installable package runs: a WebAssembly
/// module implementing plugin ABI 1 (`docs/marketplace/artifact-contract.md`).
pub const ARTIFACT: &str = WASM_ARTIFACT;
/// The manifest every package carries.
pub const MANIFEST: &str = "manifest.json";
/// The license every package carries.
pub const LICENSE: &str = "LICENSE";

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

/// Shape checks that need no manifest: count, paths, sizes, `manifest.json`
/// and `LICENSE`. The artifact is checked by [`check_artifact`] once the
/// manifest says which runtime the package declares.
pub(crate) fn check_shape(files: &Files, limits: &Limits) -> Result<(), PackageError> {
    if files.len() > limits.max_entries {
        return Err(PackageError::TooManyEntries {
            limit: limits.max_entries,
        });
    }
    check_entry_set(files.keys().map(String::as_str))?;
    let mut total: u64 = 0;
    for (path, bytes) in files {
        let size = len(bytes);
        if size > limits.max_file_bytes {
            return Err(PackageError::FileTooLarge {
                path: path.clone(),
                size,
                limit: limits.max_file_bytes,
            });
        }
        total = total.saturating_add(size);
        if total > limits.max_unpacked_bytes {
            return Err(PackageError::UnpackedTooLarge {
                limit: limits.max_unpacked_bytes,
            });
        }
    }
    for required in [MANIFEST, LICENSE] {
        if !files.contains_key(required) {
            return Err(PackageError::Missing(required));
        }
    }
    Ok(())
}

/// The artifact contract: the manifest must declare the `wasm` runtime with
/// plugin ABI 1, and `plugin.wasm` must be present, within the size limit
/// and a valid ABI 1 module ([`crate::wasm::validate_module`]). A package of
/// the #191 JavaScript contract (no `runtime`, `plugin.js`) is refused with
/// an error that names the contract; it is never reinterpreted.
pub(crate) fn check_artifact(
    files: &Files,
    manifest: &Manifest,
    limits: &Limits,
) -> Result<(), PackageError> {
    match manifest.runtime_contract() {
        RuntimeContract::LegacyJs => {
            return Err(PackageError::UnsupportedRuntime {
                found: if files.contains_key(LEGACY_JS_ARTIFACT) {
                    "a JavaScript bundle (plugin.js) without a `runtime` declaration".into()
                } else {
                    "no `runtime` declaration".into()
                },
            });
        }
        RuntimeContract::Wasm { abi } if abi != WASM_ABI_VERSION => {
            return Err(PackageError::UnsupportedRuntime {
                found: format!("a wasm module for plugin ABI {abi}"),
            });
        }
        RuntimeContract::Wasm { .. } => {}
    }
    let module = files.get(ARTIFACT).ok_or(PackageError::Missing(ARTIFACT))?;
    let size = len(module);
    if size > limits.max_artifact_bytes {
        return Err(PackageError::ArtifactTooLarge {
            size,
            limit: limits.max_artifact_bytes,
        });
    }
    crate::wasm::validate_module(module)?;
    Ok(())
}

/// Parses and validates `manifest.json` (schema rules and the cross-field
/// rules of section 7). `first_party` applies
/// [`Manifest::validate_first_party`] instead of [`Manifest::validate`]:
/// the publisher must then be `deckpilot`, and every other reserved
/// publisher stays refused.
pub(crate) fn parse_manifest(files: &Files, first_party: bool) -> Result<Manifest, PackageError> {
    let manifest = Manifest::from_slice(&files[MANIFEST])?;
    if first_party {
        manifest.validate_first_party()
    } else {
        manifest.validate()
    }
    .map_err(deckpilot_plugin_manifest::ManifestError::from)?;
    Ok(manifest)
}

/// The integrity record of `files` as `package`, checked against the
/// manifest: only files the manifest names (plus the required ones and
/// README/CHANGELOG), every named file present, size range; and every
/// navigation icon one the app may show ([`check_navigation_icons`]);
/// every locale file a valid translation file ([`check_locales`]).
pub(crate) fn integrity(
    files: &Files,
    manifest: &Manifest,
    package: PackageDigest,
) -> Result<PackageIntegrity, PackageError> {
    let digests: BTreeMap<String, Sha256Digest> = files
        .iter()
        .map(|(path, bytes)| (path.clone(), sha256(bytes)))
        .collect();
    let integrity = PackageIntegrity {
        manifest_sha256: digests[MANIFEST],
        package,
        files: digests,
    };
    integrity
        .validate(manifest)
        .map_err(PackageError::Contents)?;
    check_navigation_icons(files, manifest)?;
    check_visual_assets(files, manifest)?;
    check_locales(files, manifest)?;
    Ok(integrity)
}

/// Every locale file of `locales` (plugin API 1.6) is a translation file
/// [`deckpilot_plugin_manifest::locale::Messages::parse`] accepts: JSON
/// objects of plain-text strings within the size, count, depth and length
/// limits, well-formed placeholders and no markup. A package with another
/// file there is refused when it is built and when it is read for
/// installation; the files are hashed like every other (the manifest names
/// them), so a changed translation is a changed package.
pub(crate) fn check_locales(files: &Files, manifest: &Manifest) -> Result<(), PackageError> {
    manifest
        .catalog(|path| files.get(path).cloned())
        .map(|_| ())
        .map_err(|(path, error)| PackageError::Locale { path, error })
}

/// Every `contributes.visualAssets` file: its format matches its
/// extension and its use (icons PNG, image backgrounds PNG or JPEG), within
/// its size and pixel limits ([`deckpilot_plugin_manifest::check_visual_asset`]).
/// A package with such a file is refused when it is built and when it is
/// read for installation.
pub(crate) fn check_visual_assets(files: &Files, manifest: &Manifest) -> Result<(), PackageError> {
    for (path, used) in manifest.visual_asset_files() {
        let bytes = files.get(path).ok_or_else(|| PackageError::VisualAsset {
            path: path.to_owned(),
            error: deckpilot_plugin_manifest::VisualAssetError::Malformed("package"),
        })?;
        deckpilot_plugin_manifest::check_visual_asset(path, used, bytes).map_err(|error| {
            PackageError::VisualAsset {
                path: path.to_owned(),
                error,
            }
        })?;
    }
    Ok(())
}

/// Every `contributes.navigation` icon file: its format matches its
/// extension, PNG and WebP within 256 KiB and 512×512 pixels, SVG within
/// 64 KiB and free of scripts, event handlers, external or `data:` links
/// and the rest [`deckpilot_plugin_manifest::sanitize_svg`] refuses. A
/// package with such a file is refused when it is built and when it is
/// read for installation.
pub(crate) fn check_navigation_icons(
    files: &Files,
    manifest: &Manifest,
) -> Result<(), PackageError> {
    for path in manifest.navigation_icon_files() {
        let bytes = files
            .get(path)
            .ok_or_else(|| PackageError::NavigationIcon {
                path: path.to_owned(),
                error: deckpilot_plugin_manifest::IconError::Malformed("package"),
            })?;
        deckpilot_plugin_manifest::check_icon(path, bytes).map_err(|error| {
            PackageError::NavigationIcon {
                path: path.to_owned(),
                error,
            }
        })?;
    }
    Ok(())
}
