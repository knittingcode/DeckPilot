//! Spike 4: package signing and verification.

mod common;

use std::time::Instant;

use common::*;
use marketplace_trust_spike::client::{Client, InstallError};
use marketplace_trust_spike::package::{self, ExpectedPackage, PackageError, PackageFile};
use marketplace_trust_spike::registry::sha256_hex;
use marketplace_trust_spike::sig::{self, SigError};
use marketplace_trust_spike::signer::ThrowawayKey;

/// A registry with two plugins (and a publisher key for `acme`, U-2),
/// verified by a client. Returns what the client resolves for an install.
struct World {
    reg: Registry,
    client: Client,
    _dir: tempfile::TempDir,
    publisher: ThrowawayKey,
}

fn world() -> World {
    let mut reg = Registry::new();
    let publisher = ThrowawayKey::generate();
    reg.publisher_keys
        .insert("acme".into(), publisher.public().to_owned());
    let mut clock = plugin("acme.clock", &["1.0.0", "1.1.0"]);
    for v in &mut clock.versions {
        let comment = sig::package_comment("acme.clock", &v.version, &sha256_hex(&v.bytes));
        v.publisher_signature = Some(publisher.sign(&v.bytes, &comment));
    }
    reg.publish(T0, &[clock, plugin("other.timer", &["1.0.0"])], &[]);
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::open(reg.embedded_roots(), dir.path().join("s.json")).unwrap();
    client.refresh(&reg.site, T0).unwrap();
    World {
        reg,
        client,
        _dir: dir,
        publisher,
    }
}

impl World {
    fn expect(&self, id: &str, v: &str) -> ExpectedPackage {
        self.client
            .resolve_install(&self.reg.site, id, v, T0)
            .unwrap()
    }
}

fn pkg(id: &str, v: &str) -> Vec<u8> {
    package::build(&plugin_files(id, v, 0)).unwrap()
}

// ---------------------------------------------------------------- determinism

#[test]
fn packages_are_deterministic() {
    let files = plugin_files("acme.clock", "1.0.0", 10_000);
    let a = package::build(&files).unwrap();
    let mut reversed = files.clone();
    reversed.reverse();
    let b = package::build(&reversed).unwrap();
    assert_eq!(a, b, "input order does not matter");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(package::build(&files).unwrap(), a, "time does not matter");
    // gzip header: magic, deflate, no flags, mtime 0, XFL 2 (level 9), OS 255.
    assert_eq!(&a[..10], &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 255]);
    // Entries sorted, normalized metadata.
    let raw = {
        use std::io::Read as _;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(a.as_slice())
            .read_to_end(&mut out)
            .unwrap();
        out
    };
    let mut archive = tar::Archive::new(raw.as_slice());
    let mut paths = Vec::new();
    for e in archive.entries().unwrap() {
        let e = e.unwrap();
        let h = e.header();
        assert_eq!(h.mtime().unwrap(), 0);
        assert_eq!(h.mode().unwrap(), 0o644);
        assert_eq!((h.uid().unwrap(), h.gid().unwrap()), (0, 0));
        assert_eq!(h.username_bytes(), Some(&b""[..]));
        assert!(h.as_ustar().is_some());
        paths.push(e.path().unwrap().to_string_lossy().into_owned());
    }
    assert_eq!(
        paths,
        [
            "LICENSE",
            "README.md",
            "assets/icon.png",
            "manifest.json",
            "plugin.js"
        ]
    );
    // Golden hash: changes only if the format or flate2's deflate changes.
    println!("golden sha256 {}", sha256_hex(&pkg("acme.clock", "1.0.0")));
}

#[test]
fn invalid_paths_are_refused_when_building() {
    for bad in [
        "/abs",
        "a/../b",
        "./a",
        "a//b",
        "a b",
        "a/",
        "ü.js",
        &"x".repeat(129),
    ] {
        let files = [PackageFile::new(bad, "x")];
        assert!(
            matches!(package::build(&files), Err(PackageError::BadPath(_))),
            "{bad}"
        );
    }
    let dup = [
        PackageFile::new("README.md", "a"),
        PackageFile::new("readme.md", "b"),
    ];
    assert!(matches!(
        package::build(&dup),
        Err(PackageError::DuplicatePath(_))
    ));
    let many: Vec<_> = (0..257)
        .map(|i| PackageFile::new(&format!("f{i}"), "x"))
        .collect();
    assert_eq!(package::build(&many), Err(PackageError::TooManyEntries));
}

// ---------------------------------------------------------------- accept

#[test]
fn correct_package_is_accepted_through_the_signed_chain() {
    let w = world();
    let expected = w.expect("acme.clock", "1.1.0");
    assert!(expected.publisher.is_some(), "publisher key pinned (U-2)");
    let verified = package::verify(&pkg("acme.clock", "1.1.0"), &expected).unwrap();
    assert_eq!(verified.manifest["id"], "acme.clock");
    assert_eq!(verified.files.len(), 4);
    // Without a publisher key (the #191 v1 model) the chain alone suffices.
    let other = w.expect("other.timer", "1.0.0");
    assert!(other.publisher.is_none());
    assert!(package::verify(&pkg("other.timer", "1.0.0"), &other).is_ok());
}

// ---------------------------------------------------------------- reject

#[test]
fn every_single_byte_change_is_rejected() {
    let w = world();
    let expected = w.expect("acme.clock", "1.0.0");
    let good = pkg("acme.clock", "1.0.0");
    for i in 0..good.len() {
        for flip in [0x01u8, 0x80] {
            let mut bad = good.clone();
            bad[i] ^= flip;
            assert!(
                matches!(
                    package::verify(&bad, &expected),
                    Err(PackageError::HashMismatch { .. })
                ),
                "byte {i} ^ {flip:#x}"
            );
        }
    }
}

#[test]
fn truncated_and_extended_packages_are_rejected() {
    let w = world();
    let expected = w.expect("acme.clock", "1.0.0");
    let good = pkg("acme.clock", "1.0.0");
    for len in 0..good.len() {
        assert!(
            matches!(
                package::verify(&good[..len], &expected),
                Err(PackageError::SizeMismatch { .. })
            ),
            "truncated to {len}"
        );
    }
    let mut longer = good.clone();
    longer.extend_from_slice(&[0; 512]);
    assert!(matches!(
        package::verify(&longer, &expected),
        Err(PackageError::SizeMismatch { .. })
    ));
    // Same size, different content (e.g. a zeroed partial download).
    let zeros = vec![0; good.len()];
    assert!(matches!(
        package::verify(&zeros, &expected),
        Err(PackageError::HashMismatch { .. })
    ));
}

#[test]
fn swapped_packages_are_rejected() {
    let w = world();
    let want = w.expect("acme.clock", "1.1.0");
    for (what, bytes) in [
        ("other plugin", pkg("other.timer", "1.0.0")),
        ("older version", pkg("acme.clock", "1.0.0")),
    ] {
        let r = package::verify(&bytes, &want);
        assert!(
            matches!(
                r,
                Err(PackageError::HashMismatch { .. } | PackageError::SizeMismatch { .. })
            ),
            "{what}: {r:?}"
        );
    }
}

#[test]
fn contained_manifest_must_match_id_version_and_pinned_hash() {
    // Defense in depth: even a registry bug that pins the wrong package for
    // a version is caught after unpacking.
    let older = pkg("acme.clock", "1.0.0");
    let mut expected = ExpectedPackage {
        id: "acme.clock".into(),
        version: "1.1.0".into(),
        url: release_url("acme.clock", "1.1.0"),
        sha256: sha256_hex(&older),
        size: older.len() as u64,
        manifest_sha256: sha256_hex(&manifest_bytes("acme.clock", "1.0.0")),
        publisher: None,
    };
    assert!(matches!(
        package::verify(&older, &expected),
        Err(PackageError::WrongPlugin { .. })
    ));
    expected.version = "1.0.0".into();
    assert!(package::verify(&older, &expected).is_ok());
    expected.manifest_sha256 = sha256_hex(b"something else");
    assert!(matches!(
        package::verify(&older, &expected),
        Err(PackageError::ManifestHashMismatch { .. })
    ));
}

#[test]
fn registry_chain_signed_by_a_wrong_key_yields_no_install() {
    let mut reg = Registry::new();
    reg.index_key = ThrowawayKey::generate(); // not in keys/1.json
    reg.publish(T0, &[plugin("acme.clock", &["1.0.0"])], &[]);
    let dir = tempfile::tempdir().unwrap();
    let mut c = Client::open(reg.embedded_roots(), dir.path().join("s.json")).unwrap();
    assert!(c.refresh(&reg.site, T0).is_err());
    assert_eq!(
        c.resolve_install(&reg.site, "acme.clock", "1.0.0", T0)
            .unwrap_err(),
        InstallError::NeverVerified
    );
}

#[test]
fn publisher_signature_must_be_by_the_pinned_key_for_this_exact_file() {
    let w = world();
    let bytes = pkg("acme.clock", "1.1.0");
    let base = w.expect("acme.clock", "1.1.0");
    let key = w.publisher.public().to_owned();
    let with_sig = |s: Option<String>| ExpectedPackage {
        publisher: Some((key.clone(), s)),
        ..base.clone()
    };
    let hash = sha256_hex(&bytes);

    // Wrong key, same comment.
    let stranger = ThrowawayKey::generate();
    let wrong_key = stranger.sign(&bytes, &sig::package_comment("acme.clock", "1.1.0", &hash));
    // Signature made for a different file (the 1.0.0 package).
    let other = pkg("acme.clock", "1.0.0");
    let other_file = w.publisher.sign(
        &other,
        &sig::package_comment("acme.clock", "1.0.0", &sha256_hex(&other)),
    );
    // Right file, but the signed comment names another version or plugin.
    let other_version = w
        .publisher
        .sign(&bytes, &sig::package_comment("acme.clock", "1.0.0", &hash));
    let other_plugin = w
        .publisher
        .sign(&bytes, &sig::package_comment("other.timer", "1.1.0", &hash));

    assert!(matches!(
        package::verify(&bytes, &with_sig(Some(wrong_key))),
        Err(PackageError::PublisherSignature(SigError::Signature(_)))
    ));
    assert!(matches!(
        package::verify(&bytes, &with_sig(Some(other_file))),
        Err(PackageError::PublisherSignature(SigError::Signature(_)))
    ));
    for s in [other_version, other_plugin] {
        assert!(matches!(
            package::verify(&bytes, &with_sig(Some(s))),
            Err(PackageError::PublisherSignature(
                SigError::WrongComment { .. }
            ))
        ));
    }
    assert_eq!(
        package::verify(&bytes, &with_sig(None)).unwrap_err(),
        PackageError::PublisherSignatureMissing
    );
    assert!(package::verify(&bytes, &base).is_ok());
}

// ---------------------------------------------------------------- hostile archives

/// Raw tar with full control over headers, then gzip.
fn hostile(entries: &[(&[u8], tar::EntryType, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, kind, data) in entries {
        let mut h = tar::Header::new_ustar();
        h.as_ustar_mut().unwrap().name[..name.len()].copy_from_slice(name);
        h.set_size(data.len() as u64);
        h.set_entry_type(*kind);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append(&h, *data).unwrap();
    }
    let raw = tar.into_inner().unwrap();
    gz(&raw)
}

fn gz(raw: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(raw).unwrap();
    e.finish().unwrap()
}

/// Pinned correctly: only the unpack limits stand in the way.
fn pinned(bytes: &[u8]) -> ExpectedPackage {
    ExpectedPackage {
        id: "acme.clock".into(),
        version: "1.0.0".into(),
        url: release_url("acme.clock", "1.0.0"),
        sha256: sha256_hex(bytes),
        size: bytes.len() as u64,
        manifest_sha256: sha256_hex(&manifest_bytes("acme.clock", "1.0.0")),
        publisher: None,
    }
}

#[test]
fn hostile_archives_are_rejected_on_unpack() {
    let m = manifest_bytes("acme.clock", "1.0.0");
    let reg = tar::EntryType::Regular;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("parent traversal", hostile(&[(b"../evil.js", reg, b"x")])),
        ("absolute", hostile(&[(b"/etc/evil", reg, b"x")])),
        (
            "symlink",
            hostile(&[(b"plugin.js", tar::EntryType::Symlink, b"")]),
        ),
        (
            "directory",
            hostile(&[(b"assets", tar::EntryType::Directory, b"")]),
        ),
        (
            "case duplicate",
            hostile(&[(b"manifest.json", reg, &m), (b"MANIFEST.json", reg, &m)]),
        ),
        (
            "missing plugin.js",
            hostile(&[(b"manifest.json", reg, &m), (b"LICENSE", reg, b"x")]),
        ),
        ("gzip bomb", gz(&vec![0u8; 11 * 1024 * 1024])),
        ("not gzip", b"PK\x03\x04 zip file".to_vec()),
    ];
    for (what, bytes) in cases {
        let r = package::verify(&bytes, &pinned(&bytes));
        assert!(r.is_err(), "{what}: accepted");
        println!("{what}: {}", r.unwrap_err());
    }
    // A pax long-name override is not a plain ustar entry.
    let mut tar = tar::Builder::new(Vec::new());
    let mut h = tar::Header::new_ustar();
    h.set_size(1);
    h.set_mode(0o644);
    tar.append_data(&mut h, format!("{}/x.js", "a".repeat(120)), &b"x"[..])
        .unwrap();
    let bytes = gz(&tar.into_inner().unwrap());
    assert!(package::verify(&bytes, &pinned(&bytes)).is_err());
}

// ---------------------------------------------------------------- measurement

#[test]
fn five_megabyte_package_verification_time() {
    // ~4.9 MiB of incompressible asset bytes: close to the 5 MiB cap.
    let files = plugin_files("acme.clock", "1.0.0", 4_900 * 1024);
    let t = Instant::now();
    let bytes = package::build(&files).unwrap();
    let build = t.elapsed();
    let publisher = ThrowawayKey::generate();
    let hash = sha256_hex(&bytes);
    let signature = publisher.sign(&bytes, &sig::package_comment("acme.clock", "1.0.0", &hash));
    let mut expected = pinned(&bytes);

    let time = |e: &ExpectedPackage, full: bool| {
        let runs = 10;
        let t = Instant::now();
        for _ in 0..runs {
            if full {
                package::verify(&bytes, e).unwrap();
            } else {
                package::verify_bytes(&bytes, e).unwrap();
            }
        }
        t.elapsed() / runs
    };
    let hash_only = time(&expected, false);
    let full = time(&expected, true);
    expected.publisher = Some((publisher.public().to_owned(), Some(signature)));
    let hash_and_sig = time(&expected, false);
    let full_with_sig = time(&expected, true);
    println!(
        "package {} bytes; build {build:?}; size+sha256 {hash_only:?}; \
         + publisher minisign {hash_and_sig:?}; full verify (sha256 + gunzip + untar + manifest) \
         {full:?}; full + publisher signature {full_with_sig:?}",
        bytes.len()
    );
    assert!(bytes.len() > 4_900 * 1024 && bytes.len() as u64 <= package::MAX_ARCHIVE);
    assert!(full_with_sig < std::time::Duration::from_secs(1));
}
