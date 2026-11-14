//! Signatures (architecture sections 9.1 and 9.2) on top of
//! `deckpilot_update::signature::verify`, which is reused unchanged
//! (minisign, Ed25519, prehashed, base64 Tauri format).
//!
//! What this module adds:
//!
//! * **Exact trusted comments.** Every registry comment starts with
//!   `deckpilot-registry v1 role:` and names its role, sequence and key set
//!   (see [`keys_comment`], [`index_comment`], [`timestamp_comment`]). A
//!   signature counts only if its comment is byte-for-byte the one expected
//!   for that document, so a signature made for another role, another seq,
//!   another key set or by the Tauri updater (`timestamp:…\tfile:…\tversion:…`)
//!   never passes. minisign's global signature covers the trusted comment,
//!   so a changed comment fails verification before it is compared.
//! * **Roles with thresholds.** A threshold of *distinct* keys of a role
//!   must each have produced a valid signature. Keys are compared by their
//!   Ed25519 key, not by their text, so one key written twice (for example
//!   with another untrusted comment) counts once.

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use deckpilot_update::signature::{self, SignatureError};

use crate::model::Role;

/// Prefix of every registry trusted comment.
pub const COMMENT_PREFIX: &str = "deckpilot-registry v1 role:";

/// Why a signature or role check failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SigError {
    #[error("signature check failed: {0}")]
    Signature(#[from] SignatureError),
    #[error("the trusted comment {found:?} is not the expected {expected:?}")]
    WrongComment { expected: String, found: String },
    #[error("the trusted comment {0:?} is not a registry comment of the expected role")]
    UnparsableComment(String),
    #[error("{valid} valid signature(s) from distinct keys of the role, {threshold} required")]
    BelowThreshold { valid: usize, threshold: usize },
    #[error("the role is malformed (threshold 0 or above the number of distinct keys)")]
    BadRole,
}

/// `deckpilot-registry v1 role:keys version:<n>`.
pub fn keys_comment(version: u64) -> String {
    format!("{COMMENT_PREFIX}keys version:{version}")
}

/// `deckpilot-registry v1 role:index seq:<seq> keys:<keys_version>`.
pub fn index_comment(seq: u64, keys_version: u64) -> String {
    format!("{COMMENT_PREFIX}index seq:{seq} keys:{keys_version}")
}

/// `deckpilot-registry v1 role:timestamp seq:<seq> keys:<keys_version>`.
pub fn timestamp_comment(seq: u64, keys_version: u64) -> String {
    format!("{COMMENT_PREFIX}timestamp seq:{seq} keys:{keys_version}")
}

/// Reads `seq` and `keys_version` from a timestamp comment. Only the
/// canonical form is accepted (no leading zeros, signs or spaces).
pub(crate) fn parse_timestamp_comment(comment: &str) -> Result<(u64, u64), SigError> {
    let bad = || SigError::UnparsableComment(comment.to_owned());
    let rest = comment
        .strip_prefix(&format!("{COMMENT_PREFIX}timestamp seq:"))
        .ok_or_else(bad)?;
    let (seq, keys) = rest.split_once(" keys:").ok_or_else(bad)?;
    let seq = parse_canonical(seq).ok_or_else(bad)?;
    let keys = parse_canonical(keys).ok_or_else(bad)?;
    if comment != timestamp_comment(seq, keys) {
        return Err(bad());
    }
    Ok((seq, keys))
}

fn parse_canonical(text: &str) -> Option<u64> {
    if text.is_empty()
        || (text.len() > 1 && text.starts_with('0'))
        || !text.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    text.parse().ok()
}

/// The trusted comment of a base64 minisign signature, **not verified**.
/// Only used to learn which exact comment to then require.
///
/// Stricter than minisign: the signature must be exactly the four lines
/// minisign writes (untrusted comment, signature, trusted comment, global
/// signature), so no unsigned data can ride along after it.
pub(crate) fn trusted_comment(signature_b64: &str) -> Result<String, SigError> {
    let invalid = SigError::Signature(SignatureError::InvalidSignature);
    let text = STANDARD
        .decode(signature_b64.trim())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| invalid.clone())?;
    let lines: Vec<&str> = text
        .strip_suffix('\n')
        .unwrap_or(&text)
        .split('\n')
        .collect();
    if lines.len() != 4
        || !lines[0].starts_with("untrusted comment: ")
        || !lines[2].starts_with("trusted comment: ")
        || lines.iter().any(|line| line.ends_with('\r'))
    {
        return Err(invalid);
    }
    let signature = minisign_verify::Signature::decode(&text).map_err(|_| invalid)?;
    Ok(signature.trusted_comment().to_owned())
}

/// The Ed25519 key of a base64-wrapped minisign public key: what makes two
/// keys "the same key".
pub fn key_identity(public_key_b64: &str) -> Result<[u8; 32], SigError> {
    let invalid = SigError::Signature(SignatureError::InvalidPublicKey);
    signature::check_public_key(public_key_b64)?;
    let text = STANDARD
        .decode(public_key_b64.trim())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| invalid.clone())?;
    let line = text.lines().nth(1).ok_or_else(|| invalid.clone())?;
    let raw = STANDARD.decode(line.trim()).map_err(|_| invalid.clone())?;
    let key: [u8; 32] = raw
        .get(10..42)
        .and_then(|k| k.try_into().ok())
        .ok_or(invalid)?;
    Ok(key)
}

/// One key, exact comment.
pub fn verify_exact(
    public_key_b64: &str,
    signature_b64: &str,
    data: &[u8],
    expected_comment: &str,
) -> Result<(), SigError> {
    let found = trusted_comment(signature_b64)?;
    signature::verify(public_key_b64, signature_b64, data)?;
    if found != expected_comment {
        return Err(SigError::WrongComment {
            expected: expected_comment.to_owned(),
            found,
        });
    }
    Ok(())
}

/// A threshold of distinct keys of `role` must each have produced one of
/// `signatures` over `data` with exactly `expected_comment`.
pub fn verify_role(
    role: &Role,
    signatures: &[String],
    data: &[u8],
    expected_comment: &str,
) -> Result<(), SigError> {
    let threshold = usize::try_from(role.threshold).map_err(|_| SigError::BadRole)?;
    let mut keys = BTreeSet::new();
    let mut distinct = Vec::new();
    for key in &role.keys {
        if keys.insert(key_identity(key)?) {
            distinct.push(key);
        }
    }
    if threshold == 0 || threshold > distinct.len() {
        return Err(SigError::BadRole);
    }
    let mut wrong_comment = None;
    let mut valid = 0;
    for key in distinct {
        let signed = signatures.iter().any(|signature| {
            match verify_exact(key, signature, data, expected_comment) {
                Ok(()) => true,
                Err(e @ SigError::WrongComment { .. }) => {
                    wrong_comment = Some(e);
                    false
                }
                Err(_) => false,
            }
        });
        if signed {
            valid += 1;
        }
    }
    if valid >= threshold {
        return Ok(());
    }
    // A valid signature with the wrong comment says more than "0 of 1".
    match wrong_comment {
        Some(e) if valid == 0 && threshold == 1 => Err(e),
        _ => Err(SigError::BelowThreshold { valid, threshold }),
    }
}

/// Whether two roles are the same set of keys (by identity) with the same
/// threshold.
pub(crate) fn same_role(a: &Role, b: &Role) -> bool {
    let ids = |role: &Role| -> Option<BTreeSet<[u8; 32]>> {
        role.keys.iter().map(|k| key_identity(k).ok()).collect()
    };
    a.threshold == b.threshold && ids(a).is_some() && ids(a) == ids(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_canonical() {
        assert_eq!(
            parse_timestamp_comment("deckpilot-registry v1 role:timestamp seq:12 keys:3"),
            Ok((12, 3))
        );
        for bad in [
            "deckpilot-registry v1 role:timestamp seq:012 keys:3",
            "deckpilot-registry v1 role:timestamp seq:+12 keys:3",
            "deckpilot-registry v1 role:timestamp seq:12 keys:3 ",
            "deckpilot-registry v1 role:timestamp seq:12  keys:3",
            "deckpilot-registry v1 role:timestamp seq:12\tkeys:3",
            "deckpilot-registry v1 role:index seq:12 keys:3",
            "deckpilot-registry v2 role:timestamp seq:12 keys:3",
            "deckpilot-registry v1 role:timestamp seq: keys:3",
            "deckpilot-registry v1 role:timestamp seq:99999999999999999999 keys:3",
            "timestamp:1700000000\tfile:timestamp.json\tversion:1.0.0",
        ] {
            assert!(parse_timestamp_comment(bad).is_err(), "{bad}");
        }
        assert_eq!(keys_comment(2), "deckpilot-registry v1 role:keys version:2");
        assert_eq!(
            index_comment(5, 2),
            "deckpilot-registry v1 role:index seq:5 keys:2"
        );
    }
}
