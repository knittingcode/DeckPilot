//! Entry path rules (architecture section 8), shared by the builder and the
//! reader so a package the builder writes is always one the reader accepts.

use std::collections::{BTreeMap, BTreeSet};

use deckpilot_plugin_manifest::check_relative_path;

use crate::error::PathError;

/// Device names Windows reserves in every directory, with any extension.
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Checks one entry path: `$defs/relativePath` of the manifest schema
/// (relative, `/` separators, no `.` or `..`, `[A-Za-z0-9._-]`, at most 128
/// characters, no segment starting with `.` or `-`), plus names that cannot
/// be created on Windows.
pub fn check_entry_path(path: &str) -> Result<(), PathError> {
    check_relative_path(path).map_err(|reason| PathError::Invalid {
        path: path.to_owned(),
        reason,
    })?;
    for segment in path.split('/') {
        if segment.ends_with('.') {
            return Err(PathError::TrailingDot(path.to_owned()));
        }
        let stem = segment.split('.').next().unwrap_or(segment);
        if WINDOWS_RESERVED.contains(&stem.to_ascii_lowercase().as_str()) {
            return Err(PathError::WindowsReserved(path.to_owned()));
        }
    }
    Ok(())
}

/// Checks a whole set of entry paths: each path on its own, then no
/// duplicates, no two paths that differ only in case (macOS and Windows file
/// systems would merge them), and no file that is also the directory of
/// another file.
pub fn check_entry_set<'a>(paths: impl IntoIterator<Item = &'a str>) -> Result<(), PathError> {
    // Paths are ASCII (checked first), so ASCII case folding is complete.
    let mut files: BTreeMap<String, &str> = BTreeMap::new();
    let mut directories: BTreeMap<String, &str> = BTreeMap::new();
    let mut exact = BTreeSet::new();
    for path in paths {
        check_entry_path(path)?;
        if !exact.insert(path) {
            return Err(PathError::Duplicate(path.to_owned()));
        }
        let folded = path.to_ascii_lowercase();
        if let Some(other) = files.insert(folded.clone(), path) {
            return Err(PathError::CaseCollision(other.to_owned(), path.to_owned()));
        }
        let mut end = 0;
        while let Some(slash) = folded[end..].find('/') {
            end += slash;
            directories.entry(folded[..end].to_owned()).or_insert(path);
            end += 1;
        }
    }
    for (folded, file) in &files {
        if let Some(child) = directories.get(folded) {
            return Err(PathError::FileIsDirectory(
                (*file).to_owned(),
                (*child).to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_paths() {
        for good in [
            "manifest.json",
            "assets/keys/pr.png",
            "assets/con-icon.png",
            "assets/console.png",
            "a.b.c",
        ] {
            assert_eq!(check_entry_path(good), Ok(()), "{good}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "C:/x",
            "..",
            "../x",
            "a/../b",
            "./a",
            "a//b",
            "a/",
            ".hidden",
            "a\\b",
            "a b",
            "ä",
            "assets/CON",
            "assets/con.png",
            "aux",
            "Nul.txt",
            "lpt1.png",
            "COM9",
            "assets/x.",
            "assets./x",
        ] {
            assert!(check_entry_path(bad).is_err(), "{bad}");
        }
        assert!(check_entry_path(&"a".repeat(129)).is_err());
        assert!(check_entry_path(&"a".repeat(128)).is_ok());
    }

    #[test]
    fn path_sets() {
        assert_eq!(check_entry_set(["a", "b/c", "b/d"]), Ok(()));
        assert!(matches!(
            check_entry_set(["a", "a"]),
            Err(PathError::Duplicate(_))
        ));
        assert!(matches!(
            check_entry_set(["README.md", "readme.md"]),
            Err(PathError::CaseCollision(..))
        ));
        assert!(matches!(
            check_entry_set(["assets/X.png", "Assets/x.png"]),
            Err(PathError::CaseCollision(..))
        ));
        assert!(matches!(
            check_entry_set(["assets", "assets/x.png"]),
            Err(PathError::FileIsDirectory(..))
        ));
        assert!(matches!(
            check_entry_set(["assets/k/x.png", "Assets/K"]),
            Err(PathError::FileIsDirectory(..))
        ));
    }
}
