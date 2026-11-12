use std::sync::{Arc, Barrier};

use rusqlite::Connection;

use super::{check_complete, cipher_version, has_export};

/// Set in the child processes of the first-connection tests.
const CHILD_ENV: &str = "DECKPILOT_TEST_FIRST_CONNECTIONS";
/// Threads that open their first connections at the same moment.
const THREADS: usize = 16;
/// Fresh processes per test: the race exists once per process.
const PROCESSES: usize = 30;

#[test]
fn connections_have_a_complete_sqlcipher() {
    let memory = super::open_in_memory().unwrap();
    assert!(cipher_version(&memory).unwrap().is_some());
    assert!(has_export(&memory).unwrap());
    check_complete(&memory).unwrap();

    let dir = std::env::temp_dir().join(format!("deckpilot-engine-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = super::open(&dir.join("x.db")).unwrap();
    check_complete(&file).unwrap();
    drop(file);
    let _ = std::fs::remove_dir_all(dir);
}

/// Opens one connection per thread, all at once, as the very first
/// connections of the process; returns how many lack `sqlcipher_export`.
fn first_connections(open: fn() -> Connection) -> usize {
    let barrier = Arc::new(Barrier::new(THREADS));
    let threads: Vec<_> = (0..THREADS)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let connection = open();
                usize::from(!has_export(&connection).unwrap())
            })
        })
        .collect();
    threads.into_iter().map(|t| t.join().unwrap()).sum()
}

/// Runs in a child process: the first connections, opened through
/// [`super::open_in_memory`], all have `sqlcipher_export`.
#[test]
#[ignore = "runs in child processes of first_connections_all_get_sqlcipher_export"]
fn child_first_connections() {
    let Ok(mode) = std::env::var(CHILD_ENV) else {
        return;
    };
    let missing = match mode.as_str() {
        "checked" => first_connections(|| super::open_in_memory().unwrap()),
        _ => first_connections(|| Connection::open_in_memory().unwrap()),
    };
    // Exit code: the number of connections without the function.
    std::process::exit(i32::try_from(missing).unwrap_or(i32::MAX));
}

/// Runs [`child_first_connections`] in fresh processes; the total number
/// of connections that missed `sqlcipher_export`.
fn missing_in_children(mode: &str) -> i32 {
    (0..PROCESSES)
        .map(|_| {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "engine::tests::child_first_connections",
                    "--ignored",
                    "--test-threads=1",
                ])
                .env(CHILD_ENV, mode)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            status.code().expect("the child exited")
        })
        .sum()
}

/// The first connections of a process, opened at the same moment from many
/// threads, all have `sqlcipher_export` — needed for the backup before
/// every migration.
#[test]
fn first_connections_all_get_sqlcipher_export() {
    assert_eq!(missing_in_children("checked"), 0);
}

/// Shows the race the engine guards against: raw connections opened during
/// SQLite's first initialization can miss the function. Timing dependent,
/// so it only reports.
#[test]
#[ignore = "diagnostic: shows how often raw first connections miss sqlcipher_export"]
fn raw_first_connections_can_miss_sqlcipher_export() {
    eprintln!(
        "raw connections without sqlcipher_export: {}",
        missing_in_children("raw")
    );
}
