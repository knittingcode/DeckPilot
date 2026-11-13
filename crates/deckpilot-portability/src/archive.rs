//! The encrypted archive format (`.deckpilot` files).
//!
//! An archive is a small JSON document:
//!
//! ```json
//! { "header": "<JSON text of Header>", "ciphertext": "<base64>" }
//! ```
//!
//! The header names the format, its version, the cipher, a random 24-byte
//! nonce and the Argon2id parameters with a random salt. Its exact text is
//! the associated data of the AEAD, so changing any header field (or the
//! ciphertext) makes decryption fail. The key is derived from the export
//! password with Argon2id; it is unrelated to the database key, which is
//! never part of an export. Cipher: XChaCha20-Poly1305 (`RustCrypto`
//! `chacha20poly1305`). No primitive is implemented here.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use deckpilot_storage::crypto::{KdfParams, Password, random_array};

use crate::PortabilityError;

/// Identifies DeckPilot archives.
pub const FORMAT: &str = "deckpilot-configuration";
/// Version of the archive envelope.
pub const FORMAT_VERSION: u32 = 1;
pub const CIPHER: &str = "xchacha20poly1305";
/// Archives larger than this are rejected before anything is parsed.
pub const MAX_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
const NONCE_LEN: usize = 24;

/// Authenticated, unencrypted metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub format: String,
    pub version: u32,
    pub cipher: String,
    /// Base64, 24 bytes.
    pub nonce: String,
    pub kdf: KdfParams,
    /// Seconds since the Unix epoch.
    pub created_at: i64,
    pub app_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    header: String,
    ciphertext: String,
}

fn cipher_for(header: &Header, password: &Password) -> Result<XChaCha20Poly1305, PortabilityError> {
    let key = header.kdf.derive(password)?;
    XChaCha20Poly1305::new_from_slice(key.as_bytes()).map_err(|_| PortabilityError::Crypto)
}

/// Encrypts `plaintext` with a key derived from `password`.
pub fn seal(
    plaintext: &[u8],
    password: &Password,
    kdf: KdfParams,
    created_at: i64,
    app_version: &str,
) -> Result<Vec<u8>, PortabilityError> {
    // A fresh random nonce for every archive (never a fixed or reused one).
    let nonce: [u8; NONCE_LEN] = random_array()?;
    let header = Header {
        format: FORMAT.into(),
        version: FORMAT_VERSION,
        cipher: CIPHER.into(),
        nonce: BASE64.encode(nonce),
        kdf,
        created_at,
        app_version: app_version.into(),
    };
    let header_text = serde_json::to_string(&header)?;
    let cipher = cipher_for(&header, password)?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: header_text.as_bytes(),
            },
        )
        .map_err(|_| PortabilityError::Crypto)?;
    let envelope = Envelope {
        header: header_text,
        ciphertext: BASE64.encode(ciphertext),
    };
    let bytes = serde_json::to_vec(&envelope)?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(PortabilityError::TooLarge);
    }
    Ok(bytes)
}

/// Reads the header without decrypting, e.g. to show what an archive is.
pub fn read_header(bytes: &[u8]) -> Result<Header, PortabilityError> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(PortabilityError::TooLarge);
    }
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|_| PortabilityError::NotAnArchive)?;
    let header: Header =
        serde_json::from_str(&envelope.header).map_err(|_| PortabilityError::NotAnArchive)?;
    if header.format != FORMAT {
        return Err(PortabilityError::NotAnArchive);
    }
    if header.version != FORMAT_VERSION || header.cipher != CIPHER {
        return Err(PortabilityError::UnsupportedVersion(header.version));
    }
    Ok(header)
}

/// Decrypts an archive. A wrong password and a modified archive are
/// indistinguishable by design and both yield
/// [`PortabilityError::WrongPasswordOrDamaged`].
pub fn open(
    bytes: &[u8],
    password: &Password,
) -> Result<(Header, Zeroizing<Vec<u8>>), PortabilityError> {
    let header = read_header(bytes)?;
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|_| PortabilityError::NotAnArchive)?;
    let nonce = BASE64
        .decode(&header.nonce)
        .ok()
        .filter(|nonce| nonce.len() == NONCE_LEN)
        .ok_or(PortabilityError::NotAnArchive)?;
    let ciphertext = BASE64
        .decode(&envelope.ciphertext)
        .map_err(|_| PortabilityError::NotAnArchive)?;
    let cipher = cipher_for(&header, password)?;
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: envelope.header.as_bytes(),
            },
        )
        .map_err(|_| PortabilityError::WrongPasswordOrDamaged)?;
    Ok((header, Zeroizing::new(plaintext)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password(text: &str) -> Password {
        Password::new(text)
    }

    /// The export password of this test run: generated, so that it is never a
    /// constant that could pass for (or be reused as) a real credential.
    fn export_password() -> &'static str {
        static PASSWORD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        PASSWORD.get_or_init(generated_password)
    }

    /// A new random password, different from every other one.
    fn generated_password() -> String {
        let mut bytes = [0u8; 16];
        deckpilot_storage::crypto::random_bytes(&mut bytes).unwrap();
        format!("test-export-{:032x}", u128::from_le_bytes(bytes))
    }

    fn sealed(data: &[u8]) -> Vec<u8> {
        seal(
            data,
            &password(export_password()),
            KdfParams::for_tests(),
            0,
            "0.4.0",
        )
        .unwrap()
    }

    #[test]
    fn round_trips_with_the_right_password() {
        let archive = sealed(b"{\"hello\":1}");
        let (header, plaintext) = open(&archive, &password(export_password())).unwrap();
        assert_eq!(plaintext.as_slice(), b"{\"hello\":1}");
        assert_eq!(header.format, FORMAT);
        assert_eq!(header.kdf.algorithm, "argon2id");
        // Nothing of the content is visible. The needle has quotes, which
        // base64 text never contains, so it cannot match by chance (a bare
        // word like `hello` could).
        assert!(!archive.windows(7).any(|w| w == b"\"hello\""));
        // Every archive has its own salt and nonce.
        assert_ne!(sealed(b"same").as_slice(), sealed(b"same").as_slice());
    }

    #[test]
    fn wrong_passwords_and_tampering_are_rejected() {
        let archive = sealed(b"secret configuration");
        assert!(matches!(
            open(&archive, &password(&generated_password())),
            Err(PortabilityError::WrongPasswordOrDamaged)
        ));

        // A flipped bit in the ciphertext.
        let mut envelope: serde_json::Value = serde_json::from_slice(&archive).unwrap();
        let mut ciphertext = BASE64
            .decode(envelope["ciphertext"].as_str().unwrap())
            .unwrap();
        ciphertext[3] ^= 1;
        envelope["ciphertext"] = BASE64.encode(ciphertext).into();
        let tampered = serde_json::to_vec(&envelope).unwrap();
        assert!(matches!(
            open(&tampered, &password(export_password())),
            Err(PortabilityError::WrongPasswordOrDamaged)
        ));

        // A changed header field (authenticated as associated data).
        let mut envelope: serde_json::Value = serde_json::from_slice(&archive).unwrap();
        let header = envelope["header"]
            .as_str()
            .unwrap()
            .replace("0.4.0", "9.9.9");
        envelope["header"] = header.into();
        let tampered = serde_json::to_vec(&envelope).unwrap();
        assert!(matches!(
            open(&tampered, &password(export_password())),
            Err(PortabilityError::WrongPasswordOrDamaged)
        ));
    }

    #[test]
    fn malformed_and_oversized_input_is_rejected_early() {
        assert!(matches!(
            open(b"not json", &password(&generated_password())),
            Err(PortabilityError::NotAnArchive)
        ));
        assert!(matches!(
            open(
                b"{\"header\":\"{}\",\"ciphertext\":\"\"}",
                &password(&generated_password())
            ),
            Err(PortabilityError::NotAnArchive)
        ));
        let huge = vec![b' '; MAX_ARCHIVE_BYTES + 1];
        assert!(matches!(
            open(&huge, &password(&generated_password())),
            Err(PortabilityError::TooLarge)
        ));
        // Expensive key derivation parameters are refused before deriving.
        let archive = sealed(b"x");
        let mut envelope: serde_json::Value = serde_json::from_slice(&archive).unwrap();
        let mut header: Header =
            serde_json::from_str(envelope["header"].as_str().unwrap()).unwrap();
        header.kdf.memory_kib = 64 * 1024 * 1024;
        envelope["header"] = serde_json::to_string(&header).unwrap().into();
        let costly = serde_json::to_vec(&envelope).unwrap();
        assert!(matches!(
            open(&costly, &password(export_password())),
            Err(PortabilityError::Kdf(_))
        ));
    }

    /// An archive written by DeckPilot 0.6.0 (before the nonce came from
    /// `random_array`), with test KDF parameters and a fixture password
    /// that protects nothing else.
    const ARCHIVE_0_6_0: &str = r#"{"header":"{\"format\":\"deckpilot-configuration\",\"version\":1,\"cipher\":\"xchacha20poly1305\",\"nonce\":\"nFjfe3BgLBY857Y0AwAH16H7pBurBpl2\",\"kdf\":{\"algorithm\":\"argon2id\",\"version\":19,\"memoryKib\":8192,\"iterations\":1,\"parallelism\":1,\"salt\":\"WomupH3touDcWYAYBTtACQ==\"},\"createdAt\":1700000000,\"appVersion\":\"0.6.0\"}","ciphertext":"bK2PHBoXUhIraUwekBdEXJgbQGaypDXy/XQZI2QciQ=="}"#;

    #[test]
    fn archives_of_earlier_versions_still_open() {
        let (header, plaintext) = open(
            ARCHIVE_0_6_0.as_bytes(),
            &password("fixture: not a real password"),
        )
        .unwrap();
        assert_eq!(plaintext.as_slice(), b"{\"profiles\":[]}");
        assert_eq!(header.app_version, "0.6.0");
        assert_eq!(header.created_at, 1_700_000_000);
    }

    #[test]
    fn every_archive_has_a_fresh_full_length_nonce() {
        let nonce_of = |archive: &[u8]| {
            let header = read_header(archive).unwrap();
            BASE64.decode(header.nonce).unwrap()
        };
        let first = nonce_of(&sealed(b"same"));
        let second = nonce_of(&sealed(b"same"));
        assert_eq!(first.len(), NONCE_LEN);
        assert_eq!(second.len(), NONCE_LEN);
        assert_ne!(first, second);
        assert_ne!(first, vec![0u8; NONCE_LEN]);
    }
}
