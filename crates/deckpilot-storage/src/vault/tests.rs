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
