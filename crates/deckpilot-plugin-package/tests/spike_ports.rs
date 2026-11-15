//! Cases from the retired trust spike (`invalid_paths_are_refused_when_building`
//! and `hostile_archives_are_rejected_on_unpack`) that `package.rs` covers
//! only on the reader side: the builder refuses more than 256 entries and
//! non-ASCII or trailing-slash paths, and a zip archive (`PK\x03\x04`) is
//! not a package.

use deckpilot_plugin_package::{Limits, PackageBuilder, PackageError, PathError, read_archive};

#[test]
fn the_builder_refuses_more_entries_than_the_reader_accepts() {
    let limit = Limits::default().max_entries;
    let mut builder = PackageBuilder::new();
    for i in 0..=limit {
        builder.add(&format!("assets/f{i}.png"), vec![0]).unwrap();
    }
    assert!(matches!(
        builder.build(),
        Err(PackageError::TooManyEntries { limit: l }) if l == limit
    ));
}

#[test]
fn the_builder_refuses_non_ascii_and_directory_paths() {
    let mut builder = PackageBuilder::new();
    for bad in ["ü.js", "assets/ü.png", "a/", "./a", "assets/", "\u{0}"] {
        assert!(
            matches!(builder.add(bad, vec![]), Err(PackageError::Path(_))),
            "{bad:?}"
        );
    }
    assert!(matches!(
        builder.add(&"x".repeat(129), vec![]),
        Err(PackageError::Path(PathError::Invalid { .. }))
    ));
}

#[test]
fn a_zip_archive_is_not_a_package() {
    for bytes in [&b"PK\x03\x04 zip file"[..], b"PK\x05\x06", b"\x1f"] {
        assert_eq!(
            read_archive(bytes, &Limits::default()).unwrap_err(),
            PackageError::NotGzip,
            "{bytes:?}"
        );
    }
}
