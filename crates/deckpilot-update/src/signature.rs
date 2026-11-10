//! Minisign signature verification in the format used by the Tauri updater:
//! public keys and signatures are base64-encoded minisign text files.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{PublicKey, Signature};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignatureError {
    #[error("the public key is malformed")]
    InvalidPublicKey,
    #[error("the signature is malformed")]
    InvalidSignature,
    #[error("the signature does not match the artifact")]
    Mismatch,
}

fn decode_text(value: &str) -> Option<String> {
    let bytes = STANDARD.decode(value.trim()).ok()?;
    String::from_utf8(bytes).ok()
}

pub(crate) fn decode_signature(signature: &str) -> Result<Signature, SignatureError> {
    let text = decode_text(signature).ok_or(SignatureError::InvalidSignature)?;
    Signature::decode(&text).map_err(|_| SignatureError::InvalidSignature)
}

/// Verifies `data` against a base64-encoded minisign `signature` made with
/// the key whose base64-encoded public key is `public_key`.
pub fn verify(public_key: &str, signature: &str, data: &[u8]) -> Result<(), SignatureError> {
    let key_text = decode_text(public_key).ok_or(SignatureError::InvalidPublicKey)?;
    let key = PublicKey::decode(&key_text).map_err(|_| SignatureError::InvalidPublicKey)?;
    let signature = decode_signature(signature)?;
    key.verify(data, &signature, false)
        .map_err(|_| SignatureError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test-only key pair generated with `tauri signer generate`; its private
    // key was discarded. It is unrelated to the release signing key.
    const PUBLIC_KEY: &str = include_str!("../tests/fixtures/test-key.pub");
    const SIGNATURE: &str = include_str!("../tests/fixtures/artifact.bin.sig");
    const ARTIFACT: &[u8] = include_bytes!("../tests/fixtures/artifact.bin");

    #[test]
    fn accepts_a_valid_signature() {
        assert_eq!(verify(PUBLIC_KEY, SIGNATURE, ARTIFACT), Ok(()));
    }

    #[test]
    fn rejects_tampered_artifacts_and_bad_input() {
        let mut tampered = ARTIFACT.to_vec();
        tampered[0] ^= 0xff;
        assert_eq!(
            verify(PUBLIC_KEY, SIGNATURE, &tampered),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            verify("bm90IGEga2V5", SIGNATURE, ARTIFACT),
            Err(SignatureError::InvalidPublicKey)
        );
        assert_eq!(
            verify(PUBLIC_KEY, "%%%", ARTIFACT),
            Err(SignatureError::InvalidSignature)
        );
    }
}
