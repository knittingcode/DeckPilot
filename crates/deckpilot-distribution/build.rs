//! Reads the distribution of this build **at compile time** and fails the
//! build on any value outside the accepted ones (docs/distribution/README.md,
//! section 2.2).
//!
//! * `DECKPILOT_DISTRIBUTION_CHANNEL`: `github` (the default when unset),
//!   `homebrew`, `winget`, `apt`, `flathub`, `mac_app_store` or
//!   `microsoft_store`. Set, it must be one of these exactly; an empty value
//!   is an error, not the default.
//! * `DECKPILOT_PACKAGE_FORMAT`: optional; the installed format for channels
//!   with their own build. It must belong to the channel and to the target
//!   platform.
//!
//! The validated values reach the library as `DECKPILOT_BUILD_CHANNEL` and
//! `DECKPILOT_BUILD_PACKAGE_FORMAT` (empty when not compiled in). Nothing
//! reads them again at run time.

use std::env::{self, VarError};

#[path = "src/names.rs"]
mod names;

const CHANNEL: &str = "DECKPILOT_DISTRIBUTION_CHANNEL";
const FORMAT: &str = "DECKPILOT_PACKAGE_FORMAT";

fn read(name: &str) -> Option<String> {
    println!("cargo:rerun-if-env-changed={name}");
    match env::var(name) {
        Ok(value) => Some(value),
        Err(VarError::NotPresent) => None,
        Err(VarError::NotUnicode(_)) => fail(&format!("{name} is not valid UTF-8")),
    }
}

fn fail(message: &str) -> ! {
    // A panicking build script fails the build with this message.
    panic!("deckpilot-distribution: {message}");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/names.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let Some(&(_, platform)) = names::PLATFORMS.iter().find(|(os, _)| *os == target_os) else {
        fail(&format!(
            "target OS `{target_os}` has no distribution; supported: macos, windows, linux"
        ));
    };
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if !names::ARCHITECTURES.contains(&target_arch.as_str()) {
        fail(&format!(
            "target architecture `{target_arch}` has no distribution; supported: {}",
            names::ARCHITECTURES.join(", ")
        ));
    }

    let channel = read(CHANNEL).unwrap_or_else(|| "github".to_owned());
    let Some(&(_, platforms, formats)) = names::CHANNELS.iter().find(|(name, ..)| *name == channel)
    else {
        let accepted: Vec<&str> = names::CHANNELS.iter().map(|(name, ..)| *name).collect();
        fail(&format!(
            "{CHANNEL}={channel:?} is not a distribution channel; use one of {}",
            accepted.join(", ")
        ));
    };
    if !platforms.contains(&platform) {
        fail(&format!(
            "{CHANNEL}={channel} does not build for {platform} (only {})",
            platforms.join(", ")
        ));
    }

    let format = read(FORMAT);
    if let Some(format) = &format {
        let Some(&(_, format_platform)) = names::FORMATS.iter().find(|(name, _)| name == format)
        else {
            let accepted: Vec<&str> = names::FORMATS.iter().map(|(name, _)| *name).collect();
            fail(&format!(
                "{FORMAT}={format:?} is not a package format; use one of {}",
                accepted.join(", ")
            ));
        };
        if !formats.contains(&format.as_str()) {
            fail(&format!(
                "{FORMAT}={format} is not a format of channel {channel} (only {})",
                formats.join(", ")
            ));
        }
        if format_platform != platform {
            fail(&format!(
                "{FORMAT}={format} is a {format_platform} format, but the target is {platform}"
            ));
        }
    }

    println!("cargo:rustc-env=DECKPILOT_BUILD_CHANNEL={channel}");
    println!(
        "cargo:rustc-env=DECKPILOT_BUILD_PACKAGE_FORMAT={}",
        format.unwrap_or_default()
    );
}
