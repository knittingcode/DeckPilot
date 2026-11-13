use super::*;
use crate::ConfigStore;
use deckpilot_core::{
    ActionParameters, MachineConfig, MachineResourceBinding, ModuleId, ModuleSettings, Os,
    ParameterValue, Profile, ProfileId, Project, ProjectKey, ResourceKey, ResourceKind,
};

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("deckpilot-vault-{}", ProfileId::new()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn vault(&self) -> Vault {
        Vault::new(self.0.join("deckpilot.db")).with_test_cost()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn password(text: &str) -> Password {
    Password::new(text)
}

const MARKER: &str = "Distinctive Profile Name 7f3a";

/// Profile, module settings and a machine binding, for round-trip checks.
fn fill(store: &SqliteStore) -> Profile {
    let mut profile = Profile::new(MARKER);
    let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    project
        .resources
        .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
    profile.projects.push(project);
    store.save_profile(&profile).unwrap();
    store.set_active_profile(Some(profile.id)).unwrap();
    store
        .set_module_state(
            &ModuleId::new("projects").unwrap(),
            &ModuleSettings {
                enabled: false,
                config: ActionParameters::new().with("wrapAround", ParameterValue::Boolean(false)),
            },
        )
        .unwrap();
    let mut machine = MachineConfig::new(Os::Linux);
    machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new("shop.repo").unwrap(),
        kind: ResourceKind::Directory,
        value: "/src/shop".into(),
    });
    store.save_machine_config(&machine).unwrap();
    profile
}

fn assert_filled(store: &SqliteStore, profile: &Profile) {
    assert_eq!(
        store.load_profile(profile.id).unwrap().as_ref(),
        Some(profile)
    );
    assert_eq!(store.active_profile().unwrap(), Some(profile.id));
    let modules = store.module_states().unwrap();
    assert!(!modules[&ModuleId::new("projects").unwrap()].enabled);
    let machine = store.load_machine_config().unwrap().unwrap();
    assert_eq!(machine.bindings[0].value, "/src/shop");
}

fn file_contains(path: &Path, needle: &str) -> bool {
    let bytes = fs::read(path).unwrap();
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn open(vault: &Vault, key: &DatabaseKey) -> SqliteStore {
    SqliteStore::open(vault.database(), &Encryption::Key(key.clone())).unwrap()
}

#[test]
fn a_new_database_is_encrypted_and_needs_the_password() {
    let dir = Dir::new();
    let vault = dir.vault();
    assert_eq!(vault.state().unwrap(), DatabaseState::Missing);
    let key = vault.initialize(&password("correct horse")).unwrap();
    assert_eq!(vault.state().unwrap(), DatabaseState::Encrypted);
    let profile = {
        let store = open(&vault, &key);
        let profile = fill(&store);
        store
            .connection_for_tests()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        profile
    };
    assert!(!file_contains(vault.database(), MARKER));
    assert!(!file_contains(vault.database(), "SQLite format 3"));

    let unlocked = vault.unlock(&password("correct horse")).unwrap();
    assert!(unlocked.matches(&key));
    assert_filled(&open(&vault, &unlocked), &profile);

    assert!(matches!(
        vault.unlock(&password("wrong horse")),
        Err(StorageError::InvalidKey)
    ));
    // A wrong key leaves the data intact.
    assert_filled(&open(&vault, &key), &profile);
    assert!(SqliteStore::open(vault.database(), &Encryption::None).is_err());
    // The key file holds no secret.
    let params = fs::read_to_string(vault.vault_file()).unwrap();
    assert!(!params.contains("correct horse"));
    assert!(params.contains("argon2id"));

    assert!(vault.initialize(&password("another one")).is_err());
}

#[test]
fn changing_the_password_rekeys_without_losing_data() {
    let dir = Dir::new();
    let vault = dir.vault();
    let key = vault.initialize(&password("first password")).unwrap();
    let profile = fill(&open(&vault, &key));

    let new_key = vault
        .change_password(&password("first password"), &password("second password"))
        .unwrap();
    assert!(!new_key.matches(&key));
    assert!(matches!(
        vault.unlock(&password("first password")),
        Err(StorageError::InvalidKey)
    ));
    let unlocked = vault.unlock(&password("second password")).unwrap();
    assert_filled(&open(&vault, &unlocked), &profile);
    for leftover in [
        vault.replacement(),
        vault.previous(),
        vault.pending_vault_file(),
    ] {
        assert!(!leftover.exists(), "{}", leftover.display());
    }
    assert!(matches!(
        vault.change_password(&password("wrong"), &password("third password")),
        Err(StorageError::InvalidKey)
    ));
}

#[test]
fn an_interrupted_password_change_is_completed_or_discarded() {
    // Interrupted after the swap, before the new parameters were moved into
    // place: the new password works and the change is completed.
    let dir = Dir::new();
    let vault = dir.vault();
    let key = vault.initialize(&password("old password")).unwrap();
    let profile = fill(&open(&vault, &key));
    let old_params = fs::read(vault.vault_file()).unwrap();
    vault
        .change_password(&password("old password"), &password("new password"))
        .unwrap();
    fs::rename(vault.vault_file(), vault.pending_vault_file()).unwrap();
    fs::write(vault.vault_file(), &old_params).unwrap();
    let key = vault.unlock(&password("new password")).unwrap();
    assert!(!vault.pending_vault_file().exists());
    assert_filled(&open(&vault, &key), &profile);

    // Interrupted before the swap: the old password still works and the
    // half-written replacement is removed.
    fs::write(vault.replacement(), b"partial").unwrap();
    fs::write(vault.pending_vault_file(), &old_params).unwrap();
    let key = vault.unlock(&password("new password")).unwrap();
    assert!(!vault.replacement().exists());
    assert!(!vault.pending_vault_file().exists());
    assert_filled(&open(&vault, &key), &profile);
}

#[test]
fn an_interrupted_swap_is_completed() {
    let dir = Dir::new();
    let vault = dir.vault();
    let key = vault.initialize(&password("some password")).unwrap();
    let profile = fill(&open(&vault, &key));
    // Crash between "move the database aside" and "move the replacement in".
    fs::rename(vault.database(), vault.replacement()).unwrap();
    assert_eq!(vault.state().unwrap(), DatabaseState::Encrypted);
    assert_filled(&open(&vault, &key), &profile);
}

fn plaintext_database(vault: &Vault) -> Profile {
    let store = SqliteStore::open(vault.database(), &Encryption::None).unwrap();
    let profile = fill(&store);
    // Leave data in the write-ahead log, as a running app would.
    profile
}

#[test]
fn legacy_plaintext_databases_are_encrypted_and_kept() {
    let dir = Dir::new();
    let vault = dir.vault();
    let profile = plaintext_database(&vault);
    assert_eq!(vault.state().unwrap(), DatabaseState::Plaintext);

    let (key, legacy) = vault
        .encrypt_plaintext(&password("migration password"))
        .unwrap();
    assert_eq!(vault.state().unwrap(), DatabaseState::Encrypted);
    assert!(!file_contains(vault.database(), MARKER));
    assert!(
        !sibling(vault.database(), "-wal").exists() || {
            // Any side file now belongs to the encrypted database.
            !file_contains(&sibling(vault.database(), "-wal"), MARKER)
        }
    );
    assert_filled(&open(&vault, &key), &profile);

    // The original is kept, unencrypted, until the user removes it.
    assert_eq!(legacy, vault.legacy_copy_path());
    assert!(file_contains(&legacy, MARKER));
    assert_eq!(vault.legacy_copies(), vec![legacy.clone()]);
    assert_eq!(vault.remove_legacy_copies().unwrap(), 1);
    assert!(!legacy.exists());
    assert!(vault.unlock(&password("migration password")).is_ok());
}

#[test]
fn an_interrupted_legacy_migration_can_be_repeated() {
    let dir = Dir::new();
    let vault = dir.vault();
    let profile = plaintext_database(&vault);
    // A crash while writing the encrypted copy leaves a partial file and
    // perhaps the new parameters; the plain database is untouched.
    fs::write(vault.replacement(), b"partial encrypted copy").unwrap();
    fs::write(vault.vault_file(), b"{\"format\":1}").unwrap();
    assert_eq!(vault.state().unwrap(), DatabaseState::Plaintext);
    let (key, _) = vault
        .encrypt_plaintext(&password("second attempt"))
        .unwrap();
    assert_filled(&open(&vault, &key), &profile);
    assert!(!vault.replacement().exists());
}

#[test]
fn wrong_states_are_refused() {
    let dir = Dir::new();
    let vault = dir.vault();
    assert!(vault.unlock(&password("whatever")).is_err());
    assert!(vault.encrypt_plaintext(&password("whatever")).is_err());
    vault.initialize(&password("password one")).unwrap();
    assert!(vault.encrypt_plaintext(&password("whatever")).is_err());
}

// System-managed protection. The tests use an in-memory store and never
// touch the real credential store.

use deckpilot_keystore::MemoryKeyStore;

#[test]
fn a_system_managed_database_opens_without_a_password() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let key = vault.initialize_system(&keys).unwrap();
    assert_eq!(vault.state().unwrap(), DatabaseState::Encrypted);
    assert_eq!(
        vault.protection().unwrap().map(|p| p.mode()),
        Some(ProtectionMode::SystemManaged)
    );
    assert_eq!(keys.ids().len(), 1);
    let profile = fill(&open(&vault, &key));
    assert!(!file_contains(vault.database(), MARKER));

    // A restart: the key comes from the store.
    let unlocked = vault.unlock_system(&keys).unwrap();
    assert!(unlocked.matches(&key));
    assert_filled(&open(&vault, &unlocked), &profile);

    // The vault file names the key but does not contain it.
    let params = fs::read_to_string(vault.vault_file()).unwrap();
    assert!(params.contains("systemManaged"));
    assert!(!params.contains(key.to_hex().as_str()));

    // A password does not apply.
    assert!(matches!(
        vault.unlock(&password("anything at all")),
        Err(StorageError::CredentialsRequired(_))
    ));
    assert!(vault.initialize_system(&keys).is_err());
}

#[test]
fn an_unavailable_or_missing_system_key_leaves_the_database_locked() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let key = vault.initialize_system(&keys).unwrap();
    let profile = fill(&open(&vault, &key));

    keys.set_unavailable(true);
    assert!(matches!(
        vault.unlock_system(&keys),
        Err(StorageError::KeyStore(
            deckpilot_keystore::KeyStoreError::Unavailable { .. }
        ))
    ));
    keys.set_unavailable(false);

    let other = MemoryKeyStore::new();
    assert!(matches!(
        vault.unlock_system(&other),
        Err(StorageError::KeyStore(
            deckpilot_keystore::KeyStoreError::NotFound { .. }
        ))
    ));
    // Nothing was changed by the failed attempts.
    assert_filled(
        &open(&vault, &vault.unlock_system(&keys).unwrap()),
        &profile,
    );
}

#[test]
fn setup_fails_cleanly_when_the_store_is_unavailable() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    keys.set_unavailable(true);
    assert!(vault.initialize_system(&keys).is_err());
    assert_eq!(vault.state().unwrap(), DatabaseState::Missing);
    assert!(!vault.vault_file().exists());
    // The user can still choose a password.
    vault.initialize(&password("a password")).unwrap();
}

#[test]
fn switching_from_a_password_to_a_system_key_and_back() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let key = vault.initialize(&password("old password")).unwrap();
    let profile = fill(&open(&vault, &key));

    let system = vault.use_system_key(&key, &keys).unwrap();
    assert!(!system.matches(&key));
    assert_eq!(
        vault.protection().unwrap().map(|p| p.mode()),
        Some(ProtectionMode::SystemManaged)
    );
    assert!(vault.unlock(&password("old password")).is_err());
    assert_filled(
        &open(&vault, &vault.unlock_system(&keys).unwrap()),
        &profile,
    );

    let back = vault
        .use_password(&system, &password("new password"))
        .unwrap();
    assert_filled(&open(&vault, &back), &profile);
    assert!(vault.unlock(&password("new password")).is_ok());
    assert!(matches!(
        vault.unlock_system(&keys),
        Err(StorageError::CredentialsRequired(_))
    ));
    // The previous system key is kept, never deleted automatically.
    assert_eq!(keys.ids().len(), 1);
}

#[test]
fn a_failed_switch_keeps_the_password_protection() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let key = vault.initialize(&password("old password")).unwrap();
    let profile = fill(&open(&vault, &key));

    keys.set_unavailable(true);
    assert!(vault.use_system_key(&key, &keys).is_err());
    keys.set_unavailable(false);
    // A wrong current key is refused before anything is written.
    let wrong = SecretKey::random().unwrap();
    assert!(matches!(
        vault.use_system_key(&wrong, &keys),
        Err(StorageError::InvalidKey)
    ));
    assert_eq!(keys.ids(), Vec::new());
    assert_eq!(
        vault.protection().unwrap().map(|p| p.mode()),
        Some(ProtectionMode::Password)
    );
    assert_filled(
        &open(&vault, &vault.unlock(&password("old password")).unwrap()),
        &profile,
    );
}

#[test]
fn an_interrupted_switch_to_a_system_key_is_completed() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let key = vault.initialize(&password("old password")).unwrap();
    let profile = fill(&open(&vault, &key));
    let old_params = fs::read(vault.vault_file()).unwrap();
    vault.use_system_key(&key, &keys).unwrap();
    // Crash after the swap, before the new parameters were moved in place.
    fs::rename(vault.vault_file(), vault.pending_vault_file()).unwrap();
    fs::write(vault.vault_file(), &old_params).unwrap();

    // The agent starts with the credential store only.
    let key = vault.unlock_system(&keys).unwrap();
    assert!(!vault.pending_vault_file().exists());
    assert_eq!(
        vault.protection().unwrap().map(|p| p.mode()),
        Some(ProtectionMode::SystemManaged)
    );
    assert_filled(&open(&vault, &key), &profile);
}

#[test]
fn key_files_of_development_builds_are_still_read() {
    let dir = Dir::new();
    let vault = dir.vault();
    let key = vault.initialize(&password("old format")).unwrap();
    let Some(Protection::Password { kdf }) = vault.protection().unwrap() else {
        panic!("password protection expected");
    };
    let v1 = serde_json::json!({ "format": 1, "kdf": kdf });
    fs::write(vault.vault_file(), serde_json::to_vec(&v1).unwrap()).unwrap();
    assert!(vault.unlock(&password("old format")).unwrap().matches(&key));
    fs::write(vault.vault_file(), br#"{"format": 9}"#).unwrap();
    assert!(vault.unlock(&password("old format")).is_err());
}

#[test]
fn legacy_databases_can_be_encrypted_with_a_system_key() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let profile = plaintext_database(&vault);
    let (key, legacy) = vault.encrypt_plaintext_system(&keys).unwrap();
    assert!(legacy.exists());
    assert_eq!(vault.state().unwrap(), DatabaseState::Encrypted);
    assert!(vault.unlock_system(&keys).unwrap().matches(&key));
    assert_eq!(
        open(&vault, &key)
            .load_profile(profile.id)
            .unwrap()
            .as_ref(),
        Some(&profile)
    );
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn unencrypted_copies_are_readable_only_by_the_owner() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = Dir::new();
    let vault = dir.vault();
    plaintext_database(&vault);
    // The modes a default umask gives: the original and a copy left by an
    // earlier migration.
    fs::set_permissions(vault.database(), fs::Permissions::from_mode(0o644)).unwrap();
    let legacy = vault.legacy_copy_path();
    fs::write(&legacy, b"copy of an earlier migration").unwrap();
    fs::set_permissions(&legacy, fs::Permissions::from_mode(0o644)).unwrap();

    let (_, kept) = vault.encrypt_plaintext(&password("mode")).unwrap();
    assert_eq!(kept, legacy);
    let copies = vault.legacy_copies();
    assert_eq!(copies.len(), 2, "{copies:?}");
    assert!(
        copies
            .iter()
            .any(|copy| fs::read(copy).unwrap() == b"copy of an earlier migration")
    );
    for copy in &copies {
        assert_eq!(mode(copy), 0o600, "{}", copy.display());
        for suffix in ["-wal", "-shm"] {
            let side = sibling(copy, suffix);
            if side.exists() {
                assert_eq!(mode(&side), 0o600, "{}", side.display());
            }
        }
    }
}

/// Every file in `dir` with its contents.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    files.sort();
    files
}

#[test]
fn a_plaintext_database_with_credentials_is_not_migrated() {
    let dir = Dir::new();
    let vault = dir.vault();
    let profile = plaintext_database(&vault);
    {
        let connection = Connection::open(vault.database()).unwrap();
        connection
            .execute(
                "INSERT INTO credentials (id, description, secret, updated_at)
                 VALUES ('http.token', '', 'plain secret 5c1e', 0)",
                [],
            )
            .unwrap();
    }
    // Asking for the state creates the (empty) lock file; take the snapshot
    // after that so that only the migration is compared.
    assert_eq!(vault.state().unwrap(), DatabaseState::Plaintext);
    #[cfg(unix)]
    let before_mode = mode(vault.database());
    let before = snapshot(&dir.0);

    let error = vault.encrypt_plaintext(&password("refused")).unwrap_err();
    assert!(error.to_string().contains("credential"), "{error}");
    let keys = MemoryKeyStore::new();
    assert!(vault.encrypt_plaintext_system(&keys).is_err());

    assert_eq!(snapshot(&dir.0), before);
    #[cfg(unix)]
    assert_eq!(mode(vault.database()), before_mode);
    assert_eq!(vault.state().unwrap(), DatabaseState::Plaintext);
    let store = SqliteStore::open(vault.database(), &Encryption::None).unwrap();
    assert_eq!(
        store.load_profile(profile.id).unwrap().as_ref(),
        Some(&profile)
    );
}

#[test]
fn a_missing_database_is_never_recreated_silently() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let key = vault.initialize_system(&keys).unwrap();
    fill(&open(&vault, &key));
    // The database disappears (moved, deleted, a drive not mounted).
    remove_database(vault.database());
    assert_eq!(vault.state().unwrap(), DatabaseState::Missing);
    assert!(matches!(
        vault.initialize_system(&keys),
        Err(StorageError::DatabaseMissing)
    ));
    assert!(matches!(
        vault.initialize(&password("a new password")),
        Err(StorageError::DatabaseMissing)
    ));
    assert!(!vault.database().exists());
    assert_eq!(keys.ids().len(), 1, "no new key was created");

    // Only an explicit decision sets up a new configuration; the old key
    // file and key are kept.
    let aside = vault.discard_orphaned_key_file().unwrap();
    assert!(aside.exists());
    vault.initialize_system(&keys).unwrap();
    assert_eq!(keys.ids().len(), 2);
    assert!(vault.discard_orphaned_key_file().is_err());
}

#[test]
fn an_interrupted_first_setup_is_completed_or_discarded() {
    let dir = Dir::new();
    let vault = dir.vault();
    // Interrupted before the database existed: nothing was set up.
    fs::write(vault.setup_file(), b"{}").unwrap();
    assert_eq!(vault.state().unwrap(), DatabaseState::Missing);
    assert!(!vault.setup_file().exists());
    let key = vault.initialize(&password("first password")).unwrap();

    // Interrupted after the database was created: the setup is completed.
    let params = fs::read(vault.vault_file()).unwrap();
    fs::remove_file(vault.vault_file()).unwrap();
    fs::write(vault.setup_file(), &params).unwrap();
    assert_eq!(vault.state().unwrap(), DatabaseState::Encrypted);
    assert!(vault.has_key_file());
    assert!(
        vault
            .unlock(&password("first password"))
            .unwrap()
            .matches(&key)
    );
}

#[test]
fn an_interrupted_switch_to_a_password_is_completed_by_the_password() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let key = vault.initialize_system(&keys).unwrap();
    let profile = fill(&open(&vault, &key));
    let old_params = fs::read(vault.vault_file()).unwrap();
    vault.use_password(&key, &password("new password")).unwrap();
    // Crash after the swap, before the new parameters were moved in place.
    fs::rename(vault.vault_file(), vault.pending_vault_file()).unwrap();
    fs::write(vault.vault_file(), &old_params).unwrap();

    // The system key no longer opens the database and nothing is replaced.
    assert!(matches!(
        vault.unlock_system(&keys),
        Err(StorageError::InvalidKey)
    ));
    let key = vault.unlock(&password("new password")).unwrap();
    assert!(!vault.pending_vault_file().exists());
    assert_eq!(
        vault.protection().unwrap().map(|p| p.mode()),
        Some(ProtectionMode::Password)
    );
    assert_filled(&open(&vault, &key), &profile);
}

#[test]
fn an_interrupted_switch_before_the_swap_keeps_the_password() {
    let dir = Dir::new();
    let vault = dir.vault();
    let keys = MemoryKeyStore::new();
    let key = vault.initialize(&password("old password")).unwrap();
    let profile = fill(&open(&vault, &key));
    // A switch to a system key stored its key and the pending parameters,
    // then stopped before the database was replaced.
    let new_key = SecretKey::random().unwrap();
    let id = KeyId::parse("0123456789abcdef").unwrap();
    keys.store(&id, &Secret::new(new_key.as_bytes().to_vec()))
        .unwrap();
    Vault::write_vault(
        &vault.pending_vault_file(),
        &Protection::SystemManaged {
            key_id: id.to_string(),
        },
    )
    .unwrap();
    fs::write(vault.replacement(), b"partial").unwrap();

    // The agent alone cannot open it: still password protected.
    assert!(matches!(
        vault.unlock_system(&keys),
        Err(StorageError::CredentialsRequired(_))
    ));
    let key = vault.unlock(&password("old password")).unwrap();
    assert!(!vault.pending_vault_file().exists());
    assert!(!vault.replacement().exists());
    assert_filled(&open(&vault, &key), &profile);
    // The stored key is left alone.
    assert_eq!(keys.ids(), vec![id]);
}

#[test]
fn polling_the_state_never_disturbs_a_setup_in_progress() {
    // The app polls the state while the agent sets up the configuration;
    // the repair of interrupted setups must not mistake a running one for
    // an interrupted one.
    for _ in 0..20 {
        let dir = Dir::new();
        let vault = dir.vault();
        let keys = MemoryKeyStore::new();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let poller = {
            let vault = vault.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    vault.state().unwrap();
                }
            })
        };
        let key = vault.initialize_system(&keys).unwrap();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        poller.join().unwrap();
        assert_eq!(vault.state().unwrap(), DatabaseState::Encrypted);
        assert!(vault.has_key_file());
        assert!(vault.unlock_system(&keys).unwrap().matches(&key));
    }
}
