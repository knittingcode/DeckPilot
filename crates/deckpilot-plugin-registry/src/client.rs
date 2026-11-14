//! The client verifier (architecture sections 9.3, 10, 12.3 steps 1-2,
//! 13.5 and 14).

use std::fmt;

use chrono::{DateTime, TimeDelta, Utc};
use deckpilot_plugin_manifest::{HostEnvironment, Incompatibility, PackageDigest, PluginId};
use deckpilot_plugin_package::{ExpectedPackage, sha256};
use semver::Version;

use crate::error::{FetchError, InstallError, RegistryError};
use crate::model::{
    DOWNLOAD_PREFIX, Deprecated, FileRef, Index, Keyset, MAX_DETAIL_BYTES, MAX_INDEX_BYTES,
    PluginDetail, RevocationAction, Role, Timestamp,
};
use crate::sig::{self, index_comment, keys_comment, parse_timestamp_comment, same_role};
use crate::state::{STATE_FORMAT, TrustState, TrustStore};

/// Largest `keys/<n>.json`.
pub const MAX_KEYSET_BYTES: usize = 16 * 1024;
/// Largest `keys/<n>.json.minisig` (a JSON array of signatures).
pub const MAX_KEYSET_SIGNATURES_BYTES: usize = 16 * 1024;
/// Most signatures in `keys/<n>.json.minisig`.
pub const MAX_KEYSET_SIGNATURES: usize = 8;
/// Largest `timestamp.json`.
pub const MAX_TIMESTAMP_BYTES: usize = 4 * 1024;
/// Largest single `.minisig` file.
pub const MAX_SIGNATURE_BYTES: usize = 2 * 1024;

/// Tunable checks. [`Policy::default`] is what the product should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Allowance for a wrong local clock on expiry checks (section 9.3: 5
    /// minutes).
    pub clock_skew: TimeDelta,
    /// Longest `expires - signed_at` accepted for a timestamp. #191 signs
    /// for 7 days; 14 leaves room for a slower schedule while bounding the
    /// freeze window a stolen timestamp key can create.
    pub max_timestamp_validity: TimeDelta,
    /// Whether `index.expires` makes the registry stale. Off by default:
    /// freshness comes from the timestamp, which is re-signed daily, while
    /// nothing re-signs the index between publishes (spike finding 1). On,
    /// a registry with no publish for 90 days would block every install.
    pub enforce_index_expiry: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            clock_skew: TimeDelta::minutes(5),
            max_timestamp_validity: TimeDelta::days(14),
            enforce_index_expiry: false,
        }
    }
}

/// Why the stored state may not be used for installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    /// More than `expires + skew` since the newest verified timestamp.
    TimestampExpired,
    /// `index.expires` passed ([`Policy::enforce_index_expiry`] only).
    IndexExpired,
    /// The timestamp is verified but its index is not cached (the cache
    /// was lost or did not match). Revocations are still enforced.
    IndexUnavailable,
}

impl fmt::Display for StaleReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TimestampExpired => "timestamp expired",
            Self::IndexExpired => "index expired",
            Self::IndexUnavailable => "index not available",
        })
    }
}

/// How usable the stored state is right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    NeverVerified,
    Fresh,
    Stale(StaleReason),
}

/// The result of a refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    /// A new index (or the first one) was verified and stored.
    Updated { seq: u64 },
    /// Same index; possibly a re-signed timestamp with a later expiry.
    Unchanged { seq: u64 },
    /// The registry could not be reached; the stored state is used as is.
    Offline(Freshness),
}

/// What to do with an installed version (section 13.5). Computed from the
/// stored revocations, so it also holds offline and when stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledVerdict {
    Run,
    /// Yanked: keeps running, with a notice in Settings.
    RunWithNotice {
        reason: String,
    },
    /// Disabled: stop the host at once and refuse to load.
    Disable {
        reason: String,
        advisory: Option<String>,
    },
}

/// Everything the installer needs after consent: where to download, what
/// the bytes must be, and what to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallTarget {
    pub url: String,
    pub expected: ExpectedPackage,
    /// The detail's permissions, for the consent dialog (also inside
    /// `expected`, compared with the manifest after download).
    pub permissions: Vec<serde_json::Value>,
    pub deprecated: Option<Deprecated>,
}

/// A source of registry documents by path (`timestamp.json`,
/// `index/<seq>.json`…). This crate has no network code; the agent's
/// fetcher implements this. `max_bytes` lets it stop a download early; the
/// client checks the length again.
pub trait RegistrySource {
    fn fetch(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError>;
}

/// A timestamp whose signature and content were verified against the
/// current key set and state. Commit it with [`RegistryClient::commit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedTimestamp {
    timestamp: Timestamp,
}

impl VerifiedTimestamp {
    pub fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }

    pub fn index_ref(&self) -> &FileRef {
        &self.timestamp.index
    }
}

/// The client side of the registry trust model.
#[derive(Debug)]
pub struct RegistryClient<S: TrustStore> {
    roots: Role,
    policy: Policy,
    store: S,
    state: TrustState,
    /// The verified index matching `state.timestamp` (a cache, not trust
    /// state).
    index: Option<Index>,
}

fn too_large(document: &str, limit: usize) -> RegistryError {
    RegistryError::TooLarge {
        document: document.to_owned(),
        limit,
    }
}

fn malformed(document: &str, message: impl fmt::Display) -> RegistryError {
    RegistryError::Malformed {
        document: document.to_owned(),
        message: message.to_string(),
    }
}

fn capped<'a>(document: &str, bytes: &'a [u8], limit: usize) -> Result<&'a [u8], RegistryError> {
    if bytes.len() > limit {
        return Err(too_large(document, limit));
    }
    Ok(bytes)
}

fn signature_text(document: &str, bytes: &[u8]) -> Result<String, RegistryError> {
    let bytes = capped(document, bytes, MAX_SIGNATURE_BYTES)?;
    std::str::from_utf8(bytes)
        .map(|text| text.trim().to_owned())
        .map_err(|e| malformed(document, e))
}

/// Length then sha256, before anything parses the bytes.
fn check_ref(bytes: &[u8], reference: &FileRef) -> Result<(), String> {
    let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if length != reference.length {
        return Err(format!("length {length}, pinned {}", reference.length));
    }
    let digest = sha256(bytes);
    if digest != reference.sha256 {
        return Err(format!("sha256 {digest}, pinned {}", reference.sha256));
    }
    Ok(())
}

/// A stable fingerprint of a role: threshold and sorted key identities.
fn fingerprint(role: &Role) -> String {
    let mut text = format!("threshold:{}", role.threshold).into_bytes();
    for id in role.identities() {
        text.push(b':');
        text.extend_from_slice(&id);
    }
    sha256(&text).to_string()
}

impl<S: TrustStore> RegistryClient<S> {
    /// Opens the client with the root keys compiled into the app
    /// (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`) and the stored state.
    ///
    /// A missing state starts from the embedded roots (trust on first
    /// use); a corrupt state is an error, never silently reset. If the
    /// embedded roots changed (an app release replaced compromised roots,
    /// section 9.4), the stored key set chain is dropped and re-walked from
    /// `keys/1.json` under the new roots; the rollback floor and the
    /// revocations are kept.
    pub fn open(roots: Role, store: S, policy: Policy) -> Result<Self, RegistryError> {
        roots.validate().map_err(RegistryError::InvalidRoots)?;
        let roots_id = fingerprint(&roots);
        let loaded = store.load()?;
        let mut state = loaded
            .clone()
            .unwrap_or_else(|| TrustState::new(roots_id.clone()));
        if state.format != STATE_FORMAT {
            return Err(crate::state::StoreError::Corrupt(format!(
                "unknown state format {}",
                state.format
            ))
            .into());
        }
        if state.roots != roots_id {
            state.roots.clone_from(&roots_id);
            state.keyset = None;
            state.timestamp = None;
        }
        let mut client = Self {
            roots,
            policy,
            store,
            state,
            index: None,
        };
        if loaded.as_ref() != Some(&client.state) && loaded.is_some() {
            let state = client.state.clone();
            client.store.save(&state)?;
        }
        client.index = client.load_cached_index();
        Ok(client)
    }

    /// The cached index, only if it is exactly the one the stored timestamp
    /// pins (and still parses and validates).
    fn load_cached_index(&self) -> Option<Index> {
        let timestamp = self.state.timestamp.as_ref()?;
        let bytes = self.store.load_index().ok()??;
        check_ref(&bytes, &timestamp.index).ok()?;
        let index: Index = serde_json::from_slice(&bytes).ok()?;
        index.validate().ok()?;
        (index.seq == timestamp.seq).then_some(index)
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// The verified state (read only).
    pub fn state(&self) -> &TrustState {
        &self.state
    }

    /// The store, e.g. to inspect it in tests.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// The verified index, whatever its freshness (browsing works from the
    /// cache, section 14).
    pub fn index(&self) -> Option<&Index> {
        self.index.as_ref()
    }

    /// Version of the newest verified key set (0: none yet).
    pub fn keys_version(&self) -> u64 {
        self.state.keyset.as_ref().map_or(0, |k| k.version)
    }

    fn root_role(&self) -> &Role {
        self.state.keyset.as_ref().map_or(&self.roots, |k| &k.root)
    }

    fn effective_now(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        self.state.clock_floor.map_or(now, |floor| floor.max(now))
    }

    /// Whether installs and updates may use the stored state now.
    pub fn freshness(&self, now: DateTime<Utc>) -> Freshness {
        let Some(timestamp) = &self.state.timestamp else {
            return Freshness::NeverVerified;
        };
        let now = self.effective_now(now);
        if now >= timestamp.expires + self.policy.clock_skew {
            return Freshness::Stale(StaleReason::TimestampExpired);
        }
        let Some(index) = &self.index else {
            return Freshness::Stale(StaleReason::IndexUnavailable);
        };
        if self.policy.enforce_index_expiry && now >= index.expires + self.policy.clock_skew {
            return Freshness::Stale(StaleReason::IndexExpired);
        }
        Freshness::Fresh
    }

    /// Saves `state` (and the index bytes first, if any), then adopts it.
    /// On a store error nothing changes in memory.
    fn persist(
        &mut self,
        state: TrustState,
        index: Option<(Index, &[u8])>,
    ) -> Result<(), RegistryError> {
        if let Some((_, bytes)) = &index {
            // Index first: a crash in between leaves a cache that does not
            // match the stored timestamp, which is simply fetched again.
            self.store.save_index(bytes)?;
        }
        self.store.save(&state)?;
        self.state = state;
        if let Some((index, _)) = index {
            self.index = Some(index);
        }
        Ok(())
    }

    // ------------------------------------------------------- step 1: keys

    /// Verifies `keys/<n>.json` for `n` = current version + 1 and stores it
    /// (section 9.3 step 1). It needs a threshold of signatures from the
    /// current root role and, if the root role changes, also from the new
    /// one.
    pub fn apply_keyset(&mut self, bytes: &[u8], signatures: &[u8]) -> Result<u64, RegistryError> {
        let version = self.keys_version() + 1;
        let document = format!("keys/{version}.json");
        let signatures_document = format!("{document}.minisig");
        capped(&document, bytes, MAX_KEYSET_BYTES)?;
        capped(
            &signatures_document,
            signatures,
            MAX_KEYSET_SIGNATURES_BYTES,
        )?;
        let signatures: Vec<String> =
            serde_json::from_slice(signatures).map_err(|e| malformed(&signatures_document, e))?;
        if signatures.len() > MAX_KEYSET_SIGNATURES
            || signatures.iter().any(|s| s.len() > MAX_SIGNATURE_BYTES)
        {
            return Err(too_large(&signatures_document, MAX_KEYSET_SIGNATURES_BYTES));
        }
        let comment = keys_comment(version);
        let previous_root = self.root_role().clone();
        sig::verify_role(&previous_root, &signatures, bytes, &comment)
            .map_err(|error| RegistryError::KeysetSignature { version, error })?;
        let keyset: Keyset = serde_json::from_slice(bytes).map_err(|e| malformed(&document, e))?;
        keyset.validate().map_err(|e| malformed(&document, e))?;
        if keyset.version != version {
            return Err(malformed(
                &document,
                format!("version {} is not {version}", keyset.version),
            ));
        }
        if !same_role(&keyset.root, &previous_root) {
            // Root rotation: the new root role must agree too.
            sig::verify_role(&keyset.root, &signatures, bytes, &comment)
                .map_err(|error| RegistryError::KeysetSignature { version, error })?;
        }
        let mut state = self.state.clone();
        let reset = keyset.seq_floor_reset;
        if reset {
            // Index-key compromise recovery (section 9.4): the seq the
            // attacker inflated is replaced, the timestamp of that era is
            // dropped, and the next index may lift revocations.
            state.seq = keyset.seq_floor;
            state.timestamp = None;
            state.revocations_resettable = true;
        } else {
            state.seq = state.seq.max(keyset.seq_floor);
        }
        state.keyset = Some(keyset);
        self.persist(state, None)?;
        if reset {
            self.index = None;
        }
        Ok(version)
    }

    // -------------------------------------------------- step 2: timestamp

    /// Verifies `timestamp.json` (section 9.3 step 2) without storing it.
    ///
    /// The signature is checked over the raw bytes first, with the exact
    /// comment `role:timestamp seq:<s> keys:<kv>` (read from the signature,
    /// then required). Then: content equal to the comment, current key set,
    /// bounded validity, unexpired (with skew, against the clock floor),
    /// seq at least the stored floor, and at an equal seq the same index.
    pub fn verify_timestamp(
        &self,
        bytes: &[u8],
        signature: &[u8],
        now: DateTime<Utc>,
    ) -> Result<VerifiedTimestamp, RegistryError> {
        let keyset = self.state.keyset.as_ref().ok_or(RegistryError::NoKeyset)?;
        capped("timestamp.json", bytes, MAX_TIMESTAMP_BYTES)?;
        let signature = signature_text("timestamp.json.minisig", signature)?;
        let comment =
            sig::trusted_comment(&signature).map_err(RegistryError::TimestampSignature)?;
        let (seq, keys_version) =
            parse_timestamp_comment(&comment).map_err(RegistryError::TimestampSignature)?;
        sig::verify_role(&keyset.timestamp, &[signature], bytes, &comment)
            .map_err(RegistryError::TimestampSignature)?;
        let timestamp: Timestamp =
            serde_json::from_slice(bytes).map_err(|e| malformed("timestamp.json", e))?;
        timestamp
            .validate()
            .map_err(|e| malformed("timestamp.json", e))?;
        if timestamp.seq != seq || timestamp.keys_version != keys_version {
            return Err(malformed(
                "timestamp.json",
                "seq or keys_version differs from the signed comment",
            ));
        }
        self.check_timestamp(&timestamp, now)?;
        Ok(VerifiedTimestamp { timestamp })
    }

    fn check_timestamp(
        &self,
        timestamp: &Timestamp,
        now: DateTime<Utc>,
    ) -> Result<(), RegistryError> {
        let current = self.keys_version();
        if timestamp.keys_version != current {
            return Err(RegistryError::KeysVersion {
                document: "timestamp",
                found: timestamp.keys_version,
                current,
            });
        }
        if timestamp.expires - timestamp.signed_at > self.policy.max_timestamp_validity {
            return Err(RegistryError::TimestampValidity {
                signed_at: timestamp.signed_at,
                expires: timestamp.expires,
            });
        }
        if self.effective_now(now) >= timestamp.expires + self.policy.clock_skew {
            return Err(RegistryError::TimestampExpired {
                expires: timestamp.expires,
            });
        }
        if timestamp.seq < self.state.seq {
            return Err(RegistryError::Rollback {
                stored: self.state.seq,
                offered: timestamp.seq,
            });
        }
        if let Some(stored) = &self.state.timestamp
            && stored.seq == timestamp.seq
            && stored.index != timestamp.index
        {
            return Err(RegistryError::Equivocation { seq: timestamp.seq });
        }
        Ok(())
    }

    /// Whether committing `timestamp` needs its index (it changed, or the
    /// cache is missing).
    pub fn needs_index(&self, timestamp: &VerifiedTimestamp) -> bool {
        self.index.is_none()
            || self
                .state
                .timestamp
                .as_ref()
                .is_none_or(|stored| stored.index != timestamp.timestamp.index)
    }

    // ------------------------------------------------------ step 3: index

    /// Stores a verified timestamp, with its index if
    /// [`RegistryClient::needs_index`] (section 9.3 step 3): index, seq,
    /// revocations and timestamp are saved together.
    ///
    /// Index checks: path `index/<seq>.json`, length and sha256 against the
    /// timestamp before parsing, the index role with the exact comment, seq
    /// and key set equal to the timestamp's, the fixed download prefix,
    /// and cumulative revocations.
    pub fn commit(
        &mut self,
        timestamp: VerifiedTimestamp,
        index: Option<(&[u8], &[u8])>,
        now: DateTime<Utc>,
    ) -> Result<Refresh, RegistryError> {
        let timestamp = timestamp.timestamp;
        // The state may have moved since the timestamp was verified.
        self.check_timestamp(&timestamp, now)?;
        let fetched = match index {
            Some((bytes, signature)) => {
                Some((self.verify_index(bytes, signature, &timestamp, now)?, bytes))
            }
            None if self.needs_index(&VerifiedTimestamp {
                timestamp: timestamp.clone(),
            }) =>
            {
                return Err(RegistryError::IndexRequired);
            }
            None => None,
        };
        let mut state = self.state.clone();
        if let Some((index, _)) = &fetched {
            if !state.revocations_resettable
                && let Some(dropped) = state
                    .revocations
                    .iter()
                    .find(|old| !old.is_covered_by(&index.revocations))
            {
                return Err(RegistryError::RevocationDropped {
                    id: dropped.id.clone(),
                });
            }
            state.revocations.clone_from(&index.revocations);
            state.revocations_resettable = false;
        }
        let seq = timestamp.seq;
        state.seq = seq;
        let floor = timestamp
            .signed_at
            .max(now.min(timestamp.expires + self.policy.clock_skew));
        state.clock_floor = Some(state.clock_floor.map_or(floor, |f| f.max(floor)));
        // A replayed older signature of the same seq must not shorten the
        // stored expiry.
        let older = state.timestamp.as_ref().is_some_and(|stored| {
            stored.seq == timestamp.seq && stored.signed_at > timestamp.signed_at
        });
        if !older {
            state.timestamp = Some(timestamp);
        }
        let changed = fetched.is_some();
        self.persist(state, fetched)?;
        Ok(if changed {
            Refresh::Updated { seq }
        } else {
            Refresh::Unchanged { seq }
        })
    }

    fn verify_index(
        &self,
        bytes: &[u8],
        signature: &[u8],
        timestamp: &Timestamp,
        now: DateTime<Utc>,
    ) -> Result<Index, RegistryError> {
        let keyset = self.state.keyset.as_ref().ok_or(RegistryError::NoKeyset)?;
        let path = format!("index/{}.json", timestamp.seq);
        if timestamp.index.path != path {
            return Err(RegistryError::IndexMismatch(format!(
                "path {} for seq {}",
                timestamp.index.path, timestamp.seq
            )));
        }
        let limit = usize::try_from(MAX_INDEX_BYTES).unwrap_or(usize::MAX);
        capped(&path, bytes, limit)?;
        check_ref(bytes, &timestamp.index).map_err(RegistryError::IndexMismatch)?;
        let signature = signature_text(&format!("{path}.minisig"), signature)?;
        let comment = index_comment(timestamp.seq, keyset.version);
        sig::verify_role(&keyset.index, &[signature], bytes, &comment)
            .map_err(RegistryError::IndexSignature)?;
        let index: Index = serde_json::from_slice(bytes).map_err(|e| malformed(&path, e))?;
        if index.download_prefix != DOWNLOAD_PREFIX {
            return Err(RegistryError::BadDownloadPrefix(index.download_prefix));
        }
        index.validate().map_err(|e| malformed(&path, e))?;
        if index.seq != timestamp.seq {
            return Err(RegistryError::IndexMismatch(format!(
                "seq {} for timestamp seq {}",
                index.seq, timestamp.seq
            )));
        }
        if index.keys_version != keyset.version {
            return Err(RegistryError::KeysVersion {
                document: "index",
                found: index.keys_version,
                current: keyset.version,
            });
        }
        if self.policy.enforce_index_expiry
            && self.effective_now(now) >= index.expires + self.policy.clock_skew
        {
            return Err(RegistryError::IndexExpired {
                expires: index.expires,
            });
        }
        Ok(index)
    }

    // ---------------------------------------------------- driving it all

    /// One complete check (section 9.3 steps 1 to 3) from `source`.
    ///
    /// A source that is unavailable anywhere gives [`Refresh::Offline`] with
    /// the stored state's freshness. A security error is returned and
    /// changes nothing (a key set verified earlier in the same refresh
    /// stays stored); the caller keeps using the stored state, whose
    /// freshness decays normally.
    pub fn refresh(
        &mut self,
        source: &dyn RegistrySource,
        now: DateTime<Utc>,
    ) -> Result<Refresh, RegistryError> {
        let result = self.refresh_inner(source, now);
        self.note_clock(now)?;
        match result {
            Err(RegistryError::Fetch {
                error: FetchError::Unavailable(_),
                ..
            }) => Ok(Refresh::Offline(self.freshness(now))),
            other => other,
        }
    }

    fn refresh_inner(
        &mut self,
        source: &dyn RegistrySource,
        now: DateTime<Utc>,
    ) -> Result<Refresh, RegistryError> {
        let fetch = |path: &str, limit: usize| {
            source
                .fetch(path, limit)
                .map_err(|error| RegistryError::Fetch {
                    path: path.to_owned(),
                    error,
                })
        };
        loop {
            let path = format!("keys/{}.json", self.keys_version() + 1);
            let bytes = match fetch(&path, MAX_KEYSET_BYTES) {
                Err(RegistryError::Fetch {
                    error: FetchError::NotFound,
                    ..
                }) => break,
                other => other?,
            };
            let signatures = fetch(&format!("{path}.minisig"), MAX_KEYSET_SIGNATURES_BYTES)?;
            self.apply_keyset(&bytes, &signatures)?;
        }
        if self.state.keyset.is_none() {
            return Err(RegistryError::NoKeyset);
        }
        let bytes = fetch("timestamp.json", MAX_TIMESTAMP_BYTES)?;
        let signature = fetch("timestamp.json.minisig", MAX_SIGNATURE_BYTES)?;
        let timestamp = self.verify_timestamp(&bytes, &signature, now)?;
        if self.needs_index(&timestamp) {
            let path = timestamp.index_ref().path.clone();
            let limit = usize::try_from(MAX_INDEX_BYTES).unwrap_or(usize::MAX);
            let index = fetch(&path, limit)?;
            let signature = fetch(&format!("{path}.minisig"), MAX_SIGNATURE_BYTES)?;
            self.commit(timestamp, Some((&index, &signature)), now)
        } else {
            self.commit(timestamp, None, now)
        }
    }

    /// Records that `now` has passed (capped at the stored timestamp's
    /// expiry plus skew, so a clock far in the future cannot block the
    /// client beyond the next successful refresh). Called by
    /// [`RegistryClient::refresh`], online or not.
    pub fn note_clock(&mut self, now: DateTime<Utc>) -> Result<(), RegistryError> {
        let Some(timestamp) = &self.state.timestamp else {
            return Ok(());
        };
        let candidate = now.min(timestamp.expires + self.policy.clock_skew);
        if self
            .state
            .clock_floor
            .is_some_and(|floor| floor >= candidate)
        {
            return Ok(());
        }
        let mut state = self.state.clone();
        state.clock_floor = Some(candidate);
        self.persist(state, None)
    }

    // ------------------------------------------------- step 4: detail

    /// The pinned reference of plugin `id`'s detail document.
    pub fn detail_ref(&self, id: &PluginId) -> Option<&FileRef> {
        self.index.as_ref()?.plugin(id).map(|p| &p.detail)
    }

    /// Verifies a detail document against the cached index (section 9.3
    /// step 4): length and sha256 before parsing, then content. Works
    /// whatever the freshness (browsing).
    pub fn verify_detail(
        &self,
        id: &PluginId,
        bytes: &[u8],
    ) -> Result<PluginDetail, RegistryError> {
        let index = self.index.as_ref().ok_or(RegistryError::NoIndex)?;
        let entry = index
            .plugin(id)
            .ok_or_else(|| RegistryError::UnknownPlugin(id.clone()))?;
        let document = entry.detail.path.as_str();
        let limit = usize::try_from(MAX_DETAIL_BYTES).unwrap_or(usize::MAX);
        capped(document, bytes, limit)?;
        check_ref(bytes, &entry.detail).map_err(RegistryError::DetailMismatch)?;
        let detail: PluginDetail =
            serde_json::from_slice(bytes).map_err(|e| malformed(document, e))?;
        detail.validate().map_err(|e| malformed(document, e))?;
        if detail.id != *id || detail.publisher != entry.publisher {
            return Err(RegistryError::DetailMismatch(format!(
                "the detail is for {}",
                detail.id
            )));
        }
        for (channel, latest) in [
            ("stable", &entry.latest.stable),
            ("prerelease", &entry.latest.prerelease),
        ] {
            if let Some(latest) = latest
                && detail.version(latest).is_none_or(|v| v.yanked.is_some())
            {
                return Err(RegistryError::DetailMismatch(format!(
                    "latest.{channel} {latest} is not a published, non-yanked version"
                )));
            }
        }
        Ok(detail)
    }

    /// Whether installs and updates are possible now; the verified index.
    pub fn can_install(&self, now: DateTime<Utc>) -> Result<&Index, InstallError> {
        match self.freshness(now) {
            Freshness::NeverVerified => Err(InstallError::NeverVerified),
            Freshness::Stale(reason) => Err(InstallError::Stale(reason)),
            Freshness::Fresh => self.index.as_ref().ok_or(InstallError::NeverVerified),
        }
    }

    /// Eligibility of `id` `version` for install or update (section 12.3
    /// steps 1 and 2): fresh registry, app new enough for the index, not
    /// revoked or yanked, compatible with `host` (if given), and the pins
    /// the downloaded package must match.
    pub fn resolve_install(
        &self,
        id: &PluginId,
        version: &Version,
        detail: &[u8],
        host: Option<&HostEnvironment>,
        now: DateTime<Utc>,
    ) -> Result<InstallTarget, InstallError> {
        let index = self.can_install(now)?;
        if let (Some(required), Some(host)) = (&index.min_app_version, host)
            && host.app_version < *required
        {
            return Err(InstallError::AppTooOld {
                required: required.clone(),
                actual: host.app_version.clone(),
            });
        }
        let entry = index
            .plugin(id)
            .ok_or_else(|| InstallError::UnknownPlugin(id.clone()))?;
        let mut matching = index.revocations.iter().filter(|r| r.matches(id, version));
        if let Some(revocation) = matching
            .clone()
            .find(|r| r.action == RevocationAction::Disable)
        {
            return Err(InstallError::Revoked {
                id: id.clone(),
                version: version.clone(),
                reason: revocation.reason.clone(),
                advisory: revocation.advisory.clone(),
            });
        }
        if let Some(revocation) = matching.next() {
            return Err(InstallError::Yanked {
                id: id.clone(),
                version: version.clone(),
                reason: revocation.reason.clone(),
            });
        }
        let detail = self.verify_detail(id, detail)?;
        let entry_version =
            detail
                .version(version)
                .ok_or_else(|| InstallError::UnknownVersion {
                    id: id.clone(),
                    version: version.clone(),
                })?;
        if let Some(yanked) = &entry_version.yanked {
            return Err(InstallError::Yanked {
                id: id.clone(),
                version: version.clone(),
                reason: yanked.reason.clone(),
            });
        }
        if let Some(host) = host {
            let mut reasons = Vec::new();
            if !entry_version.engines.deckpilot.matches(&host.app_version) {
                reasons.push(Incompatibility::AppVersion {
                    required: entry_version.engines.deckpilot.clone(),
                    actual: host.app_version.clone(),
                });
            }
            if !entry_version
                .engines
                .plugin_api
                .is_satisfied_by(host.plugin_api)
            {
                reasons.push(Incompatibility::PluginApi {
                    required: entry_version.engines.plugin_api,
                    actual: host.plugin_api,
                });
            }
            if let Some(platforms) = &entry_version.platforms
                && !platforms.contains(&host.platform)
            {
                reasons.push(Incompatibility::Platform {
                    supported: platforms.clone(),
                    actual: host.platform,
                });
            }
            if !reasons.is_empty() {
                return Err(InstallError::Incompatible {
                    id: id.clone(),
                    version: version.clone(),
                    reasons,
                });
            }
        }
        Ok(InstallTarget {
            url: entry_version.package.url.clone(),
            expected: ExpectedPackage {
                id: id.clone(),
                version: version.clone(),
                package: PackageDigest {
                    sha256: entry_version.package.sha256,
                    size: entry_version.package.size,
                },
                manifest_sha256: entry_version.manifest_sha256,
                permissions: Some(entry_version.permissions.clone()),
            },
            permissions: entry_version.permissions.clone(),
            deprecated: entry.deprecated.clone(),
        })
    }

    /// What to do with an installed version, from the stored revocations
    /// (section 13.5): at every start, online or not, stale or not.
    pub fn installed_verdict(&self, id: &PluginId, version: &Version) -> InstalledVerdict {
        let mut matching = self
            .state
            .revocations
            .iter()
            .filter(|r| r.matches(id, version));
        if let Some(r) = matching
            .clone()
            .find(|r| r.action == RevocationAction::Disable)
        {
            return InstalledVerdict::Disable {
                reason: r.reason.clone(),
                advisory: r.advisory.clone(),
            };
        }
        match matching.next() {
            Some(r) => InstalledVerdict::RunWithNotice {
                reason: r.reason.clone(),
            },
            None => InstalledVerdict::Run,
        }
    }
}
