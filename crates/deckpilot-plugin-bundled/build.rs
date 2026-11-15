//! Compiles the pins of the bundled plugins into the crate
//! (`docs/marketplace/module-migration.md` §4.4, option B).
//!
//! `DECKPILOT_BUNDLED_PINS` names the `bundled/pins.json` that the release
//! build wrote next to the plugins it bundles
//! (`plugins/first-party/build.sh`). The file is copied into `OUT_DIR` and
//! embedded with `include_bytes!`, so the agent trusts exactly the digests
//! it was built with. Unset (every build today): no plugin is pinned, and
//! nothing from an application bundle is ever seeded.

use std::path::PathBuf;

/// The pins of a build that bundles no plugin.
const EMPTY: &str = "{\"pinsVersion\":1,\"plugins\":[]}\n";
const ENV: &str = "DECKPILOT_BUNDLED_PINS";

fn main() {
    println!("cargo:rerun-if-env-changed={ENV}");
    println!("cargo:rerun-if-changed=build.rs");
    let bytes = match std::env::var_os(ENV).filter(|v| !v.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("{ENV}={}: cannot be read: {e}", path.display()));
            // Malformed pins fail the build rather than an installed agent.
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .unwrap_or_else(|e| panic!("{ENV}={}: not JSON: {e}", path.display()));
            assert!(
                value.get("pinsVersion") == Some(&serde_json::Value::from(1))
                    && value
                        .get("plugins")
                        .is_some_and(serde_json::Value::is_array),
                "{ENV}={}: not a pins file (pinsVersion 1 with a plugins list)",
                path.display()
            );
            bytes
        }
        None => EMPTY.as_bytes().to_vec(),
    };
    let out =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("pins.json");
    if std::fs::read(&out).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&out, &bytes).expect("OUT_DIR is writable");
    }
}
