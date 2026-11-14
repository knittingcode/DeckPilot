//! Builds the wasm guest in `guest/` for wasm32-unknown-unknown and copies
//! it to `$OUT_DIR/guest.wasm`. A separate target directory avoids a lock
//! on the outer build's target directory.
use std::{env, path::PathBuf, process::Command};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let guest = manifest_dir.join("guest");
    println!("cargo:rerun-if-changed=guest/src");
    println!("cargo:rerun-if-changed=guest/Cargo.toml");

    let target_dir = out_dir.join("guest-target");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--manifest-path",
        ])
        .arg(guest.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir)
        // Do not leak the host build's flags into the guest build.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .status()
        .expect("failed to run cargo for the guest");
    assert!(
        status.success(),
        "guest build failed; run `rustup target add wasm32-unknown-unknown`"
    );
    let wasm = target_dir.join("wasm32-unknown-unknown/release/spike_guest.wasm");
    std::fs::copy(&wasm, out_dir.join("guest.wasm")).expect("copy guest.wasm");
}
