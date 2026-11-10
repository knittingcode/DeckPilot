//! Key material and password-based key derivation.
//!
//! DeckPilot implements no cryptographic primitives. Passwords are turned
//! into 256-bit keys with Argon2id (the `RustCrypto` `argon2` crate); the database key is
//! handed to SQLCipher as a raw key, so SQLCipher's own PBKDF2 passphrase
//! derivation is not used and there is exactly one password-hashing step.
//! Secrets live in zeroizing buffers and are redacted from `Debug` output.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

/// Length of derived keys in bytes (256 bits).
pub const KEY_LEN: usize = 32;
/// Length of random salts in bytes.
pub const SALT_LEN: usize = 16;
/// Shortest accepted password.
pub const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_CHARS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CryptoError {
    #[error("the password must be at least {MIN_PASSWORD_CHARS} characters long")]
    PasswordTooShort,
    #[error("the password is too long")]
    PasswordTooLong,
    #[error("the passwords do not match")]
    ConfirmationMismatch,
    #[error("unsupported key derivation parameters: {0}")]
    UnsupportedKdf(String),
    #[error("key derivation failed")]
    Derivation,
    #[error("the system random number generator failed")]
    Random,
    #[error("invalid key encoding")]
    Encoding,
}

/// Fills `buffer` from the operating system's secure random generator.
pub fn random_bytes(buffer: &mut [u8]) -> Result<(), CryptoError> {
    getrandom::fill(buffer).map_err(|_| CryptoError::Random)
}

/// A password typed by the user. Zeroized when dropped, never printed.
#[derive(Clone)]
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Checks a new password and its confirmation.
    pub fn validate_new(&self, confirmation: &Password) -> Result<(), CryptoError> {
        let length = self.0.chars().count();
        if length < MIN_PASSWORD_CHARS {
            return Err(CryptoError::PasswordTooShort);
        }
        if length > MAX_PASSWORD_CHARS {
            return Err(CryptoError::PasswordTooLong);
        }
        if !constant_time_eq(self.0.as_bytes(), confirmation.0.as_bytes()) {
            return Err(CryptoError::ConfirmationMismatch);
        }
        Ok(())
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for Password {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(Self(Zeroizing::new(value)))
    }
}

/// A 256-bit key. Zeroized when dropped, never printed.
#[derive(Clone)]
pub struct SecretKey(Zeroizing<[u8; KEY_LEN]>);

impl SecretKey {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// A fresh random key.
    pub fn random() -> Result<Self, CryptoError> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        random_bytes(bytes.as_mut())?;
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Lowercase hex in a zeroizing buffer.
    pub fn to_hex(&self) -> Zeroizing<String> {
        let mut out = Zeroizing::new(String::with_capacity(KEY_LEN * 2));
        for byte in self.0.iter() {
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        out
    }

    pub fn from_hex(text: &str) -> Result<Self, CryptoError> {
        let bytes = text.as_bytes();
        if bytes.len() != KEY_LEN * 2 {
            return Err(CryptoError::Encoding);
        }
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
            let high = hex_value(pair[0]).ok_or(CryptoError::Encoding)?;
            let low = hex_value(pair[1]).ok_or(CryptoError::Encoding)?;
            key[index] = (high << 4) | low;
        }
        Ok(Self(key))
    }

    /// Compares two keys without an early exit.
    pub fn matches(&self, other: &SecretKey) -> bool {
        constant_time_eq(self.0.as_ref(), other.0.as_ref())
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(<redacted>)")
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// Equality without data-dependent early exit.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Argon2id parameters, stored next to the data they protect. None of these
/// values is secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KdfParams {
    /// Always `argon2id`.
    pub algorithm: String,
    /// Argon2 version, `19` (0x13).
    pub version: u32,
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    /// Base64, at least 16 random bytes.
    pub salt: String,
}

/// Default cost: 64 MiB, 3 passes, 1 lane. Above the OWASP minimum
/// (19 MiB, 2 passes) and well under a second on current hardware.
pub const DEFAULT_MEMORY_KIB: u32 = 64 * 1024;
pub const DEFAULT_ITERATIONS: u32 = 3;
pub const DEFAULT_PARALLELISM: u32 = 1;

// Limits applied before deriving with parameters read from a file, so that a
// crafted export archive cannot demand unbounded memory or time.
const MIN_MEMORY_KIB: u32 = 8 * 1024;
const MAX_MEMORY_KIB: u32 = 1024 * 1024;
const MAX_ITERATIONS: u32 = 16;
const MAX_PARALLELISM: u32 = 8;

impl KdfParams {
    /// Production parameters with a fresh random salt.
    pub fn generate() -> Result<Self, CryptoError> {
        Self::generate_with(DEFAULT_MEMORY_KIB, DEFAULT_ITERATIONS)
    }

    /// Parameters with a fresh salt and the given cost. Tests use the
    /// cheapest accepted cost to stay fast.
    pub fn generate_with(memory_kib: u32, iterations: u32) -> Result<Self, CryptoError> {
        let mut salt = [0u8; SALT_LEN];
        random_bytes(&mut salt)?;
        Ok(Self {
            algorithm: "argon2id".into(),
            version: 0x13,
            memory_kib,
            iterations,
            parallelism: DEFAULT_PARALLELISM,
            salt: BASE64.encode(salt),
        })
    }

    /// The cheapest parameters accepted, for tests.
    ///
    /// # Panics
    ///
    /// If the system random generator fails.
    pub fn for_tests() -> Self {
        Self::generate_with(MIN_MEMORY_KIB, 1).expect("random generator available")
    }

    fn checked(&self) -> Result<(argon2::Params, Vec<u8>), CryptoError> {
        let unsupported = |what: &str| Err(CryptoError::UnsupportedKdf(what.into()));
        if self.algorithm != "argon2id" {
            return unsupported("algorithm");
        }
        if self.version != 0x13 {
            return unsupported("version");
        }
        if !(MIN_MEMORY_KIB..=MAX_MEMORY_KIB).contains(&self.memory_kib) {
            return unsupported("memory");
        }
        if !(1..=MAX_ITERATIONS).contains(&self.iterations) {
            return unsupported("iterations");
        }
        if !(1..=MAX_PARALLELISM).contains(&self.parallelism) {
            return unsupported("parallelism");
        }
        let salt = BASE64
            .decode(&self.salt)
            .map_err(|_| CryptoError::UnsupportedKdf("salt".into()))?;
        if !(SALT_LEN..=64).contains(&salt.len()) {
            return unsupported("salt");
        }
        let params = argon2::Params::new(
            self.memory_kib,
            self.iterations,
            self.parallelism,
            Some(KEY_LEN),
        )
        .map_err(|_| CryptoError::UnsupportedKdf("parameters".into()))?;
        Ok((params, salt))
    }

    /// Derives a 256-bit key from `password`.
    pub fn derive(&self, password: &Password) -> Result<SecretKey, CryptoError> {
        let (params, salt) = self.checked()?;
        let argon =
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        argon
            .hash_password_into(password.expose().as_bytes(), &salt, key.as_mut())
            .map_err(|_| CryptoError::Derivation)?;
        Ok(SecretKey(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_is_deterministic_per_salt() {
        let params = KdfParams::for_tests();
        let password = Password::new("correct horse");
        let a = params.derive(&password).unwrap();
        let b = params.derive(&password).unwrap();
        assert!(a.matches(&b));
        let other_salt = KdfParams::for_tests();
        assert!(!a.matches(&other_salt.derive(&password).unwrap()));
        assert!(!a.matches(&params.derive(&Password::new("wrong horse")).unwrap()));
    }

    #[test]
    fn parameters_are_bounded() {
        let mut params = KdfParams::for_tests();
        params.memory_kib = 4 * 1024 * 1024;
        assert!(matches!(
            params.derive(&Password::new("password")),
            Err(CryptoError::UnsupportedKdf(_))
        ));
        let mut params = KdfParams::for_tests();
        params.algorithm = "pbkdf2".into();
        assert!(params.derive(&Password::new("password")).is_err());
        let mut params = KdfParams::for_tests();
        params.salt = BASE64.encode([0u8; 4]);
        assert!(params.derive(&Password::new("password")).is_err());
    }

    #[test]
    fn keys_round_trip_through_hex_and_are_redacted() {
        let key = SecretKey::random().unwrap();
        let parsed = SecretKey::from_hex(&key.to_hex()).unwrap();
        assert!(key.matches(&parsed));
        assert!(SecretKey::from_hex("zz").is_err());
        assert_eq!(format!("{key:?}"), "SecretKey(<redacted>)");
        assert_eq!(
            format!("{:?}", Password::new("hunter22")),
            "Password(<redacted>)"
        );
    }

    #[test]
    fn new_passwords_are_validated() {
        let p = |s: &str| Password::new(s);
        assert_eq!(
            p("short").validate_new(&p("short")),
            Err(CryptoError::PasswordTooShort)
        );
        assert_eq!(
            p("long enough").validate_new(&p("long enougH")),
            Err(CryptoError::ConfirmationMismatch)
        );
        assert_eq!(p("long enough").validate_new(&p("long enough")), Ok(()));
    }
}
