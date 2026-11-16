//! The registry's trust anchor: which root role a client opens with.
//!
//! A release build that is to use the public registry embeds the root
//! role at compile time (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`, checked by
//! `build.rs`; [`EMBEDDED`]). The agent can also be given a `roots.json`
//! at run time (`--plugin-registry-roots` or its environment variable),
//! for development and self-hosted registries. [`choose`] decides between
//! them:
//!
//! | Build | Embedded roots | Configured `roots.json` | Trusted |
//! | --- | --- | --- | --- |
//! | any | no | no | nothing: no registry, nothing fetched |
//! | any | no | yes | the configured file |
//! | debug, or feature `test-hooks` | yes | yes | the configured file |
//! | release | yes | yes | the embedded roots; the file is ignored |
//! | any | yes | no | the embedded roots |
//!
//! So a release build with embedded roots cannot be pointed at other roots
//! by its command line or environment, while a release build without
//! them keeps working with a self-hosted registry exactly as before.
//!
//! The roots are only the anchor: the key set chain (`keys/<n>.json`,
//! root rotation included) is still walked from them on every refresh
//! ([`crate::RegistryClient::open`] and `apply_keyset`).

use std::path::{Path, PathBuf};

use crate::Role;

/// The build-time variable holding the root role to embed.
pub const BUILD_ENV: &str = "DECKPILOT_REGISTRY_ROOT_PUBKEYS";

const RAW: &str = include_str!(concat!(env!("OUT_DIR"), "/embedded-roots.json"));

/// The root role compiled into this build (the text of `roots.json`,
/// checked at compile time), or `None` when the build embeds none.
pub const EMBEDDED: Option<&str> = if RAW.is_empty() { None } else { Some(RAW) };

/// Whether a configured `roots.json` takes precedence over embedded
/// roots: only in debug builds and with the `test-hooks` feature.
pub const OVERRIDE_ALLOWED: bool = cfg!(any(debug_assertions, feature = "test-hooks"));

/// Where a root role comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootsSource {
    /// Compiled into the build ([`EMBEDDED`]).
    Embedded(&'static str),
    /// A `roots.json` given at run time.
    File(PathBuf),
}

impl RootsSource {
    /// The root role, parsed and validated.
    pub fn load(&self) -> Result<Role, String> {
        let role: Role = match self {
            Self::Embedded(text) => serde_json::from_str(text)
                .map_err(|e| format!("the embedded registry root keys are malformed: {e}"))?,
            Self::File(path) => std::fs::read(path)
                .map_err(|e| format!("the registry root keys {}: {e}", path.display()))
                .and_then(|bytes| {
                    serde_json::from_slice(&bytes)
                        .map_err(|e| format!("the registry root keys are malformed: {e}"))
                })?,
        };
        role.validate()
            .map_err(|e| format!("the registry root keys are invalid: the root role {e}"))?;
        Ok(role)
    }

    /// For logs: `embedded` or the file's path.
    pub fn describe(&self) -> String {
        match self {
            Self::Embedded(_) => "embedded".to_owned(),
            Self::File(path) => path.display().to_string(),
        }
    }
}

/// The outcome of [`choose`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RootsChoice {
    /// The roots to trust; `None`: no registry may be used.
    pub source: Option<RootsSource>,
    /// A configured `roots.json` that was not used, because this release
    /// build embeds its roots (the caller should say so).
    pub ignored: Option<PathBuf>,
}

/// The roots of this build, given the `roots.json` configured at run time
/// (see the module documentation for the rules).
pub fn choose(configured: Option<&Path>) -> RootsChoice {
    choose_from(EMBEDDED, OVERRIDE_ALLOWED, configured)
}

/// [`choose`] with the build's facts as parameters (for tests).
pub fn choose_from(
    embedded: Option<&'static str>,
    override_allowed: bool,
    configured: Option<&Path>,
) -> RootsChoice {
    match (embedded, configured) {
        (None, None) => RootsChoice::default(),
        (None, Some(file)) => RootsChoice {
            source: Some(RootsSource::File(file.to_path_buf())),
            ignored: None,
        },
        (Some(_), Some(file)) if override_allowed => RootsChoice {
            source: Some(RootsSource::File(file.to_path_buf())),
            ignored: None,
        },
        (Some(text), configured) => RootsChoice {
            source: Some(RootsSource::Embedded(text)),
            ignored: configured.map(Path::to_path_buf),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roots_check::check_roots;

    const KEY_A: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    const KEY_B: &str = "RWTk9L8C3/TFMgNbVXg+S3wJqf4XhI5IqE/YH3+fbIRzHW+0bEPRwX0y";

    /// A base64-wrapped minisign public key, as `keyset init` writes them.
    fn wrapped(raw: &str) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .encode(format!("untrusted comment: minisign public key\n{raw}\n"))
    }

    fn role(keys: &[String], threshold: u64) -> String {
        serde_json::json!({"keys": keys, "threshold": threshold}).to_string()
    }

    #[test]
    fn the_build_check_agrees_with_the_client() {
        let a = wrapped(KEY_A);
        let b = wrapped(KEY_B);
        let samples = [
            (role(std::slice::from_ref(&a), 1), true),
            (role(&[a.clone(), b.clone()], 1), true),
            (role(&[a.clone(), b.clone()], 2), true),
            (role(&[a.clone(), b.clone()], 3), false),
            (role(std::slice::from_ref(&a), 0), false),
            (role(&[], 1), false),
            (role(&[a.clone(), a.clone()], 1), false),
            (role(&[KEY_A.to_owned()], 1), false),
            (role(&["not a key".to_owned()], 1), false),
            (role(&[wrapped("RWQnotakey")], 1), false),
            (role(&vec![a.clone(); 5], 1), false),
            (
                format!("{{\"keys\":[\"{a}\"],\"threshold\":1,\"extra\":true}}"),
                false,
            ),
            (
                format!("{{\"keys\":[\"{a}\"],\"threshold\":1,\"threshold\":1}}"),
                false,
            ),
            (format!("{{\"keys\":[\"{a}\"]}}"), false),
            (format!("{{\"keys\":[\"{a}\"],\"threshold\":1.0}}"), false),
            (format!("{{\"keys\":[\"{a}\"],\"threshold\":-1}}"), false),
            ("[]".to_owned(), false),
            (String::new(), false),
            ("{".to_owned(), false),
        ];
        for (text, valid) in samples {
            let build = check_roots(&text);
            let client = serde_json::from_str::<Role>(&text)
                .map_err(|e| e.to_string())
                .and_then(|r| r.validate());
            assert_eq!(build.is_ok(), valid, "build check of {text}: {build:?}");
            assert_eq!(client.is_ok(), valid, "client check of {text}: {client:?}");
        }
    }

    #[test]
    fn embedded_roots_load() {
        // Every build that embeds roots has checked them; the client must
        // accept them too.
        if let Some(text) = EMBEDDED {
            RootsSource::Embedded(text).load().unwrap();
            assert!(check_roots(text.trim()).is_ok());
        }
    }

    #[test]
    fn precedence() {
        let file = Path::new("/dev/roots.json");
        let embedded = Some("{}");
        assert_eq!(choose_from(None, false, None), RootsChoice::default());
        assert_eq!(choose_from(None, true, None), RootsChoice::default());
        for allowed in [false, true] {
            assert_eq!(
                choose_from(None, allowed, Some(file)).source,
                Some(RootsSource::File(file.into())),
                "without embedded roots a configured file is used"
            );
            assert_eq!(
                choose_from(embedded, allowed, None),
                RootsChoice {
                    source: Some(RootsSource::Embedded("{}")),
                    ignored: None
                }
            );
        }
        assert_eq!(
            choose_from(embedded, true, Some(file)),
            RootsChoice {
                source: Some(RootsSource::File(file.into())),
                ignored: None
            }
        );
        assert_eq!(
            choose_from(embedded, false, Some(file)),
            RootsChoice {
                source: Some(RootsSource::Embedded("{}")),
                ignored: Some(file.into())
            }
        );
    }

    #[test]
    fn override_only_in_debug_or_test_builds() {
        assert_eq!(
            OVERRIDE_ALLOWED,
            cfg!(debug_assertions) || cfg!(feature = "test-hooks")
        );
    }
}
