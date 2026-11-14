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
use deckpilot_plugin_package::{ARTIFACT, BuiltPackage, LICENSE, MANIFEST, PackageBuilder};

use super::{PublishError, Result};

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

/// Builds the package of the plugin in `plugin_dir` with `module` as its
/// executable artifact.
pub fn package(plugin_dir: &Path, module: &Path) -> Result<BuiltPackage> {
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
        add(&mut builder, file, read(&plugin_dir.join(file))?)?;
    }
    builder.build().map_err(|e| PublishError(e.to_string()))
}
