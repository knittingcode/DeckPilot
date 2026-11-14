//! Spike 5: client-side verifier for the static registry (§9.3, §10, §13.5,
//! §14 of the design).
//!
//! The registry is a set of static files (GitHub Pages). [`Source`]
//! abstracts fetching them so tests can serve honest, stale, tampered or
//! no responses. Trust state is persisted between runs in one small JSON
//! file ([`TrustState`]), written atomically.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{Duration, OffsetDateTime};

use crate::package::ExpectedPackage;
use crate::registry::{
    DOWNLOAD_PREFIX, Detail, FileRef, Index, Keyset, Revocation, RevocationAction, Role, Timestamp,
    sha256_hex,
};
use crate::sig::{self, SigError};

pub const CLOCK_SKEW: Duration = Duration::minutes(5);
pub const MAX_KEYSET: usize = 16 * 1024;
pub const MAX_KEYSET_SIGS: usize = 16 * 1024;
pub const MAX_TIMESTAMP: usize = 4 * 1024;
pub const MAX_SIG: usize = 2 * 1024;
pub const MAX_INDEX: usize = 4 * 1024 * 1024;
pub const MAX_DETAIL: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FetchError {
    #[error("not found")]
    NotFound,
    #[error("network unreachable")]
    Offline,
}

pub trait Source {
    fn get(&self, path: &str) -> Result<Vec<u8>, FetchError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrustError {
    #[error("{0}: {1}")]
    Fetch(String, FetchError),
    #[error("{0} exceeds its size cap")]
    TooLarge(String),
    #[error("{0} is malformed: {1}")]
    Malformed(String, String),
    #[error("no key set is published (keys/1.json missing)")]
    NoKeyset,
    #[error("keys/{0}.json: {1}")]
    KeysetSignature(u64, SigError),
    #[error("timestamp signature: {0}")]
    TimestampSignature(SigError),
    #[error("index signature: {0}")]
    IndexSignature(SigError),
    #[error("{doc} names key set {found}, the current key set is {current}")]
    KeysVersion {
        doc: &'static str,
        found: u64,
        current: u64,
    },
    #[error("timestamp expired at {0} (freeze or stale mirror)")]
    TimestampExpired(OffsetDateTime),
    #[error("index expired at {0}")]
    IndexExpired(OffsetDateTime),
    #[error("rollback: offered seq {offered} is below the stored seq {stored}")]
    Rollback { stored: u64, offered: u64 },
    #[error("two different indexes were signed for seq {0}")]
    Equivocation(u64),
    #[error("index reference does not match the timestamp: {0}")]
    IndexMismatch(String),
    #[error("index download prefix {0:?} is not the registry's")]
    BadDownloadPrefix(String),
    #[error("revocation for {0} was dropped (revocations are cumulative)")]
    RevocationDropped(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InstallError {
    #[error("the registry has never been verified on this machine")]
    NeverVerified,
    #[error("the registry is stale ({0}); installs and updates are blocked")]
    Stale(StaleReason),
    #[error("plugin {0} is not in the registry")]
    UnknownPlugin(String),
    #[error("version {0} is not in the registry")]
    UnknownVersion(String),
    #[error("{0} is revoked (disable): {1}")]
    Revoked(String, String),
    #[error("{0} is yanked: {1}")]
    Yanked(String, String),
    #[error("detail file: {0}")]
    Detail(String),
    #[error("{0}")]
    Trust(TrustError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaleReason {
    TimestampExpired,
    IndexExpired,
}

impl std::fmt::Display for StaleReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StaleReason::TimestampExpired => "timestamp expired",
            StaleReason::IndexExpired => "index expired",
        })
    }
}

/// How usable the stored (verified) state is right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    NeverVerified,
    Fresh,
    Stale(StaleReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    /// A new index (or the first one) was verified and stored.
    Updated { seq: u64 },
    /// Same index; possibly a re-signed timestamp with a later expiry.
    Unchanged { seq: u64 },
    /// The registry could not be reached. Nothing changed; the stored state
    /// is used with the given freshness (§14).
    Offline(Freshness),
}

/// What to do with an installed plugin version (§13.5). Works from the
/// stored revocations alone, so it also applies offline and when stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledVerdict {
    Run,
    /// Yanked: keeps running, notice in Settings.
    RunWithNotice {
        reason: String,
    },
    /// Disabled: stop the host, refuse to load.
    Disable {
        reason: String,
        advisory: Option<String>,
    },
}

/// Persisted between runs. In the product this lives in the encrypted DB;
/// here it is one small JSON file. The verified index itself is cached in a
/// separate file next to it and only trusted if its sha256 equals
/// `timestamp.index.sha256` from this state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustState {
    /// Newest verified key set (None: only the embedded roots are known).
    pub keyset: Option<Keyset>,
    /// Highest seq ever accepted: the rollback floor.
    pub seq: u64,
    pub timestamp: Option<Timestamp>,
    /// Revocations of the newest verified index, enforced at every start.
    pub revocations: Vec<Revocation>,
}

pub struct Client {
    roots: Role,
    state_path: PathBuf,
    pub state: TrustState,
    /// Verified index matching `state.timestamp` (cache, not trust state).
    pub index: Option<Index>,
}

fn malformed(path: &str, e: impl std::fmt::Display) -> TrustError {
    TrustError::Malformed(path.to_owned(), e.to_string())
}

fn fetch_capped(src: &dyn Source, path: &str, cap: usize) -> Result<Vec<u8>, TrustError> {
    let bytes = src
        .get(path)
        .map_err(|e| TrustError::Fetch(path.to_owned(), e))?;
    if bytes.len() > cap {
        return Err(TrustError::TooLarge(path.to_owned()));
    }
    Ok(bytes)
}

fn sig_text(path: &str, bytes: Vec<u8>) -> Result<String, TrustError> {
    String::from_utf8(bytes)
        .map(|s| s.trim().to_owned())
        .map_err(|e| malformed(path, e))
}

/// Offline anywhere in a refresh means "use the cache".
fn is_offline(e: &TrustError) -> bool {
    matches!(e, TrustError::Fetch(_, FetchError::Offline))
}

impl Client {
    /// `roots` are the root public keys compiled into the app
    /// (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`).
    pub fn open(roots: Role, state_path: impl AsRef<Path>) -> std::io::Result<Self> {
        let state_path = state_path.as_ref().to_owned();
        let state = match std::fs::read(&state_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => TrustState::default(),
            Err(e) => return Err(e),
        };
        // The cache is only used if it is exactly the index the stored
        // (verified) timestamp pins; otherwise it is refetched.
        let index = state.timestamp.as_ref().and_then(|ts| {
            let bytes = std::fs::read(cache_path(&state_path)).ok()?;
            check_ref(&bytes, &ts.index).ok()?;
            serde_json::from_slice(&bytes).ok()
        });
        Ok(Self {
            roots,
            state_path,
            state,
            index,
        })
    }

    fn save(&self) {
        write_atomic(
            &self.state_path,
            &serde_json::to_vec_pretty(&self.state).expect("state serializes"),
        );
    }

    pub fn keys_version(&self) -> u64 {
        self.state.keyset.as_ref().map_or(0, |k| k.version)
    }

    fn root_role(&self) -> &Role {
        self.state.keyset.as_ref().map_or(&self.roots, |k| &k.root)
    }

    pub fn freshness(&self, now: OffsetDateTime) -> Freshness {
        match (&self.state.timestamp, &self.index) {
            (Some(ts), Some(index)) => {
                if now >= ts.expires + CLOCK_SKEW {
                    Freshness::Stale(StaleReason::TimestampExpired)
                } else if now >= index.expires + CLOCK_SKEW {
                    Freshness::Stale(StaleReason::IndexExpired)
                } else {
                    Freshness::Fresh
                }
            }
            _ => Freshness::NeverVerified,
        }
    }

    /// One check (§9.3 steps 1-3). On a security error the stored state is
    /// left as it was (except an already verified key set advance), the
    /// error is returned, and the caller keeps using the stored state.
    pub fn refresh(
        &mut self,
        src: &dyn Source,
        now: OffsetDateTime,
    ) -> Result<Refresh, TrustError> {
        match self.refresh_inner(src, now) {
            Err(e) if is_offline(&e) => Ok(Refresh::Offline(self.freshness(now))),
            other => other,
        }
    }

    fn refresh_inner(
        &mut self,
        src: &dyn Source,
        now: OffsetDateTime,
    ) -> Result<Refresh, TrustError> {
        self.advance_keysets(src)?;
        let keyset = self.state.keyset.clone().ok_or(TrustError::NoKeyset)?;

        // Step 2: timestamp. Verify the signature over the raw bytes first;
        // the comment carries seq and key set version, parsing comes after.
        let ts_bytes = fetch_capped(src, "timestamp.json", MAX_TIMESTAMP)?;
        let ts_sig = sig_text(
            "timestamp.json.minisig",
            fetch_capped(src, "timestamp.json.minisig", MAX_SIG)?,
        )?;
        let comment = sig::trusted_comment(&ts_sig).map_err(TrustError::TimestampSignature)?;
        let (c_seq, c_kv) = sig::parse_seq_comment(&comment, "timestamp")
            .map_err(TrustError::TimestampSignature)?;
        sig::verify_role(&keyset.timestamp, &[ts_sig], &ts_bytes, &comment)
            .map_err(TrustError::TimestampSignature)?;
        let ts: Timestamp =
            serde_json::from_slice(&ts_bytes).map_err(|e| malformed("timestamp.json", e))?;
        if ts.kind != "deckpilot-registry-timestamp" || ts.spec_version != 1 {
            return Err(malformed("timestamp.json", "wrong _type or spec_version"));
        }
        if ts.seq != c_seq || ts.keys_version != c_kv {
            return Err(malformed(
                "timestamp.json",
                "content differs from signed comment",
            ));
        }
        if ts.keys_version != keyset.version {
            return Err(TrustError::KeysVersion {
                doc: "timestamp",
                found: ts.keys_version,
                current: keyset.version,
            });
        }
        if now >= ts.expires + CLOCK_SKEW {
            return Err(TrustError::TimestampExpired(ts.expires));
        }
        if ts.seq < self.state.seq {
            return Err(TrustError::Rollback {
                stored: self.state.seq,
                offered: ts.seq,
            });
        }
        if let Some(stored) = &self.state.timestamp
            && stored.seq == ts.seq
            && stored.index != ts.index
        {
            return Err(TrustError::Equivocation(ts.seq));
        }

        // Step 3: index, only if it changed.
        let cached = self
            .state
            .timestamp
            .as_ref()
            .zip(self.index.as_ref())
            .filter(|(stored, _)| stored.index == ts.index)
            .map(|(_, index)| index.clone());
        let (index, raw) = match cached {
            Some(index) => (index, None),
            None => {
                let (index, raw) = self.fetch_index(src, &ts, &keyset)?;
                (index, Some(raw))
            }
        };
        let changed = raw.is_some();
        if now >= index.expires + CLOCK_SKEW {
            return Err(TrustError::IndexExpired(index.expires));
        }
        for old in &self.state.revocations {
            if !index.revocations.contains(old) {
                return Err(TrustError::RevocationDropped(old.id.clone()));
            }
        }

        // Store index, seq and timestamp together.
        let seq = ts.seq;
        self.state.seq = seq;
        self.state.revocations = index.revocations.clone();
        // Cache first: a crash in between leaves a cache that no longer
        // matches the stored timestamp, which is then simply refetched.
        if let Some(raw) = raw {
            write_atomic(&cache_path(&self.state_path), &raw);
        }
        self.index = Some(index);
        self.state.timestamp = Some(ts);
        self.save();
        Ok(if changed {
            Refresh::Updated { seq }
        } else {
            Refresh::Unchanged { seq }
        })
    }

    /// Step 1: follow `keys/<n+1>.json` while it exists.
    fn advance_keysets(&mut self, src: &dyn Source) -> Result<(), TrustError> {
        loop {
            let n = self.keys_version() + 1;
            let path = format!("keys/{n}.json");
            let bytes = match fetch_capped(src, &path, MAX_KEYSET) {
                Err(TrustError::Fetch(_, FetchError::NotFound)) => return Ok(()),
                other => other?,
            };
            let sig_path = format!("{path}.minisig");
            let sigs_bytes = fetch_capped(src, &sig_path, MAX_KEYSET_SIGS)?;
            let sigs: Vec<String> =
                serde_json::from_slice(&sigs_bytes).map_err(|e| malformed(&sig_path, e))?;
            let comment = sig::keys_comment(n);
            let previous_root = self.root_role().clone();
            sig::verify_role(&previous_root, &sigs, &bytes, &comment)
                .map_err(|e| TrustError::KeysetSignature(n, e))?;
            let keyset: Keyset = serde_json::from_slice(&bytes).map_err(|e| malformed(&path, e))?;
            if keyset.kind != "deckpilot-registry-keys"
                || keyset.spec_version != 1
                || keyset.version != n
            {
                return Err(malformed(&path, "wrong _type, spec_version or version"));
            }
            if keyset.root != previous_root {
                // Root rotation: the new root role must sign too.
                sig::verify_role(&keyset.root, &sigs, &bytes, &comment)
                    .map_err(|e| TrustError::KeysetSignature(n, e))?;
            }
            for role in [&keyset.root, &keyset.index, &keyset.timestamp] {
                if role.threshold == 0 || role.threshold as usize > role.keys.len() {
                    return Err(malformed(&path, "role threshold out of range"));
                }
            }
            if keyset.seq_floor_reset {
                self.state.seq = keyset.seq_floor;
            } else {
                self.state.seq = self.state.seq.max(keyset.seq_floor);
            }
            self.state.keyset = Some(keyset);
            self.save();
        }
    }

    fn fetch_index(
        &self,
        src: &dyn Source,
        ts: &Timestamp,
        keyset: &Keyset,
    ) -> Result<(Index, Vec<u8>), TrustError> {
        let path = format!("index/{}.json", ts.seq);
        if ts.index.path != path {
            return Err(TrustError::IndexMismatch(format!(
                "path {} for seq {}",
                ts.index.path, ts.seq
            )));
        }
        let bytes = fetch_capped(src, &path, MAX_INDEX)?;
        check_ref(&bytes, &ts.index).map_err(TrustError::IndexMismatch)?;
        let sig_path = format!("{path}.minisig");
        let index_sig = sig_text(&sig_path, fetch_capped(src, &sig_path, MAX_SIG)?)?;
        let comment = sig::seq_comment("index", ts.seq, keyset.version);
        sig::verify_role(&keyset.index, &[index_sig], &bytes, &comment)
            .map_err(TrustError::IndexSignature)?;
        let index: Index = serde_json::from_slice(&bytes).map_err(|e| malformed(&path, e))?;
        if index.kind != "deckpilot-registry-index" || index.spec_version != 1 {
            return Err(malformed(&path, "wrong _type or spec_version"));
        }
        if index.seq != ts.seq {
            return Err(TrustError::IndexMismatch(format!(
                "seq {} != {}",
                index.seq, ts.seq
            )));
        }
        if index.keys_version != keyset.version {
            return Err(TrustError::KeysVersion {
                doc: "index",
                found: index.keys_version,
                current: keyset.version,
            });
        }
        if index.download_prefix != DOWNLOAD_PREFIX {
            return Err(TrustError::BadDownloadPrefix(index.download_prefix));
        }
        Ok((index, bytes))
    }

    pub fn can_install(&self, now: OffsetDateTime) -> Result<&Index, InstallError> {
        match self.freshness(now) {
            Freshness::NeverVerified => Err(InstallError::NeverVerified),
            Freshness::Stale(r) => Err(InstallError::Stale(r)),
            Freshness::Fresh => Ok(self.index.as_ref().expect("fresh implies index")),
        }
    }

    /// §9.3 step 4 and the install checks: returns what the package bytes
    /// must be. Refuses revoked and yanked versions.
    pub fn resolve_install(
        &self,
        src: &dyn Source,
        id: &str,
        version: &str,
        now: OffsetDateTime,
    ) -> Result<ExpectedPackage, InstallError> {
        let index = self.can_install(now)?;
        let entry = index
            .plugins
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| InstallError::UnknownPlugin(id.to_owned()))?;
        let label = format!("{id}@{version}");
        if let Some(r) = index.revocations.iter().find(|r| r.matches(id, version)) {
            return Err(match r.action {
                RevocationAction::Disable => InstallError::Revoked(label, r.reason.clone()),
                RevocationAction::Yank => InstallError::Yanked(label, r.reason.clone()),
            });
        }
        let expected_path = format!("plugins/{id}/{}.json", entry.detail.sha256);
        if entry.detail.path != expected_path {
            return Err(InstallError::Detail("path is not content-addressed".into()));
        }
        let bytes =
            fetch_capped(src, &entry.detail.path, MAX_DETAIL).map_err(InstallError::Trust)?;
        check_ref(&bytes, &entry.detail).map_err(InstallError::Detail)?;
        let detail: Detail =
            serde_json::from_slice(&bytes).map_err(|e| InstallError::Detail(e.to_string()))?;
        if detail.kind != "deckpilot-registry-plugin" || detail.id != id {
            return Err(InstallError::Detail("detail is for another plugin".into()));
        }
        let v = detail
            .versions
            .iter()
            .find(|v| v.version == version)
            .ok_or_else(|| InstallError::UnknownVersion(label.clone()))?;
        if let Some(y) = &v.yanked {
            return Err(InstallError::Yanked(label, y.reason.clone()));
        }
        if !v.package.url.starts_with(DOWNLOAD_PREFIX) {
            return Err(InstallError::Detail(
                "package URL outside the registry releases".into(),
            ));
        }
        let publisher = index
            .publishers
            .iter()
            .find(|p| p.namespace == entry.publisher)
            .and_then(|p| p.signing_key.clone())
            .map(|key| (key, v.publisher_signature.clone()));
        Ok(ExpectedPackage {
            id: id.to_owned(),
            version: version.to_owned(),
            url: v.package.url.clone(),
            sha256: v.package.sha256.clone(),
            size: v.package.size,
            manifest_sha256: v.manifest_sha256.clone(),
            publisher,
        })
    }

    /// At every start, online or not (§13.5).
    pub fn installed_verdict(&self, id: &str, version: &str) -> InstalledVerdict {
        let matching = || {
            self.state
                .revocations
                .iter()
                .filter(|r| r.matches(id, version))
        };
        if let Some(r) = matching().find(|r| r.action == RevocationAction::Disable) {
            return InstalledVerdict::Disable {
                reason: r.reason.clone(),
                advisory: r.advisory.clone(),
            };
        }
        if let Some(r) = matching().next() {
            return InstalledVerdict::RunWithNotice {
                reason: r.reason.clone(),
            };
        }
        InstalledVerdict::Run
    }
}

fn check_ref(bytes: &[u8], r: &FileRef) -> Result<(), String> {
    if bytes.len() as u64 != r.length {
        return Err(format!("length {} != {}", bytes.len(), r.length));
    }
    let hash = sha256_hex(bytes);
    if hash != r.sha256 {
        return Err(format!("sha256 {hash} != {}", r.sha256));
    }
    Ok(())
}

fn cache_path(state_path: &Path) -> PathBuf {
    state_path.with_extension("index-cache.json")
}

fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).expect("write");
    std::fs::rename(&tmp, path).expect("rename");
}
