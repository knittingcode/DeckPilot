//! Persisted trust state and where it is stored.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Keyset, Revocation, Timestamp};

/// Format of [`TrustState`]; any other value is refused.
pub const STATE_FORMAT: u32 = 1;

/// What the client remembers between runs (section 12.4 `registry_state`).
///
/// Everything here was verified before it was stored. It is small (a few
/// KiB plus the revocations) whatever the size of the index, which is
/// cached separately and trusted only when its sha256 equals the stored
/// timestamp's reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustState {
    pub format: u32,
    /// Fingerprint of the embedded root keys the key set chain started
    /// from. A different app build with other roots restarts the chain.
    pub roots: String,
    /// Newest verified key set (`None`: only the embedded roots are known).
    pub keyset: Option<Keyset>,
    /// Rollback floor: the highest seq ever accepted (or a key set's
    /// `seq_floor`). Never decreases except through `seq_floor_reset`.
    pub seq: u64,
    /// Newest verified timestamp.
    pub timestamp: Option<Timestamp>,
    /// Revocations of the newest verified index, enforced at every start.
    pub revocations: Vec<Revocation>,
    /// Set by a key set with `seq_floor_reset`: the next verified index may
    /// drop stored revocations. Cleared once an index is accepted.
    #[serde(default)]
    pub revocations_resettable: bool,
    /// Lower bound for "now": the latest time this client knows has passed
    /// (verified `signed_at`, or the local clock capped at the stored
    /// timestamp's expiry). Setting the clock back cannot make an expired
    /// timestamp fresh again.
    #[serde(default, with = "crate::datetime::option")]
    pub clock_floor: Option<DateTime<Utc>>,
}

impl TrustState {
    pub(crate) fn new(roots: String) -> Self {
        Self {
            format: STATE_FORMAT,
            roots,
            keyset: None,
            seq: 0,
            timestamp: None,
            revocations: Vec::new(),
            revocations_resettable: false,
            clock_floor: None,
        }
    }
}

/// Why the trust store failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("the trust store could not be read or written: {0}")]
    Io(String),
    /// The stored state does not parse. The client refuses to start from
    /// scratch silently: that would drop the rollback floor and the
    /// revocations.
    #[error("the stored trust state is corrupt: {0}")]
    Corrupt(String),
}

/// Where the client keeps its state. The product keeps it in the encrypted
/// database (section 12.4); [`FileTrustStore`] and [`MemoryTrustStore`] are
/// provided for tools and tests.
///
/// `save` must be atomic: after a crash, either the old or the new state is
/// read back, never a mix.
pub trait TrustStore {
    fn load(&self) -> Result<Option<TrustState>, StoreError>;
    fn save(&mut self, state: &TrustState) -> Result<(), StoreError>;
    /// The cached index bytes (verified when they were stored; checked
    /// against the stored timestamp again when read).
    fn load_index(&self) -> Result<Option<Vec<u8>>, StoreError>;
    fn save_index(&mut self, bytes: &[u8]) -> Result<(), StoreError>;
}

fn parse_state(bytes: &[u8]) -> Result<TrustState, StoreError> {
    serde_json::from_slice(bytes).map_err(|e| StoreError::Corrupt(e.to_string()))
}

fn serialize_state(state: &TrustState) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec_pretty(state).map_err(|e| StoreError::Io(e.to_string()))
}

/// Two files in one directory: `registry-trust.json` (the state) and
/// `registry-index.json` (the index cache), each replaced atomically
/// (temporary file in the same directory, fsync, rename, directory fsync).
#[derive(Debug, Clone)]
pub struct FileTrustStore {
    dir: PathBuf,
}

impl FileTrustStore {
    pub const STATE_FILE: &'static str = "registry-trust.json";
    pub const INDEX_FILE: &'static str = "registry-index.json";

    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn state_path(&self) -> PathBuf {
        self.dir.join(Self::STATE_FILE)
    }

    pub fn index_path(&self) -> PathBuf {
        self.dir.join(Self::INDEX_FILE)
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::Io(format!("{}: {e}", path.display()))),
    }
}

/// Replaces `path` with `bytes` atomically.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

impl TrustStore for FileTrustStore {
    fn load(&self) -> Result<Option<TrustState>, StoreError> {
        read_optional(&self.state_path())?
            .map(|bytes| parse_state(&bytes))
            .transpose()
    }

    fn save(&mut self, state: &TrustState) -> Result<(), StoreError> {
        let path = self.state_path();
        write_atomic(&path, &serialize_state(state)?)
            .map_err(|e| StoreError::Io(format!("{}: {e}", path.display())))
    }

    fn load_index(&self) -> Result<Option<Vec<u8>>, StoreError> {
        read_optional(&self.index_path())
    }

    fn save_index(&mut self, bytes: &[u8]) -> Result<(), StoreError> {
        let path = self.index_path();
        write_atomic(&path, bytes).map_err(|e| StoreError::Io(format!("{}: {e}", path.display())))
    }
}

/// In memory, serialized like the file store (so a clone behaves like the
/// state read back after a restart). `fail_saves` simulates a full disk.
#[derive(Debug, Clone, Default)]
pub struct MemoryTrustStore {
    pub state: Option<Vec<u8>>,
    pub index: Option<Vec<u8>>,
    pub fail_saves: bool,
}

impl TrustStore for MemoryTrustStore {
    fn load(&self) -> Result<Option<TrustState>, StoreError> {
        self.state.as_deref().map(parse_state).transpose()
    }

    fn save(&mut self, state: &TrustState) -> Result<(), StoreError> {
        if self.fail_saves {
            return Err(StoreError::Io("simulated failure".into()));
        }
        self.state = Some(serialize_state(state)?);
        Ok(())
    }

    fn load_index(&self) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.index.clone())
    }

    fn save_index(&mut self, bytes: &[u8]) -> Result<(), StoreError> {
        if self.fail_saves {
            return Err(StoreError::Io("simulated failure".into()));
        }
        self.index = Some(bytes.to_vec());
        Ok(())
    }
}
