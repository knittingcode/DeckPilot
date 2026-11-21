//! Pins, and the files of a bundled plugin against them. The fixture is
//! `plugins/first-party/test-fixtures/example-default` with a minimal ABI 1
//! module standing in for its built `plugin.wasm`.

use std::path::Path;

use super::*;

const FIXTURE: &str = "../../plugins/first-party/test-fixtures/example-default";
const ID: &str = "deckpilot.example-default";

fn fixture_file(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURE)
            .join(name),
    )
    .unwrap()
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
        [
            "LICENSE",
            "assets/icon-128.png",
            "manifest.json",
            "plugin.wasm"
        ]
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

type Edit<'a> = &'a dyn Fn(&mut serde_json::Value);

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
    let cases: [(&str, Edit<'_>); 7] = [
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
        ("unknown field", &|v| {
            v["plugins"][0]["signature"] = "x".into();
        }),
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
        // Plugin API 1.8 visual assets: PNG or JPEG, nested under assets/.
        "assets/keys/a.png",
        "assets/visual/Sky_Blue.jpg",
        "assets/bg.jpeg",
        // Plugin API 1.6 translations of canonical tags.
        "locales/en.json",
        "locales/pl.json",
        "locales/pt-BR.json",
        "locales/zh-Hant-TW.json",
        "locales/es-419.json",
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
        "assets/icon.webp",
        "assets/icon.PNG",
        "assets/icon.png.js",
        "assets/.hidden.png",
        "assets/con.png",
        "assets/x./a.png",
        "assets/../plugin.png",
        "assets//a.png",
        "assets\\a.png",
        "icon.png",
        "other/icon.png",
        "locales/EN.json",
        "locales/pl_PL.json",
        "locales/pl-pl.json",
        "locales/english.json",
        "locales/en.yaml",
        "locales/en.json.js",
        "locales/x/en.json",
        "locales/.json",
        "locales/../en.json",
        "assets/en.json",
        "pins.json",
        "index.json",
    ] {
        assert!(!is_allowed_file(bad), "{bad}");
    }
}

#[test]
fn oversized_images_and_translations_are_refused_in_the_pins() {
    let dir = tempfile::tempdir().unwrap();
    let (_, pins) = bundle(dir.path());
    let good = serde_json::to_value(&pins).unwrap();
    let with = |file: &str, size: usize| {
        let mut value = good.clone();
        let mut digest = value["plugins"][0]["files"]["LICENSE"].clone();
        digest["size"] = size.into();
        value["plugins"][0]["files"][file] = digest;
        Pins::parse(&serde_json::to_vec(&value).unwrap())
    };
    let locale = deckpilot_plugin_manifest::locale::MAX_LOCALE_FILE_BYTES;
    let image = deckpilot_plugin_manifest::MAX_BACKGROUND_BYTES;
    with("locales/pl.json", locale).unwrap();
    with("assets/visual/sky.jpg", image).unwrap();
    assert!(matches!(
        with("locales/pl.json", locale + 1),
        Err(BundleError::Pins(_))
    ));
    assert!(matches!(
        with("assets/visual/sky.jpg", image + 1),
        Err(BundleError::Pins(_))
    ));
    assert!(matches!(
        with("locales/pl-pl.json", 10),
        Err(BundleError::Pins(_))
    ));
}

/// A PNG header of `width` x `height` (no image data: only the header is
/// read).
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

/// A JPEG with an APP0 segment and a frame header of `width` x `height`.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];
    bytes.extend([0xff, 0xc0, 0x00, 0x0b, 0x08]);
    bytes.extend(height.to_be_bytes());
    bytes.extend(width.to_be_bytes());
    bytes.extend([0x01, 0x01, 0x11, 0x00]);
    bytes
}

const EN: &str = r#"{"plugin": {"name": "Bundled Example"}}"#;
const PL: &str = r#"{"plugin": {"name": "Przykład wbudowany"}}"#;

/// The fixture at plugin API 1.8 with translations (`locales/en.json`,
/// `locales/pl.json`) and visual assets (a PNG icon and a JPEG
/// background nested under `assets/visual/`), staged and pinned as
/// `plugins/first-party/build.sh` does.
fn localized_bundle(root: &Path) -> (PathBuf, Pins) {
    let bundled = root.join(BUNDLED_DIR);
    let dir = bundled.join(ID);
    bundle(root);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fixture_file("manifest.json")).unwrap();
    manifest["engines"]["pluginApi"] = "^1.8".into();
    manifest["name"] = "t:plugin.name".into();
    manifest["locales"] = serde_json::json!(["en", "pl"]);
    manifest["contributes"]["visualAssets"] = serde_json::json!({
        "icons": [{"id": "wave", "name": "Wave", "asset": "assets/visual/wave.png"}],
        "backgrounds": [
            {"id": "sky", "name": "Sky", "type": "image", "asset": "assets/visual/sky.jpg"}
        ]
    });
    std::fs::write(
        dir.join(MANIFEST),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("assets/visual")).unwrap();
    std::fs::create_dir_all(dir.join(LOCALES_DIR)).unwrap();
    std::fs::write(dir.join("assets/visual/wave.png"), png(64, 64)).unwrap();
    std::fs::write(dir.join("assets/visual/sky.jpg"), jpeg(400, 200)).unwrap();
    std::fs::write(dir.join("locales/en.json"), EN).unwrap();
    std::fs::write(dir.join("locales/pl.json"), PL).unwrap();
    let pins = Pins::compute(
        &bundled,
        &[(id(), Version::new(0, 1, 0), BundleClass::Default)],
    )
    .unwrap();
    std::fs::write(bundled.join(PINS_FILE), pins.to_json_pretty()).unwrap();
    (bundled, pins)
}

#[test]
fn translations_and_visual_assets_are_pinned_and_verify() {
    let dir = tempfile::tempdir().unwrap();
    let (bundled, pins) = localized_bundle(dir.path());
    let text = std::fs::read(bundled.join(PINS_FILE)).unwrap();
    assert_eq!(Pins::parse(&text).unwrap(), pins);
    let pin = &pins.plugins[0];
    assert_eq!(
        pin.files.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "LICENSE",
            "assets/icon-128.png",
            "assets/visual/sky.jpg",
            "assets/visual/wave.png",
            "locales/en.json",
            "locales/pl.json",
            "manifest.json",
            "plugin.wasm"
        ]
    );
    let package = verify_bundled(&bundled.join(ID), pin).unwrap();
    assert_eq!(package.files().len(), 8);
    verify_installed(&bundled.join(ID), pin).unwrap();
}

#[test]
fn tampered_or_extra_translations_and_visual_assets_are_refused() {
    for file in ["locales/pl.json", "assets/visual/sky.jpg"] {
        let dir = tempfile::tempdir().unwrap();
        let (bundled, pins) = localized_bundle(dir.path());
        let path = bundled.join(ID).join(file);
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&path, &bytes).unwrap();
        refused(verify_bundled(&bundled.join(ID), &pins.plugins[0]), file);
        refused(verify_installed(&bundled.join(ID), &pins.plugins[0]), file);
    }
    let dir = tempfile::tempdir().unwrap();
    let (bundled, pins) = localized_bundle(dir.path());
    let plugin = bundled.join(ID);
    let pin = &pins.plugins[0];
    // Unpinned files next to the pinned ones.
    for (file, bytes) in [
        ("locales/de.json", EN.as_bytes().to_vec()),
        ("assets/visual/extra.png", png(8, 8)),
        ("locales/notes.txt", b"x".to_vec()),
    ] {
        std::fs::write(plugin.join(file), bytes).unwrap();
        refused(verify_bundled(&plugin, pin), file);
        std::fs::remove_file(plugin.join(file)).unwrap();
    }
    // No directory under locales/ (translations are directly in it).
    std::fs::create_dir(plugin.join("locales/nested")).unwrap();
    refused(verify_bundled(&plugin, pin), "locales/nested");
    std::fs::remove_dir(plugin.join("locales/nested")).unwrap();
    // Another top-level directory.
    std::fs::create_dir(plugin.join("scripts")).unwrap();
    refused(verify_bundled(&plugin, pin), "scripts");
    std::fs::remove_dir(plugin.join("scripts")).unwrap();
    verify_bundled(&plugin, pin).unwrap();

    // Pinned and allowed, but invalid for the package rules: a translation
    // that is not a translation file, and a visual asset the manifest does
    // not name (a bad release build that pinned whatever it staged).
    std::fs::write(plugin.join("locales/pl.json"), "[]").unwrap();
    let repinned = Pins::compute(
        &bundled,
        &[(id(), Version::new(0, 1, 0), BundleClass::Default)],
    )
    .unwrap();
    let result = verify_bundled(&plugin, &repinned.plugins[0]);
    assert!(
        matches!(result, Err(BundleError::Package { .. })),
        "{result:?}"
    );
    std::fs::write(plugin.join("locales/pl.json"), PL).unwrap();
    std::fs::write(plugin.join("assets/visual/unnamed.png"), png(8, 8)).unwrap();
    let repinned = Pins::compute(
        &bundled,
        &[(id(), Version::new(0, 1, 0), BundleClass::Default)],
    )
    .unwrap();
    let result = verify_bundled(&plugin, &repinned.plugins[0]);
    assert!(
        matches!(result, Err(BundleError::Package { .. })),
        "{result:?}"
    );
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
        assert_eq!(
            resource_dirs(Path::new("/tmp/target/debug/deckpilot-agent")),
            Vec::<PathBuf>::new()
        );
    }
}

#[test]
fn the_bundled_ids_are_those_of_the_release_bundle() {
    // plugins/first-party/bundled.json lists the plugin directories; each
    // manifest names its id.
    let first_party = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/first-party");
    let list: serde_json::Value =
        serde_json::from_slice(&std::fs::read(first_party.join("bundled.json")).unwrap()).unwrap();
    let mut ids: Vec<String> = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let dir = first_party.join(entry["path"].as_str().unwrap());
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).unwrap()).unwrap();
            manifest["id"].as_str().unwrap().to_owned()
        })
        .collect();
    ids.sort();
    let mut listed: Vec<String> = BUNDLED_PLUGIN_IDS.iter().map(|s| (*s).to_owned()).collect();
    listed.sort();
    assert_eq!(ids, listed, "update BUNDLED_PLUGIN_IDS");

    // Pinned plugins count too (a test or future bundle), at any version.
    let dir = tempfile::tempdir().unwrap();
    let (_, pins) = bundle(dir.path());
    assert!(is_bundled_id(&id(), &pins));
    assert!(!is_bundled_id(&id(), &Pins::empty()));
    for listed in BUNDLED_PLUGIN_IDS {
        assert!(is_bundled_id(
            &PluginId::parse(listed).unwrap(),
            &Pins::empty()
        ));
    }
    assert!(!is_bundled_id(
        &PluginId::parse("deckpilot.weather").unwrap(),
        &Pins::empty()
    ));
}
