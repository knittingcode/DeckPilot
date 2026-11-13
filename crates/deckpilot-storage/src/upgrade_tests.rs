//! Upgrades of real databases checked row by row — 0.5.0-2
//! (`tests/fixtures/v0.5.0-2`, schema 5), 0.6.0 (`tests/fixtures/v0.6.0`,
//! schema 7) and `main` before schema 9 (`tests/fixtures/schema8`) — and
//! upgrades that fail or are killed half-way.
//!
//! These are unit tests because failures are injected at points inside the
//! upgrade ([`Failpoint`]), which only test builds have. A killed process is
//! simulated by running this test binary again in a child process that
//! aborts at the failpoint, with the transaction open.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::crypto::Password;
use crate::sqlite::{ABORT_AT_ENV, FAIL_AT, Failpoint};
use crate::vault::Vault;
use crate::{DatabaseKey, Encryption, SqliteStore, StorageError};

/// A database created by a released DeckPilot's development tools.
#[derive(Debug, Clone, Copy)]
struct Fixture {
    dir: &'static str,
    /// Its schema version.
    version: u32,
}

/// `deckpilot-dev validation seed` of 0.5.0-2.
const V0_5_0: Fixture = Fixture {
    dir: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v0.5.0-2"),
    version: 5,
};
/// `deckpilot-dev validation seed` of 0.6.0 (`main` before schema 8), plus a
/// workflow run, a credential, an active project and launches, so that
/// every table of schema 7 has rows.
const V0_6_0: Fixture = Fixture {
    dir: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v0.6.0"),
    version: 7,
};
/// The 0.6.0 fixture upgraded by `main` before schema 9 (shared variables
/// and provenance, #97), plus shared variables with global, profile and
/// project values and a provenance row, so that every schema 8 table has
/// rows.
const SCHEMA_8: Fixture = Fixture {
    dir: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/schema8"),
    version: 8,
};
const FIXTURES: [Fixture; 3] = [V0_5_0, V0_6_0, SCHEMA_8];
/// The schema this build upgrades to.
const CURRENT: u32 = 9;
/// The tables schema 9 adds, all empty after the upgrade.
const SCHEMA_9_TABLES: [&str; 5] = [
    "work_session_reviews",
    "work_session_settings",
    "work_session_transitions",
    "work_session_intervals",
    "work_sessions",
];
/// The development password the fixtures were created with.
const PASSWORD: &str = "deckpilot-dev";
/// The database the child process upgrades.
const CHILD_DATABASE_ENV: &str = "DECKPILOT_TEST_DATABASE";

/// A copy of the fixture in a temporary directory, removed on drop.
struct Copy {
    dir: PathBuf,
    key: DatabaseKey,
    version: u32,
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn copy(fixture: Fixture) -> Copy {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-interrupted-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["deckpilot.db", "deckpilot.vault.json"] {
        std::fs::copy(Path::new(fixture.dir).join(file), dir.join(file)).unwrap();
    }
    let key = Vault::new(dir.join("deckpilot.db"))
        .unlock(&Password::new(PASSWORD))
        .unwrap();
    Copy {
        dir,
        key,
        version: fixture.version,
    }
}

impl Copy {
    fn database(&self) -> PathBuf {
        self.dir.join("deckpilot.db")
    }

    fn open(&self) -> crate::Result<SqliteStore> {
        SqliteStore::open(self.database(), &Encryption::Key(self.key.clone()))
    }

    /// Every file in `backups/`, sorted.
    fn backup_files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.dir.join("backups"))
            .map(|dir| {
                dir.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn backup(&self, name: &str) -> PathBuf {
        self.dir.join("backups").join(name)
    }
}

/// Opens `path` with `key` without DeckPilot's open, which would upgrade it.
fn raw(path: &Path, key: &DatabaseKey) -> Connection {
    let connection = Connection::open(path).unwrap();
    crate::sqlite::apply_key(&connection, &Encryption::Key(key.clone())).unwrap();
    connection
}

/// The schema version and, per table, its definition and every row.
#[derive(Debug, PartialEq, Eq)]
struct Dump {
    version: u32,
    tables: BTreeMap<String, (String, Vec<String>)>,
}

fn dump(connection: &Connection) -> Dump {
    let tables: Vec<(String, String)> = connection
        .prepare(
            "SELECT name, sql FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let mut dumped = BTreeMap::new();
    for (name, sql) in tables {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM \"{name}\""))
            .unwrap();
        let columns = statement.column_count();
        let mut rows: Vec<String> = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map(|values| format!("{values:?}"))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        rows.sort();
        dumped.insert(name, (sql, rows));
    }
    Dump {
        version: connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap(),
        tables: dumped,
    }
}

/// Every fixture with every failpoint inside the upgrade transaction: after
/// each migration it runs, and just before the commit.
fn inside_the_transaction() -> Vec<(Fixture, Failpoint)> {
    FIXTURES
        .into_iter()
        .flat_map(|fixture| {
            (fixture.version + 1..=CURRENT)
                .map(Failpoint::Migration)
                .chain([Failpoint::Commit])
                .map(move |point| (fixture, point))
        })
        .collect()
}

/// Runs `open` with an injected failure at `point`.
fn failing_at<T>(point: Failpoint, open: impl FnOnce() -> T) -> T {
    FAIL_AT.with(|fail| fail.set(Some(point)));
    let result = open();
    FAIL_AT.with(|fail| fail.set(None));
    result
}

/// The database is still the fixture, row for row, at its schema.
fn assert_untouched(copy: &Copy, original: &Dump) {
    let now = dump(&raw(&copy.database(), &copy.key));
    assert_eq!(now.version, copy.version);
    assert!(now == *original, "the database changed");
}

/// `name` is a complete, verified backup holding the original state.
fn assert_backup_of(copy: &Copy, name: &str, original: &Dump) {
    assert!(
        name.starts_with(&format!("deckpilot-schema{}-", copy.version))
            && Path::new(name).extension().is_some_and(|e| e == "db"),
        "{name}"
    );
    let backup = raw(&copy.backup(name), &copy.key);
    crate::sqlite::check_integrity(&backup).unwrap();
    assert!(dump(&backup) == *original, "the backup differs");
}

/// The next start upgrades normally.
fn assert_next_start_upgrades(copy: &Copy) {
    let store = copy.open().unwrap();
    assert_eq!(store.schema_version().unwrap(), CURRENT);
    store.check_integrity().unwrap();
    let last = store.migration_history().unwrap().pop().unwrap();
    assert_eq!(
        (last.from_version, last.to_version),
        (copy.version, CURRENT)
    );
    assert_eq!(last.result, "ok");
    assert!(Path::new(&last.backup.unwrap()).exists());
    // Only complete backups remain.
    assert!(
        copy.backup_files()
            .iter()
            .all(|name| Path::new(name).extension().is_some_and(|e| e == "db")),
        "{:?}",
        copy.backup_files()
    );
}

#[test]
fn every_row_of_every_table_is_kept_and_reopening_changes_nothing() {
    for fixture in FIXTURES {
        every_row_is_kept(fixture);
    }
}

fn every_row_is_kept(fixture: Fixture) {
    let copy = copy(fixture);
    let original = dump(&raw(&copy.database(), &copy.key));
    assert_eq!(original.version, fixture.version);
    // The fixture has data in the tables that matter.
    let mut filled = vec![
        "profiles",
        "projects",
        "default_projects",
        "machine_config",
        "settings",
        "migration_history",
    ];
    if fixture.version >= 7 {
        filled.extend([
            "active_projects",
            "app_launches",
            "credentials",
            "workflow_executions",
        ]);
    }
    if fixture.version >= 8 {
        filled.extend(["shared_variables", "shared_variable_values", "provenance"]);
    }
    for table in filled {
        assert!(!original.tables[table].1.is_empty(), "{table} is empty");
    }

    let store = copy.open().unwrap();
    let upgraded = dump(store.connection_for_tests());
    assert_eq!(upgraded.version, CURRENT);
    for (table, (sql, rows)) in &original.tables {
        let (new_sql, new_rows) = &upgraded.tables[table];
        assert_eq!(new_sql, sql, "the definition of {table} changed");
        if table == "migration_history" {
            // One row more: this upgrade.
            assert_eq!(new_rows.len(), rows.len() + 1);
            assert!(rows.iter().all(|row| new_rows.contains(row)));
        } else {
            assert_eq!(new_rows, rows, "rows of {table} changed");
        }
    }
    let added: BTreeSet<&str> = upgraded
        .tables
        .keys()
        .filter(|table| !original.tables.contains_key(*table))
        .map(String::as_str)
        .collect();
    let mut expected = BTreeSet::from(SCHEMA_9_TABLES);
    if fixture.version < 8 {
        expected.extend(["provenance", "shared_variable_values", "shared_variables"]);
    }
    if fixture.version < 6 {
        expected.insert("workflow_executions");
    }
    if fixture.version < 7 {
        expected.insert("credentials");
    }
    assert_eq!(added, expected, "{fixture:?}");
    // The new tables start empty: no provenance is invented for existing
    // (seeded) data, no variable and no work session is created.
    for table in &expected {
        assert!(upgraded.tables[*table].1.is_empty(), "{table}");
    }
    drop(store);

    // Opening again (the agent and the app, every start) writes nothing.
    for _ in 0..2 {
        let store = copy.open().unwrap();
        assert!(
            dump(store.connection_for_tests()) == upgraded,
            "reopening changed the database"
        );
    }
    assert_eq!(copy.backup_files().len(), 1);
}

#[test]
fn a_failed_upgrade_changes_nothing_and_the_next_start_succeeds() {
    for (fixture, point) in inside_the_transaction() {
        let copy = copy(fixture);
        let original = dump(&raw(&copy.database(), &copy.key));
        let failed = failing_at(point, || copy.open());
        assert!(
            matches!(&failed, Err(StorageError::Invalid(m)) if m.contains("injected")),
            "{point:?}: {failed:?}"
        );
        assert_untouched(&copy, &original);
        // The backup made before the upgrade is complete and restorable.
        let backups = copy.backup_files();
        assert_eq!(backups.len(), 1, "{point:?}: {backups:?}");
        assert_backup_of(&copy, &backups[0], &original);

        assert_next_start_upgrades(&copy);
        assert_eq!(copy.backup_files().len(), 2);
    }
}

#[test]
fn a_backup_that_cannot_be_verified_is_removed_and_nothing_is_upgraded() {
    for fixture in FIXTURES {
        backup_that_cannot_be_verified(fixture);
    }
}

fn backup_that_cannot_be_verified(fixture: Fixture) {
    let copy = copy(fixture);
    let original = dump(&raw(&copy.database(), &copy.key));
    let failed = failing_at(Failpoint::Backup, || copy.open());
    assert!(
        matches!(failed, Err(StorageError::Invalid(_))),
        "{failed:?}"
    );
    assert_untouched(&copy, &original);
    assert_eq!(copy.backup_files(), Vec::<String>::new());
    assert_next_start_upgrades(&copy);
    assert_eq!(copy.backup_files().len(), 1);
}

/// Run by [`a_killed_upgrade_is_rolled_back`] in a child process that
/// aborts at the failpoint named in [`ABORT_AT_ENV`].
#[test]
#[ignore = "runs in a child process of a_killed_upgrade_is_rolled_back"]
fn child_upgrade() {
    let Ok(database) = std::env::var(CHILD_DATABASE_ENV) else {
        return;
    };
    let key = Vault::new(&database)
        .unlock(&Password::new(PASSWORD))
        .unwrap();
    let opened = SqliteStore::open(&database, &Encryption::Key(key));
    panic!("the failpoint was not reached: {opened:?}");
}

/// Upgrades `copy` in a child process that aborts at `point`.
fn kill_upgrade_at(copy: &Copy, point: Failpoint) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "upgrade_tests::child_upgrade",
            "--ignored",
            "--test-threads=1",
        ])
        .env(ABORT_AT_ENV, format!("{point:?}"))
        .env(CHILD_DATABASE_ENV, copy.database())
        .output()
        .unwrap();
    // Aborted, not finished and not a failed assertion (exit code 101).
    assert!(
        !output.status.success() && output.status.code() != Some(101),
        "{point:?}: {:?}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn a_killed_upgrade_is_rolled_back() {
    let points = FIXTURES
        .into_iter()
        .map(|fixture| (fixture, Failpoint::Backup))
        .chain(inside_the_transaction());
    for (fixture, point) in points {
        let copy = copy(fixture);
        let original = dump(&raw(&copy.database(), &copy.key));
        kill_upgrade_at(&copy, point);

        assert_untouched(&copy, &original);
        let backups = copy.backup_files();
        if point == Failpoint::Backup {
            // The unverified copy is left under its temporary name only.
            assert!(
                backups.iter().all(|name| name.contains(".db.partial")),
                "{backups:?}"
            );
        } else {
            assert_eq!(backups.len(), 1, "{point:?}: {backups:?}");
            assert_backup_of(&copy, &backups[0], &original);
        }

        assert_next_start_upgrades(&copy);
        let expected = if point == Failpoint::Backup { 1 } else { 2 };
        assert_eq!(copy.backup_files().len(), expected, "{point:?}");
    }
}

/// `result` of the newest `migration_history` row, read without opening the
/// database through DeckPilot.
fn last_result(copy: &Copy) -> String {
    raw(&copy.database(), &copy.key)
        .query_row(
            "SELECT result FROM migration_history ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

/// The upgrade was committed, but the integrity check that follows failed.
/// The database stays upgraded and marked unverified; every start checks it
/// again and refuses it, naming the backup, until a check passes. It is
/// never used unchecked, and nothing is upgraded or backed up again.
#[test]
fn a_failed_check_after_the_commit_refuses_the_database_until_it_passes() {
    for fixture in FIXTURES {
        failed_check_after_the_commit(fixture);
    }
}

fn failed_check_after_the_commit(fixture: Fixture) {
    let copy = copy(fixture);
    let original = dump(&raw(&copy.database(), &copy.key));
    let failed = failing_at(Failpoint::Verify, || copy.open());
    let Err(StorageError::MigrationFailed {
        backup: Some(backup),
        ..
    }) = failed
    else {
        panic!("{failed:?}");
    };
    let backups = copy.backup_files();
    assert_eq!(backups.len(), 1);
    assert_eq!(copy.backup(&backups[0]).display().to_string(), backup);
    assert_backup_of(&copy, &backups[0], &original);
    let upgraded = dump(&raw(&copy.database(), &copy.key));
    assert_eq!(upgraded.version, CURRENT);
    assert_eq!(last_result(&copy), "unverified");

    // The next start checks again; while the check fails, the database is
    // refused the same way and left exactly as it is.
    for _ in 0..2 {
        let refused = failing_at(Failpoint::Verify, || copy.open());
        assert!(
            matches!(&refused, Err(StorageError::MigrationFailed { backup: Some(b), .. }) if *b == backup),
            "{refused:?}"
        );
        assert!(
            dump(&raw(&copy.database(), &copy.key)) == upgraded,
            "a refused start changed the database"
        );
        assert_eq!(copy.backup_files(), backups);
    }

    // Once a check passes, the upgrade is recorded as verified, without
    // upgrading or backing up again.
    let store = copy.open().unwrap();
    let history = store.migration_history().unwrap();
    assert_eq!(
        history.len(),
        original.tables["migration_history"].1.len() + 1
    );
    let last = history.last().unwrap();
    assert_eq!(
        (last.from_version, last.to_version),
        (fixture.version, CURRENT)
    );
    assert_eq!(last.result, "ok");
    drop(store);
    assert_eq!(copy.backup_files(), backups);
    copy.open().unwrap();
}

#[test]
fn the_backup_named_by_a_failed_check_restores_the_previous_state() {
    for fixture in FIXTURES {
        backup_named_by_a_failed_check_restores(fixture);
    }
}

fn backup_named_by_a_failed_check_restores(fixture: Fixture) {
    let copy = copy(fixture);
    let original = dump(&raw(&copy.database(), &copy.key));
    let failed = failing_at(Failpoint::Verify, || copy.open());
    let Err(StorageError::MigrationFailed {
        backup: Some(backup),
        ..
    }) = failed
    else {
        panic!("{failed:?}");
    };
    // Restoring, as documented: the backup replaces the database and its
    // side files, which belong to the upgraded database.
    std::fs::copy(&backup, copy.database()).unwrap();
    for side in ["deckpilot.db-wal", "deckpilot.db-shm"] {
        let _ = std::fs::remove_file(copy.dir.join(side));
    }
    let restored = dump(&raw(&copy.database(), &copy.key));
    assert!(restored == original, "the restored database differs");
    assert_next_start_upgrades(&copy);
}

#[test]
fn a_process_killed_before_the_check_is_verified_on_the_next_start() {
    for fixture in FIXTURES {
        let copy = copy(fixture);
        kill_upgrade_at(&copy, Failpoint::Verify);
        // Committed, but never checked.
        assert_eq!(dump(&raw(&copy.database(), &copy.key)).version, CURRENT);
        assert_eq!(last_result(&copy), "unverified");
        assert_eq!(copy.backup_files().len(), 1);

        assert_next_start_upgrades(&copy);
        assert_eq!(last_result(&copy), "ok");
        assert_eq!(copy.backup_files().len(), 1);
    }
}

/// Gestures (`docs/gestures-and-work-session.md`, section 3.3): every
/// binding of a released version decodes unchanged and stays a raw press,
/// which fires at once; nothing is converted to a gesture.
#[test]
fn bindings_of_released_versions_stay_raw_and_unchanged() {
    use crate::ConfigStore;
    use deckpilot_core::{BindingTarget, Trigger};

    for fixture in FIXTURES {
        let copy = copy(fixture);
        let store = copy.open().unwrap();
        let mut bindings = 0;
        let mut presses = 0;
        for summary in store.list_profiles().unwrap() {
            let profile = store.load_profile(summary.id).unwrap().unwrap();
            let stored: String = store
                .connection_for_tests()
                .query_row(
                    "SELECT document FROM profiles WHERE id = ?1",
                    [summary.id.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
            let stored: serde_json::Value = serde_json::from_str(&stored).unwrap();
            assert_eq!(
                serde_json::to_value(&profile.layouts).unwrap(),
                stored["layouts"],
                "{fixture:?}"
            );
            for page in profile.layouts.iter().flat_map(|l| &l.pages) {
                assert_eq!(page.gestures, []);
                for binding in &page.bindings {
                    bindings += 1;
                    assert_eq!(binding.trigger.gesture(), None, "{fixture:?}");
                    presses += usize::from(binding.trigger == Trigger::Press);
                    assert!(!page.bound_gestures(binding.control).any());
                    if let BindingTarget::Action { overrides, .. } = &binding.target {
                        assert_eq!(*overrides, deckpilot_core::ActionParameters::new());
                    }
                }
            }
        }
        assert!(bindings >= 10, "{fixture:?}: {bindings} bindings");
        assert!(presses >= 10, "{fixture:?}: {presses} presses");
    }
}
