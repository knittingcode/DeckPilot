//! The check of an embedded root role, shared by `build.rs` (which runs it
//! on `DECKPILOT_REGISTRY_ROOT_PUBKEYS` and fails the build on any
//! problem) and the library (whose tests hold it to [`crate::Role`]'s own
//! `validate`, so the build never accepts roots the client refuses).
//!
//! Only `serde`, `serde_json`, `base64` and `minisign-verify`: the build
//! script cannot use the crate it builds.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Largest accepted value (four keys of at most 512 characters fit many
/// times over).
pub const MAX_ROOTS_BYTES: usize = 16 * 1024;

/// `roots.json`: exactly the fields of the registry's `Role`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Roots {
    keys: Vec<String>,
    threshold: u64,
}

/// The Ed25519 key of a base64-wrapped minisign public key (as the
/// client's `key_identity` reads it), if it is one.
fn identity(key: &str) -> Option<[u8; 32]> {
    let text = String::from_utf8(STANDARD.decode(key).ok()?).ok()?;
    minisign_verify::PublicKey::decode(&text).ok()?;
    let raw = STANDARD.decode(text.lines().nth(1)?.trim()).ok()?;
    raw.get(10..42)?.try_into().ok()
}

/// Checks `text` as a root role: the content of `keys/roots.json` (the
/// `roots.json` that `deckpilot-registry keyset init` writes): 1 to 4
/// different minisign public keys, base64-wrapped (40-512 characters), and
/// a threshold from 1 to 4 and at most the number of keys. Nothing else.
pub fn check_roots(text: &str) -> Result<(), String> {
    if text.len() > MAX_ROOTS_BYTES {
        return Err(format!(
            "is {} bytes; at most {MAX_ROOTS_BYTES} are allowed",
            text.len()
        ));
    }
    let roots: Roots = serde_json::from_str(text)
        .map_err(|e| format!("is not a root role ({{\"keys\": [...], \"threshold\": n}}): {e}"))?;
    if roots.keys.is_empty() || roots.keys.len() > 4 {
        return Err(format!("has {} keys; 1 to 4 are allowed", roots.keys.len()));
    }
    let mut seen = Vec::new();
    for (n, key) in roots.keys.iter().enumerate() {
        let n = n + 1;
        if key.len() < 40
            || key.len() > 512
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
        {
            return Err(format!("key {n} is not base64 (40-512 characters)"));
        }
        let id = identity(key)
            .ok_or_else(|| format!("key {n} is not a base64-wrapped minisign public key"))?;
        if seen.contains(&id) {
            return Err(format!("key {n} is listed twice"));
        }
        seen.push(id);
    }
    let keys = u64::try_from(seen.len()).unwrap_or(u64::MAX);
    if roots.threshold == 0 || roots.threshold > 4 || roots.threshold > keys {
        return Err(format!("has threshold {} for {keys} keys", roots.threshold));
    }
    Ok(())
}
