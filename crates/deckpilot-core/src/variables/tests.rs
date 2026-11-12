use serde_json::json;

use super::doubles::MemoryVariables;
use super::*;
use crate::graph::{NodeId, parse_template, references_in};
use crate::{ExecutionId, Profile, ResourceKey, WorkflowId};

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

fn reference(text: &str) -> VariableRef {
    VariableRef::parse(text).unwrap()
}

fn profile() -> Profile {
    crate::testing::with_projects("Work", &["SHOP", "BLOG"])
}

fn text_kind() -> ParameterKind {
    ParameterKind::Text {
        multiline: false,
        max_length: None,
    }
}

#[test]
fn every_legacy_template_parses_to_the_same_references() {
    for template in [
        "plain text",
        "cd {{ steps.repo.output.value }}!",
        "{{steps.git_status.output.branch}} and {{ steps.vars.output.count }}",
        "{{ steps.a.output.b }}{{ steps.c.output.d }}",
    ] {
        let legacy = references_in(template).unwrap();
        let extended = references_in_expression(template).unwrap();
        assert_eq!(
            extended,
            legacy
                .into_iter()
                .map(VariableRef::Step)
                .collect::<Vec<_>>(),
            "{template}"
        );
        assert_eq!(
            parse_expression(template).unwrap().len(),
            parse_template(template).unwrap().len()
        );
    }
    assert_eq!(
        parse_expression("{{ steps.a.output.b"),
        Err(ExpressionError::Unclosed)
    );
}

#[test]
fn new_roots_parse_and_print_the_same_text() {
    for text in [
        "shared.projects.favorite",
        "shared.team.deploy.target",
        "system.project.current.id",
        "system.project.current.key",
        "system.profile.current.id",
        "system.device.current.id",
        "system.platform",
        "workflow.id",
        "workflow.execution.id",
        "steps.vars.output.count",
    ] {
        let parsed = reference(&format!("  {text} "));
        assert_eq!(parsed.to_string(), text);
        let json = serde_json::to_value(&parsed).unwrap();
        assert_eq!(json, json!(text));
        assert_eq!(serde_json::from_value::<VariableRef>(json).unwrap(), parsed);
    }
    for variable in SystemVariable::ALL {
        assert_eq!(reference(variable.path()), VariableRef::System(variable));
    }
}

#[test]
fn anything_outside_the_grammar_is_rejected() {
    for bad in [
        "shared.favorite",
        "shared.Projects.favorite",
        "shared.system.x",
        "shared.steps.x",
        "shared.a.b.c.d.e",
        "shared.a.b-c",
        "system.project.current",
        "system.secret",
        "workflow.vars.x",
        "env.HOME",
        "steps.x",
        "1 + 1",
        "shared.a.$(rm)",
    ] {
        assert!(VariableRef::parse(bad).is_err(), "{bad}");
    }
    assert!(serde_json::from_value::<VariableRef>(json!("env.HOME")).is_err());
    assert_eq!(
        VariableKey::new("secrets.token"),
        Err(KeyError::Reserved("secrets".into()))
    );
    assert_eq!(key("projects.favorite").namespace(), "projects");
}

#[test]
fn values_and_types_have_stable_shapes() {
    let project = profile().projects[0].id;
    assert_eq!(
        serde_json::to_value(VariableValue::ProjectId(project)).unwrap(),
        json!({"type": "projectId", "value": project.to_string()})
    );
    assert_eq!(
        serde_json::to_value(VariableValue::Json(json!({"a": [1]}))).unwrap(),
        json!({"type": "json", "value": {"a": [1]}})
    );
    assert_eq!(
        serde_json::to_value(VariableType::Resource { kind: None }).unwrap(),
        json!({"type": "resource"})
    );
    assert_eq!(
        serde_json::to_value(VariableType::SecretRef).unwrap(),
        json!({"type": "secretRef"})
    );
    assert_eq!(
        serde_json::to_value(ScopeKey::Profile {
            profile: profile().id
        })
        .unwrap()["kind"],
        json!("profile")
    );
}

#[test]
fn values_keep_their_native_type_and_assignability_matches_conversion() {
    let profile = profile();
    let values = [
        VariableValue::String("a".into()),
        VariableValue::Integer(3),
        VariableValue::Boolean(true),
        VariableValue::ProjectId(profile.projects[0].id),
        VariableValue::ProfileId(profile.id),
        VariableValue::Resource(crate::ResourceRef::Fixed {
            key: ResourceKey::new("shop.repo").unwrap(),
        }),
        VariableValue::Json(json!({"a": 1})),
        VariableValue::SecretRef(crate::CredentialId::new("github.token").unwrap()),
    ];
    let kinds = [
        text_kind(),
        ParameterKind::Integer {
            min: None,
            max: None,
        },
        ParameterKind::Boolean,
        ParameterKind::Choice {
            options: vec!["a".into()],
        },
        ParameterKind::Resource { kind: None },
        ParameterKind::Project,
        ParameterKind::Code {
            language: "json".into(),
            max_length: None,
        },
        ParameterKind::Credential,
    ];
    for value in &values {
        for kind in &kinds {
            assert_eq!(
                value.value_type().assignable_to(kind),
                value.to_parameter(kind).is_some(),
                "{value:?} -> {kind:?}"
            );
        }
    }
    assert_eq!(
        VariableValue::Integer(3).to_parameter(&kinds[1]),
        Some(ParameterValue::Integer(3))
    );
    assert_eq!(
        VariableValue::ProjectId(profile.projects[0].id).to_parameter(&ParameterKind::Project),
        Some(ParameterValue::Project(profile.projects[0].id))
    );
    assert_eq!(
        VariableValue::Integer(3).to_parameter(&text_kind()),
        Some(ParameterValue::Text("3".into()))
    );
    // A secret reference fills a credential parameter with its name and
    // never becomes text.
    let secret = &values[7];
    assert_eq!(secret.render_text(), None);
    assert_eq!(secret.to_parameter(&text_kind()), None);
    assert_eq!(
        secret.to_parameter(&ParameterKind::Credential),
        Some(ParameterValue::Text("github.token".into()))
    );
}

#[test]
fn the_most_specific_applicable_scope_wins() {
    let profile = profile();
    let (shop, blog) = (profile.projects[0].id, profile.projects[1].id);
    let other = Profile::new("Other").id;
    let execution = ExecutionId::new();
    let mut definition = VariableDefinition::new(
        key("deploy.target"),
        VariableType::String,
        &[
            VariableScope::Global,
            VariableScope::Profile,
            VariableScope::Project,
            VariableScope::Execution,
        ],
    );
    definition.default = Some(VariableValue::String("default".into()));
    let text = |s: &str| VariableValue::String(s.into());
    let values = vec![
        (ScopeKey::Global, text("global")),
        (ScopeKey::Profile { profile: other }, text("other profile")),
        (
            ScopeKey::Profile {
                profile: profile.id,
            },
            text("profile"),
        ),
        (
            ScopeKey::Project {
                profile: profile.id,
                project: blog,
            },
            text("blog"),
        ),
        (
            ScopeKey::Project {
                profile: profile.id,
                project: shop,
            },
            text("shop"),
        ),
        (ScopeKey::Execution { execution }, text("run")),
    ];
    let pick_with = |definition: &VariableDefinition,
                     values: &[(ScopeKey, VariableValue)],
                     context: &ResolutionContext| {
        select_value(definition, values.iter().map(|(s, v)| (s, v)), context)
            .map(|(value, _)| value.render_text().unwrap())
    };
    let pick = |values: &[(ScopeKey, VariableValue)], context: &ResolutionContext| {
        pick_with(&definition, values, context)
    };
    let base = ResolutionContext::preview(profile.id);
    let in_shop = base
        .clone()
        .with_project(Some(shop), ProjectSource::Application);
    assert_eq!(pick(&values, &in_shop).as_deref(), Some("shop"));
    assert_eq!(
        pick(&values, &in_shop.clone().for_execution(execution)).as_deref(),
        Some("run")
    );
    assert_eq!(pick(&values, &base).as_deref(), Some("profile"));
    assert_eq!(
        pick(&values, &ResolutionContext::preview(other)).as_deref(),
        Some("other profile")
    );
    assert_eq!(pick(&values[..1], &in_shop).as_deref(), Some("global"));
    assert_eq!(pick(&[], &in_shop).as_deref(), Some("default"));
    // Values at scopes the definition does not allow are ignored.
    let global_only = VariableDefinition {
        scopes: [VariableScope::Global].into(),
        ..definition.clone()
    };
    assert_eq!(
        pick_with(&global_only, &values, &in_shop).as_deref(),
        Some("global")
    );
}

fn favorite(profile: &Profile) -> MemoryVariables {
    let definition = VariableDefinition::new(
        key("projects.favorite"),
        VariableType::ProjectId,
        &[VariableScope::Profile],
    );
    let favorite = profile.projects[1].id;
    MemoryVariables::new(profile.clone())
        .define(definition)
        .set(
            &key("projects.favorite"),
            ScopeKey::Profile {
                profile: profile.id,
            },
            VariableValue::ProjectId(favorite),
        )
}

#[test]
fn previews_resolve_shared_and_system_variables_in_the_given_context() {
    let profile = profile();
    let variables = favorite(&profile);
    let shop = profile.projects[0].id;
    let context =
        ResolutionContext::preview(profile.id).with_project(Some(shop), ProjectSource::Explicit);
    let item = |reference: &str, expected: Option<ParameterKind>| PreviewItem {
        reference: reference.into(),
        expected,
    };
    let status = |reference: &str, expected: Option<ParameterKind>| {
        preview_reference(&variables, &item(reference, expected), &context).status
    };
    assert_eq!(
        status("shared.projects.favorite", Some(ParameterKind::Project)),
        ReferenceStatus::Resolved {
            value: VariableValue::ProjectId(profile.projects[1].id),
            source: ValueSource::Scope {
                scope: ScopeKey::Profile {
                    profile: profile.id
                }
            },
        }
    );
    assert_eq!(
        status("shared.projects.favorite", Some(ParameterKind::Boolean)),
        ReferenceStatus::TypeMismatch {
            found: VariableType::ProjectId
        }
    );
    assert_eq!(
        status("system.project.current.key", None),
        ReferenceStatus::Resolved {
            value: VariableValue::String("SHOP".into()),
            source: ValueSource::System,
        }
    );
    assert_eq!(
        status("shared.projects.missing", None),
        ReferenceStatus::Missing {
            reason: MissingReason::UnknownVariable
        }
    );
    assert!(matches!(
        status("shared.Bad", None),
        ReferenceStatus::Invalid { .. }
    ));
    for runtime_only in ["workflow.execution.id", "steps.git.output.branch"] {
        assert_eq!(
            status(runtime_only, None),
            ReferenceStatus::UnavailableUntilRuntime
        );
    }
    assert_eq!(
        status("system.device.current.id", None),
        ReferenceStatus::Missing {
            reason: MissingReason::NoDevice
        }
    );
}

#[test]
fn a_run_project_is_never_replaced_by_the_active_project() {
    let profile = profile();
    let variables = favorite(&profile);
    let active = profile.projects[0].id;
    let id = reference("system.project.current.id");
    let preview =
        ResolutionContext::preview(profile.id).with_project(Some(active), ProjectSource::Execution);
    assert_eq!(
        variables.resolve(&id, &preview),
        Resolution::UnavailableUntilRuntime
    );
    let run_project = profile.projects[1].id;
    let running = preview
        .with_project(Some(run_project), ProjectSource::Execution)
        .for_execution(ExecutionId::new());
    assert_eq!(
        bind(
            "project_id",
            &ParameterKind::Project,
            &id,
            variables.resolve(&id, &running)
        ),
        Ok(ParameterValue::Project(run_project))
    );
}

#[test]
fn values_never_cross_profiles() {
    let profile = profile();
    let other = crate::testing::with_projects("Home", &["HOME"]);
    let definition = VariableDefinition::new(
        key("projects.favorite"),
        VariableType::ProjectId,
        &[VariableScope::Global],
    );
    // A global value naming another profile's project.
    let variables = MemoryVariables::new(profile.clone())
        .define(definition)
        .set(
            &key("projects.favorite"),
            ScopeKey::Global,
            VariableValue::ProjectId(other.projects[0].id),
        );
    let favorite = reference("shared.projects.favorite");
    let context = ResolutionContext::preview(profile.id);
    assert_eq!(
        variables.resolve(&favorite, &context),
        Resolution::Missing(MissingReason::OtherProfile)
    );
    assert_eq!(
        variables.resolve(&favorite, &ResolutionContext::preview(other.id)),
        Resolution::Missing(MissingReason::OtherProfile)
    );
}

#[test]
fn sensitive_values_are_only_ever_redacted() {
    let profile = profile();
    let mut definition = VariableDefinition::new(
        key("ops.hostname"),
        VariableType::String,
        &[VariableScope::Global],
    );
    definition.sensitive = true;
    let variables = MemoryVariables::new(profile.clone())
        .define(definition)
        .set(
            &key("ops.hostname"),
            ScopeKey::Global,
            VariableValue::String("very-internal.example".into()),
        );
    let item = PreviewItem {
        reference: "shared.ops.hostname".into(),
        expected: Some(text_kind()),
    };
    let result = preview_reference(&variables, &item, &ResolutionContext::preview(profile.id));
    assert_eq!(
        result.status,
        ReferenceStatus::Redacted {
            value_type: VariableType::String,
            source: ValueSource::Scope {
                scope: ScopeKey::Global
            }
        }
    );
    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains("very-internal"), "{json}");
    // At run time the action still receives it.
    let hostname = reference("shared.ops.hostname");
    let running = ResolutionContext::preview(profile.id).for_execution(ExecutionId::new());
    assert_eq!(
        bind(
            "host",
            &text_kind(),
            &hostname,
            variables.resolve(&hostname, &running)
        ),
        Ok(ParameterValue::Text("very-internal.example".into()))
    );
}

#[test]
fn errors_name_the_parameter_and_reference_but_no_value() {
    let profile = profile();
    let mut definition = VariableDefinition::new(
        key("ops.token_name"),
        VariableType::String,
        &[VariableScope::Global],
    );
    definition.default = Some(VariableValue::String("s3cr3t".into()));
    let variables = MemoryVariables::new(profile.clone())
        .define(definition)
        .deny(key("ops.denied"));
    let running = ResolutionContext::preview(profile.id).for_execution(ExecutionId::new());
    let token = reference("shared.ops.token_name");
    let error = bind(
        "count",
        &ParameterKind::Integer {
            min: None,
            max: None,
        },
        &token,
        variables.resolve(&token, &running),
    )
    .unwrap_err();
    assert_eq!(error.code(), "reference-type");
    let message = error.to_string();
    assert!(message.contains("`count`") && message.contains("shared.ops.token_name"));
    assert!(!message.contains("s3cr3t"), "{message}");
    assert_eq!(
        variables.resolve(&reference("shared.ops.denied"), &running),
        Resolution::PermissionDenied
    );
    let missing = reference("system.project.current.id");
    let error = bind(
        "project_id",
        &ParameterKind::Project,
        &missing,
        variables.resolve(&missing, &running),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "parameter `project_id`: {{ system.project.current.id }} has no value (no project is active)"
    );
}

#[test]
fn values_that_look_like_references_stay_text() {
    let profile = profile();
    let definition =
        VariableDefinition::new(key("a.b"), VariableType::String, &[VariableScope::Global]);
    let variables = MemoryVariables::new(profile.clone())
        .define(definition)
        .set(
            &key("a.b"),
            ScopeKey::Global,
            VariableValue::String("{{ shared.a.b }}".into()),
        );
    let context = ResolutionContext::preview(profile.id);
    assert_eq!(
        preview(variables.resolve(&reference("shared.a.b"), &context), None),
        ReferenceStatus::Resolved {
            value: VariableValue::String("{{ shared.a.b }}".into()),
            source: ValueSource::Scope {
                scope: ScopeKey::Global
            }
        }
    );
}

#[test]
fn runtime_values_exist_while_running() {
    let profile = profile();
    let variables =
        MemoryVariables::new(profile.clone()).output(NodeId::new("git").unwrap(), "branch", "main");
    let execution = ExecutionId::new();
    let workflow = WorkflowId::new();
    let running = ResolutionContext::preview(profile.id)
        .with_workflow(workflow)
        .for_execution(execution);
    let value = |text: &str| match variables.resolve(&reference(text), &running) {
        Resolution::Value { value, .. } => value.render_text(),
        _ => None,
    };
    assert_eq!(value("workflow.execution.id"), Some(execution.to_string()));
    assert_eq!(value("workflow.id"), Some(workflow.to_string()));
    assert_eq!(value("steps.git.output.branch").as_deref(), Some("main"));
    assert_eq!(
        variables.resolve(&reference("steps.git.output.other"), &running),
        Resolution::Missing(MissingReason::NoOutput)
    );
}

#[test]
fn definitions_check_their_values() {
    let mut definition =
        VariableDefinition::new(key("a.b"), VariableType::Integer, &[VariableScope::Profile]);
    definition.default = Some(VariableValue::String("x".into()));
    assert_eq!(definition.issues().len(), 1);
    assert!(
        definition
            .check_value(&ScopeKey::Global, &VariableValue::Integer(1))
            .is_err()
    );
    let profile = Profile::new("Work").id;
    assert!(
        definition
            .check_value(&ScopeKey::Profile { profile }, &VariableValue::Integer(1))
            .is_ok()
    );
    definition.scopes.clear();
    assert!(matches!(
        definition.issues()[0],
        DefinitionIssue::NoScopes(_)
    ));
}

#[test]
fn contexts_from_the_app_need_only_a_profile() {
    let profile = Profile::new("Work").id;
    let context: ResolutionContext =
        serde_json::from_value(json!({ "profile": profile.to_string() })).unwrap();
    assert_eq!(context, ResolutionContext::preview(profile));
}

fn entry(reference: &str, group: VariableGroup, value_type: VariableType) -> CatalogEntry {
    CatalogEntry {
        reference: VariableRef::parse(reference).unwrap(),
        label: "Favorite project".into(),
        description: "Opened by the morning workflow".into(),
        group,
        value_type,
        scope: Some(VariableScope::Profile),
        module: None,
        sensitive: false,
        dynamic: false,
        availability: ReferenceAvailability::Available,
    }
}

#[test]
fn the_catalog_filters_by_words_group_type_and_availability() {
    let favorite = entry(
        "shared.projects.favorite",
        VariableGroup::Profile,
        VariableType::ProjectId,
    );
    let filter = |query: &str| CatalogFilter {
        query: query.into(),
        ..CatalogFilter::default()
    };
    assert!(filter("").matches(&favorite));
    assert!(filter("FAVORITE morning").matches(&favorite));
    assert!(filter("projectid profile").matches(&favorite));
    assert!(!filter("favorite evening").matches(&favorite));
    let typed = CatalogFilter {
        compatible_with: Some(ParameterKind::Boolean),
        ..CatalogFilter::default()
    };
    assert!(!typed.matches(&favorite));
    let grouped = CatalogFilter {
        groups: vec![VariableGroup::System],
        ..CatalogFilter::default()
    };
    assert!(!grouped.matches(&favorite));
    let unavailable = CatalogEntry {
        availability: ReferenceAvailability::NotInContext,
        ..favorite.clone()
    };
    assert!(!filter("").matches(&unavailable));
    assert!(
        CatalogFilter {
            include_unavailable: true,
            ..CatalogFilter::default()
        }
        .matches(&unavailable)
    );
    let json = serde_json::to_value(&favorite).unwrap();
    assert_eq!(json["reference"], json!("shared.projects.favorite"));
    assert_eq!(json["valueType"], json!({"type": "projectId"}));
    assert!(json.get("value").is_none());
    assert_eq!(
        serde_json::from_value::<CatalogEntry>(json).unwrap(),
        favorite
    );
}

/// The app's mirror of these types; every serialized name must be there.
const FRONTEND_TYPES: &str = include_str!("../../../../apps/desktop/src/lib/types.ts");

#[test]
fn the_frontend_mirrors_every_variant() {
    let names = [
        VariableType::String,
        VariableType::Integer,
        VariableType::Boolean,
        VariableType::ProjectId,
        VariableType::ProfileId,
        VariableType::Resource { kind: None },
        VariableType::Json,
        VariableType::SecretRef,
    ]
    .map(|t| t.name().to_owned())
    .into_iter()
    .chain(
        VariableScope::PRECEDENCE
            .into_iter()
            .map(|s| s.name().to_owned()),
    )
    .chain(
        [
            MissingReason::UnknownVariable,
            MissingReason::NoValue,
            MissingReason::NoProject,
            MissingReason::NoDevice,
            MissingReason::NoWorkflow,
            MissingReason::NoExecution,
            MissingReason::NoOutput,
            MissingReason::OtherProfile,
        ]
        .map(|r| {
            serde_json::to_value(r)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        }),
    )
    .chain(
        [
            VariableGroup::Shared,
            VariableGroup::System,
            VariableGroup::Profile,
            VariableGroup::Project,
            VariableGroup::Workflow,
            VariableGroup::StepOutput,
        ]
        .map(|g| {
            serde_json::to_value(g)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        }),
    )
    .chain(
        [
            ReferenceStatus::UnavailableUntilRuntime,
            ReferenceStatus::PermissionDenied,
            ReferenceStatus::Missing {
                reason: MissingReason::NoValue,
            },
            ReferenceStatus::TypeMismatch {
                found: VariableType::Json,
            },
            ReferenceStatus::Invalid {
                message: String::new(),
            },
            ReferenceStatus::Resolved {
                value: VariableValue::Boolean(true),
                source: ValueSource::System,
            },
            ReferenceStatus::Redacted {
                value_type: VariableType::Json,
                source: ValueSource::Default,
            },
        ]
        .map(|s| {
            serde_json::to_value(s).unwrap()["status"]
                .as_str()
                .unwrap()
                .to_owned()
        }),
    );
    for name in names {
        assert!(
            FRONTEND_TYPES.contains(&format!("\"{name}\"")),
            "types.ts lacks \"{name}\""
        );
    }
}
