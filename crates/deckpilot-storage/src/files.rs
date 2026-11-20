//! Renaming and removing database files in a way that works on Windows.
//!
//! On Windows a file cannot be renamed, replaced or removed while another
//! handle has it open without `FILE_SHARE_DELETE` — and SQLite opens its
//! files without it. Besides a connection that is still open, short-lived
//! handles of other programs cause the same error: the virus scanner and
//! the search indexer open a file right after it was written and closed,
//! which is exactly when a freshly written copy is renamed into place. The
//! rename then fails with "Access is denied" (`ERROR_ACCESS_DENIED`) or a
//! sharing violation, typically for a few to a few hundred milliseconds.
//!
//! [`rename`] and [`remove`] retry those errors for a bounded time
//! ([`RETRY_FOR`]) and then fail with an error that names the file. Every
//! other error, and every error on other systems, is returned at once.
//! Callers must still close their own connections first: a handle this
//! process keeps open never goes away by waiting, and the operation then
//! fails after the bounded wait instead of succeeding.

use std::path::Path;
use std::time::{Duration, Instant};

/// How long a rename or removal is retried while the file is in use.
pub(crate) const RETRY_FOR: Duration = Duration::from_secs(3);
/// The first pause between attempts; it doubles up to [`MAX_PAUSE`].
const FIRST_PAUSE: Duration = Duration::from_millis(10);
const MAX_PAUSE: Duration = Duration::from_millis(200);

/// Whether `error` means that another handle has the file open (Windows),
/// which goes away by itself when that handle is closed.
fn in_use(error: &std::io::Error) -> bool {
    if !cfg!(windows) {
        return false;
    }
    // ERROR_ACCESS_DENIED (a pending delete or an open handle),
    // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
    matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

/// Runs `step` until it succeeds, fails with an error other than
/// [`in_use`], or [`RETRY_FOR`] has passed.
fn retrying(mut step: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let deadline = Instant::now() + RETRY_FOR;
    let mut pause = FIRST_PAUSE;
    loop {
        match step() {
            Err(error) if in_use(&error) && Instant::now() < deadline => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(MAX_PAUSE);
            }
            other => return other,
        }
    }
}

/// Renames `from` to `to`, replacing `to` if it exists (on Windows too:
/// `std::fs::rename` uses `MOVEFILE_REPLACE_EXISTING`). Retried while the
/// file is in use; the error names both files.
pub(crate) fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    retrying(|| std::fs::rename(from, to)).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "could not rename {} to {}: {error}{}",
                from.display(),
                to.display(),
                hint(&error)
            ),
        )
    })
}

/// Removes the file `path`; a file that does not exist is not an error.
/// Retried while the file is in use; the error names the file.
pub(crate) fn remove(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    retrying(|| match std::fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    })
    .map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "could not remove {}: {error}{}",
                path.display(),
                hint(&error)
            ),
        )
    })
}

fn hint(error: &std::io::Error) -> &'static str {
    if in_use(error) {
        " (the file is still in use by another program or connection)"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new empty directory, removed when dropped.
    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "deckpilot-files-{}",
                deckpilot_core::ProfileId::new()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_file_is_removed_without_an_error() {
        let dir = Dir::new();
        remove(dir.path().join("missing.db")).unwrap();
    }

    #[test]
    fn rename_replaces_an_existing_target() {
        let dir = Dir::new();
        let (from, to) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&from, b"new").unwrap();
        std::fs::write(&to, b"old").unwrap();
        rename(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"new");
        assert!(!from.exists());
    }

    #[test]
    fn other_errors_are_returned_at_once_and_name_the_file() {
        let dir = Dir::new();
        let from = dir.path().join("missing");
        let started = Instant::now();
        let error = rename(&from, dir.path().join("b")).unwrap_err();
        assert!(started.elapsed() < RETRY_FOR);
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(error.to_string().contains("missing"), "{error}");
    }
}
