//! `package`: builds a `.dpplugin` from a plugin directory and its compiled
//! module (`docs/marketplace/artifact-contract.md`).
//!
//! The package holds `manifest.json`, `LICENSE` and every file the manifest
//! names, read from the plugin directory, plus the module as
//! `plugin.wasm`. Nothing else in the directory (sources, build output) is
//! packaged. The result is deterministic and checked by the same rules the
//! client applies (`deckpilot-plugin-package`), the module included.

use std::path::Path;

use deckpilot_plugin_manifest::Manifest;
use deckpilot_plugin_package::{
    ARTIFACT, BuiltPackage, LICENSE, MANIFEST, PackageBuilder, check_entry_path,
};

use super::{PublishError, Result};

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

/// Builds the package of the plugin in `plugin_dir` with `module` as its
/// executable artifact. `first_party` checks the manifest with the
/// first-party rules (a `deckpilot.*` plugin, `--first-party`); without it
/// the reserved publisher `deckpilot` is refused like every other.
pub fn package(plugin_dir: &Path, module: &Path, first_party: bool) -> Result<BuiltPackage> {
    let manifest_bytes = read(&plugin_dir.join(MANIFEST))?;
    let manifest = Manifest::from_slice(&manifest_bytes)
        .map_err(|e| PublishError(format!("{MANIFEST}: {e}")))?;
    let mut builder = PackageBuilder::new();
    let add = |builder: &mut PackageBuilder, path: &str, bytes: Vec<u8>| {
        builder
            .add(path, bytes)
            .map(|_| ())
            .map_err(|e| PublishError(e.to_string()))
    };
    add(&mut builder, MANIFEST, manifest_bytes.clone())?;
    add(&mut builder, LICENSE, read(&plugin_dir.join(LICENSE))?)?;
    add(&mut builder, ARTIFACT, read(module)?)?;
    for file in manifest.referenced_files() {
        // The path rules first: a path such as `assets/../../x` must be
        // refused before anything outside the plugin directory is read.
        check_entry_path(file).map_err(|e| PublishError(e.to_string()))?;
        add(&mut builder, file, read(&plugin_dir.join(file))?)?;
    }
    if first_party {
        if manifest.id.publisher() != deckpilot_plugin_manifest::FIRST_PARTY_PUBLISHER {
            return Err(PublishError(format!(
                "--first-party is for deckpilot.* plugins, not {}",
                manifest.id
            )));
        }
        builder.first_party();
    }
    builder.build().map_err(|e| PublishError(e.to_string()))
}
