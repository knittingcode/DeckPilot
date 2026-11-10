//! Protected storage for DeckPilot's database key.
//!
//! In the default *system-managed* protection mode the configuration
//! database is encrypted with a random key that is kept in the operating
//! system's credential store, protected by the user's login:
//!
//! | Platform | Store                                                        |
//! | -------- | ------------------------------------------------------------ |
//! | macOS    | the login Keychain (generic password item)                   |
//! | Windows  | Credential Manager, encrypted per user with DPAPI (local, not roaming) |
//! | Linux    | the Secret Service (GNOME Keyring, KDE Wallet) over D-Bus    |
//!
//! There is deliberately no fallback: if the platform store is not
//! available, [`SystemKeyStore::open`] fails and the caller must ask the user
//! to choose password protection instead. A key is never written to a file.
//!
//! Each key has its own random [`KeyId`]; storing never overwrites an
//! existing entry, so a key that protects a database cannot be replaced by
//! accident. Development and test builds use a separate service name
//! ([`Scope`]), so they never touch the keys of an installed DeckPilot.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use zeroize::Zeroizing;

/// Length of a database key in bytes.
pub const KEY_LEN: usize = 32;

/// Raw key bytes; zeroized when dropped and redacted from `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Identifies one key in the store. Random, not secret, recorded in the
/// vault file next to the database.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyId(String);

impl KeyId {
    /// Accepts 8–64 lowercase hex digits.
    pub fn parse(value: &str) -> Result<Self, KeyStoreError> {
        let valid = (8..=64).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(KeyStoreError::Failed(format!(
                "invalid key identifier `{value}`"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The account name of the entry in the platform store.
    fn account(&self) -> String {
        format!("database-key-{}", self.0)
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    /// The store holds no key with this identifier.
    #[error("the database key is not in the {store}")]
    NotFound { store: String },
    /// Refused: an entry with this identifier exists already.
    #[error("a key with this identifier already exists in the {store}")]
    AlreadyExists { store: String },
    /// The store could not be reached at all (no D-Bus session, the
    /// keyring daemon not running yet). Retrying later can succeed without
    /// involving the user.
    #[error("the {store} cannot be reached: {reason}")]
    Unreachable { store: String, reason: String },
    /// The store refused or failed the operation (locked, access denied, a
    /// prompt was cancelled). Never retried automatically: that could
    /// prompt the user again and again.
    #[error("the {store} is not available: {reason}")]
    Unavailable { store: String, reason: String },
    #[error("{0}")]
    Failed(String),
}

/// Where keys of this build are kept, so that development and test builds
/// never read or write the keys of an installed DeckPilot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Production,
    Development,
}

impl Scope {
    /// The service name of entries in the platform store.
    pub fn service(self) -> &'static str {
        match self {
            Self::Production => "DeckPilot",
            Self::Development => "DeckPilot Development",
        }
    }
}

/// A store for database keys.
pub trait KeyStore: Send + Sync + fmt::Debug {
    /// Human-readable name, e.g. "macOS Keychain".
    fn name(&self) -> String;
    fn load(&self, id: &KeyId) -> Result<Secret, KeyStoreError>;
    /// Stores a new key. Never overwrites: fails with
    /// [`KeyStoreError::AlreadyExists`] if `id` is taken.
    fn store(&self, id: &KeyId, secret: &Secret) -> Result<(), KeyStoreError>;
    fn delete(&self, id: &KeyId) -> Result<(), KeyStoreError>;
}

/// The name of this platform's credential store.
pub fn platform_store_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS Keychain"
    } else if cfg!(target_os = "windows") {
        "Windows Credential Manager"
    } else if cfg!(target_os = "linux") {
        "Secret Service"
    } else {
        "system credential store"
    }
}

/// The operating system's credential store. The connection is made on
/// first use and again after a failed attempt, so a store that becomes
/// available after the agent started (e.g. a keyring daemon that starts
/// late at login) is picked up.
pub struct SystemKeyStore {
    store: Mutex<Option<Arc<keyring_core::CredentialStore>>>,
    scope: Scope,
}

impl fmt::Debug for SystemKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemKeyStore")
            .field("store", &platform_store_name())
            .field("scope", &self.scope)
            .finish()
    }
}

impl SystemKeyStore {
    /// The store of this platform. Fails only on platforms without one;
    /// whether the store can be reached is found out on first use.
    pub fn open(scope: Scope) -> Result<Self, KeyStoreError> {
        if !SUPPORTED {
            return Err(KeyStoreError::Unavailable {
                store: platform_store_name().into(),
                reason: "this platform has no supported credential store".into(),
            });
        }
        Ok(Self {
            store: Mutex::new(None),
            scope,
        })
    }

    fn connection(&self) -> Result<Arc<keyring_core::CredentialStore>, KeyStoreError> {
        let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(store) = store.as_ref() {
            return Ok(store.clone());
        }
        let connected = platform_store().map_err(|error| KeyStoreError::Unreachable {
            store: platform_store_name().into(),
            reason: error.to_string(),
        })?;
        *store = Some(connected.clone());
        Ok(connected)
    }

    fn entry(&self, id: &KeyId) -> Result<keyring_core::Entry, KeyStoreError> {
        let account = id.account();
        let store = self.connection()?;
        #[cfg(target_os = "windows")]
        let entry = {
            // Local persistence: the key stays on this computer and is not
            // copied to other machines with a roaming profile.
            let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
            store.build(self.scope.service(), &account, Some(&modifiers))
        };
        #[cfg(target_os = "linux")]
        let entry = {
            // The label shown in Seahorse or KWallet.
            let label = format!("{} database key", self.scope.service());
            let modifiers = std::collections::HashMap::from([("label", label.as_str())]);
            store.build(self.scope.service(), &account, Some(&modifiers))
        };
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let entry = store.build(self.scope.service(), &account, None);
        entry.map_err(|error| failure(&error))
    }
}

const SUPPORTED: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));

#[cfg(target_os = "macos")]
fn platform_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(target_os = "windows")]
fn platform_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(target_os = "linux")]
fn platform_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    Err(keyring_core::Error::NotSupportedByStore(
        "no credential store on this platform".into(),
    ))
}

fn unavailable(error: &keyring_core::Error) -> KeyStoreError {
    KeyStoreError::Unavailable {
        store: platform_store_name().into(),
        reason: error.to_string(),
    }
}

fn failure(error: &keyring_core::Error) -> KeyStoreError {
    match error {
        keyring_core::Error::NoEntry => KeyStoreError::NotFound {
            store: platform_store_name().into(),
        },
        keyring_core::Error::NoStorageAccess(_)
        | keyring_core::Error::PlatformFailure(_)
        | keyring_core::Error::NoDefaultStore
        | keyring_core::Error::NotSupportedByStore(_) => unavailable(error),
        other => KeyStoreError::Failed(format!("{}: {other}", platform_store_name())),
    }
}

impl KeyStore for SystemKeyStore {
    fn name(&self) -> String {
        platform_store_name().into()
    }

    fn load(&self, id: &KeyId) -> Result<Secret, KeyStoreError> {
        let secret = self
            .entry(id)?
            .get_secret()
            .map(Secret::new)
            .map_err(|error| failure(&error))?;
        if secret.expose().len() == KEY_LEN {
            Ok(secret)
        } else {
            Err(KeyStoreError::Failed(format!(
                "the key in the {} has an unexpected length",
                platform_store_name()
            )))
        }
    }

    fn store(&self, id: &KeyId, secret: &Secret) -> Result<(), KeyStoreError> {
        let entry = self.entry(id)?;
        match entry.get_secret() {
            Ok(_) => {
                return Err(KeyStoreError::AlreadyExists { store: self.name() });
            }
            Err(keyring_core::Error::NoEntry) => {}
            Err(error) => return Err(failure(&error)),
        }
        entry
            .set_secret(secret.expose())
            .map_err(|error| failure(&error))
    }

    fn delete(&self, id: &KeyId) -> Result<(), KeyStoreError> {
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(failure(&error)),
        }
    }
}

/// A store in process memory, for tests. It can simulate an unavailable
/// store. Nothing is persisted.
#[derive(Debug, Clone, Default)]
pub struct MemoryKeyStore {
    inner: Arc<Mutex<MemoryState>>,
}

#[derive(Debug, Default)]
struct MemoryState {
    entries: BTreeMap<KeyId, Secret>,
    unavailable: bool,
    unreachable: bool,
    /// Operations attempted, to check that nothing retries in a loop.
    attempts: usize,
}

impl MemoryKeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, MemoryState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes every operation fail as if the store were not reachable.
    pub fn set_unavailable(&self, unavailable: bool) {
        self.state().unavailable = unavailable;
    }

    /// Makes every operation fail as if the store could not be connected
    /// to yet (e.g. a keyring daemon that starts late).
    pub fn set_unreachable(&self, unreachable: bool) {
        self.state().unreachable = unreachable;
    }

    /// Number of operations attempted so far.
    pub fn attempts(&self) -> usize {
        self.state().attempts
    }

    /// Identifiers of the stored keys.
    pub fn ids(&self) -> Vec<KeyId> {
        self.state().entries.keys().cloned().collect()
    }

    /// Removes every key, as if the store had been reset.
    pub fn clear(&self) {
        self.state().entries.clear();
    }

    fn check(state: &mut MemoryState) -> Result<(), KeyStoreError> {
        state.attempts += 1;
        if state.unreachable {
            return Err(KeyStoreError::Unreachable {
                store: "test key store".into(),
                reason: "simulated".into(),
            });
        }
        if state.unavailable {
            Err(KeyStoreError::Unavailable {
                store: "test key store".into(),
                reason: "simulated".into(),
            })
        } else {
            Ok(())
        }
    }
}

impl KeyStore for MemoryKeyStore {
    fn name(&self) -> String {
        "test key store".into()
    }

    fn load(&self, id: &KeyId) -> Result<Secret, KeyStoreError> {
        let mut state = self.state();
        Self::check(&mut state)?;
        state
            .entries
            .get(id)
            .cloned()
            .ok_or_else(|| KeyStoreError::NotFound { store: self.name() })
    }

    fn store(&self, id: &KeyId, secret: &Secret) -> Result<(), KeyStoreError> {
        let mut state = self.state();
        Self::check(&mut state)?;
        if state.entries.contains_key(id) {
            return Err(KeyStoreError::AlreadyExists { store: self.name() });
        }
        state.entries.insert(id.clone(), secret.clone());
        Ok(())
    }

    fn delete(&self, id: &KeyId) -> Result<(), KeyStoreError> {
        let mut state = self.state();
        Self::check(&mut state)?;
        state.entries.remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> KeyId {
        KeyId::parse(value).unwrap()
    }

    #[test]
    fn key_ids_are_validated() {
        assert!(KeyId::parse("0123abcd").is_ok());
        assert!(KeyId::parse("short").is_err());
        assert!(KeyId::parse("0123ABCD").is_err());
        assert!(KeyId::parse("../../etc").is_err());
        assert_eq!(id("0123abcd").account(), "database-key-0123abcd");
    }

    #[test]
    fn memory_store_never_overwrites() {
        let store = MemoryKeyStore::new();
        let key = Secret::new(vec![1; KEY_LEN]);
        store.store(&id("aaaaaaaa"), &key).unwrap();
        let other = Secret::new(vec![2; KEY_LEN]);
        assert!(matches!(
            store.store(&id("aaaaaaaa"), &other),
            Err(KeyStoreError::AlreadyExists { .. })
        ));
        assert_eq!(store.load(&id("aaaaaaaa")).unwrap(), key);
    }

    #[test]
    fn memory_store_simulates_failures() {
        let store = MemoryKeyStore::new();
        assert!(matches!(
            store.load(&id("aaaaaaaa")),
            Err(KeyStoreError::NotFound { .. })
        ));
        store.set_unavailable(true);
        assert!(matches!(
            store.store(&id("aaaaaaaa"), &Secret::new(vec![0; KEY_LEN])),
            Err(KeyStoreError::Unavailable { .. })
        ));
    }

    #[test]
    fn secrets_are_redacted() {
        let secret = Secret::new(vec![7; KEY_LEN]);
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    }

    #[test]
    fn scopes_are_isolated() {
        assert_ne!(Scope::Production.service(), Scope::Development.service());
    }
}
