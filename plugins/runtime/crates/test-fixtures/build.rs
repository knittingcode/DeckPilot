//! Builds the wasm guests for wasm32-unknown-unknown with the workspace's
//! `guest` profile and the workspace's Cargo.lock (`--locked`), in one
//! cargo invocation, into a target directory of its own (so it does not wait
//! on the outer build's lock), and copies each module to `$OUT_DIR`:
//!
//! * `guest/test-guest` (raw ABI, deliberate attacks) → `test_guest.wasm`
//! * `guest/sdk-test-guest` (SDK, faults and lifecycle) → `sdk_test_guest.wasm`
//! * `../examples/hello-weather` (the sample plugin) → `hello_weather.wasm`
//!
//! Reproducible: pinned toolchain (rust-toolchain.toml), locked
//! dependencies, `strip = true`, no flags leaking from the outer build, and
//! the checkout and Cargo home remapped (`--remap-path-prefix`) so that
//! panic locations do not embed machine-specific paths.
use std::env;
use std::path::PathBuf;
use std::process::Command;

/// (package, module file in the target directory, file in `$OUT_DIR`).
const GUESTS: &[(&str, &str, &str)] = &[
    ("deckpilot-test-guest", "deckpilot_test_guest", "test_guest"),
    (
        "deckpilot-sdk-test-guest",
        "deckpilot_sdk_test_guest",
        "sdk_test_guest",
    ),
    ("hello-weather", "hello_weather", "hello_weather"),
];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest_dir.join("../..").canonicalize().unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    for path in [
        "guest/sdk",
        "guest/plugin-sdk",
        "guest/test-guest",
        "guest/sdk-test-guest",
        "../examples/hello-weather/src",
        "../examples/hello-weather/Cargo.toml",
        "crates/abi/src",
        "Cargo.toml",
        "Cargo.lock",
    ] {
        println!("cargo:rerun-if-changed={}", workspace.join(path).display());
    }

    let target_dir = out_dir.join("guest-target");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    // plugins/ (the runtime workspace and the examples) and Cargo's home.
    // Not canonicalized: on Windows that yields a verbatim (\\?\) path that
    // the compiler's source paths do not start with.
    let plugins_root = manifest_dir.ancestors().nth(3).unwrap().to_path_buf();
    let cargo_home = env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
        env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(|h| PathBuf::from(h).join(".cargo"))
    });
    let mut remap = vec![format!(
        "--remap-path-prefix={}=/deckpilot/plugins",
        plugins_root.display()
    )];
    if let Some(home) = cargo_home {
        remap.push(format!("--remap-path-prefix={}=/cargo", home.display()));
    }
    let mut cmd = Command::new(cargo);
    cmd.current_dir(&workspace).args(["build", "--locked"]);
    for (package, _, _) in GUESTS {
        cmd.args(["--package", package]);
    }
    let status = cmd
        .args(["--target", "wasm32-unknown-unknown", "--profile", "guest"])
        .arg("--target-dir")
        .arg(&target_dir)
        // Do not leak the outer build's flags or wrappers (e.g. clippy-driver).
        .env_remove("RUSTFLAGS")
        .env("CARGO_ENCODED_RUSTFLAGS", remap.join("\u{1f}"))
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_PRIMARY_PACKAGE")
        .status()
        .expect("failed to run cargo for the test guests");
    assert!(
        status.success(),
        "guest build failed; install the target with `rustup target add wasm32-unknown-unknown`"
    );
    for (_, artifact, file) in GUESTS {
        let wasm = target_dir.join(format!("wasm32-unknown-unknown/guest/{artifact}.wasm"));
        std::fs::copy(&wasm, out_dir.join(format!("{file}.wasm")))
            .unwrap_or_else(|e| panic!("copy {}: {e}", wasm.display()));
    }
}
