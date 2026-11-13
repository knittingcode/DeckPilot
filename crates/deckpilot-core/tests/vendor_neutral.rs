//! Core works with generic device models, capabilities and events only: no
//! dependency on a driver crate, a vendor protocol library or USB ids. Those
//! belong in the vendor adapters (e.g. `deckpilot-hid` for Elgato).

use std::fs;
use std::path::Path;

const FORBIDDEN: [&str; 5] = [
    "deckpilot-hid",
    "deckpilot_hid",
    "elgato-streamdeck",
    "elgato_streamdeck",
    "hidapi",
];

fn sources(dir: &Path, found: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push((
                path.display().to_string(),
                fs::read_to_string(&path).unwrap(),
            ));
        }
    }
}

#[test]
fn core_has_no_vendor_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let mut files = vec![("Cargo.toml".to_owned(), manifest)];
    sources(&root.join("src"), &mut files);
    for (file, text) in &files {
        for needle in FORBIDDEN {
            assert!(!text.contains(needle), "{file} mentions {needle}");
        }
        // USB vendor id of Elgato.
        assert!(
            !text.to_lowercase().contains("0x0fd9"),
            "{file} has a USB id"
        );
    }
}
