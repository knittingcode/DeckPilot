//! The plugin store's key, derived from the configuration database key.
//!
//! `plugins.db` is a SQLCipher database of its own. Its key is not stored
//! anywhere (no keychain item, no file): it is HKDF-SHA256 of the
//! configuration database key with a salt and info label that name this
//! use only, so the configuration key itself is never handed to the
//! plugin store, and a plugin store key reveals nothing about it.
//!
//! The configuration key changes when its protection changes. The agent
//! re-encrypts `plugins.db` when it makes that change itself (switching to
//! a system-managed key); a password change made by the configuration app
//! while the agent is locked leaves `plugins.db` under the old key, which
//! the agent then reports as a plugin store it cannot open.

use deckpilot_plugin_store::StoreKey;
use deckpilot_storage::DatabaseKey;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

/// HKDF salt: fixed, public, specific to DeckPilot's plugin store.
const SALT: &[u8] = b"deckpilot.plugin-store.v1";
/// HKDF info: what the output key is for.
const INFO: &[u8] = b"deckpilot plugins.db sqlcipher key";

fn derive(config: &DatabaseKey) -> Zeroizing<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(Some(SALT), config.as_bytes());
    let mut okm = Zeroizing::new([0u8; 32]);
    hkdf.expand(INFO, okm.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

/// The key of `plugins.db` for the configuration key `config`.
pub fn plugin_store_key(config: &DatabaseKey) -> StoreKey {
    StoreKey::from_bytes(*derive(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
    }

    #[test]
    fn the_key_is_derived_deterministically_and_separately() {
        let config = DatabaseKey::from_bytes([7; 32]);
        let other = DatabaseKey::from_bytes([8; 32]);
        assert_eq!(*derive(&config), *derive(&config));
        assert_ne!(*derive(&config), *derive(&other));
        // Never the configuration key itself.
        assert_ne!(&*derive(&config), config.as_bytes());
        // RFC 5869 HKDF-SHA256 with this salt and info, computed
        // independently: a change of the derivation would lock every
        // existing plugins.db, so it must not go unnoticed.
        assert_eq!(
            hex(&*derive(&DatabaseKey::from_bytes([0; 32]))),
            "a33d574850f1202c8b1b56fb3a0611dfab605a3a56c8473f85671087d5a619fa"
        );
        assert_eq!(format!("{:?}", plugin_store_key(&config)), "StoreKey(..)");
    }
}
