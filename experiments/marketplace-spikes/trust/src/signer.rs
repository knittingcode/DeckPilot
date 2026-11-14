//! Registry-side signing helpers (what `publish.yml` would do with
//! `minisign -S`). Keys are generated in memory and never written to disk:
//! this spike only ever uses throwaway keys.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

pub struct ThrowawayKey {
    keys: minisign::KeyPair,
    public_b64: String,
}

impl ThrowawayKey {
    pub fn generate() -> Self {
        let keys = minisign::KeyPair::generate_unencrypted_keypair().expect("key generation");
        let public_b64 = STANDARD.encode(keys.pk.to_box().expect("public key box").to_string());
        Self { keys, public_b64 }
    }

    /// Base64-wrapped minisign public key, the `DECKPILOT_UPDATER_PUBKEY`
    /// format the key set schema uses.
    pub fn public(&self) -> &str {
        &self.public_b64
    }

    /// Base64-wrapped minisign signature (Tauri format) with the given
    /// trusted comment.
    pub fn sign(&self, data: &[u8], trusted_comment: &str) -> String {
        let sig = minisign::sign(
            None,
            &self.keys.sk,
            std::io::Cursor::new(data),
            Some(trusted_comment),
            None,
        )
        .expect("signing");
        STANDARD.encode(sig.to_string())
    }
}
