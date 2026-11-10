//! Builds installable packages that contain both executables.

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
    run(Command::new("cargo").current_dir(root).args([
        "build",
        "--release",
        "--locked",
        "-p",
        "deckpilot-agent",
    ]))?;
    let agent = root
        .join("target/release")
        .join(format!("deckpilot-agent{}", std::env::consts::EXE_SUFFIX));
    let staged = stage_agent(root, &agent, &target)?;
    println!("staged agent: {}", staged.display());
    let mut build = Command::new(program("pnpm"));
    build.current_dir(root).args([
        "--filter",
        "@deckpilot/desktop",
        "tauri",
        "build",
        "--config",
        "src-tauri/tauri.release.conf.json",
    ]);
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
    #[test]
    fn the_updater_config_carries_the_public_key() {
        let config: serde_json::Value =
            serde_json::from_str(&super::updater_config("dW50cnVzdGVk\n")).unwrap();
        assert_eq!(config["plugins"]["updater"]["pubkey"], "dW50cnVzdGVk");
    }
}
