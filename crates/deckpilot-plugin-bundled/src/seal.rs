//! The application bundle's code seal (`docs/marketplace/module-migration.md`
//! §4.4, option A), checked on macOS in addition to the pins.
//!
//! The seal (`Contents/_CodeSignature/CodeResources`) covers every file of
//! the `.app`, `Contents/Resources/bundled` included. The check is
//! `SecStaticCodeCheckValidity` with strict validation and nested code, and
//! the requirement `anchor apple generic` (a Developer ID or Apple
//! signature).
//!
//! The policy ([`decide`]):
//!
//! | Seal | Release build | Debug build |
//! | --- | --- | --- |
//! | valid, Apple-anchored | seed | seed |
//! | unsigned, or ad-hoc (sealed but not Apple-anchored) | **refused** | seed (development) |
//! | broken, or the check errors | **refused** | **refused** |
//! | agent outside an `.app` | **refused** | seed (development) |
//!
//! Elsewhere there is no seal the agent can check (AppImage, `.deb`,
//! `.rpm`, per-user NSIS) and the pins alone decide.

use std::fmt;
use std::path::{Path, PathBuf};

/// What the seal check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seal {
    /// Not macOS: no seal to check.
    NotApplicable,
    /// The agent does not run from inside an `.app` (a development build).
    NotInAppBundle,
    /// Sealed and signed with an Apple-anchored identity.
    Valid { bundle: PathBuf },
    /// The bundle has no signature.
    Unsigned { bundle: PathBuf },
    /// Sealed (nothing changed) but ad-hoc or otherwise not Apple-anchored.
    AdHoc { bundle: PathBuf },
    /// The seal is broken, or the check itself failed.
    Invalid {
        bundle: PathBuf,
        code: i32,
        message: String,
    },
}

impl fmt::Display for Seal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotApplicable => f.write_str("no code seal on this platform"),
            Self::NotInAppBundle => f.write_str("the agent is not inside an application bundle"),
            Self::Valid { bundle } => write!(f, "{} has a valid code seal", bundle.display()),
            Self::Unsigned { bundle } => write!(f, "{} is not signed", bundle.display()),
            Self::AdHoc { bundle } => write!(
                f,
                "{} is sealed but not signed with a Developer ID",
                bundle.display()
            ),
            Self::Invalid {
                bundle,
                code,
                message,
            } => write!(
                f,
                "the code seal of {} does not verify ({code}: {message})",
                bundle.display()
            ),
        }
    }
}

/// Whether bundled plugins may be seeded given `seal`, in a debug build
/// (`debug`) or a release build. `Err` explains the refusal.
pub fn decide(seal: &Seal, debug: bool) -> Result<(), String> {
    match seal {
        Seal::NotApplicable | Seal::Valid { .. } => Ok(()),
        Seal::Unsigned { .. } | Seal::AdHoc { .. } | Seal::NotInAppBundle if debug => Ok(()),
        Seal::Unsigned { .. } | Seal::AdHoc { .. } | Seal::NotInAppBundle => Err(format!(
            "{seal}: a release build seeds bundled plugins only from a signed application bundle"
        )),
        Seal::Invalid { .. } => Err(format!("{seal}: bundled plugins are not seeded")),
    }
}

/// The `.app` an executable at `exe` belongs to
/// (`<name>.app/Contents/MacOS/<exe>`).
pub fn app_bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_app = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension().is_some_and(|e| e == "app");
    is_app.then(|| bundle.to_path_buf())
}

/// Checks the seal of the application bundle the agent at `exe` runs from.
pub fn check(exe: &Path) -> Seal {
    if !cfg!(target_os = "macos") {
        return Seal::NotApplicable;
    }
    match app_bundle_of(exe) {
        Some(bundle) => check_bundle(&bundle),
        None => Seal::NotInAppBundle,
    }
}

/// `errSecCSUnsigned`.
pub const UNSIGNED: i32 = -67062;
/// `errSecCSReqFailed`: the code is valid but does not satisfy the
/// requirement.
pub const REQUIREMENT_FAILED: i32 = -67050;

/// Checks the seal of `bundle` (macOS only; elsewhere
/// [`Seal::NotApplicable`]).
#[cfg(target_os = "macos")]
pub fn check_bundle(bundle: &Path) -> Seal {
    use core_foundation::url::CFURL;
    use security_framework::os::macos::code_signing::{Flags, SecRequirement, SecStaticCode};

    let invalid = |code: i32, message: String| Seal::Invalid {
        bundle: bundle.to_path_buf(),
        code,
        message,
    };
    let Some(url) = CFURL::from_path(bundle, true) else {
        return invalid(0, "the path cannot be turned into a URL".into());
    };
    let code = match SecStaticCode::from_path(&url, Flags::NONE) {
        Ok(code) => code,
        Err(e) => return invalid(e.code(), e.message().unwrap_or_default()),
    };
    let flags = Flags::STRICT_VALIDATE | Flags::CHECK_NESTED_CODE;
    let requirement = |text: &str| text.parse::<SecRequirement>();
    let anchored = match requirement("anchor apple generic") {
        Ok(r) => r,
        Err(e) => return invalid(e.code(), e.message().unwrap_or_default()),
    };
    match code.check_validity(flags, &anchored) {
        Ok(()) => Seal::Valid {
            bundle: bundle.to_path_buf(),
        },
        Err(e) if e.code() == UNSIGNED => Seal::Unsigned {
            bundle: bundle.to_path_buf(),
        },
        Err(e) if e.code() == REQUIREMENT_FAILED => {
            // Valid but not Apple-anchored: an ad-hoc (development)
            // signature, if the seal itself holds.
            let any = match requirement("always") {
                Ok(r) => r,
                Err(e) => return invalid(e.code(), e.message().unwrap_or_default()),
            };
            match code.check_validity(flags, &any) {
                Ok(()) => Seal::AdHoc {
                    bundle: bundle.to_path_buf(),
                },
                Err(e) => invalid(e.code(), e.message().unwrap_or_default()),
            }
        }
        Err(e) => invalid(e.code(), e.message().unwrap_or_default()),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn check_bundle(_bundle: &Path) -> Seal {
    Seal::NotApplicable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> PathBuf {
        PathBuf::from("/Applications/DeckPilot.app")
    }

    #[test]
    fn release_builds_fail_closed() {
        let invalid = Seal::Invalid {
            bundle: bundle(),
            code: -67061,
            message: "invalid signature".into(),
        };
        for seal in [
            Seal::Unsigned { bundle: bundle() },
            Seal::AdHoc { bundle: bundle() },
            Seal::NotInAppBundle,
            invalid.clone(),
        ] {
            assert!(decide(&seal, false).is_err(), "{seal}");
        }
        // A broken seal is refused even in a debug build.
        assert!(decide(&invalid, true).is_err());
        for seal in [Seal::NotApplicable, Seal::Valid { bundle: bundle() }] {
            decide(&seal, false).unwrap();
            decide(&seal, true).unwrap();
        }
        // Only a debug build tolerates an unsigned development bundle.
        for seal in [
            Seal::Unsigned { bundle: bundle() },
            Seal::AdHoc { bundle: bundle() },
            Seal::NotInAppBundle,
        ] {
            decide(&seal, true).unwrap();
        }
    }

    #[test]
    fn the_app_bundle_is_found_from_the_agent() {
        assert_eq!(
            app_bundle_of(Path::new(
                "/Applications/DeckPilot.app/Contents/MacOS/deckpilot-agent"
            )),
            Some(bundle())
        );
        for path in [
            "/usr/bin/deckpilot-agent",
            "/Applications/DeckPilot/Contents/MacOS/deckpilot-agent",
            "/Applications/DeckPilot.app/Contents/Resources/deckpilot-agent",
            "deckpilot-agent",
        ] {
            assert_eq!(app_bundle_of(Path::new(path)), None, "{path}");
        }
    }

    #[cfg(target_os = "macos")]
    mod macos {
        use super::super::*;

        const SYSTEM_APP: &str = "/System/Applications/Calculator.app";

        #[test]
        fn a_system_app_has_a_valid_seal() {
            let seal = check(&Path::new(SYSTEM_APP).join("Contents/MacOS/Calculator"));
            assert!(matches!(seal, Seal::Valid { .. }), "{seal}");
        }

        /// `Test.app` with a Mach-O executable (a copy of `/usr/bin/true`)
        /// and a bundled plugin file in its resources; unsigned.
        fn test_app(dir: &Path) -> (PathBuf, PathBuf) {
            let app = dir.join("Test.app");
            let macos = app.join("Contents/MacOS");
            std::fs::create_dir_all(&macos).unwrap();
            let plugin = app.join("Contents/Resources/bundled/deckpilot.example-default");
            std::fs::create_dir_all(&plugin).unwrap();
            std::fs::write(plugin.join("manifest.json"), "{}\n").unwrap();
            std::fs::write(
                app.join("Contents/Info.plist"),
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleExecutable</key><string>tool</string>\
                 <key>CFBundleIdentifier</key><string>test.deckpilot.seal</string>\
                 <key>CFBundlePackageType</key><string>APPL</string></dict></plist>\n",
            )
            .unwrap();
            std::fs::copy("/usr/bin/true", macos.join("tool")).unwrap();
            (app, macos.join("tool"))
        }

        fn ad_hoc_sign(app: &Path) {
            let output = std::process::Command::new("/usr/bin/codesign")
                .args(["--force", "--deep", "--sign", "-", "--timestamp=none"])
                .arg(app)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        #[test]
        fn an_unsigned_bundle_is_tolerated_only_in_debug_builds() {
            let dir = tempfile::tempdir().unwrap();
            let (app, exe) = test_app(dir.path());
            // The copied executable keeps Apple's signature, the bundle has
            // none of its own.
            std::process::Command::new("/usr/bin/codesign")
                .args(["--remove-signature"])
                .arg(&exe)
                .status()
                .unwrap();
            let seal = check(&exe);
            assert_eq!(seal, Seal::Unsigned { bundle: app });
            assert!(decide(&seal, false).is_err());
            decide(&seal, true).unwrap();
        }

        /// An ad-hoc signed bundle (a local development package) is sealed
        /// but not Apple-anchored; once a resource changes after signing
        /// (a plugin file swapped in `Contents/Resources`), it fails closed
        /// in every build.
        #[test]
        fn a_changed_resource_breaks_the_seal() {
            let dir = tempfile::tempdir().unwrap();
            let (app, exe) = test_app(dir.path());
            ad_hoc_sign(&app);
            let seal = check(&exe);
            assert_eq!(
                seal,
                Seal::AdHoc {
                    bundle: app.clone()
                }
            );
            assert!(decide(&seal, false).is_err());
            decide(&seal, true).unwrap();

            std::fs::write(
                app.join("Contents/Resources/bundled/deckpilot.example-default/manifest.json"),
                "{\"tampered\":true}\n",
            )
            .unwrap();
            let seal = check(&exe);
            assert!(matches!(seal, Seal::Invalid { .. }), "{seal}");
            assert!(decide(&seal, true).is_err());
            assert!(decide(&seal, false).is_err());

            // A file added after signing breaks it too.
            ad_hoc_sign(&app);
            assert!(matches!(check(&exe), Seal::AdHoc { .. }));
            std::fs::write(
                app.join("Contents/Resources/bundled/deckpilot.example-default/plugin.wasm"),
                b"\0asm",
            )
            .unwrap();
            assert!(matches!(check(&exe), Seal::Invalid { .. }));
        }
    }
}
