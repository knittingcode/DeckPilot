//! The real credential store of this platform (macOS Keychain, Windows
//! Credential Manager, Secret Service on Linux).
//!
//! Only runs with `DECKPILOT_TEST_SYSTEM_KEYSTORE=1`, because it writes to
//! the user's credential store: CI sets it. Every entry is created under a
//! new throwaway service name (`DeckPilot Test <random>`) and deleted again,
//! also when an assertion fails; entries of an installed or development
//! DeckPilot are never read, listed or changed. Stored values are compared,
//! never printed.

use deckpilot_keystore::{
    KEY_LEN, KeyId, KeyStore, KeyStoreError, Secret, SystemKeyStore, TEST_SERVICE_PREFIX,
};

const ENABLE_ENV: &str = "DECKPILOT_TEST_SYSTEM_KEYSTORE";
const CHILD_SERVICE_ENV: &str = "DECKPILOT_TEST_KEYSTORE_SERVICE";
const CHILD_ID_ENV: &str = "DECKPILOT_TEST_KEYSTORE_ID";
const CHILD_EXPECTED_ENV: &str = "DECKPILOT_TEST_KEYSTORE_EXPECTED";

fn enabled() -> bool {
    if std::env::var(ENABLE_ENV).is_ok_and(|value| value == "1") {
        true
    } else {
        eprintln!("skipped: set {ENABLE_ENV}=1 to use the real credential store");
        false
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).unwrap();
    hex(&buffer)
}

/// A key stored under a throwaway service, deleted on drop.
struct Entry {
    service: String,
    id: KeyId,
    secret: Secret,
}

impl Entry {
    fn store(&self) -> SystemKeyStore {
        SystemKeyStore::open_for_tests(&self.service).unwrap()
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        let _ = self.store().delete(&self.id);
    }
}

fn new_entry() -> Entry {
    let mut secret = vec![0u8; KEY_LEN];
    getrandom::fill(&mut secret).unwrap();
    Entry {
        service: format!("{TEST_SERVICE_PREFIX}{}", random_hex(8)),
        id: KeyId::parse(&random_hex(16)).unwrap(),
        secret: Secret::new(secret),
    }
}

#[test]
fn a_key_is_kept_across_processes_and_removed_on_request() {
    if !enabled() {
        return;
    }
    let entry = new_entry();
    let store = entry.store();
    assert!(matches!(
        store.load(&entry.id),
        Err(KeyStoreError::NotFound { .. })
    ));
    store.store(&entry.id, &entry.secret).unwrap();
    assert_eq!(store.load(&entry.id).unwrap(), entry.secret);
    // Never overwritten.
    let other = Secret::new(vec![0x5a; KEY_LEN]);
    assert!(matches!(
        store.store(&entry.id, &other),
        Err(KeyStoreError::AlreadyExists { .. })
    ));
    assert_eq!(store.load(&entry.id).unwrap(), entry.secret);

    // A new connection, then a new process (as after a restart or an
    // update of the agent) find the same key.
    assert_eq!(entry.store().load(&entry.id).unwrap(), entry.secret);
    let expected = hex(entry.secret.expose());
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_loads_the_key", "--ignored"])
        .env(ENABLE_ENV, "1")
        .env(CHILD_SERVICE_ENV, &entry.service)
        .env(CHILD_ID_ENV, entry.id.as_str())
        .env(CHILD_EXPECTED_ENV, expected)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{:?}\n{stdout}",
        output.status
    );

    // Another service does not see it.
    let elsewhere = new_entry();
    assert!(matches!(
        elsewhere.store().load(&entry.id),
        Err(KeyStoreError::NotFound { .. })
    ));

    store.delete(&entry.id).unwrap();
    assert!(matches!(
        store.load(&entry.id),
        Err(KeyStoreError::NotFound { .. })
    ));
    // Deleting a missing key is not an error.
    store.delete(&entry.id).unwrap();
}

/// Run by the test above in a child process.
#[test]
#[ignore = "runs in a child process of a_key_is_kept_across_processes_and_removed_on_request"]
fn child_loads_the_key() {
    let (Ok(service), Ok(id), Ok(expected)) = (
        std::env::var(CHILD_SERVICE_ENV),
        std::env::var(CHILD_ID_ENV),
        std::env::var(CHILD_EXPECTED_ENV),
    ) else {
        // Not started by the test above.
        return;
    };
    let store = SystemKeyStore::open_for_tests(&service).unwrap();
    let loaded = store.load(&KeyId::parse(&id).unwrap()).unwrap();
    assert!(
        hex(loaded.expose()) == expected,
        "the key read by another process differs"
    );
}

#[test]
fn a_key_that_is_not_a_database_key_is_refused() {
    if !enabled() {
        return;
    }
    let entry = new_entry();
    let store = entry.store();
    store.store(&entry.id, &Secret::new(vec![1; 7])).unwrap();
    assert!(matches!(
        store.load(&entry.id),
        Err(KeyStoreError::Failed(_))
    ));
}
