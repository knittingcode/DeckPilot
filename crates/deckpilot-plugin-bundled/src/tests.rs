//! Pins, and the files of a bundled plugin against them. The fixture is
//! `plugins/first-party/test-fixtures/example-default` with a minimal ABI 1
//! module standing in for its built `plugin.wasm`.

use std::path::Path;

use super::*;

const FIXTURE: &str = "../../plugins/first-party/test-fixtures/example-default";
const ID: &str = "deckpilot.example-default";

fn fixture_file(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE).join(name)).unwrap()
}

fn id() -> PluginId {
    PluginId::parse(ID).unwrap()
}

/// `<root>/bundled/deckpilot.example-default/...` and its pins.
fn bundle(root: &Path) -> (PathBuf, Pins) {
    let bundled = root.join(BUNDLED_DIR);
    let dir = bundled.join(ID);
    std::fs::create_dir_all(dir.join(ICONS_DIR)).unwrap();
    std::fs::write(dir.join(MANIFEST), fixture_file("manifest.json")).unwrap();
    std::fs::write(dir.join(LICENSE), fixture_file("LICENSE")).unwrap();
    std::fs::write(
        dir.join(ARTIFACT),
        deckpilot_plugin_package::wasm::minimal_abi_module(),
    )
    .unwrap();
    std::fs::write(
        dir.join("assets/icon-128.png"),
        fixture_file("assets/icon-128.png"),
    )
    .unwrap();
    let pins = Pins::compute(
        &bundled,
        &[(id(), Version::new(0, 1, 0), BundleClass::Default)],
    )
    .unwrap();
    std::fs::write(bundled.join(PINS_FILE), pins.to_json_pretty()).unwrap();
    (bundled, pins)
}

#[test]
fn the_embedded_pins_parse() {
    let pins = Pins::embedded().unwrap();
    // No build bundles anything until the release enables it.
    if std::env::var_os("DECKPILOT_BUNDLED_PINS").is_none() {
        assert_eq!(pins, Pins::empty());
    }
}

#[test]
fn pins_round_trip_and_name_every_file() {
    let dir = tempfile::tempdir().unwrap();
    let (bundled, pins) = bundle(dir.path());
    let text = std::fs::read(bundled.join(PINS_FILE)).unwrap();
    assert_eq!(Pins::parse(&text).unwrap(), pins);
    let pin = pins.pinned(&id(), &Version::new(0, 1, 0)).unwrap();
    assert_eq!(pin.class, BundleClass::Default);
    assert_eq!(
        pin.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["LICENSE", "assets/icon-128.png", "manifest.json", "plugin.wasm"]
    );
    assert!(pins.pinned(&id(), &Version::new(0, 1, 1)).is_none());
    let json: serde_json::Value = serde_json::from_slice(&text).unwrap();
    assert_eq!(json["pinsVersion"], 1);
    assert_eq!(json["plugins"][0]["class"], "default");
    assert_eq!(
        json["plugins"][0]["files"]["plugin.wasm"]["size"],
        deckpilot_plugin_package::wasm::minimal_abi_module().len()
    );
}

#[test]
fn bad_pins_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (_, pins) = bundle(dir.path());
    let good = serde_json::to_value(&pins).unwrap();
    let bad = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut value = good.clone();
        edit(&mut value);
        Pins::parse(&serde_json::to_vec(&value).unwrap())
    };
    let cases: [(&str, &dyn Fn(&mut serde_json::Value)); 7] = [
        ("version 2", &|v| v["pinsVersion"] = 2.into()),
        ("third party", &|v| {
            v["plugins"][0]["id"] = "acme.example-default".into();
        }),
        ("twice", &|v| {
            let copy = v["plugins"][0].clone();
            v["plugins"].as_array_mut().unwrap().push(copy);
        }),
        ("no wasm", &|v| {
            v["plugins"][0]["files"]
                .as_object_mut()
                .unwrap()
                .remove("plugin.wasm");
        }),
        ("a package", &|v| {
            let digest = v["plugins"][0]["files"]["LICENSE"].clone();
            v["plugins"][0]["files"]["deckpilot.example-default-0.1.0.dpplugin"] = digest;
        }),
        ("a signature", &|v| {
            let digest = v["plugins"][0]["files"]["LICENSE"].clone();
            v["plugins"][0]["files"]["plugin.wasm.minisig"] = digest;
        }),
        ("unknown field", &|v| v["plugins"][0]["signature"] = "x".into()),
    ];
    for (name, edit) in cases {
        assert!(
            matches!(bad(edit), Err(BundleError::Pins(_))),
            "{name} was accepted"
        );
    }
    assert!(Pins::parse(b"not json").is_err());
}

#[test]
fn allowed_files() {
    for ok in [
        "manifest.json",
        "plugin.wasm",
        "LICENSE",
        "assets/icon-128.png",
    ] {
        assert!(is_allowed_file(ok), "{ok}");
    }
    for bad in [
        "plugin.js",
        "README.md",
        "x.dpplugin",
        "plugin.wasm.minisig",
        "assets/.png",
        "assets/icon.svg",
        "assets/keys/a.png",
        "assets/../plugin.png",
        "pins.json",
        "index.json",
    ] {
        assert!(!is_allowed_file(bad), "{bad}");
    }
}

#[test]
fn the_pinned_files_verify() {
    let dir = tempfile::tempdir().unwrap();
    let (bundled, pins) = bundle(dir.path());
    let pin = &pins.plugins[0];
    let package = verify_bundled(&bundled.join(ID), pin).unwrap();
    assert_eq!(package.manifest().id, id());
    assert_eq!(package.files().len(), 4);
    verify_installed(&bundled.join(ID), pin).unwrap();
    check_module(pin, &deckpilot_plugin_package::wasm::minimal_abi_module()).unwrap();
    assert_eq!(unpinned_entries(&bundled, &pins), Vec::<String>::new());
}

fn refused(result: Result<impl std::fmt::Debug, BundleError>, file: &str) {
    match result {
        Err(BundleError::File { path, .. }) if path == file => {}
        other => panic!("{file}: {other:?}"),
    }
}

#[test]
fn a_tampered_manifest_or_module_is_refused() {
    for file in [MANIFEST, ARTIFACT, LICENSE, "assets/icon-128.png"] {
        let dir = tempfile::tempdir().unwrap();
        let (bundled, pins) = bundle(dir.path());
        let pin = &pins.plugins[0];
        let path = bundled.join(ID).join(file);
        // Same size, one bit flipped.
        let mut bytes = std::fs::read(&path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x20;
        std::fs::write(&path, &bytes).unwrap();
        refused(verify_bundled(&bundled.join(ID), pin), file);
        refused(verify_installed(&bundled.join(ID), pin), file);
        // Another size.
        bytes.push(b'\n');
        std::fs::write(&path, &bytes).unwrap();
        refused(verify_bundled(&bundled.join(ID), pin), file);
    }
    let dir = tempfile::tempdir().unwrap();
    let (_, pins) = bundle(dir.path());
    let mut module = deckpilot_plugin_package::wasm::minimal_abi_module();
    module.push(0);
    refused(check_module(&pins.plugins[0], &module), ARTIFACT);
}

#[test]
fn extra_missing_or_linked_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (bundled, pins) = bundle(dir.path());
    let pin = &pins.plugins[0];
    let plugin = bundled.join(ID);

    std::fs::write(plugin.join("plugin.js"), "export {}").unwrap();
    refused(verify_bundled(&plugin, pin), "plugin.js");
    std::fs::remove_file(plugin.join("plugin.js")).unwrap();

    std::fs::write(plugin.join("assets/extra.png"), "x").unwrap();
    refused(verify_bundled(&plugin, pin), "assets/extra.png");
    std::fs::remove_file(plugin.join("assets/extra.png")).unwrap();

    let license = std::fs::read(plugin.join(LICENSE)).unwrap();
    std::fs::remove_file(plugin.join(LICENSE)).unwrap();
    refused(verify_bundled(&plugin, pin), LICENSE);

    #[cfg(unix)]
    {
        let outside = dir.path().join("LICENSE");
        std::fs::write(&outside, &license).unwrap();
        std::os::unix::fs::symlink(&outside, plugin.join(LICENSE)).unwrap();
        refused(verify_bundled(&plugin, pin), LICENSE);
        std::fs::remove_file(plugin.join(LICENSE)).unwrap();

        // The plugin directory itself as a link.
        std::fs::write(plugin.join(LICENSE), &license).unwrap();
        let linked = bundled.join("deckpilot.linked");
        std::os::unix::fs::symlink(&plugin, &linked).unwrap();
        refused(verify_bundled(&linked, pin), ".");
    }
    #[cfg(not(unix))]
    std::fs::write(plugin.join(LICENSE), &license).unwrap();

    // Not listed in the pins: reported, never seeded.
    std::fs::create_dir(bundled.join("deckpilot.unpinned")).unwrap();
    assert!(unpinned_entries(&bundled, &pins).contains(&"deckpilot.unpinned".to_owned()));
}

#[test]
fn a_third_party_manifest_under_first_party_pins_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (bundled, _) = bundle(dir.path());
    let plugin = bundled.join(ID);
    let text = String::from_utf8(fixture_file("manifest.json"))
        .unwrap()
        .replace("\"id\": \"deckpilot\"", "\"id\": \"acme\"");
    std::fs::write(plugin.join(MANIFEST), text).unwrap();
    // Re-pinned to the changed bytes (a bad release build): the manifest
    // rules still refuse it.
    let pins = Pins::compute(
        &bundled,
        &[(id(), Version::new(0, 1, 0), BundleClass::Default)],
    )
    .unwrap();
    let result = verify_bundled(&plugin, &pins.plugins[0]);
    assert!(
        matches!(result, Err(BundleError::Package { .. })),
        "{result:?}"
    );
}

#[test]
fn resources_are_found_next_to_an_installed_agent() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, resources) = if cfg!(target_os = "macos") {
        let contents = dir.path().join("DeckPilot.app/Contents");
        (
            contents.join("MacOS/deckpilot-agent"),
            contents.join("Resources"),
        )
    } else if cfg!(windows) {
        let install = dir.path().join("DeckPilot");
        (install.join("deckpilot-agent.exe"), install)
    } else {
        let usr = dir.path().join("usr");
        (usr.join("bin/deckpilot-agent"), usr.join("lib/DeckPilot"))
    };
    assert_eq!(resource_dirs(&exe), vec![resources.clone()]);
    assert_eq!(locate(&exe), None);
    std::fs::create_dir_all(resources.join(BUNDLED_DIR)).unwrap();
    assert_eq!(locate(&exe), Some(resources.join(BUNDLED_DIR)));
    if cfg!(target_os = "macos") {
        // A development build outside an application bundle has none.
        assert!(resource_dirs(Path::new("/tmp/target/debug/deckpilot-agent")).is_empty());
    }
}
