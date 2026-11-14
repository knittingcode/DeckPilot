//! Builds `guest/test-guest` for wasm32-unknown-unknown with the workspace's
//! `guest` profile and the workspace's Cargo.lock (`--locked`), into a
//! target directory of its own (so it does not wait on the outer build's
//! lock), and copies the module to `$OUT_DIR/test_guest.wasm`.
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest_dir.join("../..").canonicalize().unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    for path in [
        "guest/sdk",
        "guest/test-guest",
        "crates/abi/src",
        "Cargo.lock",
    ] {
        println!("cargo:rerun-if-changed={}", workspace.join(path).display());
    }

    let target_dir = out_dir.join("guest-target");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(&workspace)
        .args([
            "build",
            "--locked",
            "--package",
            "deckpilot-test-guest",
            "--target",
            "wasm32-unknown-unknown",
            "--profile",
            "guest",
        ])
        .arg("--target-dir")
        .arg(&target_dir)
        // Do not leak the outer build's flags or wrappers (e.g. clippy-driver).
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_PRIMARY_PACKAGE")
        .status()
        .expect("failed to run cargo for the test guest");
    assert!(
        status.success(),
        "test guest build failed; install the target with `rustup target add wasm32-unknown-unknown`"
    );
    let wasm = target_dir.join("wasm32-unknown-unknown/guest/deckpilot_test_guest.wasm");
    std::fs::copy(&wasm, out_dir.join("test_guest.wasm")).expect("copy test guest");
}
