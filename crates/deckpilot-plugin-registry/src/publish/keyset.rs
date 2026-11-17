//! `keys/<n>.json`: root-signed key sets (architecture sections 9.1, 9.4).
//!
//! Writing a key set and signing it are separate steps, so each root key
//! holder can sign on their own offline machine: `keyset init` or
//! `keyset rotate` writes the document with an empty signature array, and
//! `keyset sign` adds one signature per run.

use std::fmt::Write as _;

use super::keys::SigningKey;
use super::layout::{self, Workspace, read, read_json, read_optional, sig_path, write_json};
use super::{PublishError, Result, fail, to_json};
use crate::client::MAX_KEYSET_SIGNATURES;
use crate::model::{FirstParty, Keyset, KeysetType, Role, SPEC_VERSION, Timestamp};
use crate::sig::{keys_comment, same_role, verify_exact, verify_role};
use crate::{MemoryTrustStore, Policy, RegistryClient};

/// The embedded root role (`roots.json`).
pub(crate) fn roots(ws: &Workspace) -> Result<Role> {
    let path = ws.roots_path();
    if !path.is_file() {
        return fail(format!(
            "{} is missing; run `keyset init` first",
            path.display()
        ));
    }
    let role: Role = read_json(&path)?;
    role.validate()
        .map_err(|e| PublishError(format!("{}: {e}", path.display())))?;
    Ok(role)
}

fn read_signatures(ws: &Workspace, version: u64) -> Result<Vec<String>> {
    let path = sig_path(&ws.keyset_path(version));
    match read_optional(&path)? {
        Some(bytes) => layout::parse(&path, &bytes),
        None => Ok(Vec::new()),
    }
}

/// Walks `keys/1.json` to `keys/<upto>.json` with the client's own code
/// (the same checks every client runs) and returns the key sets.
pub(crate) fn verified_chain(ws: &Workspace, upto: u64) -> Result<Vec<Keyset>> {
    let mut client =
        RegistryClient::open(roots(ws)?, MemoryTrustStore::default(), Policy::default())
            .map_err(|e| PublishError(e.to_string()))?;
    let mut chain = Vec::new();
    for version in 1..=upto {
        let path = ws.keyset_path(version);
        let bytes = read(&path)?;
        let signatures = read_optional(&sig_path(&path))?.unwrap_or_else(|| b"[]".to_vec());
        client.apply_keyset(&bytes, &signatures).map_err(|e| {
            PublishError(format!(
                "keys/{version}.json is not accepted by clients: {e} (sign it with \
                 `keyset sign`)"
            ))
        })?;
        chain.push(layout::parse(&path, &bytes)?);
    }
    Ok(chain)
}

/// The newest key set, fully signed.
pub(crate) fn current(ws: &Workspace) -> Result<Keyset> {
    let newest = ws.newest_keyset();
    if newest == 0 {
        return fail("no key set; run `keyset init` first");
    }
    verified_chain(ws, newest)?
        .pop()
        .ok_or_else(|| PublishError("no key set".into()))
}

/// The highest seq the site has published (timestamp or index files).
pub(crate) fn published_seq(ws: &Workspace) -> Result<u64> {
    let mut seq = ws.index_seqs()?.last().copied().unwrap_or(0);
    if let Some(bytes) = read_optional(&ws.timestamp_path())? {
        let timestamp: Timestamp = layout::parse(&ws.timestamp_path(), &bytes)?;
        seq = seq.max(timestamp.seq);
    }
    Ok(seq)
}

/// Index and timestamp signatures are single `.minisig` files (section
/// 9.2), so only a threshold of 1 can ever be met for those roles.
fn single_signer(name: &str, role: &Role) -> Result<()> {
    if role.threshold != 1 {
        return fail(format!(
            "the {name} role needs threshold 1: its documents carry one signature"
        ));
    }
    Ok(())
}

fn check(keyset: &Keyset) -> Result<()> {
    keyset
        .validate()
        .map_err(|e| PublishError(format!("keys/{}.json: {e}", keyset.version)))?;
    single_signer("index", &keyset.index)?;
    single_signer("timestamp", &keyset.timestamp)
}

fn write_keyset(ws: &Workspace, keyset: &Keyset) -> Result<()> {
    let path = ws.keyset_path(keyset.version);
    if path.exists() {
        return fail(format!("{} already exists", path.display()));
    }
    let bytes = to_json(keyset)?;
    if bytes.len() > crate::MAX_KEYSET_BYTES {
        return fail("the key set is larger than 16 KiB");
    }
    layout::write(&path, &bytes)?;
    write_json(&sig_path(&path), &Vec::<String>::new())
}

/// `keyset init`: `roots.json` and an unsigned `keys/1.json`.
/// `first_party` (`--first-party`) authorizes the reserved `deckpilot`
/// namespace ([`FirstParty`]); nothing else ever adds it.
pub(crate) fn init(
    ws: &Workspace,
    root: &Role,
    index: Role,
    timestamp: Role,
    seq_floor: u64,
    first_party: bool,
) -> Result<Keyset> {
    if ws.roots_path().exists() || ws.newest_keyset() > 0 {
        return fail("the registry already has roots or a key set; use `keyset rotate`");
    }
    let keyset = Keyset {
        kind: KeysetType::Value,
        spec_version: SPEC_VERSION,
        version: 1,
        root: root.clone(),
        index,
        timestamp,
        seq_floor,
        seq_floor_reset: false,
        first_party: first_party.then(FirstParty::deckpilot),
    };
    check(&keyset)?;
    write_json(&ws.roots_path(), root)?;
    write_keyset(ws, &keyset)?;
    Ok(keyset)
}

/// What `keyset rotate` changes; `None` keeps the current value.
#[derive(Debug, Default)]
pub(crate) struct Rotation {
    pub root: Option<Vec<String>>,
    pub root_threshold: Option<u64>,
    pub index: Option<Role>,
    pub timestamp: Option<Role>,
    pub seq_floor: Option<u64>,
    pub seq_floor_reset: bool,
    /// `Some(true)`: authorize `deckpilot` (`--first-party`); `Some(false)`:
    /// withdraw the authorization (`--no-first-party`); `None`: keep it.
    pub first_party: Option<bool>,
}

/// `keyset rotate`: an unsigned `keys/<n+1>.json`.
pub(crate) fn rotate(ws: &Workspace, rotation: Rotation) -> Result<Keyset> {
    let previous = current(ws)?;
    let root = Role {
        keys: rotation.root.unwrap_or_else(|| previous.root.keys.clone()),
        threshold: rotation.root_threshold.unwrap_or(previous.root.threshold),
    };
    let keyset = Keyset {
        kind: KeysetType::Value,
        spec_version: SPEC_VERSION,
        version: previous.version + 1,
        root,
        index: rotation.index.unwrap_or_else(|| previous.index.clone()),
        timestamp: rotation
            .timestamp
            .unwrap_or_else(|| previous.timestamp.clone()),
        // Default: the seq live now, so a client that saw it keeps its floor
        // and the next index (seq + 1) is accepted.
        seq_floor: match rotation.seq_floor {
            Some(floor) => floor,
            None => published_seq(ws)?,
        },
        seq_floor_reset: rotation.seq_floor_reset,
        first_party: match rotation.first_party {
            Some(true) => Some(FirstParty::deckpilot()),
            Some(false) => None,
            None => previous.first_party.clone(),
        },
    };
    let unchanged = Keyset {
        version: previous.version,
        ..keyset.clone()
    };
    if unchanged == previous {
        return fail(
            "nothing changes; give new keys, a new seq floor, --seq-floor-reset, --first-party \
             or --no-first-party",
        );
    }
    if !keyset.seq_floor_reset && keyset.seq_floor < previous.seq_floor {
        return fail(format!(
            "seq_floor {} is below the current {} (without --seq-floor-reset it only rises)",
            keyset.seq_floor, previous.seq_floor
        ));
    }
    check(&keyset)?;
    write_keyset(ws, &keyset)?;
    Ok(keyset)
}

fn count(role: &Role, signatures: &[String], data: &[u8], comment: &str) -> usize {
    let mut ids = std::collections::BTreeSet::new();
    for key in &role.keys {
        if signatures
            .iter()
            .any(|s| verify_exact(key, s, data, comment).is_ok())
            && let Ok(id) = crate::sig::key_identity(key)
        {
            ids.insert(id);
        }
    }
    ids.len()
}

/// `keyset sign`: adds `key`'s signature to `keys/<version>.json.minisig`
/// (the newest key set by default). The key must be a root of the
/// previous key set (the embedded roots for version 1) or of the new one.
/// Returns a status line.
pub(crate) fn sign(ws: &Workspace, version: Option<u64>, key: &SigningKey) -> Result<String> {
    let newest = ws.newest_keyset();
    let version = version.unwrap_or(newest);
    if version == 0 || version > newest {
        return fail(format!("keys/{version}.json does not exist"));
    }
    let previous_root = if version == 1 {
        roots(ws)?
    } else {
        verified_chain(ws, version - 1)?
            .pop()
            .map(|k| k.root)
            .ok_or_else(|| PublishError("no previous key set".into()))?
    };
    let path = ws.keyset_path(version);
    let bytes = read(&path)?;
    let keyset: Keyset = layout::parse(&path, &bytes)?;
    if keyset.version != version {
        return fail(format!("{} has version {}", path.display(), keyset.version));
    }
    check(&keyset)?;
    let id = key.identity();
    let roles = [&previous_root, &keyset.root];
    if !roles.iter().any(|role| role.identities().contains(&id)) {
        return fail(format!(
            "this key is not a root key of keys/{}.json or keys/{version}.json",
            version - 1
        ));
    }
    let comment = keys_comment(version);
    let mut signatures = read_signatures(ws, version)?;
    let already = signatures
        .iter()
        .any(|s| verify_exact(key.public(), s, &bytes, &comment).is_ok());
    if !already {
        if signatures.len() >= MAX_KEYSET_SIGNATURES {
            return fail("the key set already has the maximum number of signatures");
        }
        signatures.push(key.sign(&bytes, &comment)?);
        let encoded = to_json(&signatures)?;
        if encoded.len() > crate::MAX_KEYSET_SIGNATURES_BYTES {
            return fail("the signature array would exceed 16 KiB");
        }
        layout::write(&sig_path(&path), &encoded)?;
    }
    let mut status = format!(
        "keys/{version}.json: {} {}; previous roots {}/{}",
        if already {
            "already signed by"
        } else {
            "signed by"
        },
        short(key.public()),
        count(&previous_root, &signatures, &bytes, &comment),
        previous_root.threshold
    );
    if !same_role(&previous_root, &keyset.root) {
        let _ = write!(
            status,
            ", new roots {}/{}",
            count(&keyset.root, &signatures, &bytes, &comment),
            keyset.root.threshold
        );
    }
    let complete = verify_role(&previous_root, &signatures, &bytes, &comment).is_ok()
        && (same_role(&previous_root, &keyset.root)
            || verify_role(&keyset.root, &signatures, &bytes, &comment).is_ok());
    status.push_str(if complete {
        " (complete)"
    } else {
        " (more signatures needed)"
    });
    Ok(status)
}

/// A short, non-secret label for a public key: its minisign key id.
pub(crate) fn short(public: &str) -> String {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::STANDARD;
    engine
        .decode(public)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|text| {
            let line = text.lines().nth(1)?.to_owned();
            let raw = engine.decode(line.trim()).ok()?;
            let id = raw.get(2..10)?;
            Some(id.iter().rev().fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02X}");
                s
            }))
        })
        .map_or_else(|| "key".to_owned(), |id| format!("key {id}"))
}
