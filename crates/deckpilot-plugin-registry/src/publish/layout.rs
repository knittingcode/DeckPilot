//! The working directory and its files.
//!
//! ```text
//! <dir>/roots.json                         embedded root role (public; the app's
//!                                          DECKPILOT_REGISTRY_ROOT_PUBKEYS)
//! <dir>/registry/publishers/<login>.json   publisher entries of the index
//! <dir>/registry/plugins/<id>/plugin.json  listing and detail metadata
//! <dir>/registry/plugins/<id>/versions/<version>.json   version history
//! <dir>/registry/revocations.json          cumulative revocations
//! <dir>/releases/<id>--v<version>/<id>-<version>.dpplugin   release assets
//! <dir>/site/v1/                           the Pages tree, served as is:
//!     keys/<n>.json, keys/<n>.json.minisig
//!     index/<seq>.json, index/<seq>.json.minisig
//!     plugins/<id>/<sha256>.json
//!     timestamp.json, timestamp.json.minisig
//! ```

use std::path::{Path, PathBuf};

use deckpilot_plugin_manifest::PluginId;
use semver::Version;
use serde::de::DeserializeOwned;

use super::{PublishError, Result, fail, to_json};
use crate::error::FetchError;
use crate::model::DOWNLOAD_PREFIX;
use crate::{RegistrySource, write_atomic};

/// The working directory of the publishing tool.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `site/v1`: exactly what is deployed to Pages.
    pub fn site(&self) -> PathBuf {
        self.root.join("site").join("v1")
    }

    pub fn roots_path(&self) -> PathBuf {
        self.root.join("roots.json")
    }

    pub fn registry(&self) -> PathBuf {
        self.root.join("registry")
    }

    pub fn publishers_dir(&self) -> PathBuf {
        self.registry().join("publishers")
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.registry().join("plugins")
    }

    pub fn plugin_info_path(&self, id: &PluginId) -> PathBuf {
        self.plugins_dir().join(id.as_str()).join("plugin.json")
    }

    pub fn versions_dir(&self, id: &PluginId) -> PathBuf {
        self.plugins_dir().join(id.as_str()).join("versions")
    }

    pub fn version_path(&self, id: &PluginId, version: &Version) -> PathBuf {
        self.versions_dir(id).join(format!("{version}.json"))
    }

    pub fn revocations_path(&self) -> PathBuf {
        self.registry().join("revocations.json")
    }

    /// Local copy of a release asset, by its URL under the download prefix.
    pub fn release_file(&self, url: &str) -> Option<PathBuf> {
        let rest = url.strip_prefix(DOWNLOAD_PREFIX)?;
        let (tag, name) = rest.split_once('/')?;
        let safe = |s: &str| {
            !s.is_empty()
                && !s.starts_with('.')
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        };
        (safe(tag) && safe(name)).then(|| self.root.join("releases").join(tag).join(name))
    }

    pub fn keyset_path(&self, version: u64) -> PathBuf {
        self.site().join("keys").join(format!("{version}.json"))
    }

    pub fn index_path(&self, seq: u64) -> PathBuf {
        self.site().join("index").join(format!("{seq}.json"))
    }

    pub fn timestamp_path(&self) -> PathBuf {
        self.site().join("timestamp.json")
    }

    /// The newest key set version present (`keys/1.json`, `keys/2.json`…
    /// without gaps); 0 if there is none.
    pub fn newest_keyset(&self) -> u64 {
        let mut version = 0;
        while self.keyset_path(version + 1).is_file() {
            version += 1;
        }
        version
    }

    /// Every `index/<seq>.json` present, ascending.
    pub fn index_seqs(&self) -> Result<Vec<u64>> {
        let dir = self.site().join("index");
        let mut seqs = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(seqs),
            Err(e) => return fail(format!("{}: {e}", dir.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|e| PublishError(format!("{}: {e}", dir.display())))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Some(number) = name.strip_suffix(".json")
                && let Some(seq) = canonical_u64(number)
            {
                seqs.push(seq);
            }
        }
        seqs.sort_unstable();
        Ok(seqs)
    }
}

/// `<path>.minisig`.
pub(crate) fn sig_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".minisig");
    PathBuf::from(name)
}

pub(crate) fn canonical_u64(text: &str) -> Option<u64> {
    if text.is_empty()
        || (text.len() > 1 && text.starts_with('0'))
        || !text.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    text.parse().ok()
}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => fail(format!("{}: {e}", path.display())),
    }
}

pub(crate) fn parse<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    parse(path, &read(path)?)
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic(path, bytes).map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

pub(crate) fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    write(path, &to_json(value)?)
}

/// Writes an immutable file: refused if it exists with other bytes.
pub(crate) fn write_immutable(path: &Path, bytes: &[u8]) -> Result<()> {
    match read_optional(path)? {
        Some(existing) if existing == bytes => Ok(()),
        Some(_) => fail(format!(
            "{} already exists with other content (published files are immutable)",
            path.display()
        )),
        None => write(path, bytes),
    }
}

pub(crate) fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => fail(format!("{}: {e}", path.display())),
    }
}

/// The generated site as the client sees it, read from a directory. Only
/// the paths the registry uses are served (letters, digits, `.`, `-`, `_`
/// and `/`, no empty, hidden or `..` component).
#[derive(Debug, Clone)]
pub struct SiteSource {
    dir: PathBuf,
}

impl SiteSource {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl RegistrySource for SiteSource {
    fn fetch(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        let valid = path.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        });
        if !valid {
            return Err(FetchError::NotFound);
        }
        let file = self.dir.join(path);
        match std::fs::metadata(&file) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(FetchError::NotFound),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(FetchError::NotFound);
            }
            Err(e) => return Err(FetchError::Unavailable(e.to_string())),
        }
        let bytes = std::fs::read(&file).map_err(|e| FetchError::Unavailable(e.to_string()))?;
        // Like a fetcher that stops at the cap; the client refuses it.
        let keep = bytes.len().min(max_bytes.saturating_add(1));
        Ok(bytes[..keep].to_vec())
    }
}
