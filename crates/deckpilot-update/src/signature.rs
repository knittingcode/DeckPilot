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
    /// The signed (trusted) comment does not name the version. The app
    /// requires it (`requireSignedVersion`), so such an update would be
    /// refused by every installation.
    #[error("the signature does not record the version it was made for")]
    MissingVersion,
    #[error("the artifact was signed for version {signed}, but is announced as {announced}")]
    VersionMismatch { signed: String, announced: String },
}

fn decode_text(value: &str) -> Option<String> {
    let bytes = STANDARD.decode(value.trim()).ok()?;
    String::from_utf8(bytes).ok()
}

pub(crate) fn decode_signature(signature: &str) -> Result<Signature, SignatureError> {
    let text = decode_text(signature).ok_or(SignatureError::InvalidSignature)?;
    Signature::decode(&text).map_err(|_| SignatureError::InvalidSignature)
}

/// Checks that `public_key` is a well-formed base64-encoded minisign public
/// key, as `DECKPILOT_UPDATER_PUBKEY` must be.
pub fn check_public_key(public_key: &str) -> Result<(), SignatureError> {
    let text = decode_text(public_key).ok_or(SignatureError::InvalidPublicKey)?;
    PublicKey::decode(&text)
        .map(|_| ())
        .map_err(|_| SignatureError::InvalidPublicKey)
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

/// Verifies `data` like the installed app does: the signature must match,
/// and its signed comment must name `version` (`version:<x.y.z>`, written by
/// the Tauri CLI). This binds the artifact to the announced version and
/// prevents serving an older, validly signed release as a newer one.
pub fn verify_release_artifact(
    public_key: &str,
    signature: &str,
    data: &[u8],
    version: &semver::Version,
) -> Result<(), SignatureError> {
    verify(public_key, signature, data)?;
    let decoded = decode_signature(signature)?;
    let signed = decoded
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .ok_or(SignatureError::MissingVersion)?;
    match semver::Version::parse(signed.trim_start_matches('v')) {
        Ok(signed) if signed == *version => Ok(()),
        _ => Err(SignatureError::VersionMismatch {
            signed: signed.to_owned(),
            announced: version.to_string(),
        }),
    }
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
    fn checks_public_key_format() {
        assert_eq!(check_public_key(PUBLIC_KEY), Ok(()));
        assert_eq!(
            check_public_key("not a key"),
            Err(SignatureError::InvalidPublicKey)
        );
        assert_eq!(
            check_public_key(SIGNATURE),
            Err(SignatureError::InvalidPublicKey)
        );
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

    #[test]
    fn interrupted_or_corrupted_downloads_are_rejected() {
        let truncated = &ARTIFACT[..ARTIFACT.len() / 2];
        let mut extended = ARTIFACT.to_vec();
        extended.push(0);
        let zeroed = vec![0u8; ARTIFACT.len()];
        for (what, data) in [
            ("truncated", truncated),
            ("extended", &extended[..]),
            ("zeroed", &zeroed[..]),
            ("empty", &[][..]),
        ] {
            assert_eq!(
                verify(PUBLIC_KEY, SIGNATURE, data),
                Err(SignatureError::Mismatch),
                "{what}"
            );
        }
        // A valid signature by another key (not the release key).
        let (_, foreign) = signed(ARTIFACT, "timestamp:1\tversion:9.9.9");
        assert_eq!(
            verify(PUBLIC_KEY, &foreign, ARTIFACT),
            Err(SignatureError::Mismatch)
        );
    }

    /// Signs `data` with a key made for this test only (never stored).
    fn signed(data: &[u8], trusted_comment: &str) -> (String, String) {
        let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let public = STANDARD.encode(keys.pk.to_box().unwrap().to_string());
        let signature = minisign::sign(
            None,
            &keys.sk,
            std::io::Cursor::new(data),
            Some(trusted_comment),
            None,
        )
        .unwrap();
        (public, STANDARD.encode(signature.to_string()))
    }

    #[test]
    fn release_artifacts_must_be_signed_for_their_version() {
        let version = semver::Version::new(1, 2, 3);
        let data = b"release artifact";
        let (key, signature) = signed(data, "timestamp:1\tfile:app.tar.gz\tversion:1.2.3");
        assert_eq!(
            verify_release_artifact(&key, &signature, data, &version),
            Ok(())
        );
        // An older release served as a newer one.
        assert_eq!(
            verify_release_artifact(&key, &signature, data, &semver::Version::new(1, 2, 4)),
            Err(SignatureError::VersionMismatch {
                signed: "1.2.3".into(),
                announced: "1.2.4".into(),
            })
        );
        // Signed without the version: every installation would refuse it.
        let (key, unversioned) = signed(data, "timestamp:1\tfile:app.tar.gz");
        assert_eq!(
            verify_release_artifact(&key, &unversioned, data, &version),
            Err(SignatureError::MissingVersion)
        );
        // A modified artifact fails before the comment is even read.
        assert_eq!(
            verify_release_artifact(&key, &unversioned, b"tampered", &version),
            Err(SignatureError::Mismatch)
        );
    }
}
