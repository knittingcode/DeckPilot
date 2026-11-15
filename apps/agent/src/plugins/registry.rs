//! The registry the agent installs from: for now a local directory laid
//! out as the publishing tool writes it (`site/v1/` for the signed
//! documents, `releases/` for the packages), read through a
//! [`RegistrySource`]. Everything is verified by `RegistryClient`
//! against root keys given separately (they stand for the roots a release
//! build will embed). No network code.

use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use deckpilot_ipc::plugins::{RegistryFreshness, RegistryStatus};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_registry::{
    DOWNLOAD_PREFIX, FetchError, FileTrustStore, Freshness, MAX_DETAIL_BYTES, PluginDetail, Policy,
    Refresh, RegistryClient, RegistryError, RegistrySource, Role,
};

/// Largest package read (the package limit).
const MAX_PACKAGE_READ: u64 = deckpilot_plugin_manifest::MAX_PACKAGE_BYTES;

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

/// The configured registry and its verified state.
pub(super) struct LocalRegistry {
    /// Shown to the user and recorded as the install source.
    pub name: String,
    source: DirectorySource,
    releases: PathBuf,
    client: RegistryClient<FileTrustStore>,
}

impl std::fmt::Debug for LocalRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalRegistry")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl LocalRegistry {
    /// Opens the registry in `dir` with the root role in `roots`, keeping
    /// the verified state in `state_dir`.
    pub fn open(dir: &Path, roots: &Path, state_dir: &Path) -> Result<Self, String> {
        let roots: Role = std::fs::read(roots)
            .map_err(|e| format!("the registry root keys {}: {e}", roots.display()))
            .and_then(|bytes| {
                serde_json::from_slice(&bytes)
                    .map_err(|e| format!("the registry root keys are malformed: {e}"))
            })?;
        let client = RegistryClient::open(roots, FileTrustStore::new(state_dir), Policy::default())
            .map_err(|e| format!("the registry state cannot be read: {e}"))?;
        Ok(Self {
            name: format!("local:{}", dir.display()),
            source: DirectorySource {
                root: dir.join("site").join("v1"),
            },
            releases: dir.join("releases"),
            client,
        })
    }

    pub fn client(&self) -> &RegistryClient<FileTrustStore> {
        &self.client
    }

    /// Re-reads the registry and verifies it (key sets, timestamp, index).
    pub fn refresh(&mut self) -> Result<Refresh, RegistryError> {
        self.client.refresh(&self.source, Utc::now())
    }

    pub fn status(&self) -> RegistryStatus {
        RegistryStatus {
            source: self.name.clone(),
            freshness: match self.client.freshness(Utc::now()) {
                Freshness::NeverVerified => RegistryFreshness::NeverVerified,
                Freshness::Fresh => RegistryFreshness::Fresh,
                Freshness::Stale(_) => RegistryFreshness::Stale,
            },
            seq: self.client.index().map(|index| index.seq),
        }
    }

    /// The detail document of `id`, as pinned by the verified index.
    pub fn detail_bytes(&self, id: &PluginId) -> Result<Vec<u8>, String> {
        let reference = self
            .client
            .detail_ref(id)
            .ok_or_else(|| format!("{id} is not in the registry"))?;
        let limit = usize::try_from(MAX_DETAIL_BYTES).unwrap_or(usize::MAX);
        self.source
            .fetch(&reference.path, limit)
            .map_err(|e| format!("the registry detail of {id}: {e}"))
    }

    /// The verified detail of `id`.
    pub fn detail(&self, id: &PluginId) -> Result<PluginDetail, String> {
        let bytes = self.detail_bytes(id)?;
        self.client
            .verify_detail(id, &bytes)
            .map_err(|e| e.to_string())
    }

    /// The package at `url` (a pinned release asset URL), from `releases/`.
    pub fn package(&self, url: &str) -> Result<Vec<u8>, String> {
        let relative = url
            .strip_prefix(DOWNLOAD_PREFIX)
            .ok_or_else(|| format!("{url} is not a release asset of this registry"))?;
        let path = below(&self.releases, relative)
            .ok_or_else(|| format!("{url} is not a release asset of this registry"))?;
        read_capped(&path, MAX_PACKAGE_READ)
            .map_err(|e| format!("the package {relative} could not be read: {e}"))
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
}
