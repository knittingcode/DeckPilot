//! `dependencies` and `requiresDependencies` (plugin API 1.4): parsed by
//! the model, checked by validation and by the bundled JSON Schema alike;
//! manifests without them keep working.

use deckpilot_plugin_manifest::{DependencyTarget, IssueCode, Manifest, SCHEMA_JSON};
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("fixtures/example-manifest.json");

fn schema_ok(instance: &Value) -> bool {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    jsonschema::options()
        .build(&schema)
        .unwrap()
        .is_valid(instance)
}

/// The example at plugin API 1.4 with a required and an optional
/// dependency, the first action needing the optional one.
fn with_dependencies() -> Value {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["engines"]["pluginApi"] = json!("^1.4");
    value["dependencies"] = json!([
        { "id": "acme.projects", "version": ">=1.2.0 <2.0.0" },
        { "id": "projects", "version": ">=0.8.0" },
        {
            "id": "acme.calendar",
            "version": ">=1.0.0",
            "optional": true,
            "justification": "Shows your next meeting."
        }
    ]);
    value["contributes"]["actions"][0]["requiresDependencies"] = json!(["acme.calendar"]);
    value
}

fn issues(value: &Value) -> Vec<(String, IssueCode)> {
    let manifest = Manifest::from_json(&value.to_string()).expect("structure");
    match manifest.validate() {
        Ok(()) => Vec::new(),
        Err(errors) => errors.0.into_iter().map(|i| (i.path, i.code)).collect(),
    }
}

#[test]
fn a_legacy_manifest_has_no_dependencies() {
    let manifest = Manifest::parse_and_validate(EXAMPLE).unwrap();
    assert_eq!(manifest.dependencies(), []);
    assert_eq!(manifest.required_dependencies().count(), 0);
    let action = &manifest.contributes.actions()[0];
    assert_eq!(action.requires_dependencies(), Vec::<String>::new());
    // It serializes back without the new fields.
    let json = manifest.to_json_pretty();
    assert!(!json.contains("dependencies"), "{json}");
    assert!(!json.contains("requiresDependencies"), "{json}");
}

#[test]
fn dependencies_parse_validate_and_round_trip() {
    let value = with_dependencies();
    assert!(schema_ok(&value));
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    let required: Vec<&str> = manifest
        .required_dependencies()
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(required, ["acme.projects", "projects"]);
    let optional: Vec<&str> = manifest
        .optional_dependencies()
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(optional, ["acme.calendar"]);
    assert!(matches!(
        manifest.dependency("projects").unwrap().id,
        DependencyTarget::Builtin(_)
    ));
    assert!(
        manifest
            .dependency("acme.projects")
            .unwrap()
            .id
            .plugin()
            .is_some()
    );
    let action = &manifest.contributes.actions()[0];
    let needed: Vec<&str> = manifest
        .action_dependencies(action)
        .map(|d| d.id.as_str())
        .collect();
    assert_eq!(needed, ["acme.calendar"]);
    let again: Value = serde_json::from_str(&manifest.to_json_pretty()).unwrap();
    assert_eq!(again, value);
}

#[test]
fn dependencies_need_plugin_api_1_4() {
    let mut value = with_dependencies();
    value["engines"]["pluginApi"] = json!("^1.3");
    let found = issues(&value);
    assert!(
        found.contains(&("dependencies".into(), IssueCode::Conflict)),
        "{found:?}"
    );
    assert!(
        found.contains(&(
            "contributes.actions[0].requiresDependencies".into(),
            IssueCode::Conflict
        )),
        "{found:?}"
    );
}

#[test]
fn bad_dependency_declarations_are_refused() {
    // Itself, twice, an unsatisfiable range, an empty justification.
    let mut value = with_dependencies();
    value["dependencies"] = json!([
        { "id": "acme.github-prs", "version": ">=1.0.0" },
        { "id": "acme.projects", "version": ">=2.0.0 <1.0.0" },
        { "id": "acme.projects", "version": ">=1.0.0", "justification": "" }
    ]);
    value["contributes"]["actions"][0]["requiresDependencies"] = json!(["acme.unknown"]);
    let found = issues(&value);
    for expected in [
        ("dependencies[0].id", IssueCode::Conflict),
        ("dependencies[1].version", IssueCode::UnsatisfiableRange),
        ("dependencies[2].justification", IssueCode::Length),
        ("dependencies[2]", IssueCode::Duplicate),
        (
            "contributes.actions[0].requiresDependencies[0]",
            IssueCode::UnknownReference,
        ),
    ] {
        assert!(
            found.contains(&(expected.0.into(), expected.1)),
            "{expected:?} not in {found:?}"
        );
    }
}

#[test]
fn malformed_dependencies_do_not_parse_or_pass_the_schema() {
    for (name, dependency) in [
        (
            "an unknown field",
            json!({"id": "acme.x", "version": ">=1.0.0", "grants": []}),
        ),
        ("no version", json!({"id": "acme.x"})),
        (
            "a caret range",
            json!({"id": "acme.x", "version": "^1.0.0"}),
        ),
        (
            "an upper-case id",
            json!({"id": "Acme.x", "version": ">=1.0.0"}),
        ),
        ("two dots", json!({"id": "a.b.c", "version": ">=1.0.0"})),
        (
            "a non-boolean optional",
            json!({"id": "acme.x", "version": ">=1.0.0", "optional": "yes"}),
        ),
    ] {
        let mut value = with_dependencies();
        value["dependencies"] = json!([dependency]);
        assert!(
            Manifest::from_json(&value.to_string()).is_err(),
            "{name}: the model accepted it"
        );
        assert!(!schema_ok(&value), "{name}: the schema accepted it");
    }
}

#[test]
fn naming_a_required_dependency_on_an_action_is_only_a_warning() {
    let mut value = with_dependencies();
    value["contributes"]["actions"][0]["requiresDependencies"] = json!(["acme.projects"]);
    let manifest = Manifest::parse_and_validate(&value.to_string()).unwrap();
    assert!(
        manifest
            .review_warnings()
            .iter()
            .any(|w| w.path == "contributes.actions[0].requiresDependencies[0]")
    );
}
