//! Platform metadata for the agent executable (only the `deckpilot-agent`
//! binary, not tests or other targets):
//!
//! - macOS: embeds `Info.plist` so that code signing uses a fixed
//!   identifier (see the comment in that file).
//! - Windows (MSVC): embeds an application manifest that requests Common
//!   Controls 6. The tray menu (`muda`) imports `TaskDialogIndirect`, which
//!   only version 6 of `comctl32.dll` exports; without the manifest Windows
//!   loads version 5 and refuses to start the executable
//!   (`STATUS_ENTRYPOINT_NOT_FOUND`, 0xC0000139), even with `--headless`.

fn main() {
    println!("cargo:rerun-if-changed=Info.plist");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "macos" {
        let plist =
            std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("Info.plist");
        println!(
            "cargo:rustc-link-arg-bin=deckpilot-agent=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
    if os == "windows" && env == "msvc" {
        println!("cargo:rustc-link-arg-bin=deckpilot-agent=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=deckpilot-agent=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
}
