//! What a release build carries (docs/distribution/README.md, sections 2.2
//! and 2.3, rule R9).
//!
//! The test builds this crate's `distribution` example with Cargo, in its
//! own target directory, once in the release profile and once in the debug
//! profile, and checks the binaries:
//!
//! * the release binary does not contain the development override's
//!   variable name, and setting the variable does not change its record;
//! * it carries the record as text (`RECORD`), which the packaging and
//!   release checks read from built binaries;
//! * the debug binary contains the name and honours the override, which
//!   shows that the search above would have found it;
//! * invalid compile-time values fail the build.
//!
//! The crate has no dependencies, so the builds take seconds.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Spelled in two parts so that this test's own binary is the only place
/// the name is assembled; the checked binaries are the example's.
fn override_variable() -> String {
    ["DECKPILOT_DISTRIBUTION", "_OVERRIDE"].concat()
}

const BUILD_VARIABLES: [&str; 2] = ["DECKPILOT_DISTRIBUTION_CHANNEL", "DECKPILOT_PACKAGE_FORMAT"];

/// One target directory per test, so that the tests can run in parallel.
fn target_dir(test: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("deckpilot-distribution-{test}"))
}

fn cargo_build(test: &str, release: bool, env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "build",
            "--locked",
            "--quiet",
            "-p",
            "deckpilot-distribution",
            "--example",
            "distribution",
            "--target-dir",
        ])
        .arg(target_dir(test))
        .env("CARGO_INCREMENTAL", "0")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove(override_variable());
    if release {
        command.arg("--release");
    }
    for variable in BUILD_VARIABLES {
        command.env_remove(variable);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("cargo runs")
}

fn build(release: bool) -> PathBuf {
    let output = cargo_build("profiles", release, &[]);
    assert!(
        output.status.success(),
        "the example builds: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let profile = if release { "release" } else { "debug" };
    target_dir("profiles")
        .join(profile)
        .join("examples")
        .join(format!("distribution{}", std::env::consts::EXE_SUFFIX))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// The record the binary prints, with the override set to `override_value`.
fn record(binary: &Path, override_value: Option<&str>) -> String {
    let mut command = Command::new(binary);
    command.env_remove(override_variable());
    if let Some(value) = override_value {
        command.env(override_variable(), value);
    }
    let output = command.output().expect("the example runs");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).expect("UTF-8 output")
}

fn field<'a>(record: &'a str, key: &str) -> &'a str {
    record
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("{key} missing from {record}"))
}

#[test]
fn the_release_build_has_no_override_and_the_debug_build_does() {
    let name = override_variable();

    let release = build(true);
    let bytes = std::fs::read(&release).expect("release binary");
    assert!(
        !contains(&bytes, name.as_bytes()),
        "{name} occurs in the release binary {}",
        release.display()
    );
    // The record the packaging and release checks read from a binary
    // without running it (tools/ci/check-packages.sh record).
    let embedded = format!(
        "deckpilot-distribution-record:1;channel=github;format=;version={};end",
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        contains(&bytes, embedded.as_bytes()),
        "{embedded} is not embedded in {}",
        release.display()
    );
    let plain = record(&release, None);
    assert_eq!(field(&plain, "distribution_channel"), "github");
    assert_eq!(field(&plain, "update_provider"), "github_release");
    assert_eq!(field(&plain, "development_override"), "false");
    assert_eq!(
        field(&plain, "application_version"),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(field(&plain, "platform"), std::env::consts::OS);
    assert_eq!(field(&plain, "architecture"), std::env::consts::ARCH);
    // R9: the environment cannot change a release build's record.
    for value in ["apt", "mac_app_store", "microsoft_store:msix", "nonsense"] {
        assert_eq!(record(&release, Some(value)), plain, "{name}={value}");
    }

    // The positive control: the same search finds the name in a debug
    // build, which honours the override.
    let debug = build(false);
    let bytes = std::fs::read(&debug).expect("debug binary");
    assert!(
        contains(&bytes, name.as_bytes()),
        "{name} should occur in the debug binary {}",
        debug.display()
    );
    assert_eq!(record(&debug, None), plain);
    let apt = record(&debug, Some("apt"));
    assert_eq!(field(&apt, "distribution_channel"), "apt");
    assert_eq!(field(&apt, "update_provider"), "external_package_manager");
    assert_eq!(field(&apt, "package_format"), "deb");
    assert_eq!(field(&apt, "development_override"), "true");
}

#[test]
fn invalid_compile_time_values_fail_the_build() {
    // A channel that never builds for the host platform.
    let foreign = match std::env::consts::OS {
        "linux" => "mac_app_store",
        _ => "apt",
    };
    // A format of another platform.
    let foreign_format = match std::env::consts::OS {
        "windows" => "deb",
        _ => "msi",
    };
    let cases: [&[(&str, &str)]; 6] = [
        &[("DECKPILOT_DISTRIBUTION_CHANNEL", "unknown")],
        &[("DECKPILOT_DISTRIBUTION_CHANNEL", "GitHub")],
        &[("DECKPILOT_DISTRIBUTION_CHANNEL", "")],
        &[("DECKPILOT_DISTRIBUTION_CHANNEL", foreign)],
        &[("DECKPILOT_PACKAGE_FORMAT", "zip")],
        &[
            ("DECKPILOT_DISTRIBUTION_CHANNEL", "github"),
            ("DECKPILOT_PACKAGE_FORMAT", foreign_format),
        ],
    ];
    for env in cases {
        let output = cargo_build("invalid", false, env);
        assert!(!output.status.success(), "{env:?} must fail the build");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("deckpilot-distribution:"),
            "{env:?}: {stderr}"
        );
    }
    // A valid explicit value builds.
    let output = cargo_build(
        "invalid",
        false,
        &[("DECKPILOT_DISTRIBUTION_CHANNEL", "github")],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
