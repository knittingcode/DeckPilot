//! Builds installable packages that contain both executables.
//!
//! With `DECKPILOT_RELEASE_PLUGINS=1` (off by default here; the release
//! workflow always builds this layout, `tools/ci/stage-plugin-runtime.sh`)
//! the package also contains the plugin runtime as app components
//! (`docs/marketplace/module-migration.md` §4.5): the agent with the
//! `plugins` feature and the pins of the bundled first-party plugins
//! compiled in, the `deckpilot-plugin-host` external binary next to it,
//! and `bundled/` as a resource (`tauri.plugins.conf.json`).
//!
//! On macOS the host is not an external binary: the bundler would sign it
//! with the entitlements of the whole app (none), and under the hardened
//! runtime wasmtime's executable memory needs
//! `com.apple.security.cs.allow-unsigned-executable-memory`. With a signing
//! identity (`APPLE_SIGNING_IDENTITY`) the host is signed first, with only
//! that entitlement (`tools/ci/sign-macos-plugin-host.sh`), and copied into
//! `Contents/MacOS` as it is (`tauri.plugins.macos.conf.json`); the bundler
//! signs the app around it, and `tools/ci/check-macos-entitlements.sh`
//! checks the result (docs/marketplace/distribution.md, "macOS: the plugin
//! host's entitlement").

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
/// On macOS, on top of [`PLUGINS_CONFIG`]: the host as a file copied into
/// `Contents/MacOS` (signed before, with its entitlement) instead of an
/// external binary the bundler would sign without it.
pub const PLUGINS_MACOS_CONFIG: &str = "src-tauri/tauri.plugins.macos.conf.json";
/// Where the macOS host is staged (relative to `apps/desktop/src-tauri`), as
/// [`PLUGINS_MACOS_CONFIG`] names it.
pub const MACOS_STAGED_HOST: &str = "binaries/macos/deckpilot-plugin-host";

/// The Tauri configurations of a package, in merge order.
pub fn configs(plugins: bool, macos: bool) -> Vec<&'static str> {
    let mut configs = vec!["src-tauri/tauri.release.conf.json"];
    if plugins {
        configs.push(PLUGINS_CONFIG);
        if macos {
            configs.push(PLUGINS_MACOS_CONFIG);
        }
    }
    configs
}

/// Whether the macOS plugin host is signed before bundling, from
/// `APPLE_SIGNING_IDENTITY` and whether `APPLE_CERTIFICATE` is set: with an
/// identity, yes (the bundler signs the app with it); with neither, no (the
/// bundler signs nothing either). A certificate without the identity would
/// make the bundler sign the app but leave the host without its signature
/// and entitlement, so it is refused.
pub fn sign_macos_host(identity: Option<&str>, certificate: bool) -> Result<bool, String> {
    match identity.map(str::trim).filter(|i| !i.is_empty()) {
        Some(_) => Ok(true),
        None if certificate => Err(format!(
            "{RELEASE_PLUGINS_ENV}=1 on macOS signs the plugin host before bundling: set \
             APPLE_SIGNING_IDENTITY to the identity of APPLE_CERTIFICATE (in the keychain \
             search list)"
        )),
        None => Ok(false),
    }
}

/// Whether this package includes the plugin runtime and bundled plugins.
pub fn release_plugins(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Copies a built plugin host to the location Tauri expects for an external
/// binary (`binaries/deckpilot-plugin-host-<target>`), next to the agent;
/// on macOS to [`MACOS_STAGED_HOST`] instead.
pub fn stage_host(root: &Path, host: &Path, target: &str, macos: bool) -> Result<PathBuf, String> {
    let tauri = root.join("apps/desktop/src-tauri");
    let destination = if macos {
        tauri.join(MACOS_STAGED_HOST)
    } else {
        tauri.join("binaries").join(format!(
            "deckpilot-plugin-host-{target}{}",
            std::env::consts::EXE_SUFFIX
        ))
    };
    if let Some(dir) = destination.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
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

/// Builds the plugin host and stages it; on macOS with a signing identity,
/// signs it with its one entitlement.
fn package_host(root: &Path, target: &str, macos: bool, sign: bool) -> Result<(), String> {
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
    let staged = stage_host(root, &host, target, macos)?;
    println!("staged plugin host: {}", staged.display());
    if sign {
        // Only the host gets an entitlement; the bundler signs the app and
        // the agent without any.
        run(Command::new("bash")
            .current_dir(root)
            .arg("tools/ci/sign-macos-plugin-host.sh")
            .arg(&staged))?;
    } else if macos {
        println!(
            "APPLE_SIGNING_IDENTITY is not set: the plugin host keeps the linker's ad hoc \
             signature, like the rest of this unsigned package"
        );
    }
    Ok(())
}

/// Release build of the agent and the configuration app for this machine.
pub fn package(root: &Path, extra: &[String]) -> Result<(), String> {
    crate::versions::check(root, None)?;
    let target = host_triple()?;
    let plugins = release_plugins(std::env::var(RELEASE_PLUGINS_ENV).ok().as_deref());
    let macos = cfg!(target_os = "macos");
    let sign_host = plugins
        && macos
        && sign_macos_host(
            std::env::var("APPLE_SIGNING_IDENTITY").ok().as_deref(),
            std::env::var_os("APPLE_CERTIFICATE").is_some_and(|c| !c.is_empty()),
        )?;
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
        package_host(root, &target, macos, sign_host)?;
        run(Command::new("bash")
            .current_dir(root)
            .arg("tools/release-pipeline/check-bundled-plugins.sh")
            .arg(&bundled)
            .arg(&agent))?;
    }
    let mut build = Command::new(program("pnpm"));
    build
        .current_dir(root)
        .args(["--filter", "@deckpilot/desktop", "tauri", "build"]);
    for config in configs(plugins, macos) {
        build.args(["--config", config]);
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
    run(build.args(extra))?;
    if sign_host {
        // The app as bundled: exactly the host's entitlement on exactly the
        // host, none on the app or the agent.
        let app = root.join("target/release/bundle/macos/DeckPilot.app");
        if app.is_dir() {
            run(Command::new("bash")
                .current_dir(root)
                .arg("tools/ci/check-macos-entitlements.sh")
                .arg(&app))?;
        }
    }
    Ok(())
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

    /// `tauri.release.conf.json` alone bundles the agent and nothing else.
    /// The plugins configuration, applied on top (by the release and
    /// Packaging workflows, and with `DECKPILOT_RELEASE_PLUGINS=1` here),
    /// adds the plugin host next to the agent and `bundled/` as a resource.
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
    /// Without `DECKPILOT_RELEASE_PLUGINS=1` the package is built with the
    /// release configuration alone, as before, on every platform.
    #[test]
    fn the_configurations_are_the_release_one_unless_plugins_are_packaged() {
        for macos in [false, true] {
            assert_eq!(
                super::configs(false, macos),
                ["src-tauri/tauri.release.conf.json"]
            );
        }
        assert_eq!(
            super::configs(true, false),
            ["src-tauri/tauri.release.conf.json", super::PLUGINS_CONFIG]
        );
        assert_eq!(
            super::configs(true, true),
            [
                "src-tauri/tauri.release.conf.json",
                super::PLUGINS_CONFIG,
                super::PLUGINS_MACOS_CONFIG
            ]
        );
    }

    /// On macOS the host is copied into Contents/MacOS as signed before
    /// bundling (with its entitlement), not an external binary the bundler
    /// would sign with the app's (none); the agent stays an external binary.
    #[test]
    fn the_macos_plugins_configuration_copies_the_signed_host() {
        let macos = config("tauri.plugins.macos.conf.json");
        assert_eq!(
            macos["bundle"]["externalBin"],
            serde_json::json!(["binaries/deckpilot-agent"])
        );
        assert_eq!(
            macos["bundle"]["macOS"]["files"],
            serde_json::json!({ "MacOS/deckpilot-plugin-host": super::MACOS_STAGED_HOST })
        );
        // Nothing else: the merge keeps bundled/ and the hardened runtime.
        assert_eq!(macos["bundle"].as_object().unwrap().len(), 2);
        assert_eq!(macos["bundle"]["macOS"].as_object().unwrap().len(), 1);
    }

    /// No configuration sets entitlements for the app: the bundler would
    /// apply them to the app, the agent and every external binary. The
    /// plugin host's one entitlement is applied to the host alone, by
    /// tools/ci/sign-macos-plugin-host.sh.
    #[test]
    fn no_configuration_sets_app_entitlements() {
        for name in [
            "tauri.conf.json",
            "tauri.release.conf.json",
            "tauri.plugins.conf.json",
            "tauri.plugins.macos.conf.json",
        ] {
            let text = config(name).to_string();
            assert!(!text.contains("ntitlements"), "{name}");
        }
        assert_eq!(
            config("tauri.conf.json")["bundle"]["macOS"]["hardenedRuntime"],
            true
        );
    }

    #[test]
    fn the_macos_host_is_signed_with_the_apps_identity() {
        use super::sign_macos_host;
        assert_eq!(sign_macos_host(None, false), Ok(false));
        assert_eq!(sign_macos_host(Some(" "), false), Ok(false));
        assert_eq!(sign_macos_host(Some("-"), false), Ok(true));
        let identity = "Developer ID Application: Name (TEAMID)";
        assert_eq!(sign_macos_host(Some(identity), false), Ok(true));
        assert_eq!(sign_macos_host(Some(identity), true), Ok(true));
        // The bundler would sign the app with the certificate and leave the
        // host without its signature.
        assert!(sign_macos_host(None, true).is_err());
        assert!(sign_macos_host(Some(""), true).is_err());
    }

    #[test]
    fn the_host_is_staged_where_the_configurations_name_it() {
        let root =
            std::env::temp_dir().join(format!("deckpilot-stage-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let built = root.join("built-host");
        std::fs::write(&built, b"host").unwrap();
        let mac = super::stage_host(&root, &built, "aarch64-apple-darwin", true).unwrap();
        assert_eq!(
            mac,
            root.join("apps/desktop/src-tauri")
                .join(super::MACOS_STAGED_HOST)
        );
        let other = super::stage_host(&root, &built, "x86_64-unknown-linux-gnu", false).unwrap();
        assert_eq!(
            other,
            root.join("apps/desktop/src-tauri/binaries").join(format!(
                "deckpilot-plugin-host-x86_64-unknown-linux-gnu{}",
                std::env::consts::EXE_SUFFIX
            ))
        );
        assert_eq!(std::fs::read(&mac).unwrap(), b"host");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_updater_config_carries_the_public_key() {
        let config: serde_json::Value =
            serde_json::from_str(&super::updater_config("dW50cnVzdGVk\n")).unwrap();
        assert_eq!(config["plugins"]["updater"]["pubkey"], "dW50cnVzdGVk");
    }
}
