//! Verification of release artifacts before a draft release is published.

use std::path::Path;

use deckpilot_update::manifest::{Manifest, parse_version};
use deckpilot_update::signature;

/// Checks `latest.json` in `dir`: the version must equal `expected_version`,
/// every artifact URL must be a DeckPilot release download, and every
/// artifact present in `dir` must carry a valid signature for `public_key`.
/// Returns one line per verified artifact.
pub fn verify(dir: &Path, public_key: &str, expected_version: &str) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(dir.join("latest.json"))
        .map_err(|e| format!("cannot read latest.json: {e}"))?;
    let manifest = Manifest::parse(&text).map_err(|e| e.to_string())?;
    let version = parse_version(&manifest.version).map_err(|e| e.to_string())?;
    if version.to_string() != expected_version {
        return Err(format!(
            "latest.json announces {version}, but the sources are version {expected_version}"
        ));
    }
    if manifest.platforms.is_empty() {
        return Err("latest.json lists no platforms".into());
    }
    let mut verified = Vec::new();
    // URL prefix and signature format, as the client checks them. The
    // version equals the sources' (above); a pre-release version is accepted
    // here for test releases, but the stable channel never offers one.
    manifest.validate_artifacts().map_err(|e| e.to_string())?;
    for (target, artifact) in &manifest.platforms {
        let name = artifact.url.rsplit('/').next().unwrap_or_default();
        let path = dir.join(name);
        let data =
            std::fs::read(&path).map_err(|e| format!("{target}: cannot read {name}: {e}"))?;
        // As the installed app checks it: a valid signature that names the
        // announced version (the app requires it, see `requireSignedVersion`).
        signature::verify_release_artifact(public_key, &artifact.signature, &data, &version)
            .map_err(|e| format!("{target}: {name}: {e}"))?;
        verified.push(format!("{target}: {name}"));
    }
    Ok(verified)
}

/// The section of `version` in the changelog (`## [x.y.z] ...` up to the
/// next `## `), without its heading. Fails if it is missing or empty: a
/// release always has notes.
pub fn release_notes(changelog: &str, version: &str) -> Result<String, String> {
    // A test release (`0.5.0-1`) uses the notes of the version it leads to.
    if let Some((base, _)) = version.split_once('-')
        && !changelog.contains(&format!("## [{version}]"))
    {
        return release_notes(changelog, base);
    }
    let heading = format!("## [{version}]");
    let mut lines = changelog
        .lines()
        .skip_while(|line| !line.starts_with(&heading));
    if lines.next().is_none() {
        return Err(format!("CHANGELOG.md has no section for {version}"));
    }
    let notes: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    let notes = notes.join("\n").trim().to_owned();
    if notes.is_empty() {
        return Err(format!("the CHANGELOG.md section for {version} is empty"));
    }
    Ok(notes)
}

/// `verify-release`, `check-updater-key`, `verify-signature`,
/// `release-notes` and `release-tag`.
pub fn command(name: &str, args: &[String], root: &Path) -> Result<(), String> {
    match name {
        // The release workflow's classification of a tag, as `key=value`
        // lines for `GITHUB_OUTPUT`; fails for anything but a release tag.
        "release-tag" => {
            let tag = args
                .first()
                .ok_or("expected the tag, e.g. v0.6.0-alpha.1")?;
            let parsed =
                deckpilot_update::channel::ReleaseTag::parse(tag).map_err(|e| e.to_string())?;
            print!("{}", parsed.describe()?);
            Ok(())
        }
        "verify-release" => {
            let dir = args
                .first()
                .ok_or("expected the directory with the release files")?;
            let key = updater_key()?;
            let version = crate::versions::check(root, None)?;
            let verified = verify(Path::new(dir), &key, &version)?;
            for line in verified {
                println!("verified {line}");
            }
            Ok(())
        }
        "check-updater-key" => {
            let key = updater_key()?;
            deckpilot_update::signature::check_public_key(&key)
                .map_err(|e| format!("DECKPILOT_UPDATER_PUBKEY: {e}"))?;
            println!("DECKPILOT_UPDATER_PUBKEY is a valid updater public key");
            Ok(())
        }
        "verify-signature" => {
            let (Some(file), Some(signature)) = (args.first(), args.get(1)) else {
                return Err("expected the file and its .sig file".into());
            };
            let key = updater_key()?;
            let data = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let signature =
                std::fs::read_to_string(signature).map_err(|e| format!("{signature}: {e}"))?;
            deckpilot_update::signature::verify(&key, &signature, &data)
                .map_err(|e| format!("{file}: {e}"))?;
            println!("{file}: signature verified with DECKPILOT_UPDATER_PUBKEY");
            Ok(())
        }
        "release-notes" => {
            let version = crate::versions::check(root, None)?;
            let changelog = std::fs::read_to_string(root.join("CHANGELOG.md"))
                .map_err(|e| format!("CHANGELOG.md: {e}"))?;
            println!("{}", release_notes(&changelog, &version)?);
            Ok(())
        }
        other => Err(format!("unknown release command `{other}`")),
    }
}

/// The updater public key from the environment. Public, but still never
/// printed: log output should not need it.
fn updater_key() -> Result<String, String> {
    std::env::var("DECKPILOT_UPDATER_PUBKEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| "DECKPILOT_UPDATER_PUBKEY is not set".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    /// A release directory with one artifact, signed by a key made for this
    /// test only, with `signed_version` in the signed comment. Returns the
    /// directory and the public key.
    fn release_dir(
        version: &str,
        signed_version: Option<&str>,
        url_name: &str,
    ) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("deckpilot-release-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let data = b"release artifact";
        std::fs::write(dir.join("artifact.bin"), data).unwrap();
        let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let comment = match signed_version {
            Some(v) => format!("timestamp:1\tfile:artifact.bin\tversion:{v}"),
            None => "timestamp:1\tfile:artifact.bin".to_owned(),
        };
        let signature = minisign::sign(
            None,
            &keys.sk,
            std::io::Cursor::new(&data[..]),
            Some(&comment),
            None,
        )
        .unwrap();
        let manifest = serde_json::json!({
            "version": version,
            "platforms": { "linux-x86_64": {
                "signature": STANDARD.encode(signature.to_string()),
                "url": format!("{}v{version}/{url_name}", deckpilot_update::DOWNLOAD_PREFIX)
            }}
        });
        std::fs::write(dir.join("latest.json"), manifest.to_string()).unwrap();
        (dir, STANDARD.encode(keys.pk.to_box().unwrap().to_string()))
    }

    #[test]
    fn accepts_a_consistent_signed_release() {
        let (dir, key) = release_dir("1.2.3", Some("1.2.3"), "artifact.bin");
        assert_eq!(
            verify(&dir, &key, "1.2.3").unwrap(),
            ["linux-x86_64: artifact.bin"]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_version_mismatch_tampering_and_missing_files() {
        let (dir, key) = release_dir("1.2.3", Some("1.2.3"), "artifact.bin");
        assert!(
            verify(&dir, &key, "1.2.4")
                .unwrap_err()
                .contains("announces")
        );
        std::fs::write(dir.join("artifact.bin"), b"tampered").unwrap();
        assert!(
            verify(&dir, &key, "1.2.3")
                .unwrap_err()
                .contains("does not match")
        );
        std::fs::remove_dir_all(&dir).unwrap();

        let (missing, key) = release_dir("1.2.3", Some("1.2.3"), "other.bin");
        assert!(
            verify(&missing, &key, "1.2.3")
                .unwrap_err()
                .contains("cannot read")
        );
        std::fs::remove_dir_all(missing).unwrap();

        // A key other than the configured one.
        let (dir, _) = release_dir("1.2.3", Some("1.2.3"), "artifact.bin");
        let (other, other_key) = release_dir("1.2.3", Some("1.2.3"), "artifact.bin");
        assert!(
            verify(&dir, &other_key, "1.2.3")
                .unwrap_err()
                .contains("does not match")
        );
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn test_releases_with_pre_release_versions_are_verified_too() {
        let (dir, key) = release_dir("0.5.0-1", Some("0.5.0-1"), "artifact.bin");
        assert_eq!(
            verify(&dir, &key, "0.5.0-1").unwrap(),
            ["linux-x86_64: artifact.bin"]
        );
        assert!(
            verify(&dir, &key, "0.5.0")
                .unwrap_err()
                .contains("announces")
        );
        std::fs::remove_dir_all(dir).unwrap();
        // The stable channel still refuses it.
        let text = serde_json::json!({ "version": "0.5.0-1", "platforms": {} }).to_string();
        let manifest = Manifest::parse(&text).unwrap();
        assert!(
            manifest
                .update_for(&semver::Version::new(0, 4, 0), "linux-x86_64")
                .is_err()
        );
    }

    #[test]
    fn release_notes_come_from_the_changelog() {
        let changelog =
            "# Changelog\n\n## [1.2.3] - 2026-01-01\n\n### Added\n\n- A\n\n## [1.2.2]\n\n- B\n";
        assert_eq!(
            release_notes(changelog, "1.2.3").unwrap(),
            "### Added\n\n- A"
        );
        assert_eq!(release_notes(changelog, "1.2.2").unwrap(), "- B");
        assert!(
            release_notes(changelog, "1.2.4")
                .unwrap_err()
                .contains("no section")
        );
        assert_eq!(
            release_notes(changelog, "1.2.3-1").unwrap(),
            "### Added\n\n- A"
        );
        assert!(
            release_notes("## [1.0.0]\n\n## [0.9.0]\n- x", "1.0.0")
                .unwrap_err()
                .contains("empty")
        );
        // The repository's own changelog covers the current version.
        let own =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md"))
                .unwrap();
        assert!(release_notes(&own, env!("CARGO_PKG_VERSION")).is_ok());
    }

    #[test]
    fn rejects_signatures_the_app_would_refuse() {
        // Signed without the version: installations require it.
        let (dir, key) = release_dir("1.2.3", None, "artifact.bin");
        assert!(
            verify(&dir, &key, "1.2.3")
                .unwrap_err()
                .contains("does not record the version")
        );
        std::fs::remove_dir_all(dir).unwrap();
        // An older artifact announced as the new version.
        let (dir, key) = release_dir("1.2.3", Some("1.2.2"), "artifact.bin");
        assert!(
            verify(&dir, &key, "1.2.3")
                .unwrap_err()
                .contains("signed for version 1.2.2")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
