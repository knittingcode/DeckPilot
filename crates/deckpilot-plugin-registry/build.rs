//! Compiles the registry's trusted root role into the crate
//! (`crates/deckpilot-plugin-registry/PUBLISHING.md`, "Embedding the
//! roots in a release build").
//!
//! `DECKPILOT_REGISTRY_ROOT_PUBKEYS` holds the root role at **build time**:
//! the content of `keys/roots.json` of the registry repository (JSON,
//! `{"keys": [...], "threshold": n}`), or the absolute path of such a
//! file. It is checked here (`src/roots_check.rs`) and anything malformed
//! fails the build: a release never ships with partial or unreadable
//! roots. The checked text is written to `OUT_DIR/embedded-roots.json` and
//! reaches the library as `roots::EMBEDDED`.
//!
//! Unset or empty (every build today): nothing is embedded, the file is
//! empty and `roots::EMBEDDED` is `None`. Nothing reads the variable at
//! run time.

use std::env::{self, VarError};
use std::path::{Path, PathBuf};

#[path = "src/roots_check.rs"]
mod roots_check;

const ENV: &str = "DECKPILOT_REGISTRY_ROOT_PUBKEYS";

fn fail(message: &str) -> ! {
    // A panicking build script fails the build with this message.
    panic!("deckpilot-plugin-registry: {ENV} {message}");
}

/// The root role to embed, checked; empty when none is.
fn roots() -> String {
    let value = match env::var(ENV) {
        Ok(value) if value.is_empty() => return String::new(),
        Ok(value) => value,
        Err(VarError::NotPresent) => return String::new(),
        Err(VarError::NotUnicode(_)) => fail("is not valid UTF-8"),
    };
    let text = if value.trim_start().starts_with('{') {
        value
    } else {
        let path = Path::new(value.trim());
        if !path.is_absolute() {
            fail(&format!(
                "is neither a root role (JSON) nor an absolute path: {value:?}"
            ));
        }
        println!("cargo:rerun-if-changed={}", path.display());
        std::fs::read_to_string(path).unwrap_or_else(|e| {
            fail(&format!(
                "names {}, which cannot be read: {e}",
                path.display()
            ))
        })
    };
    let text = text.trim();
    if let Err(problem) = roots_check::check_roots(text) {
        fail(&format!(
            "is not a valid root role, nothing is embedded: it {problem}"
        ));
    }
    format!("{text}\n")
}

fn main() {
    println!("cargo:rerun-if-env-changed={ENV}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/roots_check.rs");
    let roots = roots();
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("embedded-roots.json");
    if std::fs::read_to_string(&out).ok().as_deref() != Some(roots.as_str()) {
        std::fs::write(&out, &roots).expect("OUT_DIR is writable");
    }
}
