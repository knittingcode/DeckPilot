//! Keeps the application, agent and frontend versions in sync.
//!
//! The version in `[workspace.package]` of the root `Cargo.toml` is the
//! single source: every crate, the agent and the app inherit it. The
//! frontend package and the Tauri configuration repeat it because their
//! tools need it there; `set-version` writes all of them (and `Cargo.lock`)
//! and `check-versions` refuses any difference.

use std::path::Path;

/// Version declared in `[workspace.package]` of the root `Cargo.toml`;
/// every crate, including the agent, inherits it.
fn workspace_version(root: &Path) -> Result<String, String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
        } else if in_package && let Some(value) = line.strip_prefix("version") {
            let value = value
                .trim_start()
                .trim_start_matches('=')
                .trim()
                .trim_matches('"');
            return Ok(value.to_owned());
        }
    }
    Err("no version in [workspace.package]".into())
}

fn json_version(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    value["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{} has no version", path.display()))
}

/// The versions of the workspace's own packages in `Cargo.lock`.
fn lockfile_versions(root: &Path) -> Result<Vec<(String, String)>, String> {
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    let mut name: Option<String> = None;
    for line in lock.lines() {
        if let Some(value) = line.strip_prefix("name = ") {
            name = Some(value.trim_matches('"').to_owned());
        } else if let Some(value) = line.strip_prefix("version = ")
            && let Some(name) = name.take()
            && name.starts_with("deckpilot")
        {
            found.push((name, value.trim_matches('"').to_owned()));
        }
    }
    Ok(found)
}

/// Sets `version` everywhere the application version is declared: the
/// workspace, the frontend package, the Tauri configuration and the
/// workspace's entries in `Cargo.lock`. Formatting is kept.
pub fn set(root: &Path, version: &str) -> Result<(), String> {
    semver::Version::parse(version).map_err(|e| format!("`{version}` is not a version: {e}"))?;
    let current = workspace_version(root)?;
    let replace = |relative: &str, from: &str, to: &str, once: bool| -> Result<(), String> {
        let path = root.join(relative);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{relative}: {e}"))?;
        if !text.contains(from) {
            return Err(format!("{relative} does not contain {from}"));
        }
        let updated = if once {
            text.replacen(from, to, 1)
        } else {
            text.replace(from, to)
        };
        std::fs::write(&path, updated).map_err(|e| format!("{relative}: {e}"))
    };
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let section = manifest
        .find("[workspace.package]")
        .ok_or("no [workspace.package] in Cargo.toml")?;
    let line = format!("version = \"{current}\"");
    let at = manifest[section..]
        .find(&line)
        .ok_or("no version line in [workspace.package]")?
        + section;
    let updated = format!(
        "{}version = \"{version}\"{}",
        &manifest[..at],
        &manifest[at + line.len()..]
    );
    std::fs::write(root.join("Cargo.toml"), updated).map_err(|e| e.to_string())?;
    for json in [
        "apps/desktop/package.json",
        "apps/desktop/src-tauri/tauri.conf.json",
    ] {
        replace(
            json,
            &format!("\"version\": \"{current}\""),
            &format!("\"version\": \"{version}\""),
            true,
        )?;
    }
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).map_err(|e| e.to_string())?;
    let mut out = String::with_capacity(lock.len());
    let mut ours = false;
    for line in lock.lines() {
        if let Some(name) = line.strip_prefix("name = ") {
            ours = name.trim_matches('"').starts_with("deckpilot");
        }
        if ours && line == format!("version = \"{current}\"") {
            out.push_str("version = \"");
            out.push_str(version);
            out.push('"');
            ours = false;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    std::fs::write(root.join("Cargo.lock"), out).map_err(|e| e.to_string())?;
    check(root, None).map(|_| ())
}

/// Checks that all versions match (and the release tag, if given).
pub fn check(root: &Path, tag: Option<&str>) -> Result<String, String> {
    let sources = [
        ("Cargo.toml [workspace.package]", workspace_version(root)?),
        (
            "apps/desktop/package.json",
            json_version(&root.join("apps/desktop/package.json"))?,
        ),
        (
            "apps/desktop/src-tauri/tauri.conf.json",
            json_version(&root.join("apps/desktop/src-tauri/tauri.conf.json"))?,
        ),
    ];
    let expected = sources[0].1.clone();
    let mut problems: Vec<String> = sources
        .iter()
        .filter(|(_, version)| *version != expected)
        .map(|(source, version)| format!("{source} has {version}, expected {expected}"))
        .collect();
    for (name, version) in lockfile_versions(root)? {
        if version != expected {
            problems.push(format!(
                "Cargo.lock has {name} {version}, expected {expected}"
            ));
        }
    }
    if let Some(tag) = tag
        && tag.trim_start_matches('v') != expected
    {
        problems.push(format!("tag {tag} does not match version {expected}"));
    }
    let changelog =
        std::fs::read_to_string(root.join("CHANGELOG.md")).map_err(|e| e.to_string())?;
    if let Err(problem) = crate::release::release_notes(&changelog, &expected) {
        problems.push(problem);
    }
    if problems.is_empty() {
        Ok(expected)
    } else {
        Err(problems.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_versions_are_consistent() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let version = check(&root, None).unwrap();
        assert!(check(&root, Some(&format!("v{version}"))).is_ok());
        assert!(check(&root, Some("v99.0.0")).is_err());
    }

    #[test]
    fn set_version_writes_every_declaration() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::temp_dir().join(format!("deckpilot-versions-{}", std::process::id()));
        for file in [
            "Cargo.toml",
            "Cargo.lock",
            "CHANGELOG.md",
            "apps/desktop/package.json",
            "apps/desktop/src-tauri/tauri.conf.json",
        ] {
            let target = dir.join(file);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(root.join(file), target).unwrap();
        }
        let current = check(&dir, None).unwrap();
        let mut changelog = std::fs::read_to_string(dir.join("CHANGELOG.md")).unwrap();
        changelog.push_str("\n## [9.8.7]\n\n- test\n");
        std::fs::write(dir.join("CHANGELOG.md"), changelog).unwrap();
        set(&dir, "9.8.7").unwrap();
        assert_eq!(check(&dir, Some("v9.8.7")).unwrap(), "9.8.7");
        assert!(
            std::fs::read_to_string(dir.join("Cargo.lock"))
                .unwrap()
                .contains("name = \"deckpilot-agent\"\nversion = \"9.8.7\"")
        );
        // Only the workspace's packages change.
        assert_eq!(
            std::fs::read_to_string(dir.join("Cargo.lock"))
                .unwrap()
                .matches("9.8.7")
                .count(),
            lockfile_versions(&dir).unwrap().len()
        );
        assert!(set(&dir, "not-a-version").is_err());
        assert_ne!(current, "9.8.7");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
