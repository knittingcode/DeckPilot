//! Contract tests: the #191 example, a manifest using every field, negative
//! fixtures checked against both the Rust model and the bundled JSON
//! Schema, round-trips, compatibility and package integrity.

use deckpilot_plugin_manifest::{
    ApiLevel, HostEnvironment, Incompatibility, IssueCode, MAX_MANIFEST_BYTES, Manifest,
    ManifestError, PackageDigest, PackageIntegrity, Permission, PermissionKind, Platform,
    SCHEMA_JSON, Sha256Digest,
};
use semver::Version;
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("fixtures/example-manifest.json");
const FULL: &str = include_str!("fixtures/full-manifest.json");

fn schema() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .expect("the bundled schema compiles")
}

fn schema_errors(validator: &jsonschema::Validator, instance: &Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}

fn example() -> Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

fn at<'a>(value: &'a mut Value, pointer: &str) -> &'a mut Value {
    value
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("fixture pointer {pointer} does not exist"))
}

fn set(value: &mut Value, pointer: &str, new: Value) {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let parent = if parent.is_empty() {
        value
    } else {
        at(value, parent)
    };
    match parent {
        Value::Object(map) => {
            map.insert(key.to_owned(), new);
        }
        Value::Array(items) => items[key.parse::<usize>().unwrap()] = new,
        _ => panic!("{pointer}: parent is not a container"),
    }
}

fn push(value: &mut Value, pointer: &str, item: Value) {
    at(value, pointer).as_array_mut().unwrap().push(item);
}

// ---------------------------------------------------------------- positive

#[test]
fn example_manifest_parses_and_validates() {
    let manifest = Manifest::parse_and_validate(EXAMPLE).unwrap();
    assert_eq!(manifest.id.as_str(), "acme.github-prs");
    assert_eq!(manifest.id.publisher(), "acme");
    assert_eq!(manifest.id.variable_namespace(), "plugins.acme.githubPrs");
    assert_eq!(manifest.version, Version::new(1, 0, 0));
    assert_eq!(manifest.engines.deckpilot.as_str(), ">=0.8.0 <1.0.0");
    assert_eq!(manifest.engines.plugin_api.minimum, ApiLevel::new(1, 0));
    assert_eq!(manifest.permissions.len(), 4);
    assert!(matches!(
        manifest.permission(PermissionKind::Network),
        Some(Permission::Network { hosts, .. }) if hosts == &["api.github.com"]
    ));
    assert!(manifest.permission(PermissionKind::Notifications).is_none());
    let action = &manifest.contributes.actions()[0];
    assert_eq!(action.id, "pr-count");
    assert_eq!(action.parameters.len(), 2);
    assert_eq!(manifest.contributes.variables().len(), 2);
    assert_eq!(manifest.contributes.settings().len(), 2);
    assert_eq!(
        manifest.review_warnings(),
        [] as [deckpilot_plugin_manifest::Issue; 0]
    );
    assert_eq!(
        manifest.referenced_files().into_iter().collect::<Vec<_>>(),
        [
            "README.md",
            "assets/icon-128.png",
            "assets/icon-32.png",
            "assets/keys/pr.png"
        ]
    );
}

/// G9: a `notifications` permission may name the settings the agent
/// routes the plugin's notifications by; without `routing` it is the
/// permission it always was.
#[test]
fn notification_routing_names_settings_of_the_right_kind() {
    let mut value = example();
    let fields = value["contributes"]["settings"]["fields"]
        .as_array_mut()
        .unwrap();
    fields.push(
        json!({"key": "desktop", "label": "Desktop", "required": true,
        "kind": {"type": "boolean"}, "default": {"type": "boolean", "value": true}}),
    );
    fields.push(
        json!({"key": "least", "label": "Show from", "required": true,
        "kind": {"type": "choice", "options": ["info", "warning", "error"]},
        "default": {"type": "choice", "value": "info"}}),
    );
    push(
        &mut value,
        "/permissions",
        json!({"kind": "notifications",
               "routing": {"desktop": "desktop", "minimumSeverity": "least"},
               "justification": "Tells you when the review count changes."}),
    );
    assert_eq!(schema_errors(&schema(), &value), [] as [String; 0]);
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let routing = manifest.notification_routing().unwrap();
    assert_eq!(
        routing.members(),
        [("desktop", "desktop"), ("minimumSeverity", "least")]
    );
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);

    // A choice of something else than severities is refused.
    let mut wrong = value.clone();
    wrong["contributes"]["settings"]["fields"][3]["kind"]["options"] = json!(["info", "loud"]);
    let error = Manifest::parse_and_validate(&wrong.to_string()).unwrap_err();
    assert!(
        error.to_string().contains("routing.minimumSeverity"),
        "{error}"
    );

    // Without routing: nothing changes.
    assert_eq!(
        Manifest::parse_and_validate(EXAMPLE)
            .unwrap()
            .notification_routing(),
        None
    );
}

#[test]
fn full_manifest_parses_and_validates() {
    let manifest = Manifest::parse_and_validate(FULL).unwrap();
    assert_eq!(manifest.permissions.len(), 9);
    assert_eq!(manifest.contributes.actions()[0].parameters.len(), 8);
    assert_eq!(manifest.contributes.credentials().len(), 4);
    let network = manifest.permission(PermissionKind::Network).unwrap();
    let slot = &network.user_origins()[0];
    assert_eq!(slot.setting, "serverUrl");
    assert!(slot.path_prefix() && slot.private_network());
    assert!(manifest.opens_user_origin("serverUrl"));
    assert!(!manifest.opens_user_origin("pollMinutes"));
    assert_eq!(
        manifest.contributes.credentials()[3].user_origin.as_deref(),
        Some("serverUrl")
    );
    assert_eq!(
        manifest.review_warnings(),
        [] as [deckpilot_plugin_manifest::Issue; 0]
    );
}

/// Plugin API 1.3: `controls`, `navigation` and `views`.
#[test]
fn controls_navigation_and_views_parse_and_validate() {
    use deckpilot_plugin_manifest::{
        ControlKind, IconTheme, InputCapability, NavigationIcon, ViewKind,
    };
    let value = navigation_example();
    assert_eq!(schema_errors(&schema(), &value), [] as [String; 0]);
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);

    let controls = manifest.contributes.actions()[0].controls.as_ref().unwrap();
    let sets = controls.as_sets();
    assert_eq!(
        sets.keys().copied().collect::<Vec<_>>(),
        [
            ControlKind::Key,
            ControlKind::Encoder,
            ControlKind::TouchZone
        ]
    );
    assert!(sets[&ControlKind::Encoder].contains(&InputCapability::RotateClockwise));
    assert_eq!(
        sets[&ControlKind::TouchZone].iter().collect::<Vec<_>>(),
        [&InputCapability::Tap]
    );

    let navigation = manifest.contributes.navigation();
    assert_eq!(navigation.len(), 2);
    assert_eq!(
        manifest.contributes.view_of(&navigation[0]).unwrap().kind,
        ViewKind::Variables
    );
    assert_eq!(
        navigation[0].icon.file(IconTheme::Dark),
        Some("assets/nav-dark.svg")
    );
    assert_eq!(
        navigation[1].icon.file(IconTheme::Dark),
        Some("assets/setup.png")
    );
    // One theme only: the other theme shows the same file.
    let light_only: NavigationIcon =
        serde_json::from_value(json!({"light": "assets/l.svg"})).unwrap();
    assert_eq!(light_only.file(IconTheme::Dark), Some("assets/l.svg"));

    // The icons are package files the integrity record must hold.
    let files = manifest.referenced_files();
    for icon in ["assets/nav.svg", "assets/nav-dark.svg", "assets/setup.png"] {
        assert!(files.contains(icon), "{icon}");
    }

    // A manifest without them is what it was: keys only, no navigation.
    let plain = Manifest::parse_and_validate(EXAMPLE).unwrap();
    assert!(plain.contributes.actions()[0].controls.is_none());
    assert_eq!(plain.contributes.navigation().len(), 0);
    assert!(plain.navigation_icon_files().is_empty());

    // Every message names the level to declare.
    let mut old = value.clone();
    set(&mut old, "/engines/pluginApi", json!("^1.2"));
    let error = Manifest::parse_and_validate(&old.to_string()).unwrap_err();
    assert!(
        error.to_string().contains(
            "contributes.actions[0].controls: needs plugin API 1.3: set engines.pluginApi to \
             `^1.3` (it is `^1.2`)"
        ),
        "{error}"
    );
    // A later major is at least 1.3 too.
    let mut later = value;
    set(&mut later, "/engines/pluginApi", json!("^2.0"));
    Manifest::parse_and_validate(&later.to_string()).unwrap();
}

/// Plugin API 1.4: a view says which toolbar selectors the app shows with
/// it. Optional (absent is `none`), one of four values, and it needs
/// `^1.4`; it grants nothing.
#[test]
fn views_declare_their_toolbar_context() {
    use deckpilot_plugin_manifest::{VIEW_CONTEXT_API, ViewContext};
    assert_eq!(VIEW_CONTEXT_API, ApiLevel::new(1, 4));
    let validator = schema();

    // Absent: `none`, and a 1.3 manifest stays valid as it was.
    let plain = Manifest::parse_and_validate(&navigation_example().to_string()).unwrap();
    for view in plain.contributes.views() {
        assert_eq!(view.context, None);
        assert_eq!(view.context(), ViewContext::None);
    }

    for (name, context) in [
        ("none", ViewContext::None),
        ("device", ViewContext::Device),
        ("profile", ViewContext::Profile),
        ("device+profile", ViewContext::DeviceProfile),
    ] {
        let mut value = navigation_example();
        set(&mut value, "/engines/pluginApi", json!("^1.4"));
        set(&mut value, "/contributes/views/0/context", json!(name));
        assert_eq!(
            schema_errors(&validator, &value),
            [] as [String; 0],
            "{name}"
        );
        let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
        assert_eq!(manifest.contributes.views()[0].context(), context);
        assert_eq!(context.as_str(), name);
        // It round-trips as written.
        assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
        // The declaration grants no permission and changes none.
        assert_eq!(manifest.permissions, plain.permissions);
    }

    // Anything else is refused, by the model and by the schema.
    for bad in [
        json!("devices"),
        json!("Device"),
        json!("device,profile"),
        json!(""),
        json!(1),
        json!(["device"]),
    ] {
        let mut value = navigation_example();
        set(&mut value, "/engines/pluginApi", json!("^1.4"));
        set(&mut value, "/contributes/views/0/context", bad.clone());
        assert!(
            Manifest::parse_and_validate(&value.to_string()).is_err(),
            "{bad} is not a context"
        );
        assert!(
            !schema_errors(&validator, &value).is_empty(),
            "{bad}: schema"
        );
    }

    // It needs plugin API 1.4, and says so.
    let mut old = navigation_example();
    set(&mut old, "/contributes/views/1/context", json!("profile"));
    let error = Manifest::parse_and_validate(&old.to_string()).unwrap_err();
    assert!(
        error.to_string().contains(
            "contributes.views[1].context: needs plugin API 1.4: set engines.pluginApi to \
             `^1.4` (it is `^1.3`)"
        ),
        "{error}"
    );
}

#[test]
fn dialogs_are_a_permission_of_plugin_api_1_5() {
    use deckpilot_plugin_manifest::DIALOGS_API;
    assert_eq!(DIALOGS_API, ApiLevel::new(1, 5));
    let validator = schema();
    let dialogs = json!({"kind": "dialogs", "justification": "Asks which repository to open."});

    let mut value = example();
    set(&mut value, "/engines/pluginApi", json!("^1.5"));
    push(&mut value, "/permissions", dialogs.clone());
    assert_eq!(schema_errors(&validator, &value), [] as [String; 0]);
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let permission = manifest
        .permissions
        .iter()
        .find(|p| p.kind() == PermissionKind::Dialogs)
        .unwrap();
    assert_eq!(PermissionKind::Dialogs.as_str(), "dialogs");
    assert_eq!(permission.justification(), "Asks which repository to open.");
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);

    // Below 1.5: refused, with the level to set.
    for level in ["^1.0", "^1.3", "^1.4"] {
        let mut old = example();
        set(&mut old, "/engines/pluginApi", json!(level));
        push(&mut old, "/permissions", dialogs.clone());
        let error = Manifest::parse_and_validate(&old.to_string()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("needs plugin API 1.5: set engines.pluginApi to `^1.5`"),
            "{level}: {error}"
        );
    }

    // Strict: no members but the justification, which it needs; `ui` and
    // `customUi` stay forbidden.
    for bad in [
        json!({"kind": "dialogs"}),
        json!({"kind": "dialogs", "justification": "Asks which repository.", "secret": true}),
        json!({"kind": "dialogs", "justification": "Asks which repository.", "html": true}),
        json!({"kind": "ui", "justification": "Shows its own window."}),
        json!({"kind": "customUi", "justification": "Shows its own window."}),
    ] {
        let mut value = example();
        set(&mut value, "/engines/pluginApi", json!("^1.5"));
        push(&mut value, "/permissions", bad.clone());
        assert!(
            Manifest::parse_and_validate(&value.to_string()).is_err(),
            "{bad}"
        );
        assert!(
            !schema_errors(&validator, &value).is_empty(),
            "{bad}: schema"
        );
    }
}

#[test]
fn action_titles_are_plugin_api_1_8_and_data_only() {
    use deckpilot_plugin_manifest::TITLE_API;
    assert_eq!(TITLE_API, ApiLevel::new(1, 8));
    let validator = schema();
    let with = |level: &str, title: Value| {
        let mut value = example();
        set(&mut value, "/engines/pluginApi", json!(level));
        set(&mut value, "/contributes/actions/0/title", title);
        value
    };
    let title = json!({
        "default": "CPU",
        "dynamic": true,
        "position": "bottom",
        "supports": ["visibility", "position", "font", "style", "color", "layout"]
    });
    let value = with("^1.8", title.clone());
    assert_eq!(schema_errors(&validator, &value), [] as [String; 0]);
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let declared = manifest.contributes.actions()[0].title.as_ref().unwrap();
    assert_eq!(declared.default.as_deref(), Some("CPU"));
    assert_eq!(declared.dynamic, Some(true));
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
    // Every member is optional.
    assert!(Manifest::parse_and_validate(&with("^1.8", json!({})).to_string()).is_ok());

    // Below 1.8: refused, with the level to set.
    for level in ["^1.0", "^1.7"] {
        let error =
            Manifest::parse_and_validate(&with(level, title.clone()).to_string()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("needs plugin API 1.8: set engines.pluginApi to `^1.8`"),
            "{level}: {error}"
        );
    }

    // Bad values, refused by the validator and the schema alike; nothing
    // but data (no UI, no script).
    for bad in [
        json!({"default": ""}),
        json!({"default": "A title far longer than thirty-two characters"}),
        json!({"position": "left"}),
        json!({"supports": ["everything"]}),
        json!({"supports": ["color", "color"]}),
        json!({"editor": "index.html"}),
        json!({"render": true}),
    ] {
        let value = with("^1.8", bad.clone());
        assert!(
            Manifest::parse_and_validate(&value.to_string()).is_err(),
            "{bad}"
        );
        assert!(
            !schema_errors(&validator, &value).is_empty(),
            "{bad}: schema"
        );
    }
}

#[test]
fn visual_assets_are_plugin_api_1_8_data_confined_to_the_package() {
    use deckpilot_plugin_manifest::VISUAL_ASSETS_API;
    assert_eq!(VISUAL_ASSETS_API, ApiLevel::new(1, 8));
    let validator = schema();
    let with = |level: &str, assets: Value| {
        let mut value = example();
        set(&mut value, "/engines/pluginApi", json!(level));
        set(&mut value, "/contributes/visualAssets", assets);
        value
    };
    let assets = json!({
        "icons": [
            {"id": "sunny", "name": "Sunny", "asset": "assets/icons/sunny.png", "keywords": ["sun"]}
        ],
        "backgrounds": [
            {"id": "sky", "name": "Sky", "type": "image", "asset": "assets/backgrounds/sky.jpg"},
            {"id": "dusk", "name": "Dusk", "type": "gradient", "colors": ["#ff8800", "#220044"], "angle": 135}
        ]
    });
    let value = with("^1.8", assets.clone());
    assert_eq!(schema_errors(&validator, &value), [] as [String; 0]);
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let declared = manifest.contributes.visual_assets();
    assert_eq!(declared.icons()[0].id, "sunny");
    assert_eq!(declared.backgrounds().len(), 2);
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
    // Their files are package files: hashed into the integrity record.
    let files = manifest.referenced_files();
    for file in ["assets/icons/sunny.png", "assets/backgrounds/sky.jpg"] {
        assert!(files.iter().any(|f| f == file), "{file}");
    }
    assert_eq!(manifest.visual_asset_files().len(), 2);
    // Names are display texts: translatable like every other (`t:`).
    let mut translated = value.clone();
    set(
        &mut translated,
        "/contributes/visualAssets/backgrounds/1/name",
        json!("t:assets.dusk"),
    );
    let error = Manifest::parse_and_validate(&translated.to_string()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("contributes.visualAssets.backgrounds[1].name"),
        "{error}"
    );

    // Below 1.8: refused, with the level to set.
    for level in ["^1.0", "^1.7"] {
        let error =
            Manifest::parse_and_validate(&with(level, assets.clone()).to_string()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("needs plugin API 1.8: set engines.pluginApi to `^1.8`"),
            "{level}: {error}"
        );
    }

    // Icons and backgrounds share the plugin's ids (the schema cannot say
    // so; the validator does).
    let twice = json!({
        "icons": [{"id": "same", "name": "A", "asset": "assets/a.png"}],
        "backgrounds": [{"id": "same", "name": "B", "type": "gradient", "colors": ["#000000", "#ffffff"]}]
    });
    let error = Manifest::parse_and_validate(&with("^1.8", twice).to_string()).unwrap_err();
    assert!(error.to_string().contains("same"), "{error}");

    // Bad declarations, refused by the validator and the schema alike.
    let icon = |icon: Value| json!({"icons": [icon]});
    let background = |background: Value| json!({"backgrounds": [background]});
    for bad in [
        // Paths outside the package or outside assets/, wrong formats.
        icon(json!({"id": "a", "name": "A", "asset": "../secret.png"})),
        icon(json!({"id": "a", "name": "A", "asset": "/etc/passwd.png"})),
        icon(json!({"id": "a", "name": "A", "asset": "assets/../x.png"})),
        icon(json!({"id": "a", "name": "A", "asset": "file:///x.png"})),
        icon(json!({"id": "a", "name": "A", "asset": "icons/a.png"})),
        icon(json!({"id": "a", "name": "A", "asset": "assets/a.svg"})),
        icon(json!({"id": "a", "name": "A", "asset": "assets/a.jpg"})),
        background(json!({"id": "a", "name": "A", "type": "image", "asset": "assets/a.gif"})),
        // Ids and names.
        icon(json!({"id": "Bad.Id", "name": "A", "asset": "assets/a.png"})),
        icon(json!({"id": "a", "name": "", "asset": "assets/a.png"})),
        icon(json!({"id": "a", "name": "A".repeat(49), "asset": "assets/a.png"})),
        // Types and their fields.
        background(json!({"id": "a", "name": "A", "type": "image"})),
        background(json!({"id": "a", "name": "A", "type": "gradient", "colors": ["#000000"]})),
        background(
            json!({"id": "a", "name": "A", "type": "gradient", "colors": ["#000000", "red"]}),
        ),
        background(
            json!({"id": "a", "name": "A", "type": "gradient", "colors": ["#000000", "#ffffff"], "angle": 360}),
        ),
        background(
            json!({"id": "a", "name": "A", "type": "gradient", "colors": ["#000000", "#ffffff"], "asset": "assets/a.png"}),
        ),
        background(json!({"id": "a", "name": "A", "type": "video", "asset": "assets/a.png"})),
        // Data only: no code, no CSS.
        background(
            json!({"id": "a", "name": "A", "type": "gradient", "colors": ["#000000", "#ffffff"], "css": "linear-gradient(red, blue)"}),
        ),
        icon(json!({"id": "a", "name": "A", "asset": "assets/a.png", "script": "main.js"})),
    ] {
        let value = with("^1.8", bad.clone());
        assert!(
            Manifest::parse_and_validate(&value.to_string()).is_err(),
            "{bad}"
        );
        assert!(
            !schema_errors(&validator, &value).is_empty(),
            "{bad}: schema"
        );
    }
}

#[test]
fn editor_navigation_is_plugin_api_1_7_and_an_allowlist() {
    use deckpilot_plugin_manifest::{EDITOR_NAVIGATION_API, EDITOR_NAVIGATION_OPERATIONS};
    assert_eq!(EDITOR_NAVIGATION_API, ApiLevel::new(1, 7));
    let validator = schema();
    let with = |level: &str, navigation: Value| {
        let mut value = example();
        set(&mut value, "/engines/pluginApi", json!(level));
        set(
            &mut value,
            "/contributes/actions/0/editorNavigation",
            navigation,
        );
        value
    };
    // `repository` is a text parameter of the example's first action,
    // `filter` a choice.
    let open = json!({"operation": "folder.open", "targetParameter": "repository"});
    let value = with("^1.7", open.clone());
    assert_eq!(schema_errors(&validator, &value), [] as [String; 0]);
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let declared = manifest.contributes.actions()[0]
        .editor_navigation
        .as_ref()
        .unwrap();
    assert_eq!(declared.operation, "folder.open");
    assert_eq!(declared.target_parameter.as_deref(), Some("repository"));
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
    for (operation, takes_target) in EDITOR_NAVIGATION_OPERATIONS {
        let navigation = if takes_target {
            json!({"operation": operation, "targetParameter": "repository"})
        } else {
            json!({"operation": operation})
        };
        let value = with("^1.7", navigation);
        assert!(
            Manifest::parse_and_validate(&value.to_string()).is_ok(),
            "{operation}"
        );
        assert_eq!(schema_errors(&validator, &value), [] as [String; 0]);
    }

    // Below 1.7: refused, with the level to set.
    for level in ["^1.0", "^1.5", "^1.6"] {
        let error =
            Manifest::parse_and_validate(&with(level, open.clone()).to_string()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("needs plugin API 1.7: set engines.pluginApi to `^1.7`"),
            "{level}: {error}"
        );
    }

    // Only the allowlist, only the action's own text parameters, a target
    // exactly where the operation takes one; nothing executable.
    for (bad, schema_refuses) in [
        (json!({"operation": "url.open"}), true),
        (json!({"operation": "profile.switch"}), true),
        (json!({"operation": "folder.open"}), false),
        (
            json!({"operation": "folder.open", "targetParameter": "missing"}),
            false,
        ),
        (
            json!({"operation": "folder.open", "targetParameter": "filter"}),
            false,
        ),
        (
            json!({"operation": "page.next", "targetParameter": "repository"}),
            false,
        ),
        (
            json!({"operation": "folder.back", "pageParameter": "repository"}),
            false,
        ),
        (
            json!({"operation": "folder.back", "callback": "navigate"}),
            true,
        ),
        (json!({"operation": "layout.back", "script": "go()"}), true),
    ] {
        let value = with("^1.7", bad.clone());
        assert!(
            Manifest::parse_and_validate(&value.to_string()).is_err(),
            "{bad}"
        );
        assert_eq!(
            !schema_errors(&validator, &value).is_empty(),
            schema_refuses,
            "{bad}: schema"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)] // one contract, step by step
fn localization_is_plugin_api_1_6() {
    use deckpilot_plugin_manifest::LOCALIZATION_API;
    use deckpilot_plugin_manifest::locale::{Catalog, LocaleTag, Messages};
    assert_eq!(LOCALIZATION_API, ApiLevel::new(1, 6));
    let validator = schema();

    let localized = || {
        let mut value = example();
        set(&mut value, "/engines/pluginApi", json!("^1.6"));
        set(&mut value, "/locales", json!(["en", "pl", "pl-PL"]));
        set(&mut value, "/name", json!("t:plugin.name"));
        set(&mut value, "/description", json!("t:plugin.description"));
        set(
            &mut value,
            "/contributes/actions/0/name",
            json!("t:actions.count.name"),
        );
        set(
            &mut value,
            "/permissions/0/justification",
            json!("t:permissions.network"),
        );
        value
    };
    let value = localized();
    assert_eq!(schema_errors(&validator, &value), [] as [String; 0]);
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    assert_eq!(manifest.default_locale(), "en");
    assert_eq!(
        manifest.locale_files(),
        ["locales/en.json", "locales/pl.json", "locales/pl-PL.json"]
    );
    // The locale files are files the manifest names (hashed and pinned).
    let files = manifest.referenced_files();
    for file in ["locales/en.json", "locales/pl.json", "locales/pl-PL.json"] {
        assert!(files.contains(file), "{file}");
    }
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
    let keys: Vec<String> = manifest
        .translation_keys()
        .into_iter()
        .map(|(_, key)| key)
        .collect();
    assert_eq!(
        keys,
        [
            "plugin.name",
            "plugin.description",
            "permissions.network",
            "actions.count.name"
        ]
    );

    // Translated: names and labels change, identifiers never do.
    let mut locales = std::collections::BTreeMap::new();
    locales.insert(
        LocaleTag::parse("en").unwrap(),
        Messages::from_pairs([
            ("plugin.name", "PR count"),
            ("plugin.description", "Counts pull requests."),
            ("actions.count.name", "Count PRs"),
        ])
        .unwrap(),
    );
    locales.insert(
        LocaleTag::parse("pl").unwrap(),
        Messages::from_pairs([
            ("plugin.name", "Liczba PR"),
            ("actions.count.name", "Policz PR"),
        ])
        .unwrap(),
    );
    let catalog = Catalog::new(LocaleTag::parse("en").unwrap(), locales);
    let mut missing = Vec::new();
    let pl = LocaleTag::parse("pl-PL").unwrap();
    let shown = manifest.localized(&catalog, Some(&pl), &mut |k| missing.push(k.to_owned()));
    assert_eq!(shown.name, "Liczba PR");
    assert_eq!(shown.description, "Counts pull requests.");
    assert_eq!(shown.contributes.actions()[0].name, "Policz PR");
    assert_eq!(
        shown.contributes.actions()[0].id,
        manifest.contributes.actions()[0].id
    );
    assert_eq!(shown.id, manifest.id);
    // A key no locale has: a label from its last segment, reported.
    assert_eq!(shown.permissions[0].justification(), "Network");
    assert_eq!(missing, ["permissions.network"]);

    // Below 1.6: `locales` and `defaultLocale` are refused, `t:` is text.
    for level in ["^1.0", "^1.5"] {
        let mut old = localized();
        set(&mut old, "/engines/pluginApi", json!(level));
        let error = Manifest::parse_and_validate(&old.to_string()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("locales: needs plugin API 1.6: set engines.pluginApi to `^1.6`"),
            "{level}: {error}"
        );
    }
    let mut plain = example();
    set(&mut plain, "/name", json!("t:plugin.name"));
    let plain = Manifest::parse_and_validate(&plain.to_string()).unwrap();
    assert_eq!(plain.translation_keys(), [] as [(String, String); 0]);
    assert_eq!(
        plain.localized(&catalog, Some(&pl), &mut |_| {}).name,
        "t:plugin.name"
    );

    // Bad declarations.
    let refused = |f: &dyn Fn(&mut Value), path: &str, code: IssueCode| {
        let mut value = localized();
        f(&mut value);
        let errors = match Manifest::parse_and_validate(&value.to_string()) {
            Err(ManifestError::Invalid(errors)) => errors,
            other => panic!("{path}: {other:?}"),
        };
        assert!(errors.has(path, code), "{path}: {errors}");
    };
    refused(
        &|v| set(v, "/locales", json!(["en", "pl-pl"])),
        "locales[1]",
        IssueCode::InvalidValue,
    );
    refused(
        &|v| set(v, "/locales", json!(["en", "en"])),
        "locales[1]",
        IssueCode::Duplicate,
    );
    refused(
        &|v| set(v, "/locales", json!([])),
        "locales",
        IssueCode::Count,
    );
    refused(
        &|v| set(v, "/locales", json!(["../x"])),
        "locales[0]",
        IssueCode::InvalidValue,
    );
    refused(
        &|v| set(v, "/locales", json!(["pl"])),
        "locales",
        IssueCode::UnknownReference,
    );
    refused(
        &|v| set(v, "/defaultLocale", json!("de")),
        "defaultLocale",
        IssueCode::UnknownReference,
    );
    refused(
        &|v| set(v, "/name", json!("t:bad key")),
        "name",
        IssueCode::InvalidValue,
    );
    refused(
        &|v| {
            v.as_object_mut().unwrap().remove("locales");
        },
        "name",
        IssueCode::UnknownReference,
    );
    let too_many: Vec<String> = (0u8..33)
        .map(|i| format!("x{}", char::from(b'a' + i % 26)))
        .collect();
    refused(
        &|v| set(v, "/locales", json!(too_many)),
        "locales",
        IssueCode::Count,
    );
    // The schema agrees on the tags.
    let mut bad = localized();
    set(&mut bad, "/locales", json!(["en", "pl_PL"]));
    assert_ne!(schema_errors(&validator, &bad), [] as [String; 0]);
    let mut with_default = localized();
    set(&mut with_default, "/defaultLocale", json!("pl"));
    assert_eq!(schema_errors(&validator, &with_default), [] as [String; 0]);
    assert_eq!(
        Manifest::parse_and_validate(&with_default.to_string())
            .unwrap()
            .default_locale(),
        "pl"
    );
}

#[test]
fn schema_accepts_the_positive_fixtures() {
    let validator = schema();
    for (name, text) in [("example", EXAMPLE), ("full", FULL)] {
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            schema_errors(&validator, &value),
            Vec::<String>::new(),
            "{name}"
        );
    }
}

#[test]
fn schema_is_the_191_manifest_schema() {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    assert_eq!(
        schema["$id"],
        "https://knittingcode.github.io/deckpilot-plugins/v1/schemas/plugin-manifest.schema.json"
    );
    assert_eq!(schema["properties"]["manifestVersion"]["const"], 1);
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn round_trip_preserves_the_json() {
    for text in [EXAMPLE, FULL] {
        let original: Value = serde_json::from_str(text).unwrap();
        let manifest = Manifest::from_json(text).unwrap();
        let written = manifest.to_json_pretty();
        assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), original);
        let again = Manifest::from_json(&written).unwrap();
        assert_eq!(again, manifest);
        again.validate().unwrap();
    }
}

#[test]
fn review_warnings_do_not_fail_validation() {
    let mut value = example();
    set(
        &mut value,
        "/contributes/actions/0/node/effect",
        json!("modifies"),
    );
    remove_state(&mut value);
    set(&mut value, "/permissions/3/state", json!(false));
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let warnings = manifest.review_warnings();
    let paths: Vec<&str> = warnings.iter().map(|w| w.path.as_str()).collect();
    assert_eq!(
        paths,
        ["permissions[3]", "contributes.actions[0].node.retrySafe"]
    );
}

fn remove_state(value: &mut Value) {
    at(value, "/contributes/actions/0")
        .as_object_mut()
        .unwrap()
        .remove("state");
}

// ---------------------------------------------------------------- negative

/// What the Rust side must report.
#[derive(Debug)]
enum Expect {
    /// A structural error whose message contains the text.
    Structure(&'static str),
    UnsupportedVersion,
    ForbiddenPermission,
    TooLarge,
    Syntax,
    /// A validation issue with this code at exactly this path.
    Invalid(&'static str, IssueCode),
}

struct Fixture {
    name: &'static str,
    /// The manifest text.
    text: String,
    expect: Expect,
    /// Whether the JSON Schema rejects it too. `false` documents a rule the
    /// schema cannot express (section 7) or a check stricter than the
    /// schema; `None` when the text is not JSON a schema can see as-is.
    schema_rejects: Option<bool>,
}

fn mutate(name: &'static str, schema: bool, expect: Expect, f: impl FnOnce(&mut Value)) -> Fixture {
    let mut value = example();
    f(&mut value);
    Fixture {
        name,
        text: value.to_string(),
        expect,
        schema_rejects: Some(schema),
    }
}

#[allow(clippy::too_many_lines)]
fn fixtures() -> Vec<Fixture> {
    use Expect::{ForbiddenPermission, Invalid, Structure, Syntax, TooLarge, UnsupportedVersion};
    use IssueCode as C;
    let network_host = "/permissions/0/hosts/0";
    vec![
        // Unknown fields, everywhere.
        mutate(
            "unknown top-level field",
            true,
            Structure("unknown field `extra`"),
            |v| {
                set(v, "/extra", json!(1));
            },
        ),
        // Notification routing (G9): settings of the right kind only.
        mutate(
            "routing names an undeclared setting",
            false,
            Invalid("permissions[4].routing.desktop", C::UnknownReference),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({"kind": "notifications", "routing": {"desktop": "nowhere"},
                           "justification": "Tells you when the review count changes."}),
                );
            },
        ),
        mutate(
            "routing desktop is not a boolean setting",
            false,
            Invalid("permissions[4].routing.desktop", C::InvalidValue),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({"kind": "notifications", "routing": {"desktop": "pollMinutes"},
                           "justification": "Tells you when the review count changes."}),
                );
            },
        ),
        mutate(
            "routing names no setting",
            true,
            Invalid("permissions[4].routing", C::Count),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({"kind": "notifications", "routing": {},
                           "justification": "Tells you when the review count changes."}),
                );
            },
        ),
        mutate(
            "unknown routing member",
            true,
            Structure("unknown field `sound`"),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({"kind": "notifications", "routing": {"sound": "pollMinutes"},
                           "justification": "Tells you when the review count changes."}),
                );
            },
        ),
        mutate(
            "unknown action field",
            true,
            Structure("unknown field `handler`"),
            |v| {
                set(v, "/contributes/actions/0/handler", json!("x"));
            },
        ),
        mutate(
            "unknown permission field",
            true,
            Structure("unknown field `scope`"),
            |v| {
                set(v, "/permissions/0/scope", json!("all"));
            },
        ),
        mutate(
            "unknown field on a field-less kind",
            true,
            Structure("unknown field `multiline`"),
            |v| {
                set(
                    v,
                    "/contributes/actions/0/parameters/0/kind",
                    json!({ "type": "boolean", "multiline": true }),
                );
            },
        ),
        mutate(
            "unknown field in a default",
            true,
            Structure("unknown field `extra`"),
            |v| {
                set(
                    v,
                    "/contributes/actions/0/parameters/1/default",
                    json!({ "type": "choice", "value": "all", "extra": 1 }),
                );
            },
        ),
        mutate(
            "default value of the wrong JSON type",
            true,
            Structure("a default of type `integer` needs a whole number"),
            |v| {
                set(
                    v,
                    "/contributes/settings/fields/0/default/value",
                    json!("5"),
                );
            },
        ),
        mutate(
            "resource parameter kind",
            true,
            Structure("unknown variant `resource`"),
            |v| {
                set(
                    v,
                    "/contributes/actions/0/parameters/0/kind",
                    json!({ "type": "resource" }),
                );
            },
        ),
        mutate(
            "missing required field",
            true,
            Structure("missing field `license`"),
            |v| {
                v.as_object_mut().unwrap().remove("license");
            },
        ),
        // Manifest version.
        mutate("manifestVersion 2", true, UnsupportedVersion, |v| {
            set(v, "/manifestVersion", json!(2));
        }),
        // Runtime (artifact contract): wasm ABI 1 only, no `js` kind.
        mutate(
            "runtime kind js",
            true,
            Structure("unknown variant `js`"),
            |v| {
                set(v, "/runtime", json!({ "kind": "js", "abi": 1 }));
            },
        ),
        mutate(
            "runtime abi 2",
            true,
            Invalid("runtime.abi", C::Range),
            |v| {
                set(v, "/runtime", json!({ "kind": "wasm", "abi": 2 }));
            },
        ),
        mutate(
            "runtime without abi",
            true,
            Structure("missing field `abi`"),
            |v| {
                set(v, "/runtime", json!({ "kind": "wasm" }));
            },
        ),
        mutate(
            "unknown runtime field",
            true,
            Structure("unknown field `engine`"),
            |v| {
                set(
                    v,
                    "/runtime",
                    json!({ "kind": "wasm", "abi": 1, "engine": "quickjs" }),
                );
            },
        ),
        // Identity.
        mutate(
            "uppercase plugin id",
            true,
            Structure("not a valid publisher id"),
            |v| {
                set(v, "/id", json!("Acme.github-prs"));
            },
        ),
        mutate(
            "plugin id with two dots",
            true,
            Structure("exactly one dot"),
            |v| {
                set(v, "/id", json!("acme.github.prs"));
            },
        ),
        mutate(
            "plugin id with double hyphen",
            true,
            Structure("not a valid plugin name"),
            |v| {
                set(v, "/id", json!("acme.github--prs"));
            },
        ),
        mutate(
            "id does not match the publisher",
            false,
            Invalid("id", C::Mismatch),
            |v| {
                set(v, "/id", json!("other.github-prs"));
            },
        ),
        mutate(
            "reserved publisher",
            false,
            Invalid("publisher.id", C::Reserved),
            |v| {
                set(v, "/id", json!("deckpilot.github-prs"));
                set(v, "/publisher/id", json!("deckpilot"));
            },
        ),
        mutate(
            "built-in module as publisher",
            false,
            Invalid("publisher.id", C::Reserved),
            |v| {
                set(v, "/id", json!("github.prs"));
                set(v, "/publisher/id", json!("github"));
            },
        ),
        // Actions.
        mutate(
            "invalid action id",
            true,
            Invalid("contributes.actions[0].id", C::InvalidId),
            |v| {
                set(v, "/contributes/actions/0/id", json!("PR_count"));
            },
        ),
        mutate(
            "duplicate action id",
            false,
            Invalid("contributes.actions[1]", C::Duplicate),
            |v| {
                let copy = v["contributes"]["actions"][0].clone();
                push(v, "/contributes/actions", copy);
            },
        ),
        mutate(
            "duplicate parameter key",
            false,
            Invalid("contributes.actions[0].parameters[1]", C::Duplicate),
            |v| {
                set(
                    v,
                    "/contributes/actions/0/parameters/1/key",
                    json!("repository"),
                );
            },
        ),
        mutate(
            "duplicate output key",
            false,
            Invalid("contributes.actions[0].node.outputs[1]", C::Duplicate),
            |v| {
                set(
                    v,
                    "/contributes/actions/0/node/outputs/1/key",
                    json!("count"),
                );
            },
        ),
        mutate(
            "too many actions",
            true,
            Invalid("contributes.actions", C::Count),
            |v| {
                let base = v["contributes"]["actions"][0].clone();
                for i in 0..64 {
                    let mut action = base.clone();
                    action["id"] = json!(format!("action-{i}"));
                    push(v, "/contributes/actions", action);
                }
            },
        ),
        mutate("name too long", true, Invalid("name", C::Length), |v| {
            set(v, "/name", json!("x".repeat(49)));
        }),
        mutate(
            "replacedBy without deprecated",
            false,
            Invalid("contributes.actions[0].replacedBy", C::Conflict),
            |v| set(v, "/contributes/actions/0/replacedBy", json!("pr-count")),
        ),
        mutate(
            "state on a modifying action",
            false,
            Invalid("contributes.actions[0].state", C::Conflict),
            |v| set(v, "/contributes/actions/0/node/effect", json!("modifies")),
        ),
        mutate(
            "requiresProject without profile permission",
            false,
            Invalid(
                "contributes.actions[0].requiresProject",
                C::MissingPermission,
            ),
            |v| set(v, "/contributes/actions/0/requiresProject", json!(true)),
        ),
        // Defaults and kinds.
        mutate(
            "choice default not an option",
            false,
            Invalid(
                "contributes.actions[0].parameters[1].default",
                C::InvalidDefault,
            ),
            |v| {
                set(
                    v,
                    "/contributes/actions/0/parameters/1/default/value",
                    json!("mine"),
                );
            },
        ),
        mutate(
            "integer default outside min/max",
            false,
            Invalid("contributes.settings.fields[0].default", C::InvalidDefault),
            |v| {
                set(
                    v,
                    "/contributes/settings/fields/0/default/value",
                    json!(500),
                );
            },
        ),
        mutate(
            "default of the wrong type",
            false,
            Invalid("contributes.settings.fields[0].default", C::InvalidDefault),
            |v| {
                set(
                    v,
                    "/contributes/settings/fields/0/default",
                    json!({ "type": "text", "value": "5" }),
                );
            },
        ),
        mutate(
            "integer min greater than max",
            false,
            Invalid("contributes.settings.fields[0].kind", C::Conflict),
            |v| set(v, "/contributes/settings/fields/0/kind/min", json!(200)),
        ),
        mutate(
            "text maxLength 0",
            true,
            Invalid(
                "contributes.actions[0].parameters[0].kind.maxLength",
                C::Range,
            ),
            |v| {
                set(
                    v,
                    "/contributes/actions/0/parameters/0/kind/maxLength",
                    json!(0),
                );
            },
        ),
        mutate(
            "setting key privateHosts",
            false,
            Invalid("contributes.settings.fields[1].key", C::Reserved),
            |v| {
                set(
                    v,
                    "/contributes/settings/fields/1/key",
                    json!("privateHosts"),
                );
            },
        ),
        // Versions and compatibility.
        mutate(
            "invalid semver",
            true,
            Structure("unexpected end of input"),
            |v| {
                set(v, "/version", json!("1.0"));
            },
        ),
        mutate(
            "semver with build metadata",
            true,
            Invalid("version", C::InvalidVersion),
            |v| {
                set(v, "/version", json!("1.0.0+build.7"));
            },
        ),
        mutate(
            "unsupported pre-release",
            true,
            Invalid("version", C::InvalidVersion),
            |v| {
                set(v, "/version", json!("1.0.0-dev.1"));
            },
        ),
        mutate(
            "caret DeckPilot range",
            true,
            Structure("not a comparator"),
            |v| {
                set(v, "/engines/deckpilot", json!("^0.8.0"));
            },
        ),
        mutate(
            "three comparators",
            true,
            Structure("not a DeckPilot version range"),
            |v| {
                set(v, "/engines/deckpilot", json!(">=0.8.0 <1.0.0 <2.0.0"));
            },
        ),
        mutate(
            "unsatisfiable DeckPilot range",
            false,
            Invalid("engines.deckpilot", C::UnsatisfiableRange),
            |v| set(v, "/engines/deckpilot", json!(">=1.0.0 <0.9.0")),
        ),
        mutate(
            "plugin API without caret",
            true,
            Structure("not a plugin API requirement"),
            |v| {
                set(v, "/engines/pluginApi", json!("1.0"));
            },
        ),
        mutate(
            "empty platforms",
            true,
            Invalid("platforms", C::Count),
            |v| {
                set(v, "/platforms", json!([]));
            },
        ),
        mutate(
            "duplicate platform",
            true,
            Invalid("platforms[1]", C::Duplicate),
            |v| {
                set(v, "/platforms", json!(["linux", "linux"]));
            },
        ),
        mutate(
            "unknown platform",
            true,
            Structure("unknown variant `freebsd`"),
            |v| {
                set(v, "/platforms", json!(["freebsd"]));
            },
        ),
        // Permissions.
        mutate("filesystem permission", true, ForbiddenPermission, |v| {
            push(
                v,
                "/permissions",
                json!({ "kind": "filesystem", "paths": ["~"], "justification": "Reads your files." }),
            );
        }),
        mutate(
            "unknown permission kind",
            true,
            Structure("unknown variant `telepathy`"),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({ "kind": "telepathy", "justification": "Reads your mind, kindly." }),
                );
            },
        ),
        mutate(
            "bare wildcard host",
            true,
            Invalid("permissions[0].hosts[0]", C::InvalidHost),
            |v| {
                set(v, network_host, json!("*"));
            },
        ),
        mutate(
            "wildcard over a TLD",
            true,
            Invalid("permissions[0].hosts[0]", C::InvalidHost),
            |v| {
                set(v, network_host, json!("*.com"));
            },
        ),
        mutate(
            "IP literal host",
            true,
            Invalid("permissions[0].hosts[0]", C::InvalidHost),
            |v| {
                set(v, network_host, json!("127.0.0.1"));
            },
        ),
        mutate(
            "localhost",
            true,
            Invalid("permissions[0].hosts[0]", C::InvalidHost),
            |v| {
                set(v, network_host, json!("localhost"));
            },
        ),
        mutate(
            "URL as host",
            true,
            Invalid("permissions[0].hosts[0]", C::InvalidHost),
            |v| {
                set(v, network_host, json!("https://api.github.com"));
            },
        ),
        mutate(
            "port out of range",
            false,
            Invalid("permissions[0].hosts[0]", C::InvalidHost),
            |v| {
                set(v, network_host, json!("api.github.com:99999"));
            },
        ),
        mutate(
            "empty host list",
            true,
            Invalid("permissions[0].hosts", C::Count),
            |v| {
                set(v, "/permissions/0/hosts", json!([]));
            },
        ),
        mutate(
            "short justification",
            true,
            Invalid("permissions[0].justification", C::Length),
            |v| {
                set(v, "/permissions/0/justification", json!("Needs it"));
            },
        ),
        mutate(
            "two network permissions",
            false,
            Invalid("permissions[4]", C::Duplicate),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({ "kind": "network", "hosts": ["example.com"], "justification": "A second network block." }),
                );
            },
        ),
        mutate(
            "storage quota too small",
            true,
            Invalid("permissions[4].quotaKb", C::Range),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({ "kind": "storage", "quotaKb": 100, "justification": "Caches some results." }),
                );
            },
        ),
        mutate(
            "background interval too short",
            true,
            Invalid("permissions[2].minIntervalSeconds", C::Range),
            |v| set(v, "/permissions/2/minIntervalSeconds", json!(5)),
        ),
        mutate(
            "variables read of step outputs",
            false,
            Invalid("permissions[4].read[0]", C::InvalidVariable),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({ "kind": "variables", "read": ["steps.fetch.body"], "justification": "Reads a step output." }),
                );
            },
        ),
        mutate(
            "variables read wildcard into another plugin",
            false,
            Invalid("permissions[4].read[0]", C::InvalidVariable),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({ "kind": "variables", "read": ["plugins.other.tool.*"], "justification": "Reads another plugin." }),
                );
            },
        ),
        mutate(
            "variables read of the plugin's own namespace",
            false,
            Invalid("permissions[4].read[0]", C::InvalidVariable),
            |v| {
                push(
                    v,
                    "/permissions",
                    json!({ "kind": "variables", "read": ["plugins.acme.githubPrs.reviewCount"], "justification": "Reads its own value." }),
                );
            },
        ),
        // Credentials.
        mutate(
            "credential ref without a slot",
            false,
            Invalid("permissions[1].refs[0]", C::UnknownReference),
            |v| set(v, "/permissions/1/refs", json!(["other"])),
        ),
        mutate(
            "credential slot without a ref",
            false,
            Invalid("contributes.credentials[0].key", C::MissingPermission),
            |v| set(v, "/permissions/1/refs", json!(["other"])),
        ),
        mutate(
            "credential host outside the network hosts",
            false,
            Invalid("contributes.credentials[0].hosts[0]", C::MissingPermission),
            |v| {
                set(
                    v,
                    "/contributes/credentials/0/hosts",
                    json!(["uploads.github.com"]),
                );
            },
        ),
        mutate(
            "header scheme without headerName",
            true,
            Invalid("contributes.credentials[0].headerName", C::Conflict),
            |v| set(v, "/contributes/credentials/0/schemes", json!(["header"])),
        ),
        // Paths and URLs.
        mutate(
            "absolute entry",
            true,
            Invalid("entry", C::InvalidPath),
            |v| {
                set(v, "/entry", json!("/src/index.ts"));
            },
        ),
        mutate(
            "entry leaving the package",
            true,
            Invalid("entry", C::InvalidPath),
            |v| {
                set(v, "/entry", json!("../outside/index.ts"));
            },
        ),
        mutate(
            "entry traversal in the middle",
            true,
            Invalid("entry", C::InvalidPath),
            |v| {
                set(v, "/entry", json!("src/../../index.ts"));
            },
        ),
        mutate(
            "windows entry path",
            true,
            Invalid("entry", C::InvalidPath),
            |v| {
                set(v, "/entry", json!("src\\index.ts"));
            },
        ),
        mutate(
            "icon outside assets",
            true,
            Invalid("icons.128", C::InvalidPath),
            |v| {
                set(v, "/icons/128", json!("icons/icon-128.png"));
            },
        ),
        mutate(
            "key icon traversal",
            true,
            Invalid("assets.keyIcons.pr", C::InvalidPath),
            |v| {
                set(v, "/assets/keyIcons/pr", json!("assets/../manifest.png"));
            },
        ),
        mutate(
            "repository directory traversal",
            true,
            Invalid("repository.directory", C::InvalidPath),
            |v| {
                set(v, "/repository/directory", json!("../other"));
            },
        ),
        mutate(
            "repository named ..",
            false,
            Invalid("repository.url", C::InvalidUrl),
            |v| {
                set(v, "/repository/url", json!("https://github.com/acme/.."));
            },
        ),
        mutate(
            "http homepage",
            true,
            Invalid("homepage", C::InvalidUrl),
            |v| {
                set(v, "/homepage", json!("http://acme.example/"));
            },
        ),
        // Error codes.
        mutate(
            "redeclared shared error code",
            false,
            Invalid("contributes.errorCodes.timeout", C::Reserved),
            |v| {
                set(
                    v,
                    "/contributes/errorCodes/timeout",
                    json!("Took too long."),
                );
            },
        ),
        // User-entered origins (G2).
        mutate(
            "an empty network host list without user origins",
            true,
            Invalid("permissions[0].hosts", C::Count),
            |v| set(v, "/permissions/0/hosts", json!([])),
        ),
        mutate(
            "an openUrl permission without hosts or user origins",
            true,
            Invalid("permissions[4].hosts", C::Count),
            |v| {
                at(v, "/permissions").as_array_mut().unwrap().push(json!({
                    "kind": "openUrl", "hosts": [],
                    "justification": "Opens pages in the browser."
                }));
            },
        ),
        mutate(
            "an openUrl user origin that is no origin setting of the network permission",
            false,
            Invalid("permissions[4].userOrigins[0]", C::UnknownReference),
            |v| {
                at(v, "/permissions").as_array_mut().unwrap().push(json!({
                    "kind": "openUrl", "hosts": [], "userOrigins": ["serverUrl"],
                    "justification": "Opens pages of your server in the browser."
                }));
            },
        ),
        mutate(
            "a user origin naming no setting",
            false,
            Invalid("permissions[0].userOrigins[0].setting", C::UnknownReference),
            |v| {
                set(
                    v,
                    "/permissions/0/userOrigins",
                    json!([{ "setting": "serverUrl" }]),
                );
            },
        ),
        mutate(
            "a user origin in a setting that is not text",
            false,
            Invalid("permissions[0].userOrigins[0].setting", C::Conflict),
            |v| {
                set(
                    v,
                    "/permissions/0/userOrigins",
                    json!([{ "setting": "pollMinutes" }]),
                );
            },
        ),
        mutate(
            "a user origin with methods the permission lacks",
            false,
            Invalid(
                "permissions[0].userOrigins[0].methods",
                C::MissingPermission,
            ),
            |v| {
                set(
                    v,
                    "/permissions/0/userOrigins",
                    json!([{ "setting": "defaultRepository", "methods": ["DELETE"] }]),
                );
            },
        ),
        mutate(
            "an unknown field in a user origin",
            true,
            Structure("unknown field `wildcard`"),
            |v| {
                set(
                    v,
                    "/permissions/0/userOrigins",
                    json!([{ "setting": "defaultRepository", "wildcard": true }]),
                );
            },
        ),
        mutate(
            "a default origin that is not a declared host",
            false,
            Invalid("contributes.settings.fields[1].default", C::InvalidDefault),
            |v| {
                set(
                    v,
                    "/permissions/0/userOrigins",
                    json!([{ "setting": "defaultRepository" }]),
                );
                set(
                    v,
                    "/contributes/settings/fields/1/default",
                    json!({ "type": "text", "value": "https://ghe.example.com" }),
                );
            },
        ),
        mutate(
            "a default origin with a path the slot does not allow",
            false,
            Invalid("contributes.settings.fields[1].default", C::InvalidDefault),
            |v| {
                set(
                    v,
                    "/permissions/0/userOrigins",
                    json!([{ "setting": "defaultRepository" }]),
                );
                set(
                    v,
                    "/contributes/settings/fields/1/default",
                    json!({ "type": "text", "value": "https://api.github.com/api/v3" }),
                );
            },
        ),
        mutate(
            "a credential bound to no user origin",
            false,
            Invalid("contributes.credentials[0].userOrigin", C::UnknownReference),
            |v| {
                set(
                    v,
                    "/contributes/credentials/0/userOrigin",
                    json!("serverUrl"),
                );
            },
        ),
        mutate(
            "a credential without hosts or a user origin",
            true,
            Invalid("contributes.credentials[0].hosts", C::Count),
            |v| set(v, "/contributes/credentials/0/hosts", json!([])),
        ),
        // Raw text fixtures.
        Fixture {
            name: "duplicate key icon name",
            text: EXAMPLE.replace(
                r#""pr": "assets/keys/pr.png""#,
                r#""pr": "assets/keys/pr.png", "pr": "assets/keys/other.png""#,
            ),
            expect: Structure("duplicate key `pr`"),
            schema_rejects: None,
        },
        Fixture {
            name: "duplicate struct field",
            text: EXAMPLE.replace(
                r#""license": "MIT","#,
                r#""license": "MIT", "license": "GPL-3.0","#,
            ),
            expect: Structure("duplicate field `license`"),
            schema_rejects: None,
        },
        Fixture {
            name: "not JSON",
            text: "{ \"manifestVersion\": 1, ".into(),
            expect: Syntax,
            schema_rejects: None,
        },
        Fixture {
            name: "manifest too large",
            text: {
                let padding = " ".repeat(MAX_MANIFEST_BYTES);
                EXAMPLE.replacen('{', &format!("{{{padding}"), 1)
            },
            expect: TooLarge,
            schema_rejects: Some(false),
        },
    ]
    .into_iter()
    .chain(navigation_fixtures())
    .collect()
}

/// The example at plugin API 1.3 with controls, navigation and views.
fn navigation_example() -> Value {
    let mut value = example();
    set(&mut value, "/engines/pluginApi", json!("^1.3"));
    set(
        &mut value,
        "/contributes/actions/0/controls",
        json!({"key": ["press", "click", "long-press"],
               "encoder": ["rotate-clockwise", "rotate-counter-clockwise"],
               "touchZone": ["tap"]}),
    );
    set(
        &mut value,
        "/contributes/navigation",
        json!([
            {"id": "dashboard", "label": "GitHub",
             "icon": {"light": "assets/nav.svg", "dark": "assets/nav-dark.svg"},
             "view": "dashboard"},
            {"id": "setup", "label": "Setup", "icon": "assets/setup.png", "view": "settings"}
        ]),
    );
    set(
        &mut value,
        "/contributes/views",
        json!([{"id": "dashboard", "kind": "variables"}, {"id": "settings", "kind": "settings"}]),
    );
    value
}

fn mutate_navigation(
    name: &'static str,
    schema: bool,
    expect: Expect,
    f: impl FnOnce(&mut Value),
) -> Fixture {
    let mut value = navigation_example();
    f(&mut value);
    Fixture {
        name,
        text: value.to_string(),
        expect,
        schema_rejects: Some(schema),
    }
}

#[allow(clippy::too_many_lines)]
fn navigation_fixtures() -> Vec<Fixture> {
    use Expect::{Invalid, Structure};
    use IssueCode as C;
    let controls = "/contributes/actions/0/controls";
    vec![
        // Controls.
        mutate_navigation(
            "controls of an unknown kind",
            true,
            Structure("unknown variant `dial`"),
            |v| set(v, &format!("{controls}/dial"), json!(["press"])),
        ),
        mutate_navigation(
            "controls with an unknown input",
            true,
            Structure("unknown variant `push`"),
            |v| set(v, &format!("{controls}/key"), json!(["push"])),
        ),
        mutate_navigation(
            "controls without a kind",
            true,
            Invalid("contributes.actions[0].controls", C::Count),
            |v| set(v, controls, json!({})),
        ),
        mutate_navigation(
            "controls with an empty kind",
            true,
            Invalid("contributes.actions[0].controls.encoder", C::Count),
            |v| set(v, &format!("{controls}/encoder"), json!([])),
        ),
        mutate_navigation(
            "a rotation on a key",
            true,
            Invalid("contributes.actions[0].controls.key[1]", C::InvalidValue),
            |v| {
                set(
                    v,
                    &format!("{controls}/key"),
                    json!(["press", "rotate-clockwise"]),
                );
            },
        ),
        mutate_navigation(
            "a tap on an encoder",
            true,
            Invalid(
                "contributes.actions[0].controls.encoder[0]",
                C::InvalidValue,
            ),
            |v| set(v, &format!("{controls}/encoder"), json!(["tap"])),
        ),
        mutate_navigation(
            "a press on a touch zone",
            true,
            Invalid(
                "contributes.actions[0].controls.touchZone[0]",
                C::InvalidValue,
            ),
            |v| set(v, &format!("{controls}/touchZone"), json!(["press"])),
        ),
        mutate_navigation(
            "an input twice",
            true,
            Invalid("contributes.actions[0].controls.key[1]", C::Duplicate),
            |v| set(v, &format!("{controls}/key"), json!(["press", "press"])),
        ),
        mutate_navigation(
            "more than eight inputs on a kind",
            true,
            Invalid("contributes.actions[0].controls.encoder", C::Count),
            |v| {
                set(
                    v,
                    &format!("{controls}/encoder"),
                    json!([
                        "rotate-clockwise",
                        "rotate-counter-clockwise",
                        "press",
                        "release",
                        "click",
                        "double-click",
                        "long-press",
                        "long-press",
                        "press"
                    ]),
                );
            },
        ),
        // The API level gate.
        mutate_navigation(
            "controls below plugin API 1.3",
            false,
            Invalid("contributes.actions[0].controls", C::Conflict),
            |v| set(v, "/engines/pluginApi", json!("^1.2")),
        ),
        mutate_navigation(
            "navigation below plugin API 1.3",
            false,
            Invalid("contributes.navigation", C::Conflict),
            |v| set(v, "/engines/pluginApi", json!("^1.0")),
        ),
        mutate_navigation(
            "views below plugin API 1.3",
            false,
            Invalid("contributes.views", C::Conflict),
            |v| set(v, "/engines/pluginApi", json!("^1.1")),
        ),
        // Navigation and views.
        mutate_navigation(
            "five navigation entries",
            true,
            Invalid("contributes.navigation", C::Count),
            |v| {
                for i in 0..3 {
                    push(
                        v,
                        "/contributes/navigation",
                        json!({"id": format!("more{i}"), "label": "More",
                               "icon": "assets/setup.png", "view": "settings"}),
                    );
                }
            },
        ),
        mutate_navigation(
            "an entry naming an undeclared view",
            false,
            Invalid("contributes.navigation[1].view", C::UnknownReference),
            |v| set(v, "/contributes/navigation/1/view", json!("nowhere")),
        ),
        mutate_navigation(
            "two entries with one id",
            false,
            Invalid("contributes.navigation[1]", C::Duplicate),
            |v| set(v, "/contributes/navigation/1/id", json!("dashboard")),
        ),
        mutate_navigation(
            "two views with one id",
            false,
            Invalid("contributes.views[1]", C::Duplicate),
            |v| set(v, "/contributes/views/1/id", json!("dashboard")),
        ),
        mutate_navigation(
            "a navigation id that is not a local id",
            true,
            Invalid("contributes.navigation[0].id", C::InvalidId),
            |v| set(v, "/contributes/navigation/0/id", json!("Dash_board")),
        ),
        mutate_navigation(
            "an empty label",
            true,
            Invalid("contributes.navigation[0].label", C::Length),
            |v| set(v, "/contributes/navigation/0/label", json!("")),
        ),
        mutate_navigation(
            "a label longer than 24 characters",
            true,
            Invalid("contributes.navigation[0].label", C::Length),
            |v| set(v, "/contributes/navigation/0/label", json!("x".repeat(25))),
        ),
        mutate_navigation(
            "a label with a line break",
            false,
            Invalid("contributes.navigation[0].label", C::InvalidValue),
            |v| set(v, "/contributes/navigation/0/label", json!("Git\nHub")),
        ),
        mutate_navigation(
            "an unknown view kind",
            true,
            Structure("unknown variant `html`"),
            |v| set(v, "/contributes/views/0/kind", json!("html")),
        ),
        mutate_navigation(
            "an unknown navigation field",
            true,
            Structure("unknown field `url`"),
            |v| {
                set(
                    v,
                    "/contributes/navigation/0/url",
                    json!("https://x.example"),
                );
            },
        ),
        mutate_navigation(
            "an icon object without a file",
            true,
            Invalid("contributes.navigation[0].icon", C::Count),
            |v| set(v, "/contributes/navigation/0/icon", json!({})),
        ),
        mutate_navigation(
            "an icon leaving the package",
            true,
            Invalid("contributes.navigation[1].icon", C::InvalidPath),
            |v| {
                set(
                    v,
                    "/contributes/navigation/1/icon",
                    json!("assets/../x.png"),
                );
            },
        ),
        mutate_navigation(
            "an absolute icon path",
            true,
            Invalid("contributes.navigation[1].icon", C::InvalidPath),
            |v| set(v, "/contributes/navigation/1/icon", json!("/etc/x.png")),
        ),
        mutate_navigation(
            "a remote icon",
            true,
            Invalid("contributes.navigation[0].icon.light", C::InvalidPath),
            |v| {
                set(
                    v,
                    "/contributes/navigation/0/icon/light",
                    json!("https://evil.example/x.svg"),
                );
            },
        ),
        mutate_navigation(
            "a data: icon",
            true,
            Invalid("contributes.navigation[0].icon.dark", C::InvalidPath),
            |v| {
                set(
                    v,
                    "/contributes/navigation/0/icon/dark",
                    json!("data:image/svg+xml,<svg/>"),
                );
            },
        ),
        mutate_navigation(
            "a backslash in an icon path",
            true,
            Invalid("contributes.navigation[1].icon", C::InvalidPath),
            |v| set(v, "/contributes/navigation/1/icon", json!("assets\\x.png")),
        ),
        mutate_navigation(
            "an icon outside assets",
            true,
            Invalid("contributes.navigation[1].icon", C::InvalidPath),
            |v| set(v, "/contributes/navigation/1/icon", json!("nav.png")),
        ),
        mutate_navigation(
            "an icon of another format",
            true,
            Invalid("contributes.navigation[1].icon", C::InvalidPath),
            |v| set(v, "/contributes/navigation/1/icon", json!("assets/nav.gif")),
        ),
    ]
}

fn check_rust(fixture: &Fixture) -> Result<(), String> {
    let result = Manifest::from_json(&fixture.text);
    let error = match result {
        Ok(manifest) => match manifest.validate() {
            Ok(()) => return Err("was accepted".into()),
            Err(errors) => ManifestError::Invalid(errors),
        },
        Err(error) => error,
    };
    let matched = match (&fixture.expect, &error) {
        (Expect::Structure(text), ManifestError::Structure { message, .. }) => {
            message.contains(text)
        }
        (Expect::UnsupportedVersion, ManifestError::UnsupportedVersion { .. })
        | (Expect::ForbiddenPermission, ManifestError::ForbiddenPermission { .. })
        | (Expect::TooLarge, ManifestError::TooLarge { .. })
        | (Expect::Syntax, ManifestError::Syntax { .. }) => true,
        (Expect::Invalid(path, code), ManifestError::Invalid(errors)) => errors.has(path, *code),
        _ => false,
    };
    if matched {
        Ok(())
    } else {
        Err(format!("expected {:?}, got: {error}", fixture.expect))
    }
}

#[test]
fn negative_fixtures_are_refused_with_precise_errors() {
    let fixtures = fixtures();
    assert!(fixtures.len() >= 20);
    let failures: Vec<String> = fixtures
        .iter()
        .filter_map(|f| check_rust(f).err().map(|e| format!("{}: {e}", f.name)))
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn schema_agrees_with_the_negative_fixtures() {
    let validator = schema();
    let mut failures = Vec::new();
    for fixture in fixtures() {
        let Some(rejects) = fixture.schema_rejects else {
            continue;
        };
        let value: Value = serde_json::from_str(&fixture.text).unwrap();
        let errors = schema_errors(&validator, &value);
        if rejects == errors.is_empty() {
            failures.push(format!(
                "{}: schema {} (errors: {errors:?})",
                fixture.name,
                if rejects { "accepted it" } else { "refused it" }
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn error_messages_are_presentable() {
    let mut value = example();
    set(&mut value, "/entry", json!("../x.ts"));
    set(&mut value, "/name", json!(""));
    let error = Manifest::parse_and_validate(&value.to_string()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the manifest has 2 problems:\n- name: must not be empty\n- entry: `../x.ts` contains a \
         . or .. segment, which could leave the package"
    );

    let error = Manifest::from_json(r#"{"manifestVersion": 1, "nope": true}"#).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("manifest.json does not match manifestVersion 1: unknown field `nope`"),
        "{error}"
    );
    assert!(
        error.to_string().ends_with("(line 1, column 29)"),
        "{error}"
    );
}

// ---------------------------------------------------------- compatibility

#[test]
fn compatibility_with_app_version_and_platform() {
    let manifest = Manifest::from_json(EXAMPLE).unwrap();
    for (version, compatible) in [
        ("0.7.9", false),
        ("0.8.0", true),
        ("0.9.0-alpha.1", true),
        ("0.8.0-rc.1", false),
        ("0.99.0", true),
        ("1.0.0", false),
        ("1.0.0-beta.1", true),
    ] {
        let version = Version::parse(version).unwrap();
        for platform in Platform::ALL {
            assert_eq!(
                manifest.is_compatible(&version, platform),
                compatible,
                "{version} on {platform}"
            );
        }
    }

    let full = Manifest::from_json(FULL).unwrap();
    let v = Version::new(1, 4, 2);
    assert!(full.is_compatible(&v, Platform::Macos));
    assert!(full.is_compatible(&v, Platform::Linux));
    assert!(!full.is_compatible(&v, Platform::Windows));
    assert!(!full.is_compatible(&Version::new(1, 4, 3), Platform::Linux));
}

#[test]
fn compatibility_reasons() {
    let full = Manifest::from_json(FULL).unwrap();
    let ok = HostEnvironment {
        app_version: Version::new(1, 0, 0),
        plugin_api: ApiLevel::new(1, 3),
        platform: Platform::Macos,
    };
    assert_eq!(full.check_compatibility(&ok), Ok(()));

    let host = HostEnvironment {
        app_version: Version::new(0, 6, 0),
        plugin_api: ApiLevel::new(1, 0),
        platform: Platform::Windows,
    };
    let reasons = full.check_compatibility(&host).unwrap_err();
    assert!(matches!(reasons[0], Incompatibility::AppVersion { .. }));
    let messages: Vec<String> = reasons.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "needs DeckPilot >=0.8.0 <=1.4.2; this is DeckPilot 0.6.0",
            "needs plugin API ^1.1; this DeckPilot provides plugin API 1.0",
            "is not available for Windows (only for macOS, Linux)",
        ]
    );
    let next_major = HostEnvironment {
        plugin_api: ApiLevel::new(2, 0),
        ..ok
    };
    assert!(matches!(
        full.check_compatibility(&next_major).unwrap_err()[..],
        [Incompatibility::PluginApi { .. }]
    ));
}

// ---------------------------------------------------------------- integrity

fn digest(n: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([n; 32])
}

fn integrity() -> PackageIntegrity {
    let files = [
        "manifest.json",
        "plugin.js",
        "LICENSE",
        "README.md",
        "assets/icon-128.png",
        "assets/icon-32.png",
        "assets/keys/pr.png",
    ];
    PackageIntegrity {
        manifest_sha256: digest(0),
        package: PackageDigest {
            sha256: digest(9),
            size: 40_000,
        },
        files: files
            .iter()
            .enumerate()
            .map(|(i, f)| ((*f).to_owned(), digest(u8::try_from(i).unwrap())))
            .collect(),
    }
}

#[test]
fn package_integrity() {
    let manifest = Manifest::from_json(EXAMPLE).unwrap();
    let record = integrity();
    record.validate(&manifest).unwrap();
    assert_eq!(record.verify_file("plugin.js", &digest(1)), Ok(()));
    assert!(record.verify_file("plugin.js", &digest(2)).is_err());
    assert!(record.verify_file("other.js", &digest(1)).is_err());

    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["manifest_sha256"], "00".repeat(32));
    assert_eq!(
        serde_json::from_value::<PackageIntegrity>(json).unwrap(),
        record
    );

    let mut bad = integrity();
    bad.files.remove("assets/keys/pr.png");
    bad.files.remove("LICENSE");
    bad.files.insert("assets/unused.png".into(), digest(7));
    bad.files.insert("../escape.js".into(), digest(7));
    bad.files.insert("PLUGIN.JS".into(), digest(7));
    bad.files.insert("manifest.json".into(), digest(8));
    bad.package.size = 6 * 1024 * 1024;
    let errors = bad.validate(&manifest).unwrap_err();
    for (path, code) in [
        ("package.size", IssueCode::Range),
        ("files", IssueCode::PackageFile),
        ("files.assets/unused.png", IssueCode::PackageFile),
        ("files.../escape.js", IssueCode::InvalidPath),
        ("files.plugin.js", IssueCode::Duplicate),
        ("files.manifest.json", IssueCode::Mismatch),
    ] {
        assert!(errors.has(path, code), "{path} {code:?}: {errors}");
    }
    let missing: Vec<&str> = errors
        .issues()
        .iter()
        .filter(|i| i.path == "files")
        .map(|i| i.message.as_str())
        .collect();
    assert_eq!(
        missing,
        [
            "`LICENSE` is missing from the package",
            "`assets/keys/pr.png` is missing from the package"
        ]
    );
}

// ------------------------------------------------------- artifact contract

#[test]
fn runtime_declares_the_artifact() {
    use deckpilot_plugin_manifest::{REQUIRED_FILES, RuntimeContract};
    // The #191 example predates the contract: it still parses and
    // validates, and names the JavaScript bundle the installer refuses.
    let legacy = Manifest::parse_and_validate(EXAMPLE).unwrap();
    assert_eq!(legacy.runtime, None);
    assert_eq!(legacy.runtime_contract(), RuntimeContract::LegacyJs);
    assert_eq!(legacy.artifact_file(), "plugin.js");

    let wasm = Manifest::parse_and_validate(FULL).unwrap();
    assert_eq!(wasm.runtime_contract(), RuntimeContract::Wasm { abi: 1 });
    assert_eq!(wasm.artifact_file(), "plugin.wasm");
    assert_eq!(wasm.required_files(), REQUIRED_FILES);
    assert_eq!(REQUIRED_FILES, ["manifest.json", "plugin.wasm", "LICENSE"]);
    let json: Value = serde_json::from_str(&wasm.to_json_pretty()).unwrap();
    assert_eq!(json["runtime"], json!({ "kind": "wasm", "abi": 1 }));
}

// -------------------------------------------------------------- first party

fn with_publisher(publisher: &str) -> Manifest {
    let mut value = example();
    set(&mut value, "/id", json!(format!("{publisher}.github-prs")));
    set(&mut value, "/publisher/id", json!(publisher));
    serde_json::from_value(value).unwrap()
}

#[test]
fn only_the_first_party_rules_accept_the_deckpilot_publisher() {
    let first_party = with_publisher("deckpilot");
    let refused = first_party.validate().unwrap_err();
    assert!(
        refused.has("publisher.id", IssueCode::Reserved),
        "{refused}"
    );
    first_party.validate_first_party().unwrap();

    // Every other reserved publisher stays reserved, and a third-party
    // manifest is not first-party.
    for publisher in ["knittingcode", "test", "github", "core"] {
        let errors = with_publisher(publisher)
            .validate_first_party()
            .unwrap_err();
        assert!(
            errors.has("publisher.id", IssueCode::Reserved),
            "{publisher}: {errors}"
        );
    }
    let third_party = with_publisher("acme");
    third_party.validate().unwrap();
    let errors = third_party.validate_first_party().unwrap_err();
    assert!(errors.has("publisher.id", IssueCode::Mismatch), "{errors}");

    // The other rules still apply.
    let mut value = serde_json::to_value(&first_party).unwrap();
    set(&mut value, "/id", json!("deckpilot.other"));
    set(&mut value, "/publisher/id", json!("acme"));
    let mismatched: Manifest = serde_json::from_value(value).unwrap();
    let errors = mismatched.validate_first_party().unwrap_err();
    assert!(errors.has("id", IssueCode::Mismatch), "{errors}");
}
