//! Builds installable packages that contain both executables.
//!
//! With `DECKPILOT_RELEASE_PLUGINS=1` (off by default, and off in the
//! published release pipeline until the owner enables the marketplace) the
//! package also contains the plugin runtime as app components
//! (`docs/marketplace/module-migration.md` §4.5): the agent with the
//! `plugins` feature and the pins of the bundled first-party plugins
//! compiled in, the `deckpilot-plugin-host` external binary next to it,
//! and `bundled/` as a resource (`tauri.plugins.conf.json`).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::program;

/// Rust target triple of this machine, e.g. `aarch64-apple-darwin`.
pub fn host_triple() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|e| e.to_string())?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .ok_or_else(|| "cannot determine the host target".into())
}

/// Copies a built agent to the location Tauri expects for an external
/// binary (`binaries/deckpilot-agent-<target>`), so the bundler ships it
/// next to the application executable.
pub fn stage_agent(root: &Path, agent: &Path, target: &str) -> Result<PathBuf, String> {
    let dir = root.join("apps/desktop/src-tauri/binaries");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let destination = dir.join(format!(
        "deckpilot-agent-{target}{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::copy(agent, &destination).map_err(|e| format!("{}: {e}", agent.display()))?;
    Ok(destination)
}

/// Environment variable that adds the plugin runtime and the bundled
/// first-party plugins to a package (`1`; anything else: off).
pub const RELEASE_PLUGINS_ENV: &str = "DECKPILOT_RELEASE_PLUGINS";
/// Where `plugins/first-party/build.sh` stages the bundled plugins for a
/// package (`tauri.plugins.conf.json` ships `<this>/bundled` as a resource).
pub const BUNDLED_PLUGINS_DIR: &str = "target/bundled-plugins";
/// The Tauri configuration that adds the plugin host and `bundled/`.
pub const PLUGINS_CONFIG: &str = "src-tauri/tauri.plugins.conf.json";

/// Whether this package includes the plugin runtime and bundled plugins.
pub fn release_plugins(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Copies a built plugin host to the location Tauri expects for an external
/// binary (`binaries/deckpilot-plugin-host-<target>`), next to the agent.
pub fn stage_host(root: &Path, host: &Path, target: &str) -> Result<PathBuf, String> {
    let dir = root.join("apps/desktop/src-tauri/binaries");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let destination = dir.join(format!(
        "deckpilot-plugin-host-{target}{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::copy(host, &destination).map_err(|e| format!("{}: {e}", host.display()))?;
    Ok(destination)
}

/// Tauri configuration fragment with the updater public key.
pub fn updater_config(public_key: &str) -> String {
    serde_json::json!({ "plugins": { "updater": { "pubkey": public_key.trim() } } }).to_string()
}

fn run(command: &mut Command) -> Result<(), String> {
    let status = command.status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command:?} failed with {status}"))
    }
}

/// Release build of the agent and the configuration app for this machine.
pub fn package(root: &Path, extra: &[String]) -> Result<(), String> {
    crate::versions::check(root, None)?;
    let target = host_triple()?;
    let plugins = release_plugins(std::env::var(RELEASE_PLUGINS_ENV).ok().as_deref());
    let bundled = root.join(BUNDLED_PLUGINS_DIR).join("bundled");
    if plugins {
        // The plugins first: the agent is compiled with their pins.
        println!("{RELEASE_PLUGINS_ENV}=1: packaging the plugin host and the bundled plugins");
        run(Command::new("bash").current_dir(root).args([
            "plugins/first-party/build.sh",
            "--out",
            BUNDLED_PLUGINS_DIR,
        ]))?;
    }
    let mut agent_build = Command::new("cargo");
    agent_build
        .current_dir(root)
        .args(["build", "--release", "--locked", "-p", "deckpilot-agent"]);
    if plugins {
        agent_build
            .args(["--features", "plugins"])
            .env("DECKPILOT_BUNDLED_PINS", bundled.join("pins.json"));
    } else {
        agent_build.env_remove("DECKPILOT_BUNDLED_PINS");
    }
    run(&mut agent_build)?;
    let agent = root
        .join("target/release")
        .join(format!("deckpilot-agent{}", std::env::consts::EXE_SUFFIX));
    let staged = stage_agent(root, &agent, &target)?;
    println!("staged agent: {}", staged.display());
    if plugins {
        let host_target = root.join("target/plugin-runtime");
        run(Command::new("cargo")
            .current_dir(root)
            .args([
                "build",
                "--release",
                "--locked",
                "--manifest-path",
                "plugins/runtime/Cargo.toml",
                "-p",
                "deckpilot-plugin-host",
                "--bin",
                "deckpilot-plugin-host",
                "--target-dir",
            ])
            .arg(&host_target))?;
        let host = host_target.join("release").join(format!(
            "deckpilot-plugin-host{}",
            std::env::consts::EXE_SUFFIX
        ));
        let staged = stage_host(root, &host, &target)?;
        println!("staged plugin host: {}", staged.display());
        run(Command::new("bash")
            .current_dir(root)
            .arg("tools/release-pipeline/check-bundled-plugins.sh")
            .arg(&bundled)
            .arg(&agent))?;
    }
    let mut build = Command::new(program("pnpm"));
    build.current_dir(root).args([
        "--filter",
        "@deckpilot/desktop",
        "tauri",
        "build",
        "--config",
        "src-tauri/tauri.release.conf.json",
    ]);
    if plugins {
        build.args(["--config", PLUGINS_CONFIG]);
    }
    if std::env::var_os("TAURI_SIGNING_PRIVATE_KEY").is_none_or(|key| key.is_empty()) {
        // Signed updater artifacts need the release key; local packages skip them.
        println!("TAURI_SIGNING_PRIVATE_KEY is not set: building without updater artifacts");
        build.args(["--config", r#"{"bundle":{"createUpdaterArtifacts":false}}"#]);
    } else {
        // The bundler checks the updater artifacts against the public key in
        // the configuration; it is the same key the app is compiled with.
        let key = std::env::var("DECKPILOT_UPDATER_PUBKEY")
            .ok()
            .filter(|key| !key.trim().is_empty())
            .ok_or("signing updater artifacts needs DECKPILOT_UPDATER_PUBKEY (the matching public key)")?;
        let config = root.join("target/tauri.updater.conf.json");
        std::fs::create_dir_all(root.join("target")).map_err(|e| e.to_string())?;
        std::fs::write(&config, updater_config(&key)).map_err(|e| e.to_string())?;
        build.arg("--config").arg(&config);
    }
    run(build.args(extra))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn config(name: &str) -> serde_json::Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/desktop/src-tauri")
            .join(name);
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
    }

    #[test]
    fn plugins_are_packaged_only_when_asked_for() {
        assert!(super::release_plugins(Some("1")));
        for off in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert!(!super::release_plugins(off), "{off:?}");
        }
    }

    /// The published release builds with `tauri.release.conf.json` alone:
    /// the agent and nothing else. Only the plugins configuration, applied
    /// on top with `DECKPILOT_RELEASE_PLUGINS=1`, adds the plugin host next
    /// to the agent and `bundled/` as a resource.
    #[test]
    fn the_release_configuration_has_no_plugins_and_the_plugins_one_adds_them() {
        let release = config("tauri.release.conf.json");
        assert_eq!(
            release["bundle"]["externalBin"],
            serde_json::json!(["binaries/deckpilot-agent"])
        );
        assert!(release["bundle"].get("resources").is_none());

        let plugins = config("tauri.plugins.conf.json");
        // A merge replaces arrays: the agent must be listed again.
        assert_eq!(
            plugins["bundle"]["externalBin"],
            serde_json::json!(["binaries/deckpilot-agent", "binaries/deckpilot-plugin-host"])
        );
        let resources = plugins["bundle"]["resources"].as_object().unwrap();
        assert_eq!(resources.len(), 1);
        let (source, target) = resources.iter().next().unwrap();
        assert_eq!(target, "bundled/");
        // Relative to src-tauri: the directory build.sh writes.
        assert_eq!(
            source.strip_prefix("../../../"),
            Some(format!("{}/bundled/", super::BUNDLED_PLUGINS_DIR).as_str())
        );
        // Never a package or a registry file.
        let text = plugins.to_string();
        for forbidden in [".dpplugin", ".minisig", "index", "roots.json", "site/v1"] {
            assert!(!text.contains(forbidden), "{forbidden}");
        }
    }
    #[test]
    fn the_updater_config_carries_the_public_key() {
        let config: serde_json::Value =
            serde_json::from_str(&super::updater_config("dW50cnVzdGVk\n")).unwrap();
        assert_eq!(config["plugins"]["updater"]["pubkey"], "dW50cnVzdGVk");
    }
}
