//! Release builds contain no plugin code: with its default features the
//! agent depends on none of the plugin crates, and even with `plugins` it
//! never links the plugin host's engine (wasmtime), which only the
//! separate `deckpilot-plugin-host` executable contains.

use std::path::Path;
use std::process::Command;

/// The agent's normal and build dependencies (no dev-dependencies), one
/// package name per line.
fn dependencies(features: &[&str]) -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut command = Command::new(option_env!("CARGO").unwrap_or("cargo"));
    command
        .args(["tree", "--offline", "--locked", "--manifest-path"])
        .arg(&manifest)
        .args([
            "-p",
            "deckpilot-agent",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ]);
    for feature in features {
        command.args(["--features", feature]);
    }
    let output = command.output().expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn default_features_pull_no_plugin_crates() {
    let default = dependencies(&[]);
    assert!(
        default.iter().any(|p| p == "deckpilot-agent"),
        "{default:?}"
    );
    let plugin: Vec<&String> = default
        .iter()
        .filter(|p| p.starts_with("deckpilot-plugin-") || p.starts_with("wasm"))
        .collect();
    assert!(
        plugin.is_empty(),
        "the default agent build depends on plugin crates: {plugin:?}"
    );
    assert!(
        !default.iter().any(|p| p == "hkdf"),
        "`hkdf` is only for the plugins feature"
    );

    let with_plugins = dependencies(&["plugins"]);
    for expected in [
        "deckpilot-plugin-store",
        "deckpilot-plugin-registry",
        "deckpilot-plugin-package",
        "deckpilot-plugin-supervisor",
        "deckpilot-plugin-broker",
        "deckpilot-plugin-abi",
    ] {
        assert!(
            with_plugins.iter().any(|p| p == expected),
            "{expected} missing with --features plugins"
        );
    }
    assert!(
        !with_plugins
            .iter()
            .any(|p| p.starts_with("wasmtime") || p == "deckpilot-plugin-host"),
        "the agent must never link the plugin host or its engine"
    );
}
