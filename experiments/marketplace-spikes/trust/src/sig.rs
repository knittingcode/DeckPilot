//! Signature checks on top of `deckpilot_update::signature::verify`, which
//! is reused unchanged (minisign, Ed25519, base64 Tauri format).
//!
//! What this module adds for the registry (§9.1-9.2 of the design):
//!
//! * exact trusted-comment matching (domain separation: a Tauri updater
//!   signature or a signature for another role, seq or key set never passes);
//! * role verification: "a threshold of distinct keys of a role signed this".
//!
//! minisign's global signature covers the trusted comment, so a changed
//! comment fails `verify` before the comment is even compared.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use deckpilot_update::signature::{self, SignatureError};
use thiserror::Error;

use crate::registry::Role;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SigError {
    #[error("signature check failed: {0}")]
    Signature(#[from] SignatureError),
    #[error("trusted comment {found:?} is not the expected {expected:?}")]
    WrongComment { expected: String, found: String },
    #[error("trusted comment {0:?} is not a registry comment of the expected role")]
    UnparsableComment(String),
    #[error("{valid} valid signature(s) from distinct keys of the role, {threshold} required")]
    BelowThreshold { valid: usize, threshold: usize },
    #[error("the role is malformed (threshold 0 or above the key count)")]
    BadRole,
}

/// Returns the trusted comment of a base64 minisign signature, WITHOUT
/// verifying it. Only use the result after `verify` succeeded, or to derive
/// the comment that is then required exactly.
pub fn trusted_comment(signature_b64: &str) -> Result<String, SigError> {
    let text = STANDARD
        .decode(signature_b64.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or(SigError::Signature(SignatureError::InvalidSignature))?;
    let sig = minisign_verify::Signature::decode(&text)
        .map_err(|_| SigError::Signature(SignatureError::InvalidSignature))?;
    Ok(sig.trusted_comment().to_owned())
}

/// One key, exact comment.
pub fn verify_exact(
    public_key_b64: &str,
    signature_b64: &str,
    data: &[u8],
    expected_comment: &str,
) -> Result<(), SigError> {
    signature::verify(public_key_b64, signature_b64, data)?;
    let found = trusted_comment(signature_b64)?;
    if found != expected_comment {
        return Err(SigError::WrongComment {
            expected: expected_comment.to_owned(),
            found,
        });
    }
    Ok(())
}

/// A threshold of DISTINCT keys of `role` must each have produced one of
/// `signatures` over `data` with exactly `expected_comment`. Two signatures
/// by the same key count once.
pub fn verify_role(
    role: &Role,
    signatures: &[String],
    data: &[u8],
    expected_comment: &str,
) -> Result<(), SigError> {
    let threshold = role.threshold as usize;
    if threshold == 0 || threshold > role.keys.len() {
        return Err(SigError::BadRole);
    }
    let mut last_error = None;
    let valid = role
        .keys
        .iter()
        .filter(|key| {
            signatures
                .iter()
                .any(|sig| match verify_exact(key, sig, data, expected_comment) {
                    Ok(()) => true,
                    Err(e) => {
                        last_error = Some(e);
                        false
                    }
                })
        })
        .count();
    if valid >= threshold {
        return Ok(());
    }
    // A single-signature document with a wrong comment is more useful
    // reported as such than as "0 of 1".
    match last_error {
        Some(e @ SigError::WrongComment { .. }) if valid == 0 && threshold == 1 => Err(e),
        _ => Err(SigError::BelowThreshold { valid, threshold }),
    }
}

/// Parses `deckpilot-registry v1 role:<role> seq:<seq> keys:<kv>`.
pub fn parse_seq_comment(comment: &str, role: &str) -> Result<(u64, u64), SigError> {
    let bad = || SigError::UnparsableComment(comment.to_owned());
    let rest = comment
        .strip_prefix(&format!("deckpilot-registry v1 role:{role} seq:"))
        .ok_or_else(bad)?;
    let (seq, kv) = rest.split_once(" keys:").ok_or_else(bad)?;
    let seq: u64 = parse_canonical(seq).ok_or_else(bad)?;
    let kv: u64 = parse_canonical(kv).ok_or_else(bad)?;
    // Re-render and compare so that only the canonical form is accepted.
    if comment != seq_comment(role, seq, kv) {
        return Err(bad());
    }
    Ok((seq, kv))
}

fn parse_canonical(s: &str) -> Option<u64> {
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    s.parse().ok()
}

pub fn seq_comment(role: &str, seq: u64, keys_version: u64) -> String {
    format!("deckpilot-registry v1 role:{role} seq:{seq} keys:{keys_version}")
}

pub fn keys_comment(version: u64) -> String {
    format!("deckpilot-registry v1 role:keys version:{version}")
}

/// Publisher signature over a package (deferred option U-2 of the design).
/// Binds the bytes to id, version and hash, so it cannot be replayed for
/// another plugin, another version or another file.
pub fn package_comment(id: &str, version: &str, sha256_hex: &str) -> String {
    format!("deckpilot-plugin v1 id:{id} version:{version} sha256:{sha256_hex}")
}
