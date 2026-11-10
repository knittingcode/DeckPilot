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
    run(Command::new(program("pnpm"))
        .current_dir(root)
        .args([
            "--filter",
            "@deckpilot/desktop",
            "tauri",
            "build",
            "--config",
            "src-tauri/tauri.release.conf.json",
        ])
        .args(extra))
}
