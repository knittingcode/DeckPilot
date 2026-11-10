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
    for (target, artifact) in &manifest.platforms {
        // Reuse the client-side validation (URL prefix, signature format, stable version).
        manifest
            .update_for(&semver::Version::new(0, 0, 0), target)
            .map_err(|e| format!("{target}: {e}"))?;
        let name = artifact.url.rsplit('/').next().unwrap_or_default();
        let path = dir.join(name);
        let data =
            std::fs::read(&path).map_err(|e| format!("{target}: cannot read {name}: {e}"))?;
        signature::verify(public_key, &artifact.signature, &data)
            .map_err(|e| format!("{target}: {name}: {e}"))?;
        verified.push(format!("{target}: {name}"));
    }
    Ok(verified)
}

/// `verify-release`, `check-updater-key` and `verify-signature`.
pub fn command(name: &str, args: &[String], root: &Path) -> Result<(), String> {
    match name {
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

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/deckpilot-update/tests/fixtures"
    );

    fn release_dir(version: &str, url_name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("deckpilot-release-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(
            Path::new(FIXTURES).join("artifact.bin"),
            dir.join("artifact.bin"),
        )
        .unwrap();
        let signature =
            std::fs::read_to_string(Path::new(FIXTURES).join("artifact.bin.sig")).unwrap();
        let manifest = serde_json::json!({
            "version": version,
            "platforms": { "linux-x86_64": {
                "signature": signature.trim(),
                "url": format!("{}v{version}/{url_name}", deckpilot_update::DOWNLOAD_PREFIX)
            }}
        });
        std::fs::write(dir.join("latest.json"), manifest.to_string()).unwrap();
        dir
    }

    fn key() -> String {
        std::fs::read_to_string(Path::new(FIXTURES).join("test-key.pub")).unwrap()
    }

    #[test]
    fn accepts_a_consistent_signed_release() {
        let dir = release_dir("1.2.3", "artifact.bin");
        assert_eq!(
            verify(&dir, &key(), "1.2.3").unwrap(),
            ["linux-x86_64: artifact.bin"]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_version_mismatch_tampering_and_missing_files() {
        let dir = release_dir("1.2.3", "artifact.bin");
        assert!(
            verify(&dir, &key(), "1.2.4")
                .unwrap_err()
                .contains("announces")
        );
        std::fs::write(dir.join("artifact.bin"), b"tampered").unwrap();
        assert!(
            verify(&dir, &key(), "1.2.3")
                .unwrap_err()
                .contains("does not match")
        );
        std::fs::remove_dir_all(&dir).unwrap();

        let missing = release_dir("1.2.3", "other.bin");
        assert!(
            verify(&missing, &key(), "1.2.3")
                .unwrap_err()
                .contains("cannot read")
        );
        std::fs::remove_dir_all(missing).unwrap();
    }
}
