//! Keeps the application, agent and frontend versions in sync.

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
    if let Some(tag) = tag
        && tag.trim_start_matches('v') != expected
    {
        problems.push(format!("tag {tag} does not match version {expected}"));
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
}
