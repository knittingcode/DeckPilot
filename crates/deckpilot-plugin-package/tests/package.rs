//! Builder determinism, verification order, and every rejection path of the
//! reader, including hostile archives the registry might have pinned.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;

use deckpilot_plugin_manifest::{PackageDigest, PluginId, Sha256Digest};
use deckpilot_plugin_package::{
    BuiltPackage, ExpectedPackage, Files, Limits, PackageBuilder, PackageError, PathError,
    read_archive, sha256, verify_installed, verify_package,
};
use serde_json::Value;

const EXAMPLE: &str = include_str!("fixtures/example-manifest.json");

// ------------------------------------------------------------------ fixtures

fn manifest_json(id: &str, version: &str) -> Vec<u8> {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["id"] = Value::from(id);
    value["version"] = Value::from(version);
    value["publisher"]["id"] = Value::from(id.split('.').next().unwrap());
    let mut text = serde_json::to_string_pretty(&value).unwrap();
    text.push('\n');
    text.into_bytes()
}

/// Deterministic pseudo-random (incompressible) bytes.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[4]
        })
        .collect()
}

fn files(id: &str, version: &str) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("manifest.json", manifest_json(id, version)),
        (
            "plugin.js",
            format!("export function activate(ctx) {{ ctx.log('{id} {version}'); }}\n")
                .into_bytes(),
        ),
        ("LICENSE", b"MIT License\n".to_vec()),
        ("README.md", b"# GitHub PR Count\n".to_vec()),
        ("assets/icon-128.png", noise(300, 1)),
        ("assets/icon-32.png", noise(100, 2)),
        ("assets/keys/pr.png", noise(100, 3)),
    ]
}

fn build_files(list: &[(&str, Vec<u8>)]) -> Result<BuiltPackage, PackageError> {
    let mut builder = PackageBuilder::new();
    for (path, bytes) in list {
        builder.add(path, bytes.clone())?;
    }
    builder.build()
}

fn build(id: &str, version: &str) -> BuiltPackage {
    build_files(&files(id, version)).unwrap()
}

fn expected_for(built: &BuiltPackage) -> ExpectedPackage {
    let manifest = &built.manifest;
    let raw: Value = serde_json::from_slice(&manifest_json(
        manifest.id.as_str(),
        &manifest.version.to_string(),
    ))
    .unwrap();
    ExpectedPackage {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        package: built.integrity.package.clone(),
        manifest_sha256: built.integrity.manifest_sha256,
        permissions: Some(raw["permissions"].as_array().unwrap().clone()),
    }
}

/// What a registry that pinned exactly `bytes` would expect (a registry
/// bug or compromise: the digest matches, so the reader's own checks are
/// all that is left).
fn repinned(bytes: &[u8], like: &ExpectedPackage) -> ExpectedPackage {
    ExpectedPackage {
        package: PackageDigest {
            sha256: sha256(bytes),
            size: bytes.len() as u64,
        },
        ..like.clone()
    }
}

fn gzip(tar: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::best());
    encoder.write_all(tar).unwrap();
    encoder.finish().unwrap()
}

/// A ustar header with a raw name, typeflag and size (the tar crate's
/// `set_path` refuses `..`, so names are written directly).
fn raw_header(name: &[u8], typeflag: u8, size: u64) -> tar::Header {
    let mut header = tar::Header::new_ustar();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.as_mut_bytes()[..name.len()].copy_from_slice(name);
    header.as_mut_bytes()[156] = typeflag;
    header.set_cksum();
    header
}

fn push_entry(tar: &mut Vec<u8>, header: &tar::Header, data: &[u8]) {
    tar.extend_from_slice(header.as_bytes());
    tar.extend_from_slice(data);
    let padding = (512 - data.len() % 512) % 512;
    tar.extend(std::iter::repeat_n(0u8, padding));
}

fn end_marker(tar: &mut Vec<u8>) {
    tar.extend(std::iter::repeat_n(0u8, 1024));
}

/// A tar stream of the valid fixture plus `extra` raw entries.
fn tar_with(extra: &[(tar::Header, Vec<u8>)]) -> Vec<u8> {
    let mut tar = Vec::new();
    for (path, bytes) in files("acme.github-prs", "1.0.0") {
        push_entry(
            &mut tar,
            &raw_header(path.as_bytes(), b'0', bytes.len() as u64),
            &bytes,
        );
    }
    for (header, data) in extra {
        push_entry(&mut tar, header, data);
    }
    end_marker(&mut tar);
    tar
}

fn read(tar: &[u8]) -> Result<Files, PackageError> {
    read_archive(&gzip(tar), &Limits::default())
}

// ------------------------------------------------------------- determinism

#[test]
fn builds_are_deterministic() {
    let a = build("acme.github-prs", "1.0.0");
    let mut reversed = files("acme.github-prs", "1.0.0");
    reversed.reverse();
    let b = build_files(&reversed).unwrap();
    assert_eq!(a.bytes, b.bytes, "input order does not matter");
    assert_eq!(a.integrity, b.integrity);

    // gzip header: magic, deflate, no flags (no name or comment), mtime 0,
    // XFL 2 (best compression), OS 255.
    assert_eq!(&a.bytes[..10], &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 255]);

    // ustar headers: sorted, mode 0644, mtime 0, uid and gid 0, no owner
    // names, regular files only, no directory entries.
    let mut decoder = flate2::read::GzDecoder::new(a.bytes.as_slice());
    let mut tar = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut tar).unwrap();
    let mut archive = tar::Archive::new(tar.as_slice());
    let mut paths = Vec::new();
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        let header = entry.header();
        assert!(header.as_ustar().is_some());
        assert_eq!(header.entry_type(), tar::EntryType::Regular);
        assert_eq!(header.mode().unwrap(), 0o644);
        assert_eq!(header.mtime().unwrap(), 0);
        assert_eq!(header.uid().unwrap(), 0);
        assert_eq!(header.gid().unwrap(), 0);
        assert_eq!(header.username_bytes(), Some(&b""[..]));
        assert_eq!(header.groupname_bytes(), Some(&b""[..]));
        paths.push(String::from_utf8(header.path_bytes().into_owned()).unwrap());
    }
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
    assert_eq!(paths.len(), 7);
}

#[test]
fn long_paths_use_the_ustar_prefix() {
    let mut list = files("acme.github-prs", "1.0.0");
    let mut value: Value = serde_json::from_slice(&list[0].1).unwrap();
    let long = format!("assets/{}/{}.png", "d".repeat(60), "k".repeat(56));
    assert_eq!(long.len(), 128);
    value["assets"]["keyIcons"]["long"] = Value::from(long.clone());
    list[0].1 = serde_json::to_vec_pretty(&value).unwrap();
    list.push((Box::leak(long.into_boxed_str()), noise(10, 9)));
    let built = build_files(&list).unwrap();
    let read = verify_package(
        &built.bytes,
        &ExpectedPackage {
            permissions: None,
            ..expected_for(&built)
        },
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(read.files().len(), 8);
}

// ------------------------------------------------------------------ accept

#[test]
fn a_correct_package_verifies_unpacks_and_rechecks() {
    let built = build("acme.github-prs", "1.0.0");
    let verified = verify_package(&built.bytes, &expected_for(&built), &Limits::default()).unwrap();
    assert_eq!(verified.manifest().id.as_str(), "acme.github-prs");
    assert_eq!(verified.integrity(), &built.integrity);
    assert_eq!(verified.files().len(), 7);

    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("plugins/.staging");
    let target = dir.path().join("plugins/acme.github-prs/1.0.0");
    verified.unpack(&staging, &target).unwrap();
    for (path, bytes) in files("acme.github-prs", "1.0.0") {
        let on_disk = target.join(path);
        assert_eq!(std::fs::read(&on_disk).unwrap(), bytes, "{path}");
        assert!(
            std::fs::metadata(&on_disk)
                .unwrap()
                .permissions()
                .readonly()
        );
    }
    assert_eq!(
        std::fs::read_dir(&staging).unwrap().count(),
        0,
        "staging cleaned up"
    );
    verify_installed(&target, verified.integrity()).unwrap();
}

// ---------------------------------------------------- pinned digest first

#[test]
fn every_bit_flip_is_rejected_before_unpacking() {
    let built = build("acme.github-prs", "1.0.0");
    let expected = expected_for(&built);
    let original = verify_package(&built.bytes, &expected, &Limits::default())
        .unwrap()
        .files()
        .clone();
    for at in 0..built.bytes.len() {
        for bit in [0u8, 5] {
            let mut flipped = built.bytes.clone();
            flipped[at] ^= 1 << bit;
            assert!(
                matches!(
                    verify_package(&flipped, &expected, &Limits::default()),
                    Err(PackageError::HashMismatch { .. })
                ),
                "byte {at} bit {bit}"
            );
            // Even when a broken registry pinned the flipped bytes, the
            // reader never panics and never accepts different contents:
            // gzip's CRC-32 catches every single-bit change of the data, and
            // flips in unchecked header bytes leave the contents as they were.
            if let Ok(package) =
                verify_package(&flipped, &repinned(&flipped, &expected), &Limits::default())
            {
                assert_eq!(package.files(), &original, "byte {at} bit {bit}");
            }
        }
    }
}

#[test]
fn truncation_and_extension_are_rejected() {
    let built = build("acme.github-prs", "1.0.0");
    let expected = expected_for(&built);
    for len in 0..built.bytes.len() {
        let short = &built.bytes[..len];
        assert!(
            matches!(
                verify_package(short, &expected, &Limits::default()),
                Err(PackageError::SizeMismatch { .. })
            ),
            "{len}"
        );
        assert!(
            verify_package(short, &repinned(short, &expected), &Limits::default()).is_err(),
            "repinned {len}"
        );
    }
    let mut long = built.bytes.clone();
    long.push(0);
    assert!(matches!(
        verify_package(&long, &expected, &Limits::default()),
        Err(PackageError::SizeMismatch { .. })
    ));
    assert_eq!(
        verify_package(&long, &repinned(&long, &expected), &Limits::default()).unwrap_err(),
        PackageError::TrailingData(1)
    );
    // A second gzip member is trailing data too.
    let mut two = built.bytes.clone();
    two.extend_from_slice(&gzip(b"more"));
    assert!(matches!(
        verify_package(&two, &repinned(&two, &expected), &Limits::default()),
        Err(PackageError::TrailingData(_))
    ));
    let zeroed = vec![0u8; built.bytes.len()];
    assert!(matches!(
        verify_package(&zeroed, &expected, &Limits::default()),
        Err(PackageError::HashMismatch { .. })
    ));
}

#[test]
fn another_plugin_or_version_is_rejected() {
    let wanted = build("acme.github-prs", "1.1.0");
    let expected = expected_for(&wanted);
    for other in [
        build("acme.github-prs", "1.0.0"),
        build("evil.github-prs", "1.1.0"),
    ] {
        assert!(matches!(
            verify_package(&other.bytes, &expected, &Limits::default()),
            Err(PackageError::SizeMismatch { .. } | PackageError::HashMismatch { .. })
        ));
        // The registry pinned the other package's bytes and manifest hash,
        // but the index entry names acme.github-prs@1.1.0.
        let pinned = ExpectedPackage {
            manifest_sha256: other.integrity.manifest_sha256,
            ..repinned(&other.bytes, &expected)
        };
        assert!(matches!(
            verify_package(&other.bytes, &pinned, &Limits::default()),
            Err(PackageError::WrongPlugin { .. })
        ));
        // The manifest hash pinned for the wanted version does not match.
        assert!(matches!(
            verify_package(
                &other.bytes,
                &repinned(&other.bytes, &expected),
                &Limits::default()
            ),
            Err(PackageError::ManifestHashMismatch { .. })
        ));
    }
}

#[test]
fn permissions_must_equal_the_consent() {
    let built = build("acme.github-prs", "1.0.0");
    let mut expected = expected_for(&built);
    expected.permissions.as_mut().unwrap().pop();
    assert_eq!(
        verify_package(&built.bytes, &expected, &Limits::default()).unwrap_err(),
        PackageError::PermissionsMismatch
    );
    expected.permissions = Some(Vec::new());
    assert_eq!(
        verify_package(&built.bytes, &expected, &Limits::default()).unwrap_err(),
        PackageError::PermissionsMismatch
    );
    expected.permissions = None;
    assert!(verify_package(&built.bytes, &expected, &Limits::default()).is_ok());
}

#[test]
fn oversized_archives_are_rejected_before_hashing() {
    let built = build("acme.github-prs", "1.0.0");
    let limits = Limits {
        max_archive_bytes: built.bytes.len() as u64 - 1,
        ..Limits::default()
    };
    assert!(matches!(
        verify_package(&built.bytes, &expected_for(&built), &limits),
        Err(PackageError::ArchiveTooLarge { .. })
    ));
    assert!(matches!(
        read_archive(&built.bytes, &limits),
        Err(PackageError::ArchiveTooLarge { .. })
    ));
}

// ------------------------------------------------------- hostile archives

#[test]
fn the_raw_fixture_is_accepted() {
    // Guards the hostile tests below: they differ from this only in the
    // part under test.
    assert_eq!(read(&tar_with(&[])).unwrap().len(), 7);
}

#[test]
fn path_traversal_and_absolute_paths_are_rejected() {
    for name in [
        &b"../evil.js"[..],
        b"assets/../../evil.js",
        b"/etc/passwd",
        b"./manifest.json",
        b"C:/Windows/evil.dll",
        b"assets\\..\\evil.js",
        b"assets//x.png",
        b".hidden",
    ] {
        let tar = tar_with(&[(raw_header(name, b'0', 1), b"x".to_vec())]);
        assert!(
            matches!(
                read(&tar),
                Err(PackageError::Path(PathError::Invalid { .. }))
            ),
            "{}",
            String::from_utf8_lossy(name)
        );
    }
}

#[test]
fn links_devices_directories_and_extensions_are_rejected() {
    for (typeflag, kind) in [
        (b'1', "hard link"),
        (b'2', "symbolic link"),
        (b'3', "character device"),
        (b'4', "block device"),
        (b'5', "directory"),
        (b'6', "FIFO"),
        (b'7', "contiguous file"),
        (b'x', "pax extended header"),
        (b'g', "pax extended header"),
        (b'L', "GNU long name"),
        (b'K', "GNU long name"),
        (b'S', "sparse file"),
        (0, "old-style (v7) regular file"),
    ] {
        let mut header = raw_header(b"assets/link.png", typeflag, 0);
        if typeflag == b'1' || typeflag == b'2' {
            header.set_link_name("manifest.json").unwrap();
            header.set_cksum();
        }
        let tar = tar_with(&[(header, Vec::new())]);
        match read(&tar) {
            Err(PackageError::NotRegularFile { kind: found, .. }) => assert_eq!(found, kind),
            other => panic!("{kind}: {other:?}"),
        }
    }
    // A regular file that carries a link target anyway.
    let mut header = raw_header(b"assets/x.png", b'0', 0);
    header.set_link_name("/etc/passwd").unwrap();
    header.set_cksum();
    assert!(matches!(
        read(&tar_with(&[(header, Vec::new())])),
        Err(PackageError::NotRegularFile { .. })
    ));
}

#[test]
fn symlink_to_outside_is_rejected() {
    let mut header = raw_header(b"plugin2.js", b'2', 0);
    header
        .set_link_name("../../../../home/user/.ssh/id_ed25519")
        .unwrap();
    header.set_cksum();
    assert!(matches!(
        read(&tar_with(&[(header, Vec::new())])),
        Err(PackageError::NotRegularFile { .. })
    ));
}

#[test]
fn non_ustar_headers_are_rejected() {
    let mut gnu = tar::Header::new_gnu();
    gnu.set_path("assets/x.png").unwrap();
    gnu.set_size(0);
    gnu.set_entry_type(tar::EntryType::Regular);
    gnu.set_cksum();
    assert!(matches!(
        read(&tar_with(&[(gnu, Vec::new())])),
        Err(PackageError::NotUstar { .. })
    ));
    let mut old = tar::Header::new_old();
    old.set_path("assets/x.png").unwrap();
    old.set_size(0);
    old.set_entry_type(tar::EntryType::Regular);
    old.set_cksum();
    assert!(matches!(
        read(&tar_with(&[(old, Vec::new())])),
        Err(PackageError::NotUstar { .. })
    ));
}

#[test]
fn pax_path_override_is_rejected() {
    // A pax header that would rename the next entry to ../evil.js.
    let record = b"25 path=../../../evil.js\n";
    let pax = raw_header(b"PaxHeaders/x", b'x', record.len() as u64);
    let next = raw_header(b"assets/x.png", b'0', 1);
    assert!(matches!(
        read(&tar_with(&[(pax, record.to_vec()), (next, b"x".to_vec())])),
        Err(PackageError::NotRegularFile { .. })
    ));
}

#[test]
fn duplicate_and_colliding_names_are_rejected() {
    type Check = fn(&PathError) -> bool;
    let cases: [(&[u8], Check); 5] = [
        (b"manifest.json", |e| matches!(e, PathError::Duplicate(_))),
        (b"MANIFEST.json", |e| {
            matches!(e, PathError::CaseCollision(..))
        }),
        (b"Assets/Icon-128.png", |e| {
            matches!(e, PathError::CaseCollision(..))
        }),
        (b"assets/keys/pr.png/x", |e| {
            matches!(e, PathError::FileIsDirectory(..))
        }),
        (b"LICENSE/x", |e| {
            matches!(e, PathError::FileIsDirectory(..))
        }),
    ];
    for (name, check) in cases {
        let tar = tar_with(&[(raw_header(name, b'0', 1), b"x".to_vec())]);
        match read(&tar) {
            Err(PackageError::Path(e)) => assert!(check(&e), "{e:?}"),
            other => panic!("{}: {other:?}", String::from_utf8_lossy(name)),
        }
    }
}

#[test]
fn windows_reserved_and_non_utf8_names_are_rejected() {
    for name in [&b"assets/CON.png"[..], b"aux", b"assets/lpt1", b"assets/x."] {
        let tar = tar_with(&[(raw_header(name, b'0', 1), b"x".to_vec())]);
        assert!(
            matches!(
                read(&tar),
                Err(PackageError::Path(
                    PathError::WindowsReserved(_) | PathError::TrailingDot(_)
                ))
            ),
            "{}",
            String::from_utf8_lossy(name)
        );
    }
    let tar = tar_with(&[(raw_header(b"assets/\xff\xfe.png", b'0', 1), b"x".to_vec())]);
    assert!(matches!(
        read(&tar),
        Err(PackageError::Path(PathError::NotUtf8(_)))
    ));
    let tar = tar_with(&[(
        raw_header("assets/ä.png".as_bytes(), b'0', 1),
        b"x".to_vec(),
    )]);
    assert!(matches!(
        read(&tar),
        Err(PackageError::Path(PathError::Invalid { .. }))
    ));
}

#[test]
fn gzip_bombs_are_stopped() {
    // Absolute: 11 MiB of zeros, about 11 KiB compressed.
    let zeros = vec![0u8; 11 * 1024 * 1024];
    let tar = tar_with(&[(
        raw_header(b"assets/bomb.png", b'0', zeros.len() as u64),
        zeros,
    )]);
    let bomb = gzip(&tar);
    assert!(bomb.len() < 64 * 1024);
    assert!(matches!(
        read_archive(&bomb, &Limits::default()),
        Err(PackageError::UnpackedTooLarge { .. })
    ));
    // Within the absolute limit but at a ratio no real package has.
    let zeros = vec![0u8; 4 * 1024 * 1024];
    let tar = tar_with(&[(
        raw_header(b"assets/bomb.png", b'0', zeros.len() as u64),
        zeros,
    )]);
    assert!(matches!(
        read_archive(&gzip(&tar), &Limits::default()),
        Err(PackageError::CompressionRatio { .. })
    ));
    // Same through the pinned path: hashing passes, the limits still apply.
    let built = build("acme.github-prs", "1.0.0");
    let bomb = gzip(&tar);
    assert!(matches!(
        verify_package(
            &bomb,
            &repinned(&bomb, &expected_for(&built)),
            &Limits::default()
        ),
        Err(PackageError::CompressionRatio { .. })
    ));
}

#[test]
fn size_and_count_limits_are_enforced() {
    let big = noise(3000, 7);
    let tar = tar_with(&[(raw_header(b"assets/big.png", b'0', 3000), big)]);
    let limits = Limits {
        max_file_bytes: 2000,
        ..Limits::default()
    };
    assert!(matches!(
        read_archive(&gzip(&tar), &limits),
        Err(PackageError::FileTooLarge { .. })
    ));
    let limits = Limits {
        max_unpacked_bytes: 2000,
        ..Limits::default()
    };
    assert!(matches!(
        read_archive(&gzip(&tar), &limits),
        Err(PackageError::UnpackedTooLarge { .. })
    ));
    // The declared size is checked before the data is read.
    let lying = tar_with(&[(
        raw_header(b"assets/x.png", b'0', 6 * 1024 * 1024),
        Vec::new(),
    )]);
    assert!(matches!(
        read(&lying),
        Err(PackageError::FileTooLarge { .. })
    ));
    // Entry count.
    let extra: Vec<_> = (0..300)
        .map(|i| {
            (
                raw_header(format!("assets/f{i}.png").as_bytes(), b'0', 0),
                Vec::new(),
            )
        })
        .collect();
    assert!(matches!(
        read(&tar_with(&extra)),
        Err(PackageError::TooManyEntries { limit: 256 })
    ));
    // The bundle limit.
    let built = build("acme.github-prs", "1.0.0");
    let limits = Limits {
        max_bundle_bytes: 10,
        ..Limits::default()
    };
    assert!(matches!(
        verify_package(&built.bytes, &expected_for(&built), &limits),
        Err(PackageError::BundleTooLarge { .. })
    ));
}

#[test]
fn hidden_data_and_malformed_tar_are_rejected() {
    // Non-zero bytes in a file's padding.
    let mut tar = tar_with(&[]);
    let manifest_len = files("acme.github-prs", "1.0.0")[0].1.len();
    tar[512 + manifest_len] = b'!';
    assert_eq!(read(&tar).unwrap_err(), PackageError::HiddenData);
    // Data after the end-of-archive marker.
    let mut tar = tar_with(&[]);
    tar.extend_from_slice(&[b'x'; 512]);
    assert_eq!(read(&tar).unwrap_err(), PackageError::HiddenData);
    // An entry after the end-of-archive marker.
    let mut tar = tar_with(&[]);
    push_entry(&mut tar, &raw_header(b"assets/late.png", b'0', 1), b"x");
    assert_eq!(read(&tar).unwrap_err(), PackageError::HiddenData);
    // No end-of-archive marker.
    let mut tar = tar_with(&[]);
    tar.truncate(tar.len() - 1024);
    assert!(matches!(read(&tar), Err(PackageError::Tar(_))));
    // A header with a wrong checksum.
    let mut tar = tar_with(&[]);
    tar[0] ^= 1;
    assert!(matches!(read(&tar), Err(PackageError::Tar(_))));
    // Data cut short.
    let mut tar = Vec::new();
    push_entry(
        &mut tar,
        &raw_header(b"manifest.json", b'0', 5000),
        b"short",
    );
    assert!(read(&tar).is_err());
    // Not gzip at all.
    assert_eq!(
        read_archive(&tar_with(&[]), &Limits::default()).unwrap_err(),
        PackageError::NotGzip
    );
    assert_eq!(
        read_archive(b"", &Limits::default()).unwrap_err(),
        PackageError::NotGzip
    );
}

#[test]
fn contents_must_follow_the_manifest() {
    let base = files("acme.github-prs", "1.0.0");
    for missing in ["manifest.json", "plugin.js", "LICENSE"] {
        let list: Vec<_> = base
            .iter()
            .filter(|(p, _)| *p != missing)
            .cloned()
            .collect();
        assert!(
            matches!(build_files(&list), Err(PackageError::Missing(m)) if m == missing),
            "{missing}"
        );
    }
    // A referenced asset is missing.
    let list: Vec<_> = base
        .iter()
        .filter(|(p, _)| *p != "assets/keys/pr.png")
        .cloned()
        .collect();
    assert!(matches!(build_files(&list), Err(PackageError::Contents(_))));
    // A file the manifest does not name.
    let mut list = base.clone();
    list.push(("assets/extra.png", vec![1]));
    assert!(matches!(build_files(&list), Err(PackageError::Contents(_))));
    let mut list = base.clone();
    list.push(("extra.js", vec![1]));
    assert!(matches!(build_files(&list), Err(PackageError::Contents(_))));
    // An invalid manifest.
    let mut list = base.clone();
    list[0].1 = b"{\"manifestVersion\": 2}".to_vec();
    assert!(matches!(build_files(&list), Err(PackageError::Manifest(_))));
    let mut list = base.clone();
    list[0].1 = manifest_json("deckpilot.github-prs", "1.0.0");
    assert!(matches!(build_files(&list), Err(PackageError::Manifest(_))));
    // The reader applies the same rules to an archive it did not build.
    let tar = tar_with(&[(raw_header(b"extra.js", b'0', 1), b"x".to_vec())]);
    let bytes = gzip(&tar);
    let expected = ExpectedPackage {
        id: PluginId::parse("acme.github-prs").unwrap(),
        version: semver::Version::new(1, 0, 0),
        package: PackageDigest {
            sha256: sha256(&bytes),
            size: bytes.len() as u64,
        },
        manifest_sha256: sha256(&manifest_json("acme.github-prs", "1.0.0")),
        permissions: None,
    };
    assert!(matches!(
        verify_package(&bytes, &expected, &Limits::default()),
        Err(PackageError::Contents(_))
    ));
}

#[test]
fn the_builder_refuses_bad_paths() {
    let mut builder = PackageBuilder::new();
    for bad in ["../x", "/x", "a//b", "con", "x.", "a b", ".git/config"] {
        assert!(
            matches!(builder.add(bad, vec![]), Err(PackageError::Path(_))),
            "{bad}"
        );
    }
    builder.add("README.md", vec![]).unwrap();
    assert!(matches!(
        builder.add("README.md", vec![]),
        Err(PackageError::Path(PathError::Duplicate(_)))
    ));
    assert!(matches!(
        builder.add("readme.MD", vec![]),
        Err(PackageError::Path(PathError::CaseCollision(..)))
    ));
    // 128 characters without a usable `/` cannot be a ustar name.
    let mut list = files("acme.github-prs", "1.0.0");
    let mut value: Value = serde_json::from_slice(&list[0].1).unwrap();
    let long = format!("assets/{}.png", "k".repeat(117));
    value["assets"]["keyIcons"]["long"] = Value::from(long.clone());
    list[0].1 = serde_json::to_vec_pretty(&value).unwrap();
    list.push((Box::leak(long.into_boxed_str()), vec![1]));
    assert!(matches!(
        build_files(&list),
        Err(PackageError::Path(PathError::Invalid { .. }))
    ));
}

// ------------------------------------------------------------------ unpack

#[test]
fn unpack_never_overwrites_and_cleans_up() {
    let built = build("acme.github-prs", "1.0.0");
    let verified = verify_package(&built.bytes, &expected_for(&built), &Limits::default()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join(".staging");
    let target = dir.path().join("acme.github-prs/1.0.0");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("keep"), b"old").unwrap();
    assert!(matches!(
        verified.unpack(&staging, &target),
        Err(PackageError::TargetExists(_))
    ));
    assert_eq!(std::fs::read(target.join("keep")).unwrap(), b"old");
    // An empty directory is not replaced either (rename would on Unix).
    std::fs::remove_file(target.join("keep")).unwrap();
    assert!(matches!(
        verified.unpack(&staging, &target),
        Err(PackageError::TargetExists(_))
    ));
    assert!(
        !staging.exists() || std::fs::read_dir(&staging).unwrap().count() == 0,
        "no staging leftovers"
    );
    std::fs::remove_dir(&target).unwrap();
    verified.unpack(&staging, &target).unwrap();
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
}

fn make_writable(path: &Path) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn installed_files_are_checked_at_load() {
    let built = build("acme.github-prs", "1.0.0");
    let verified = verify_package(&built.bytes, &expected_for(&built), &Limits::default()).unwrap();
    let integrity = verified.integrity().clone();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("p");
    verified
        .unpack(&dir.path().join(".staging"), &target)
        .unwrap();
    verify_installed(&target, &integrity).unwrap();

    // Modified.
    let bundle = target.join("plugin.js");
    make_writable(&bundle);
    std::fs::write(&bundle, b"steal()").unwrap();
    assert!(matches!(
        verify_installed(&target, &integrity),
        Err(PackageError::InstalledMismatch { .. })
    ));
    std::fs::write(&bundle, &verified.files()["plugin.js"]).unwrap();
    verify_installed(&target, &integrity).unwrap();

    // Extra file.
    std::fs::write(target.join("assets/extra.js"), b"x").unwrap();
    assert!(matches!(
        verify_installed(&target, &integrity),
        Err(PackageError::InstalledMismatch { .. })
    ));
    std::fs::remove_file(target.join("assets/extra.js")).unwrap();

    // Missing file.
    let license = target.join("LICENSE");
    make_writable(&license);
    std::fs::remove_file(&license).unwrap();
    assert!(matches!(
        verify_installed(&target, &integrity),
        Err(PackageError::InstalledMismatch { .. })
    ));

    // A symbolic link in place of a file.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target.join("manifest.json"), &license).unwrap();
        assert!(matches!(
            verify_installed(&target, &integrity),
            Err(PackageError::InstalledMismatch { .. })
        ));
    }
}

#[test]
fn digests_of_the_integrity_record_match_the_bytes() {
    let built = build("acme.github-prs", "1.0.0");
    let files: BTreeMap<&str, Vec<u8>> = files("acme.github-prs", "1.0.0").into_iter().collect();
    for (path, digest) in &built.integrity.files {
        assert_eq!(*digest, sha256(&files[path.as_str()]), "{path}");
    }
    assert_eq!(built.integrity.package.sha256, sha256(&built.bytes));
    assert_eq!(
        built.integrity.manifest_sha256,
        Sha256Digest::parse(&sha256(&files["manifest.json"]).to_string()).unwrap()
    );
}
