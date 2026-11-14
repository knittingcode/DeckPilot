//! `index/<seq>.json`, its signature, and `timestamp.json` (architecture
//! sections 9.2, 10, 11.5 and 11.6).

use std::collections::BTreeSet;

use chrono::{DateTime, TimeDelta, Utc};
use deckpilot_plugin_package::sha256;
use semver::Version;

use super::catalog::{self, StoredPlugin};
use super::keys::SigningKey;
use super::keyset;
use super::layout::{
    Workspace, read, read_json, read_optional, remove, sig_path, write, write_immutable,
};
use super::{PublishError, Result, fail, to_json};
use crate::model::{
    DOWNLOAD_PREFIX, FileRef, Index, IndexPlugin, IndexType, Latest, MAX_INDEX_BYTES, Revocation,
    SPEC_VERSION, Timestamp, TimestampType,
};
use crate::sig::{index_comment, timestamp_comment, verify_role};

/// #191: `index.expires` is `generated_at` + 90 days (informational with
/// the default client policy; see the crate README).
pub const INDEX_VALIDITY_DAYS: i64 = 90;
/// Default timestamp validity (#191 signs for 7 days, re-signed daily).
pub const DEFAULT_TIMESTAMP_VALIDITY_DAYS: i64 = 7;
/// The client refuses longer (`Policy::max_timestamp_validity`).
pub const MAX_TIMESTAMP_VALIDITY_DAYS: i64 = 14;
/// Previous snapshots kept on the site (section 10).
pub const DEFAULT_KEEP: usize = 10;

/// Options of `build-index`.
#[derive(Debug, Clone)]
pub(crate) struct BuildIndex {
    pub source_commit: String,
    pub generated_at: DateTime<Utc>,
    pub min_app_version: Option<Version>,
    pub keep: usize,
}

fn is_revoked(revocations: &[Revocation], plugin: &StoredPlugin, version: &Version) -> bool {
    revocations
        .iter()
        .any(|r| r.matches(&plugin.info.id, version))
}

/// `latest`: the highest stable and the highest newer pre-release that are
/// neither yanked nor revoked. `None` when nothing can be installed.
fn latest(plugin: &StoredPlugin, revocations: &[Revocation]) -> Option<Latest> {
    let offered = || {
        plugin
            .versions
            .iter()
            .filter(|v| v.yanked.is_none() && !is_revoked(revocations, plugin, &v.version))
            .map(|v| &v.version)
    };
    let stable = offered().filter(|v| v.pre.is_empty()).max().cloned();
    let prerelease = offered()
        .filter(|v| !v.pre.is_empty() && stable.as_ref().is_none_or(|s| *v > s))
        .max()
        .cloned();
    (stable.is_some() || prerelease.is_some()).then_some(Latest { stable, prerelease })
}

/// Index of `seq` if it exists.
fn load_index(ws: &Workspace, seq: u64) -> Result<Index> {
    read_json(&ws.index_path(seq))
}

/// `build-index`: writes the details and an unsigned `index/<seq>.json`
/// with `seq` = published seq + 1 (at least the key set's floor). Returns
/// the seq.
#[allow(clippy::too_many_lines)] // checks, details, index, prune
pub(crate) fn build_index(ws: &Workspace, options: &BuildIndex) -> Result<u64> {
    let keyset = keyset::current(ws)?;
    let seqs = ws.index_seqs()?;
    if let Some(last) = seqs.last()
        && !sig_path(&ws.index_path(*last)).is_file()
    {
        return fail(format!(
            "index/{last}.json is not signed yet; sign it (`sign-index`) or delete it"
        ));
    }
    let previous = match seqs.last() {
        Some(seq) => Some(load_index(ws, *seq)?),
        None => None,
    };
    let seq = (keyset::published_seq(ws)? + 1).max(keyset.seq_floor);
    if options.source_commit.len() != 40
        || !options
            .source_commit
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return fail("--source-commit must be a 40-digit lowercase commit hash");
    }

    let revocations = catalog::revocations(ws)?;
    if let Some(previous) = &previous
        && let Some(dropped) = previous
            .revocations
            .iter()
            .find(|old| !old.is_covered_by(&revocations))
        && !catalog::lifting_allowed(ws)?
    {
        // The client would refuse this index (RevocationDropped).
        return fail(format!(
            "registry/revocations.json drops or weakens the published revocation of {} \
             (revocations are cumulative; see `lift-revocation`)",
            dropped.id
        ));
    }

    let mut plugins = Vec::new();
    let mut details = Vec::new();
    for plugin in catalog::plugins(ws)?.into_values() {
        let Some(latest) = latest(&plugin, &revocations) else {
            // Nothing installable: not listed. Its revocations stay.
            continue;
        };
        let detail = to_json(&plugin.detail()?)?;
        let length = u64::try_from(detail.len()).unwrap_or(u64::MAX);
        if length > crate::MAX_DETAIL_BYTES {
            return fail(format!("the detail of {} exceeds 256 KiB", plugin.info.id));
        }
        let digest = sha256(&detail);
        let path = format!("plugins/{}/{digest}.json", plugin.info.id);
        plugins.push(IndexPlugin {
            id: plugin.info.id.clone(),
            publisher: plugin.info.id.publisher().to_owned(),
            name: plugin.info.name.clone(),
            summary: plugin.info.summary.clone(),
            categories: plugin.info.categories.clone(),
            latest,
            deprecated: plugin.info.deprecated.clone(),
            detail: FileRef {
                path: path.clone(),
                sha256: digest,
                length,
            },
        });
        details.push((path, detail));
    }
    let publishers = catalog::publishers(ws)?;
    for plugin in &plugins {
        if !publishers.iter().any(|p| p.namespace == plugin.publisher) {
            return fail(format!("no publisher owns {}", plugin.id));
        }
    }
    let index = Index {
        kind: IndexType::Value,
        spec_version: SPEC_VERSION,
        seq,
        keys_version: keyset.version,
        generated_at: options.generated_at,
        expires: options.generated_at + TimeDelta::days(INDEX_VALIDITY_DAYS),
        source_commit: options.source_commit.clone(),
        download_prefix: DOWNLOAD_PREFIX.to_owned(),
        min_app_version: options.min_app_version.clone(),
        publishers,
        plugins,
        revocations,
    };
    index
        .validate()
        .map_err(|e| PublishError(format!("index/{seq}.json: {e}")))?;
    let bytes = to_json(&index)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_INDEX_BYTES {
        return fail("the index exceeds 4 MiB (sharding is spec_version 2)");
    }
    for (path, detail) in &details {
        write_immutable(&ws.site().join(path), detail)?;
    }
    let path = ws.index_path(seq);
    if path.exists() {
        return fail(format!("{} already exists", path.display()));
    }
    write(&path, &bytes)?;
    prune(ws, options.keep)?;
    Ok(seq)
}

/// Keeps the newest index and `keep` previous ones, and every detail they
/// reference (section 10); removes the rest.
fn prune(ws: &Workspace, keep: usize) -> Result<()> {
    let seqs = ws.index_seqs()?;
    let cut = seqs.len().saturating_sub(keep + 1);
    let (old, kept) = seqs.split_at(cut);
    for seq in old {
        let path = ws.index_path(*seq);
        remove(&sig_path(&path))?;
        remove(&path)?;
    }
    let mut referenced = BTreeSet::new();
    for seq in kept {
        for plugin in load_index(ws, *seq)?.plugins {
            referenced.insert(ws.site().join(&plugin.detail.path));
        }
    }
    let plugins = ws.site().join("plugins");
    let Ok(dirs) = std::fs::read_dir(&plugins) else {
        return Ok(());
    };
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            if !referenced.contains(&file.path()) {
                remove(&file.path())?;
            }
        }
    }
    Ok(())
}

/// `sign-index`: signs the newest index (or `seq`) with the index key and
/// the exact comment `role:index seq:<seq> keys:<keys_version>`.
pub(crate) fn sign_index(ws: &Workspace, seq: Option<u64>, key: &SigningKey) -> Result<u64> {
    let keyset = keyset::current(ws)?;
    let seq = match seq {
        Some(seq) => seq,
        None => *ws
            .index_seqs()?
            .last()
            .ok_or_else(|| PublishError("no index; run `build-index` first".into()))?,
    };
    let path = ws.index_path(seq);
    let bytes = read(&path)?;
    let index: Index = super::layout::parse(&path, &bytes)?;
    index
        .validate()
        .map_err(|e| PublishError(format!("{}: {e}", path.display())))?;
    if index.seq != seq {
        return fail(format!("{} holds seq {}", path.display(), index.seq));
    }
    if index.keys_version != keyset.version {
        return fail(format!(
            "index/{seq}.json names key set {}, the current key set is {}; build a new index",
            index.keys_version, keyset.version
        ));
    }
    if !keyset.index.identities().contains(&key.identity()) {
        return fail(format!(
            "this key is not an index key of keys/{}.json",
            keyset.version
        ));
    }
    let signature_path = sig_path(&path);
    if signature_path.exists() {
        return fail(format!("{} already exists", signature_path.display()));
    }
    let comment = index_comment(seq, keyset.version);
    let signature = key.sign(&bytes, &comment)?;
    verify_role(
        &keyset.index,
        std::slice::from_ref(&signature),
        &bytes,
        &comment,
    )
    .map_err(|e| PublishError(format!("self-check failed: {e}")))?;
    write(&signature_path, signature.as_bytes())?;
    Ok(seq)
}

/// `timestamp`: (re-)signs `timestamp.json` for the newest signed index,
/// valid for `validity` (at most 14 days). This is all the daily job does;
/// it never needs the index key.
pub(crate) fn timestamp(
    ws: &Workspace,
    key: &SigningKey,
    signed_at: DateTime<Utc>,
    validity: TimeDelta,
) -> Result<Timestamp> {
    if validity <= TimeDelta::zero() || validity > TimeDelta::days(MAX_TIMESTAMP_VALIDITY_DAYS) {
        return fail(format!(
            "the timestamp validity must be more than 0 and at most {MAX_TIMESTAMP_VALIDITY_DAYS} \
             days (clients refuse longer)"
        ));
    }
    let keyset = keyset::current(ws)?;
    if !keyset.timestamp.identities().contains(&key.identity()) {
        return fail(format!(
            "this key is not a timestamp key of keys/{}.json",
            keyset.version
        ));
    }
    let seq = *ws
        .index_seqs()?
        .iter()
        .rev()
        .find(|seq| sig_path(&ws.index_path(**seq)).is_file())
        .ok_or_else(|| PublishError("no signed index; run `sign-index` first".into()))?;
    let path = ws.index_path(seq);
    let bytes = read(&path)?;
    let signature = String::from_utf8(read(&sig_path(&path))?)
        .map_err(|_| PublishError(format!("index/{seq}.json.minisig is not text")))?;
    // Never pin bytes the index key did not sign for this key set.
    verify_role(
        &keyset.index,
        &[signature.trim().to_owned()],
        &bytes,
        &index_comment(seq, keyset.version),
    )
    .map_err(|e| {
        PublishError(format!(
            "index/{seq}.json is not signed for key set {}: {e}",
            keyset.version
        ))
    })?;
    if seq < keyset.seq_floor {
        return fail(format!(
            "index/{seq}.json is below the key set's seq_floor {}",
            keyset.seq_floor
        ));
    }
    if let Some(existing) = read_optional(&ws.timestamp_path())? {
        let existing: Timestamp = super::layout::parse(&ws.timestamp_path(), &existing)?;
        // After a seq_floor_reset the legitimate seq may be below what an
        // attacker made the site name; otherwise the seq never goes back.
        if existing.seq > seq && !keyset.seq_floor_reset {
            return fail(format!(
                "timestamp.json already names seq {}; refusing to roll back to {seq}",
                existing.seq
            ));
        }
        if existing.signed_at > signed_at {
            return fail(format!(
                "timestamp.json was signed at {}, later than {signed_at}; check the clock",
                existing.signed_at
            ));
        }
    }
    let timestamp = Timestamp {
        kind: TimestampType::Value,
        spec_version: SPEC_VERSION,
        seq,
        keys_version: keyset.version,
        signed_at,
        expires: signed_at + validity,
        index: FileRef {
            path: format!("index/{seq}.json"),
            sha256: sha256(&bytes),
            length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        },
    };
    timestamp
        .validate()
        .map_err(|e| PublishError(format!("timestamp.json: {e}")))?;
    let encoded = to_json(&timestamp)?;
    if encoded.len() > crate::MAX_TIMESTAMP_BYTES {
        return fail("timestamp.json exceeds 4 KiB");
    }
    let comment = timestamp_comment(seq, keyset.version);
    let signature = key.sign(&encoded, &comment)?;
    verify_role(
        &keyset.timestamp,
        std::slice::from_ref(&signature),
        &encoded,
        &comment,
    )
    .map_err(|e| PublishError(format!("self-check failed: {e}")))?;
    // The deploy uploads both files together.
    write(&sig_path(&ws.timestamp_path()), signature.as_bytes())?;
    write(&ws.timestamp_path(), &encoded)?;
    Ok(timestamp)
}
