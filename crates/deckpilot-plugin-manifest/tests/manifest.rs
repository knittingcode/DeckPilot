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

#[test]
fn full_manifest_parses_and_validates() {
    let manifest = Manifest::parse_and_validate(FULL).unwrap();
    assert_eq!(manifest.permissions.len(), 9);
    assert_eq!(manifest.contributes.actions()[0].parameters.len(), 8);
    assert_eq!(manifest.contributes.credentials().len(), 3);
    assert_eq!(
        manifest.review_warnings(),
        [] as [deckpilot_plugin_manifest::Issue; 0]
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
