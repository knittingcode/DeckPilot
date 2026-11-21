//! The registry the agent installs from, read through a
//! [`RegistrySource`] and verified by `RegistryClient` against trusted
//! root keys:
//!
//! - a local directory laid out as the publishing tool writes it
//!   (`site/v1/` for the signed documents, `releases/` for the packages),
//!   `--plugin-registry <dir>`;
//! - or a registry served over HTTPS (`fetch.rs`),
//!   `--plugin-registry-url <https URL of site/>`, with packages from the
//!   pinned release URL or `--plugin-registry-releases-url`.
//!
//! The root keys are embedded at build time
//! (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`, `deckpilot_plugin_registry::roots`)
//! or given as a `roots.json` at run time (`--plugin-registry-roots`). A
//! configured file overrides embedded roots only in debug and `test-hooks`
//! builds; a release build with embedded roots ignores it (and says so in
//! the log). Without either, nothing is fetched and the registry reports
//! "not configured" (every build today: no roots are embedded yet). With
//! roots and no other registry named, the official site
//! [`DEFAULT_REGISTRY_URL`] is used.
//!
//! The verified state (trust state and index cache) is kept in
//! `<data dir>/plugin-registry/` by the crate's `FileTrustStore`, and the
//! verified detail documents in `details/` below it, so browsing and the
//! freshness report work offline. Details are content-addressed (the
//! verified index pins their sha256), so a cached one is re-checked
//! against the index each time it is read and never trusted by itself.

use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use deckpilot_ipc::plugins::{RegistryFreshness, RegistryStaleReason, RegistryStatus, reason};
use deckpilot_ipc::{ErrorCode, IpcError};
use deckpilot_plugin_manifest::{PackageDigest, PluginId};
use deckpilot_plugin_package::sha256;
use deckpilot_plugin_registry::roots::{self, RootsChoice, RootsSource};
use deckpilot_plugin_registry::{
    DOWNLOAD_PREFIX, FetchError, FileRef, FileTrustStore, Freshness, MAX_DETAIL_BYTES,
    PluginDetail, Policy, Refresh, RegistryClient, RegistryError, RegistrySource, StaleReason,
    write_atomic,
};
use tracing::{debug, info, warn};

use super::PluginSettings;
use super::fetch::{DownloadError, HttpsConfig, HttpsSource, TlsRoots};

/// The official registry site (GitHub Pages of `knittingcode/deckpilot-plugins`,
/// documents under `v1/`). Used only when there are trusted root keys
/// (embedded, or `--plugin-registry-roots`) and no other registry is
/// named.
pub const DEFAULT_REGISTRY_URL: &str = "https://knittingcode.github.io/deckpilot-plugins/";

/// Largest package read (the package limit).
const MAX_PACKAGE_READ: u64 = deckpilot_plugin_manifest::MAX_PACKAGE_BYTES;
/// Verified detail documents, below the state directory.
const DETAILS_DIR: &str = "details";

/// Reads registry documents from a directory, refusing any path that
/// could leave it.
#[derive(Debug, Clone)]
pub(super) struct DirectorySource {
    root: PathBuf,
}

/// `relative` as a path below `root`, or `None` if it is absolute or
/// contains `..`, `.`, a drive prefix or a backslash.
fn below(root: &Path, relative: &str) -> Option<PathBuf> {
    if relative.is_empty() || relative.contains('\\') {
        return None;
    }
    let path = Path::new(relative);
    path.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| root.join(path))
}

fn read_capped(path: &Path, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => FetchError::NotFound,
        _ => FetchError::Unavailable(e.to_string()),
    })?;
    let mut bytes = Vec::new();
    // One byte more than allowed, so the client sees an oversized document.
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::Unavailable(e.to_string()))?;
    Ok(bytes)
}

impl RegistrySource for DirectorySource {
    fn fetch(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        let file = below(&self.root, path).ok_or(FetchError::NotFound)?;
        read_capped(&file, u64::try_from(max_bytes).unwrap_or(u64::MAX))
    }
}

/// Where documents and packages come from.
#[derive(Debug)]
enum Source {
    Directory {
        site: DirectorySource,
        releases: PathBuf,
    },
    Https(Box<HttpsSource>),
}

impl Source {
    fn documents(&self) -> &dyn RegistrySource {
        match self {
            Self::Directory { site, .. } => site,
            Self::Https(https) => https.as_ref(),
        }
    }
}

/// The test CA (feature `test-hooks` only, never in a release): the PEM
/// file named by `DECKPILOT_TEST_REGISTRY_CA` becomes the **only** root
/// the registry client trusts.
#[cfg(feature = "test-hooks")]
pub const TEST_CA_ENV: &str = "DECKPILOT_TEST_REGISTRY_CA";

fn tls_roots() -> Result<TlsRoots, String> {
    #[cfg(feature = "test-hooks")]
    if let Some(path) = std::env::var_os(TEST_CA_ENV) {
        let pem = std::fs::read_to_string(&path)
            .map_err(|e| format!("{TEST_CA_ENV} {}: {e}", Path::new(&path).display()))?;
        return Ok(TlsRoots::Only(vec![pem]));
    }
    Ok(TlsRoots::System)
}

/// Why a package could not be obtained, as an IPC error: unreachable or
/// refused is a registry problem, wrong bytes an invalid package.
pub(super) fn download_error(error: &DownloadError) -> IpcError {
    match error {
        DownloadError::Unavailable(m) | DownloadError::Refused(m) => {
            IpcError::new(ErrorCode::Failed, m.clone()).with_reason(reason::REGISTRY)
        }
        DownloadError::Corrupt(m) => {
            IpcError::new(ErrorCode::Failed, m.clone()).with_reason(reason::INVALID_PACKAGE)
        }
    }
}

/// The configured registry and its verified state.
pub(super) struct Registry {
    /// Shown to the user and recorded as the install source.
    pub name: String,
    source: Source,
    client: RegistryClient<FileTrustStore>,
    details: PathBuf,
    /// Why the last refresh could not reach the registry (`None`: it did,
    /// or there was no refresh yet).
    offline: Option<String>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

fn open_client(
    roots: &RootsSource,
    state_dir: &Path,
) -> Result<RegistryClient<FileTrustStore>, String> {
    let roots = roots.load()?;
    RegistryClient::open(roots, FileTrustStore::new(state_dir), Policy::default())
        .map_err(|e| format!("the registry state cannot be read: {e}"))
}

impl Registry {
    /// The registry `settings` configure, with its verified state in
    /// `state_dir`: `Ok(None)` when none is configured. The root keys are
    /// this build's ([`roots::choose`]): embedded, or the configured
    /// `roots.json` where that may override them.
    pub fn from_settings(
        settings: &PluginSettings,
        state_dir: &Path,
    ) -> Result<Option<Self>, String> {
        let choice = roots::choose(settings.registry_roots.as_deref());
        Self::from_choice(settings, &choice, state_dir)
    }

    /// [`Registry::from_settings`] with the roots already chosen.
    ///
    /// Trusted root keys are required, and none is ever taken from the
    /// network. Without them nothing is fetched (`Ok(None)` when no
    /// registry is named either). With roots and neither a directory nor a
    /// URL, the official site [`DEFAULT_REGISTRY_URL`] is used.
    pub fn from_choice(
        settings: &PluginSettings,
        choice: &RootsChoice,
        state_dir: &Path,
    ) -> Result<Option<Self>, String> {
        if let Some(ignored) = &choice.ignored {
            warn!(
                roots = %ignored.display(),
                "this build embeds the registry root keys; --plugin-registry-roots \
                 (DECKPILOT_PLUGIN_REGISTRY_ROOTS) is for development builds and is ignored"
            );
        }
        let Some(roots) = choice.source.as_ref() else {
            return if settings.registry.is_some() || settings.registry_url.is_some() {
                Err(
                    "the plugin registry has no trusted root keys (--plugin-registry-roots); \
                     nothing is fetched or trusted without them"
                        .to_owned(),
                )
            } else {
                Ok(None)
            };
        };
        info!(roots = %roots.describe(), "registry root keys");
        match (&settings.registry, &settings.registry_url) {
            (Some(_), Some(_)) => Err("both a local plugin registry (--plugin-registry) and a \
                 registry URL (--plugin-registry-url) are configured; give one"
                .to_owned()),
            (Some(dir), None) => Self::open(dir, roots, state_dir).map(Some),
            (None, url) => {
                let config = HttpsConfig::new(
                    url.as_deref().unwrap_or(DEFAULT_REGISTRY_URL),
                    settings.registry_releases_url.as_deref(),
                    tls_roots()?,
                )?;
                Self::open_https(config, roots, state_dir).map(Some)
            }
        }
    }

    /// Opens the registry in `dir` with the root role in `roots`, keeping
    /// the verified state in `state_dir`.
    pub fn open(dir: &Path, roots: &RootsSource, state_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            name: format!("local:{}", dir.display()),
            source: Source::Directory {
                site: DirectorySource {
                    root: dir.join("site").join("v1"),
                },
                releases: dir.join("releases"),
            },
            client: open_client(roots, state_dir)?,
            details: state_dir.join(DETAILS_DIR),
            offline: None,
        })
    }

    /// Opens the registry served at `config.site`.
    pub fn open_https(
        config: HttpsConfig,
        roots: &RootsSource,
        state_dir: &Path,
    ) -> Result<Self, String> {
        let source = HttpsSource::new(config)?;
        Ok(Self {
            name: source.name(),
            source: Source::Https(Box::new(source)),
            client: open_client(roots, state_dir)?,
            details: state_dir.join(DETAILS_DIR),
            offline: None,
        })
    }

    pub fn client(&self) -> &RegistryClient<FileTrustStore> {
        &self.client
    }

    /// Re-reads the registry and verifies it (key sets, timestamp, index).
    /// Unreachable: the verified state is used as it is
    /// ([`Refresh::Offline`]). A refused document changes nothing stored.
    pub fn refresh(&mut self) -> Result<Refresh, RegistryError> {
        if let Source::Https(https) = &self.source {
            let _ = https.take_unavailable();
        }
        let result = self.client.refresh(self.source.documents(), Utc::now());
        self.offline = match (&result, &self.source) {
            (Ok(Refresh::Offline(_)), Source::Https(https)) => Some(
                https
                    .take_unavailable()
                    .unwrap_or_else(|| "the registry could not be reached".to_owned()),
            ),
            (Ok(Refresh::Offline(_)), Source::Directory { .. }) => {
                Some("the registry directory could not be read".to_owned())
            }
            _ => None,
        };
        match &result {
            Ok(Refresh::Updated { seq }) => {
                info!(registry = %self.name, seq, "plugin registry verified: a new index");
                self.prune_details();
            }
            Ok(Refresh::Unchanged { seq }) => {
                debug!(registry = %self.name, seq, "plugin registry verified: the index is unchanged");
            }
            Ok(Refresh::Offline(_)) | Err(_) => {}
        }
        result
    }

    pub fn status(&self) -> RegistryStatus {
        let freshness = self.client.freshness(Utc::now());
        RegistryStatus {
            source: self.name.clone(),
            freshness: match freshness {
                Freshness::NeverVerified => RegistryFreshness::NeverVerified,
                Freshness::Fresh => RegistryFreshness::Fresh,
                Freshness::Stale(_) => RegistryFreshness::Stale,
            },
            seq: self.client.index().map(|index| index.seq),
            stale_reason: match freshness {
                Freshness::Stale(StaleReason::TimestampExpired) => {
                    Some(RegistryStaleReason::Expired)
                }
                Freshness::Stale(StaleReason::IndexExpired) => {
                    Some(RegistryStaleReason::IndexExpired)
                }
                Freshness::Stale(StaleReason::IndexUnavailable) => {
                    Some(RegistryStaleReason::IndexUnavailable)
                }
                _ => None,
            },
            expires: self
                .client
                .state()
                .timestamp
                .as_ref()
                .map(|t| t.expires.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            offline: self.offline.clone(),
        }
    }

    fn cache_path(&self, reference: &FileRef) -> PathBuf {
        self.details.join(format!("{}.json", reference.sha256))
    }

    /// The cached detail, only if it is exactly what `reference` pins.
    fn cached_detail(&self, reference: &FileRef) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.cache_path(reference)).ok()?;
        (u64::try_from(bytes.len()).ok() == Some(reference.length)
            && sha256(&bytes) == reference.sha256)
            .then_some(bytes)
    }

    /// Removes cached details the verified index no longer pins.
    fn prune_details(&self) {
        let Some(index) = self.client.index() else {
            return;
        };
        let keep: std::collections::BTreeSet<String> = index
            .plugins
            .iter()
            .map(|p| format!("{}.json", p.detail.sha256))
            .collect();
        let Ok(entries) = std::fs::read_dir(&self.details) else {
            return;
        };
        for entry in entries.flatten() {
            if !keep.contains(&entry.file_name().to_string_lossy().into_owned()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// The detail document of `id`, as pinned by the verified index: from
    /// the cache, or fetched (and cached once it matches the pin).
    pub fn detail_bytes(&self, id: &PluginId) -> Result<Vec<u8>, String> {
        let reference = self
            .client
            .detail_ref(id)
            .ok_or_else(|| format!("{id} is not in the registry"))?;
        if let Some(bytes) = self.cached_detail(reference) {
            return Ok(bytes);
        }
        let limit = usize::try_from(MAX_DETAIL_BYTES).unwrap_or(usize::MAX);
        let bytes = self
            .source
            .documents()
            .fetch(&reference.path, limit)
            .map_err(|e| format!("the registry detail of {id}: {e}"))?;
        if u64::try_from(bytes.len()).ok() == Some(reference.length)
            && sha256(&bytes) == reference.sha256
            && let Err(e) = write_atomic(&self.cache_path(reference), &bytes)
        {
            warn!(plugin = %id, error = %e, "the registry detail could not be cached");
        }
        Ok(bytes)
    }

    /// The verified detail of `id`.
    pub fn detail(&self, id: &PluginId) -> Result<PluginDetail, String> {
        let bytes = self.detail_bytes(id)?;
        self.client
            .verify_detail(id, &bytes)
            .map_err(|e| e.to_string())
    }

    /// The package at `url` (a pinned release asset URL): from `releases/`
    /// or downloaded, and only returned if it is `digest`'s size and
    /// sha256 (`verify_package` checks again, with everything else).
    pub fn package(&self, url: &str, digest: &PackageDigest) -> Result<Vec<u8>, DownloadError> {
        let bytes = match &self.source {
            Source::Https(https) => return https.package(url, digest),
            Source::Directory { releases, .. } => {
                let not_ours = || {
                    DownloadError::Refused(format!("{url} is not a release asset of this registry"))
                };
                let relative = url.strip_prefix(DOWNLOAD_PREFIX).ok_or_else(not_ours)?;
                let path = below(releases, relative).ok_or_else(not_ours)?;
                read_capped(&path, MAX_PACKAGE_READ).map_err(|e| {
                    DownloadError::Unavailable(format!(
                        "the package {relative} could not be read: {e}"
                    ))
                })?
            }
        };
        Ok(bytes)
    }

    /// The publisher signature file of the key `key_id` next to the package
    /// at `url` (`<package>.<KEYID>.minisig`): from `releases/` or
    /// downloaded, at most [`publisher::MAX_SIGNATURE_BYTES`] bytes. Only
    /// the publisher verifier decides what it is worth (`official.rs`).
    ///
    /// [`publisher::MAX_SIGNATURE_BYTES`]: deckpilot_plugin_registry::publisher::MAX_SIGNATURE_BYTES
    pub fn signature(&self, url: &str, key_id: &str) -> Result<Vec<u8>, DownloadError> {
        match &self.source {
            Source::Https(https) => https.signature(url, key_id),
            Source::Directory { releases, .. } => {
                let not_ours = || {
                    DownloadError::Refused(format!("{url} is not a release asset of this registry"))
                };
                if !deckpilot_plugin_registry::publisher::is_key_id(key_id) {
                    return Err(not_ours());
                }
                let relative = url.strip_prefix(DOWNLOAD_PREFIX).ok_or_else(not_ours)?;
                let name =
                    deckpilot_plugin_registry::publisher::signature_file_name(relative, key_id);
                let path = below(releases, &name).ok_or_else(not_ours)?;
                let limit =
                    u64::try_from(deckpilot_plugin_registry::publisher::MAX_SIGNATURE_BYTES)
                        .unwrap_or(u64::MAX);
                read_capped(&path, limit).map_err(|e| {
                    DownloadError::Unavailable(format!(
                        "the publisher signature {name} could not be read: {e}"
                    ))
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_never_leave_the_directory() {
        let root = Path::new("/r");
        assert_eq!(
            below(root, "index/1.json"),
            Some(PathBuf::from("/r/index/1.json"))
        );
        for bad in [
            "",
            "../x",
            "a/../b",
            "/etc/passwd",
            "a\\b",
            "./a",
            "a//b/..",
        ] {
            assert_eq!(below(root, bad), None, "{bad}");
        }
    }

    /// As in a build without embedded roots.
    fn unembedded(settings: &PluginSettings, dir: &Path) -> Result<Option<Registry>, String> {
        let choice = roots::choose_from(None, false, settings.registry_roots.as_deref());
        Registry::from_choice(settings, &choice, dir)
    }

    #[test]
    fn a_registry_needs_exactly_one_source_and_roots() {
        let dir = std::env::temp_dir();
        let none = PluginSettings::default();
        assert!(unembedded(&none, &dir).unwrap().is_none());
        let both = PluginSettings {
            registry: Some(dir.clone()),
            registry_url: Some("https://registry.example/".into()),
            registry_roots: Some(dir.join("roots.json")),
            ..PluginSettings::default()
        };
        assert!(unembedded(&both, &dir).unwrap_err().contains("give one"));
        let no_roots = PluginSettings {
            registry_url: Some("https://registry.example/".into()),
            ..PluginSettings::default()
        };
        assert!(
            unembedded(&no_roots, &dir)
                .unwrap_err()
                .contains("--plugin-registry-roots")
        );
        let plain_http = PluginSettings {
            registry_url: Some("http://registry.example/".into()),
            registry_roots: Some(dir.join("roots.json")),
            ..PluginSettings::default()
        };
        assert!(unembedded(&plain_http, &dir).unwrap_err().contains("https"));
    }
}
