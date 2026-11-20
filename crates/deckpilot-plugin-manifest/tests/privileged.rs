//! Plugin API 1.9's privileged kinds (`docs/marketplace/module-migration.md`
//! §7, X1 to X12): accepted only for a first-party plugin shipped inside
//! the app (`Manifest::validate_bundled`) with `^1.9`, refused for every
//! other manifest, checked against the host's tool policy, and described
//! the same way by the Rust model and the bundled JSON Schema. A `^1.8`
//! manifest without them validates exactly as before.

use deckpilot_plugin_manifest::{
    ApiLevel, IssueCode, Manifest, ManifestError, PRIVILEGED_API, Permission, PermissionKind,
    ResourceKind, SCHEMA_JSON, ToolExecutable, ViewKind,
};
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("fixtures/example-manifest.json");

fn schema() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap()
}

fn schema_ok(value: &Value) -> bool {
    schema().is_valid(value)
}

/// The example as a first-party plugin of plugin API `level`, without the
/// network and credential permissions (a `tools` plugin may not have them).
fn first_party(level: &str) -> Value {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["id"] = json!("deckpilot.git");
    value["publisher"]["id"] = json!("deckpilot");
    value["engines"]["pluginApi"] = json!(level);
    value["permissions"]
        .as_array_mut()
        .unwrap()
        .retain(|p| p["kind"] != "network" && p["kind"] != "credentials");
    value["contributes"]
        .as_object_mut()
        .unwrap()
        .remove("credentials");
    value
}

fn with_permission(level: &str, permission: Value) -> Value {
    let mut value = first_party(level);
    value["permissions"]
        .as_array_mut()
        .unwrap()
        .push(permission);
    value
}

fn third_party(mut value: Value) -> Value {
    value["id"] = json!("acme.git");
    value["publisher"]["id"] = json!("acme");
    value
}

fn parse(value: &Value) -> Manifest {
    Manifest::from_json(&value.to_string()).unwrap()
}

fn git_tools() -> Value {
    json!({
        "kind": "tools",
        "tools": [{
            "executable": "git",
            "subcommands": [
                {"name": "status", "options": ["--porcelain=v2", "--branch", "-z"]},
                {"name": "log", "options": ["-1", "--format"]},
                {"name": "config", "options": ["--get"]},
                {"name": "remote"},
                {"name": "rev-parse", "options": ["--verify", "-q"]},
                {"name": "ls-remote"}
            ],
            "network": true
        }],
        "justification": "Reads the status of the repositories you bind to keys."
    })
}

fn docker_tools() -> Value {
    json!({
        "kind": "tools",
        "tools": [{
            "executable": "docker",
            "subcommands": [
                {"name": "ps", "options": ["--no-trunc", "--format", "--all", "--filter"]},
                {"name": "compose logs", "options": ["--tail", "--since"]},
                {"name": "compose up", "options": ["--detach"]},
                {"name": "stop"}
            ],
            "modify": true
        }],
        "justification": "Shows and starts the containers of your compose projects."
    })
}

/// Every privileged kind, each in a manifest of its own.
fn privileged_manifests(level: &str) -> Vec<(&'static str, Value)> {
    let mut out = vec![
        ("tools", with_permission(level, git_tools())),
        (
            "resources",
            with_permission(
                level,
                json!({"kind": "resources", "paths": true, "open": true,
                       "justification": "Opens your project folders in your editor."}),
            ),
        ),
        (
            "projectContext",
            with_permission(
                level,
                json!({"kind": "projectContext", "select": true,
                       "justification": "Switches the active project from a key."}),
            ),
        ),
        (
            "openUrl.fromParams",
            with_permission(
                level,
                json!({"kind": "openUrl", "hosts": [], "fromParams": true,
                       "justification": "Opens the links you typed into the action."}),
            ),
        ),
    ];
    let mut host_views = with_permission(
        level,
        json!({"kind": "hostViews", "justification": "Shows the Projects screen and its card."}),
    );
    host_views["contributes"]["views"] = json!([{"id": "projects", "kind": "projects"}]);
    host_views["contributes"]["statusItems"] = json!(["activeProject"]);
    host_views["contributes"]["dashboardCards"] = json!(["activeProject"]);
    out.push(("hostViews", host_views));
    let mut resource = first_party(level);
    resource["contributes"]["actions"][0]["parameters"]
        .as_array_mut()
        .unwrap()
        .push(
            json!({"key": "repo", "label": "Repository", "required": false,
                     "kind": {"type": "resource", "kinds": ["directory"]}}),
        );
    out.push(("resource parameter", resource));
    out
}

// ---------------------------------------------------------------- accepted

#[test]
fn a_bundled_first_party_plugin_of_1_9_may_declare_every_privileged_kind() {
    assert_eq!(PRIVILEGED_API, ApiLevel::new(1, 9));
    for (what, value) in privileged_manifests("^1.9") {
        assert!(schema_ok(&value), "{what}: schema");
        let manifest = parse(&value);
        manifest
            .validate_bundled()
            .unwrap_or_else(|e| panic!("{what}: {e}"));
        // It serializes back to the same JSON.
        assert_eq!(serde_json::to_value(&manifest).unwrap(), value, "{what}");
    }
}

#[test]
fn the_model_reads_the_privileged_kinds() {
    let value = with_permission("^1.9", git_tools());
    let manifest = parse(&value);
    let Some(Permission::Tools { tools, .. }) = manifest.permission(PermissionKind::Tools) else {
        panic!("no tools permission");
    };
    assert_eq!(tools[0].executable, ToolExecutable::Git);
    assert!(tools[0].network() && !tools[0].modify());
    assert_eq!(
        tools[0].subcommands[0].options(),
        ["--porcelain=v2", "--branch", "-z"]
    );
    assert!(PermissionKind::Tools.is_first_party_only());
    assert!(!PermissionKind::Dialogs.is_first_party_only());

    let docker = parse(&with_permission("^1.9", docker_tools()));
    docker.validate_bundled().unwrap();

    let (_, views) = privileged_manifests("^1.9").remove(4);
    let manifest = parse(&views);
    assert_eq!(manifest.contributes.views()[0].kind, ViewKind::Projects);
    assert_eq!(ViewKind::Workflows.as_str(), "workflows");
    assert_eq!(manifest.contributes.status_items().len(), 1);
    assert_eq!(manifest.contributes.dashboard_cards().len(), 1);

    let (_, resource) = privileged_manifests("^1.9").remove(5);
    let manifest = parse(&resource);
    assert!(matches!(
        &manifest.contributes.actions()[0].parameters[2].kind,
        deckpilot_plugin_manifest::ParameterKind::Resource { kinds } if kinds == &[ResourceKind::Directory]
    ));
}

// ---------------------------------------------------------------- the gate

#[test]
fn every_other_manifest_is_refused_each_privileged_kind() {
    for (what, value) in privileged_manifests("^1.9") {
        // A registry `deckpilot.*` package (validate_first_party).
        let errors = parse(&value).validate_first_party().unwrap_err();
        assert!(
            errors
                .issues()
                .iter()
                .any(|i| i.code == IssueCode::FirstPartyOnly),
            "{what}: {errors}"
        );
        // A third-party manifest (validate, and the bundled rules refuse
        // its publisher anyway).
        let third = parse(&third_party(value.clone()));
        let errors = third.validate().unwrap_err();
        assert!(
            errors
                .issues()
                .iter()
                .any(|i| i.code == IssueCode::FirstPartyOnly),
            "{what}: {errors}"
        );
        assert!(
            errors
                .to_string()
                .contains("only for DeckPilot's own plugins"),
            "{what}: {errors}"
        );
        let errors = third.validate_bundled().unwrap_err();
        assert!(
            errors.has("publisher.id", IssueCode::Mismatch),
            "{what}: {errors}"
        );
    }
}

#[test]
fn below_1_9_every_privileged_kind_is_refused_with_the_level_to_set() {
    for level in ["^1.0", "^1.5", "^1.8"] {
        for (what, value) in privileged_manifests(level) {
            let errors = parse(&value).validate_bundled().unwrap_err();
            assert!(
                errors
                    .to_string()
                    .contains("needs plugin API 1.9: set engines.pluginApi to `^1.9`"),
                "{level} {what}: {errors}"
            );
        }
    }
}

#[test]
fn a_1_8_manifest_without_them_validates_as_before() {
    let value: Value = serde_json::from_str(EXAMPLE).unwrap();
    let manifest = Manifest::parse_and_validate(EXAMPLE).unwrap();
    assert_eq!(serde_json::to_value(&manifest).unwrap(), value);
    assert_eq!(
        manifest.review_warnings(),
        [] as [deckpilot_plugin_manifest::Issue; 0]
    );
    let mut eight = value.clone();
    eight["engines"]["pluginApi"] = json!("^1.8");
    Manifest::parse_and_validate(&eight.to_string()).unwrap();
}

#[test]
fn never_grantable_kinds_stay_unparseable_and_project_context_is_gated() {
    for kind in [
        "process",
        "exec",
        "shell",
        "filesystem",
        "workflows",
        "ui",
        "clipboard",
    ] {
        let value = with_permission(
            "^1.9",
            json!({"kind": kind, "justification": "Runs anything."}),
        );
        assert!(
            matches!(
                Manifest::from_json(&value.to_string()),
                Err(ManifestError::ForbiddenPermission { .. })
            ),
            "{kind}"
        );
    }
    // `projectContext` parses since 1.9, and only the bundled rules accept it.
    let value = third_party(with_permission(
        "^1.9",
        json!({"kind": "projectContext", "justification": "Reads the active project."}),
    ));
    let errors = parse(&value).validate().unwrap_err();
    assert!(
        errors.has("permissions[2]", IssueCode::FirstPartyOnly),
        "{errors}"
    );
}

// ---------------------------------------------------------- tool policy

fn tools_errors(tools: &Value) -> String {
    let value = with_permission(
        "^1.9",
        json!({"kind": "tools", "tools": tools, "justification": "Runs git for your keys."}),
    );
    match Manifest::from_json(&value.to_string()) {
        Ok(manifest) => match manifest.validate_bundled() {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    }
}

#[test]
fn only_git_and_docker_exist() {
    for executable in [
        "bash",
        "sh",
        "/usr/bin/git",
        "git.exe",
        "python",
        "Git",
        "kubectl",
    ] {
        let error =
            tools_errors(&json!([{"executable": executable, "subcommands": [{"name": "status"}]}]));
        assert!(error.contains("unknown variant"), "{executable}: {error}");
        let value = with_permission(
            "^1.9",
            json!({"kind": "tools", "justification": "Runs a program for you.",
                   "tools": [{"executable": executable, "subcommands": [{"name": "status"}]}]}),
        );
        assert!(!schema_ok(&value), "{executable}: schema");
    }
    // No argv, no shell string, no path: unknown fields.
    for extra in [
        json!({"argv": ["sh", "-c", "x"]}),
        json!({"path": "/bin/git"}),
        json!({"shell": true}),
    ] {
        let mut tool = json!({"executable": "git", "subcommands": [{"name": "status"}]});
        tool.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let error = tools_errors(&json!([tool]));
        assert!(error.contains("unknown field"), "{extra}: {error}");
    }
}

#[test]
fn forbidden_git_subcommands_and_options_are_refused() {
    let git = |name: &str, options: Value| {
        tools_errors(
            &json!([{"executable": "git", "subcommands": [{"name": name, "options": options}]}]),
        )
    };
    for option in [
        "-c",
        "-ccore.sshCommand=evil",
        "--config-env",
        "-C",
        "--git-dir",
        "--work-tree",
        "--exec-path",
        "--upload-pack",
        "--upload-pack=evil",
        "-u",
        "--receive-pack",
        "--output",
        "-o",
        "--ext-diff",
        "--textconv",
        "--exec",
        "--super-prefix",
        "--namespace",
    ] {
        let error = git("log", json!([option]));
        assert!(
            error.contains("is never allowed for git"),
            "{option}: {error}"
        );
    }
    for option in ["--all", "-v", "--follow", "-p"] {
        let error = git("log", json!([option]));
        assert!(
            error.contains("is not an option DeckPilot allows"),
            "{option}: {error}"
        );
    }
    for name in [
        "push",
        "fetch",
        "clone",
        "commit",
        "my-alias",
        "submodule",
        "remote -v",
    ] {
        let error = git(name, json!([]));
        assert!(error.contains("is not a git subcommand"), "{name}: {error}");
    }
    let error = git("--exec-path=/tmp", json!([]));
    assert!(error.contains("is never allowed"), "{error}");
    let error = git("remote", json!(["get-url"]));
    assert!(error.contains("it takes none"), "{error}");
}

#[test]
fn forbidden_docker_subcommands_and_options_are_refused() {
    let docker = |name: &str, options: Value, modify: bool| {
        tools_errors(&json!([{"executable": "docker", "modify": modify,
                             "subcommands": [{"name": name, "options": options}]}]))
    };
    for name in [
        "run",
        "exec",
        "create",
        "cp",
        "attach",
        "build",
        "push",
        "pull",
        "login",
        "inspect",
        "system",
        "volume",
        "network",
        "image",
        "kill",
        "compose config",
        "compose run",
        "compose exec",
        "compose build",
    ] {
        let error = docker(name, json!([]), false);
        assert!(error.contains("is never allowed"), "{name}: {error}");
    }
    for option in [
        "-v",
        "--volume",
        "--mount",
        "--privileged",
        "--cap-add",
        "--device",
        "--pid",
        "--network",
        "--security-opt",
        "-H",
        "--host",
        "--context",
        "--config",
        "--tlsverify",
        "-e",
        "--env",
        "--env-file",
        "--project-directory",
        "-f",
        "--file",
    ] {
        let error = docker("ps", json!([option]), false);
        assert!(
            error.contains("is never allowed for docker"),
            "{option}: {error}"
        );
    }
    for option in [
        "--volumes",
        "--rmi",
        "--build",
        "--force-recreate",
        "--renew-anon-volumes",
    ] {
        let up = docker("compose up", json!([option]), true);
        let down = docker("compose down", json!([option]), true);
        assert!(
            up.contains("is never allowed") && down.contains("is never allowed"),
            "{option}"
        );
    }
}

#[test]
fn sub_grants_are_declared_exactly_when_needed() {
    // ls-remote without network, and network without ls-remote.
    let error =
        tools_errors(&json!([{"executable": "git", "subcommands": [{"name": "ls-remote"}]}]));
    assert!(error.contains("needs `network: true`"), "{error}");
    let error = tools_errors(
        &json!([{"executable": "git", "network": true, "subcommands": [{"name": "status"}]}]),
    );
    assert!(
        error.contains("no declared git subcommand needs `network`"),
        "{error}"
    );
    // Modifying docker subcommands without modify, modify on git.
    for name in [
        "compose up",
        "compose down",
        "compose restart",
        "start",
        "stop",
        "restart",
    ] {
        let error =
            tools_errors(&json!([{"executable": "docker", "subcommands": [{"name": name}]}]));
        assert!(error.contains("needs `modify: true`"), "{name}: {error}");
    }
    let error = tools_errors(
        &json!([{"executable": "git", "modify": true, "subcommands": [{"name": "status"}]}]),
    );
    assert!(error.contains("exists only for docker"), "{error}");
    // Duplicates and limits.
    let error = tools_errors(&json!([
        {"executable": "git", "subcommands": [{"name": "status"}]},
        {"executable": "git", "subcommands": [{"name": "log"}]}
    ]));
    assert!(error.contains("permissions[2].tools[1]"), "{error}");
    assert_ne!(tools_errors(&json!([])), "");
    assert_ne!(
        tools_errors(&json!([{"executable": "git", "subcommands": []}])),
        ""
    );
    let error = tools_errors(&json!([{"executable": "git", "subcommands": [
        {"name": "status", "options": ["-z", "-z"]}]}]));
    assert!(error.contains("options[1]"), "{error}");
}

#[test]
fn a_tools_plugin_has_no_exfiltration_channel() {
    for (kind, permission) in [
        (
            "network",
            json!({"kind": "network", "hosts": ["example.com"], "justification": "Sends results to a server."}),
        ),
        (
            "openUrl",
            json!({"kind": "openUrl", "hosts": ["example.com"], "justification": "Opens a page in the browser."}),
        ),
    ] {
        let mut value = with_permission("^1.9", git_tools());
        value["permissions"]
            .as_array_mut()
            .unwrap()
            .push(permission);
        let errors = parse(&value).validate_bundled().unwrap_err();
        assert!(
            errors
                .to_string()
                .contains(&format!("cannot also declare `{kind}`")),
            "{kind}: {errors}"
        );
    }
}

// ----------------------------------------------------------- other rules

#[test]
fn host_views_need_the_permission_and_resource_settings_do_not_exist() {
    let mut value = first_party("^1.9");
    value["contributes"]["views"] = json!([{"id": "flows", "kind": "workflows"}]);
    let errors = parse(&value).validate_bundled().unwrap_err();
    assert!(
        errors.has("permissions", IssueCode::MissingPermission),
        "{errors}"
    );

    let mut value = with_permission(
        "^1.9",
        json!({"kind": "hostViews", "justification": "Shows the Workflows screen."}),
    );
    value["contributes"]["statusItems"] = json!(["somethingElse"]);
    assert!(Manifest::from_json(&value.to_string()).is_err());
    assert!(!schema_ok(&value));

    let mut value = first_party("^1.9");
    value["contributes"]["settings"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(
            json!({"key": "repo", "label": "Repository", "required": false,
                     "kind": {"type": "resource", "kinds": ["directory"]}}),
        );
    let errors = parse(&value).validate_bundled().unwrap_err();
    assert!(
        errors
            .to_string()
            .contains("a setting cannot be a resource"),
        "{errors}"
    );

    // A resource parameter needs at least one known kind, and no default.
    for kind in [
        json!({"type": "resource", "kinds": []}),
        json!({"type": "resource", "kinds": ["path"]}),
    ] {
        let mut value = first_party("^1.9");
        value["contributes"]["actions"][0]["parameters"][0]["kind"] = kind.clone();
        let refused = Manifest::from_json(&value.to_string())
            .map_err(|e| e.to_string())
            .and_then(|m| m.validate_bundled().map_err(|e| e.to_string()));
        assert!(refused.is_err(), "{kind}");
        assert!(!schema_ok(&value), "{kind}: schema");
    }
}

#[test]
fn the_two_schema_copies_are_identical() {
    let published = include_str!("../../../plugins/schemas/plugin-manifest.schema.json");
    assert_eq!(
        SCHEMA_JSON, published,
        "crates/deckpilot-plugin-manifest/schema/ and plugins/schemas/ must hold the same \
         plugin-manifest.schema.json"
    );
}
