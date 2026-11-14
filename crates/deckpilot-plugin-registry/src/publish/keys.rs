//! Minisign keys for local and test registries.
//!
//! **Production keys are not made with this.** The root keys, and the
//! index and timestamp keys that go into GitHub environment secrets, are
//! created in a key ceremony decided by the owner (U-10); this command
//! exists so the tooling can be exercised end to end with throwaway keys.
//!
//! Formats: the secret key is a standard minisign secret key file
//! (`untrusted comment:` line and base64 key, scrypt-encrypted with the
//! password unless generated with `--no-password`); the public key is the
//! base64-wrapped minisign public key the registry documents use (the
//! `DECKPILOT_UPDATER_PUBKEY` format).

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign::{KeyPair, PublicKey, SecretKey, SecretKeyBox};
use zeroize::Zeroizing;

use super::{PublishError, Result, fail};
use crate::sig::key_identity;

/// Which role a key is generated for (recorded in its untrusted comment;
/// the key sets decide what a key may sign).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRole {
    Root,
    Index,
    Timestamp,
}

impl KeyRole {
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "root" => Ok(Self::Root),
            "index" => Ok(Self::Index),
            "timestamp" => Ok(Self::Timestamp),
            other => fail(format!("unknown role `{other}` (root, index or timestamp)")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Index => "index",
            Self::Timestamp => "timestamp",
        }
    }
}

/// A key password, wiped from memory when dropped. Never printed.
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    fn reveal(&self) -> String {
        self.0.as_str().to_owned()
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

/// A decrypted secret key and its public key. `Debug` shows only the
/// public key.
pub struct SigningKey {
    secret: SecretKey,
    public: String,
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}

fn wrap_public(public: &PublicKey) -> Result<String> {
    let text = public
        .to_box()
        .map_err(|e| PublishError(format!("public key: {e}")))?
        .to_string();
    Ok(STANDARD.encode(text))
}

/// The public key file written next to a secret key.
pub fn public_path(secret: &Path) -> PathBuf {
    let mut name = secret.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}

fn create_new(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options
        .open(path)
        .map_err(|e| PublishError(format!("{}: {e}", path.display())))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

/// Generates a key pair: the secret key at `out` (mode 0600, never
/// overwritten) and the public key at `<out>.pub`. Returns the public key.
/// `password: None` writes an unencrypted secret key.
pub fn generate_key(role: KeyRole, out: &Path, password: Option<&Password>) -> Result<String> {
    let public_out = public_path(out);
    for path in [out, public_out.as_path()] {
        if path.exists() {
            return fail(format!(
                "{} already exists; refusing to overwrite",
                path.display()
            ));
        }
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| PublishError(format!("{}: {e}", dir.display())))?;
    }
    let pair = match password {
        Some(password) => KeyPair::generate_encrypted_keypair(Some(password.reveal())),
        None => KeyPair::generate_unencrypted_keypair(),
    }
    .map_err(|e| PublishError(format!("key generation failed: {e}")))?;
    let comment = format!("deckpilot-registry {} secret key", role.as_str());
    let secret = pair
        .sk
        .to_box(Some(&comment))
        .map_err(|e| PublishError(format!("secret key: {e}")))?
        .into_string();
    let public = wrap_public(&pair.pk)?;
    create_new(out, Zeroizing::new(secret).as_bytes(), true)?;
    create_new(&public_out, format!("{public}\n").as_bytes(), false)?;
    Ok(public)
}

/// Reads a public key file (`<key>.pub`, base64-wrapped minisign).
pub fn read_public(path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| PublishError(format!("{}: {e}", path.display())))?;
    let key = text.trim().to_owned();
    key_identity(&key)
        .map_err(|e| PublishError(format!("{}: not a public key: {e}", path.display())))?;
    Ok(key)
}

impl SigningKey {
    /// Loads a secret key file. An encrypted key needs `password`; an
    /// unencrypted key ignores it.
    pub fn load(path: &Path, password: Option<&Password>) -> Result<Self> {
        let text = Zeroizing::new(
            std::fs::read_to_string(path)
                .map_err(|e| PublishError(format!("{}: {e}", path.display())))?,
        );
        let bad = || PublishError(format!("{}: not a minisign secret key", path.display()));
        let encoded = text.lines().nth(1).ok_or_else(bad)?;
        let raw = Zeroizing::new(STANDARD.decode(encoded.trim()).map_err(|_| bad())?);
        let encrypted = SecretKey::from_bytes(&raw)
            .map_err(|_| bad())?
            .is_encrypted();
        let secret_box = SecretKeyBox::from_string(&text).map_err(|_| bad())?;
        let secret = if encrypted {
            let Some(password) = password else {
                return fail(format!(
                    "{} is encrypted; give its password with --password-stdin or an \
                     environment variable (--password-env)",
                    path.display()
                ));
            };
            SecretKey::from_box(secret_box, Some(password.reveal()))
        } else {
            SecretKey::from_unencrypted_box(secret_box)
        }
        .map_err(|e| PublishError(format!("{}: {e}", path.display())))?;
        let public = PublicKey::from_secret_key(&secret)
            .map_err(|e| PublishError(format!("{}: {e}", path.display())))?;
        let public = wrap_public(&public)?;
        Ok(Self { secret, public })
    }

    /// The base64-wrapped public key.
    pub fn public(&self) -> &str {
        &self.public
    }

    /// The Ed25519 key, as the client compares keys.
    pub fn identity(&self) -> [u8; 32] {
        key_identity(&self.public).unwrap_or([0; 32])
    }

    /// A base64-wrapped minisign signature of `data` with `trusted_comment`.
    pub fn sign(&self, data: &[u8], trusted_comment: &str) -> Result<String> {
        let signature = minisign::sign(
            None,
            &self.secret,
            std::io::Cursor::new(data),
            Some(trusted_comment),
            Some("signature from deckpilot-registry"),
        )
        .map_err(|e| PublishError(format!("signing failed: {e}")))?;
        Ok(STANDARD.encode(signature.to_string()))
    }
}
