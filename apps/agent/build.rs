//! macOS: embeds `Info.plist` into the executable so that code signing uses
//! a fixed identifier (see the comment in that file).

fn main() {
    println!("cargo:rerun-if-changed=Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let plist =
            std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("Info.plist");
        // Only the agent binary, not tests or other targets.
        println!(
            "cargo:rustc-link-arg-bin=deckpilot-agent=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
}
