use std::collections::BTreeMap;
use std::sync::Arc;

use deckpilot_device::{DeviceId, DeviceModelId};

use super::*;
use crate::testing::with_projects;
use crate::variables::doubles::{FixedProjects, MemoryVariables};
use crate::variables::{
    ReferenceProblem, ScopeKey, VariableDefinition, VariableKey, VariableScope, VariableType,
};
use crate::{
    ActionHandler, ActionInstance, ActionInstanceId, ActionRegistry, ActiveProjectContext,
    CancellationToken, Capability, Engine, ExecutionContext, MachineConfig, ModuleId, OnFailure,
    ParameterSpec, Progress, Workflow,
};

/// The core's variables and stand-ins for the projects and work session
/// modules.
static REGISTRY: std::sync::LazyLock<crate::variables::VariableRegistry> =
    std::sync::LazyLock::new(crate::variables::doubles::builtin_registry);

/// Reports what it received: every parameter in its data, the message as
/// its message and progress, and what its context tells it.
struct Probe(ActionType);

fn probe_type() -> crate::ActionTypeId {
    crate::ActionTypeId::new("test.probe").unwrap()
}

fn spec(key: &str, kind: ParameterKind) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: key.into(),
        description: None,
        kind,
        required: false,
        default: None,
    }
}

impl Probe {
    fn new() -> Self {
        Self(ActionType {
            id: probe_type(),
            name: "Probe".into(),
            description: String::new(),
            category: "Test".into(),
            parameters: vec![
                spec(
                    "message",
                    ParameterKind::Text {
                        multiline: false,
                        max_length: Some(100),
                    },
                ),
                spec(
                    "count",
                    ParameterKind::Integer {
                        min: Some(1),
                        max: Some(10),
                    },
                ),
                spec("project", ParameterKind::Project),
                spec(
                    "mode",
                    ParameterKind::Choice {
                        options: vec!["a".into(), "b".into()],
                    },
                ),
            ],
            requires_project: false,
            deprecated: false,
            node: crate::graph::NodeSpec::default(),
        })
    }
}

impl ActionHandler for Probe {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        assert!(parameters.is_resolved(), "handlers only see literals");
        let message = parameters.text("message").unwrap_or_default().to_owned();
        context.report_progress(None, message.clone());
        let mut result = ActionResult::success(message.clone()).with_data("message", message);
        if let Some(count) = parameters.integer("count") {
            result = result.with_data("count", count.to_string());
        }
        if let Some(project) = parameters.project("project") {
            result = result.with_data("project", project.to_string());
        }
        if let Some(mode) = parameters.text("mode") {
            result = result.with_data("mode", mode);
        }
        if let Some(device) = context.device() {
            result = result.with_data("device", device.to_string());
        }
        if let Ok(Some(project)) = context.application_project() {
            result = result.with_data("applicationProject", project.to_string());
        }
        if let Ok(Some(project)) = context.project() {
            result = result.with_data("runProject", project.key.to_string());
        }
        let name = VariableRef::parse("shared.team.name").unwrap();
        let variable = match context.variable(&name) {
            Err(_) => "denied".to_owned(),
            Ok(Resolution::Value {
                value: VariableValue::String(text),
                ..
            }) => text,
            Ok(other) => format!("{other:?}"),
        };
        result
            .with_data("variable", variable)
            .with_data("cancelled", context.is_cancelled().to_string())
    }
}

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

fn reference(text: &str) -> ParameterValue {
    ParameterValue::Reference(VariableRef::parse(text).unwrap())
}

fn template(text: &str) -> ParameterValue {
    ParameterValue::Template(text.into())
}

struct Fixture {
    profile: Profile,
    machine: MachineConfig,
}

impl Fixture {
    fn new() -> Self {
        Self {
            profile: with_projects("Work", &["SHOP", "CMR"]),
            machine: MachineConfig::new(Os::Linux),
        }
    }

    fn variables(&self) -> MemoryVariables {
        let profile = self.profile.id;
        let shop = self.profile.projects[0].id;
        MemoryVariables::new(self.profile.clone())
            .define(VariableDefinition::new(
                key("team.name"),
                VariableType::String,
                &[
                    VariableScope::Global,
                    VariableScope::Profile,
                    VariableScope::Project,
                ],
            ))
            .set(
                &key("team.name"),
                ScopeKey::Global,
                VariableValue::String("global".into()),
            )
            .set(
                &key("team.name"),
                ScopeKey::Profile { profile },
                VariableValue::String("profile".into()),
            )
            .set(
                &key("team.name"),
                ScopeKey::Project {
                    profile,
                    project: shop,
                },
                VariableValue::String("shop team".into()),
            )
            .define(VariableDefinition::new(
                key("team.size"),
                VariableType::Integer,
                &[VariableScope::Global],
            ))
            .set(
                &key("team.size"),
                ScopeKey::Global,
                VariableValue::Integer(4),
            )
            .define(VariableDefinition::new(
                key("projects.favorite"),
                VariableType::ProjectId,
                &[VariableScope::Profile],
            ))
            .set(
                &key("projects.favorite"),
                ScopeKey::Profile { profile },
                VariableValue::ProjectId(self.profile.projects[1].id),
            )
            .define(VariableDefinition {
                sensitive: true,
                ..VariableDefinition::new(
                    key("vault.pin"),
                    VariableType::String,
                    &[VariableScope::Global],
                )
            })
            .set(
                &key("vault.pin"),
                ScopeKey::Global,
                VariableValue::String("s3cr3t".into()),
            )
            .define(VariableDefinition::new(
                key("loop.self"),
                VariableType::String,
                &[VariableScope::Global],
            ))
            .set(
                &key("loop.self"),
                ScopeKey::Global,
                VariableValue::String("{{ shared.loop.self }}".into()),
            )
            .deny(key("team.secret"))
    }

    fn engine(&self, capabilities: Option<&[Capability]>) -> Engine {
        let mut registry = ActionRegistry::default();
        match capabilities {
            Some(capabilities) => registry
                .register_for_module(
                    ModuleId::new("test").unwrap(),
                    capabilities,
                    Box::new(Probe::new()),
                )
                .unwrap(),
            None => registry.register(Box::new(Probe::new())).unwrap(),
        }
        let active = FixedProjects(BTreeMap::from([(
            self.profile.id,
            self.profile.projects[0].id,
        )]));
        Engine::new(registry)
            .with_variables(Arc::new(self.variables()))
            .with_projects(Arc::new(active))
            .with_variable_registry(Arc::new(crate::variables::doubles::builtin_registry()))
    }

    fn add(&mut self, parameters: ActionParameters) -> ActionInstanceId {
        let instance = ActionInstance {
            id: ActionInstanceId::new(),
            name: "Probe".into(),
            action_type: probe_type(),
            parameters,
            description: String::new(),
            icon: None,
            label: None,
        };
        let id = instance.id;
        self.profile.actions.push(instance);
        id
    }

    /// Runs a new instance with `parameters` from a key, with `project`
    /// (an index) active, or none.
    fn run(&mut self, parameters: ActionParameters, project: Option<usize>) -> ActionResult {
        let id = self.add(parameters);
        let engine = self.engine(None);
        // A profile with projects always has one active; a context without
        // one is what a profile without projects has.
        let mut context: ActiveProjectContext = serde_json::from_value(serde_json::json!({
            "profileId": self.profile.id,
            "projectId": project.map(|index| self.profile.projects[index].id),
        }))
        .unwrap();
        engine
            .run_action(&self.profile, &self.machine, &mut context, id)
            .unwrap()
    }
}

fn message(value: ParameterValue) -> ActionParameters {
    ActionParameters::new().with("message", value)
}

#[test]
fn literal_parameters_run_exactly_as_before() {
    let mut f = Fixture::new();
    let result = f.run(
        message(ParameterValue::Text("{{ shared.team.name }}".into())),
        Some(0),
    );
    assert!(result.is_success());
    assert_eq!(result.message, "{{ shared.team.name }}");
}

#[test]
fn the_most_specific_scope_wins() {
    let mut f = Fixture::new();
    let shared = || message(reference("shared.team.name"));
    assert_eq!(f.run(shared(), Some(0)).message, "shop team");
    // Another project has no project value: the profile's applies.
    assert_eq!(f.run(shared(), Some(1)).message, "profile");
    assert_eq!(f.run(shared(), None).message, "profile");
}

#[test]
fn whole_value_references_keep_their_type() {
    let mut f = Fixture::new();
    let result = f.run(
        ActionParameters::new()
            .with("count", reference("shared.team.size"))
            .with("project", reference("shared.projects.favorite")),
        None,
    );
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.data["count"], "4");
    assert_eq!(result.data["project"], f.profile.projects[1].id.to_string());
    let result = f.run(
        ActionParameters::new().with("project", reference("system.project.current.id")),
        Some(0),
    );
    assert_eq!(result.data["project"], f.profile.projects[0].id.to_string());
}

#[test]
fn legacy_ids_and_canonical_ids_resolve_alike() {
    let mut f = Fixture::new();
    for (legacy, canonical) in [
        ("system.project.current.id", "projects.current.id"),
        ("system.project.current.key", "projects.current.key"),
        ("system.project.current.name", "projects.current.name"),
    ] {
        let old = f.run(message(template(&format!("{{{{ {legacy} }}}}"))), Some(1));
        let new = f.run(
            message(template(&format!("{{{{ {canonical} }}}}"))),
            Some(1),
        );
        assert!(old.is_success(), "{}", old.message);
        assert_eq!(old.message, new.message, "{legacy}");
    }
    // Whole-value references keep the native type for both spellings.
    for text in ["system.project.current.id", "projects.current.id"] {
        let result = f.run(
            ActionParameters::new().with("project", reference(text)),
            Some(0),
        );
        assert_eq!(result.data["project"], f.profile.projects[0].id.to_string());
    }
    // The stored spelling is kept: nothing is rewritten when saved again.
    let stored = serde_json::to_value(reference("system.project.current.key")).unwrap();
    assert_eq!(stored["value"], "system.project.current.key");
    let VariableRef::System(alias) = VariableRef::parse("system.project.current.key").unwrap()
    else {
        panic!("a legacy id");
    };
    assert_eq!(alias.canonical().as_str(), "projects.current.key");
}

#[test]
fn legacy_ids_keep_working_while_their_module_is_disabled() {
    let f = Fixture::new();
    let projects = ModuleId::new("projects").unwrap();
    let mut disabled = crate::variables::doubles::builtin_registry();
    disabled.set_available(&projects, Some("the Projects module is disabled".into()));
    let missing = crate::variables::VariableRegistry::new();
    let context = Invocation::default().context(&f.profile, Some(f.profile.projects[1].id));
    for registry in [&disabled, &missing] {
        let resolver = ContextResolver {
            profile: &f.profile,
            os: Os::Linux,
            shared: &NoSharedVariables,
            outputs: None,
            registry,
            live: &crate::variables::NoLiveState,
        };
        let resolve = |text: &str| resolver.resolve(&VariableRef::parse(text).unwrap(), &context);
        // As on main: the core resolves the project in the context.
        for (variable, value) in [
            (
                SystemVariable::ProjectCurrentKey,
                "system.project.current.key",
            ),
            (
                SystemVariable::ProjectCurrentName,
                "system.project.current.name",
            ),
            (
                SystemVariable::ProjectCurrentId,
                "system.project.current.id",
            ),
        ] {
            assert_eq!(
                resolve(value),
                system_value(variable, &f.profile, Os::Linux, &context),
                "{value}"
            );
        }
        assert!(matches!(
            resolve("system.project.current.key"),
            Resolution::Value { value: VariableValue::String(key), .. } if key == "CMR"
        ));
        // Without a session, as without the agent on main.
        assert_eq!(
            resolve("system.work.session.state"),
            Resolution::Missing(MissingReason::NoValue)
        );
        // The canonical id follows its module.
        assert_eq!(
            resolve("projects.current.key"),
            Resolution::Unavailable {
                owner: "projects".into()
            }
        );
    }
}

#[test]
fn variables_of_an_unavailable_module_fail_without_losing_the_reference() {
    let mut f = Fixture::new();
    let id = f.add(message(template("on {{ projects.current.key }}")));
    // An engine whose registry lacks the projects module, as when it is
    // disabled: the reference is reported, the configuration is kept.
    let engine = Engine::new({
        let mut registry = ActionRegistry::default();
        registry.register(Box::new(Probe::new())).unwrap();
        registry
    });
    let mut projects = ActiveProjectContext::new(&f.profile);
    let result = engine
        .run_action(&f.profile, &f.machine, &mut projects, id)
        .unwrap();
    assert_eq!(result.code.as_deref(), Some("reference-unavailable"));
    assert_eq!(
        result.message,
        "parameter `message`: {{ projects.current.key }} is unavailable: the `projects` module is disabled or not installed"
    );
    assert_eq!(
        f.profile.action(id).unwrap().parameters.get("message"),
        Some(&template("on {{ projects.current.key }}"))
    );
}

#[test]
fn templates_render_every_reference_as_text() {
    let mut f = Fixture::new();
    let result = f.run(
        message(template(
            "{{ system.profile.current.name }}/{{ system.project.current.key }} x{{ shared.team.size }} on {{ system.platform }}",
        )),
        Some(1),
    );
    assert_eq!(result.message, "Work/CMR x4 on linux");
}

#[test]
fn missing_references_name_the_parameter_and_the_reference() {
    let mut f = Fixture::new();
    let result = f.run(message(reference("shared.team.unknown")), None);
    assert!(!result.is_success());
    assert_eq!(result.code.as_deref(), Some("reference-missing"));
    assert_eq!(
        result.message,
        "parameter `message`: {{ shared.team.unknown }} has no value (no such variable)"
    );
    let result = f.run(
        ActionParameters::new().with("project", reference("system.project.current.id")),
        None,
    );
    assert_eq!(
        result.message,
        "parameter `project`: {{ system.project.current.id }} has no value (no project is active)"
    );
    let result = f.run(message(template("on {{ system.device.current.id }}")), None);
    assert!(result.message.contains("no device is in the context"));
    // Every problem is reported.
    let result = f.run(
        message(template("{{ shared.a.b }} {{ workflow.id }}")),
        None,
    );
    assert!(result.message.contains("{{ shared.a.b }}"));
    assert!(result.message.contains("{{ workflow.id }}"));
}

#[test]
fn type_mismatches_and_constraints_fail_before_the_handler() {
    let mut f = Fixture::new();
    let result = f.run(
        ActionParameters::new().with("project", reference("shared.team.size")),
        None,
    );
    assert_eq!(result.code.as_deref(), Some("reference-type"));
    assert_eq!(
        result.message,
        "parameter `project`: {{ shared.team.size }} is a integer value, the parameter expects a project"
    );
    // A resolved value is checked like a literal: out of range here.
    let result = f.run(
        ActionParameters::new().with("mode", reference("shared.team.name")),
        Some(0),
    );
    assert!(result.message.starts_with("invalid configuration:"));
    assert!(result.message.contains("`shop team` is not one of a, b"));
}

#[test]
fn permission_denied_variables_are_reported_without_values() {
    let mut f = Fixture::new();
    let result = f.run(message(reference("shared.team.secret")), None);
    assert_eq!(result.code.as_deref(), Some("reference-denied"));
    assert_eq!(
        result.message,
        "parameter `message`: {{ shared.team.secret }} may not be read here"
    );
}

#[test]
fn sensitive_values_are_redacted_from_results_and_progress() {
    let mut f = Fixture::new();
    let id = f.add(message(template("pin {{ shared.vault.pin }}")));
    let engine = f.engine(None);
    let mut context = ActiveProjectContext::new(&f.profile);
    let token = CancellationToken::new();
    let seen = std::sync::Mutex::new(Vec::new());
    let progress = |progress: Progress| seen.lock().unwrap().push(progress.message);
    let control = crate::ExecutionControl::new(&token, &progress);
    let instance = f.profile.action(id).unwrap().clone();
    let result = engine
        .execute(
            &f.profile,
            &f.machine,
            &mut context,
            &instance,
            None,
            Some(&control),
            &Invocation::default(),
        )
        .unwrap();
    assert!(result.is_success());
    assert_eq!(result.message, "pin [redacted]");
    assert_eq!(result.data["message"], "pin [redacted]");
    assert_eq!(*seen.lock().unwrap(), ["pin [redacted]"]);
    let debug = format!("{result:?}");
    assert!(!debug.contains("s3cr3t"));

    // A value that fails validation is not shown either.
    let result = f.run(
        ActionParameters::new().with("mode", reference("shared.vault.pin")),
        None,
    );
    assert!(!result.is_success());
    assert!(!result.message.contains("s3cr3t"), "{}", result.message);
    assert!(result.message.contains("[redacted]"));
}

/// `projects.token`, a sensitive module variable.
struct Vault;

impl crate::variables::VariableSource for Vault {
    fn resolve(
        &self,
        _: &crate::variables::VariableDeclaration,
        _: &crate::variables::VariableEnvironment<'_>,
    ) -> Resolution {
        // The source forgets to say it is sensitive; the declaration does.
        Resolution::Value {
            value: VariableValue::String("t0k3n".into()),
            source: ValueSource::System,
            sensitive: false,
        }
    }
}

#[test]
fn sensitive_module_variables_are_redacted_like_shared_ones() {
    use crate::variables::{DeclaredScope, PreviewItem, VariableDeclaration, preview_reference};
    let f = Fixture::new();
    let mut registry = crate::variables::VariableRegistry::new();
    registry
        .register_module(
            &ModuleId::new("projects").unwrap(),
            &[VariableDeclaration {
                sensitive: true,
                ..VariableDeclaration::new(
                    "projects.token",
                    "Token",
                    VariableType::String,
                    DeclaredScope::Machine,
                )
            }],
            Some(Arc::new(Vault)),
        )
        .unwrap();
    let resolver = ContextResolver {
        profile: &f.profile,
        os: Os::Linux,
        shared: &NoSharedVariables,
        outputs: None,
        registry: &registry,
        live: &crate::variables::NoLiveState,
    };
    let context = Invocation::default().context(&f.profile, None);
    let parameters = resolve_parameters(
        Probe::new().definition(),
        &message(template("use {{ projects.token }}")),
        &resolver,
        &context,
        &f.profile,
    )
    .unwrap();
    assert_eq!(
        parameters.redactor.redact("got t0k3n back"),
        "got [redacted] back"
    );
    let preview = preview_reference(
        &resolver,
        &PreviewItem {
            reference: "projects.token".into(),
            expected: None,
        },
        &ResolutionContext::preview(f.profile.id),
    );
    assert!(!format!("{preview:?}").contains("t0k3n"));
    assert!(matches!(
        preview.status,
        crate::variables::ReferenceStatus::Redacted { .. }
    ));
}

#[test]
fn resolved_values_are_never_expanded_again() {
    // A value that looks like a reference to itself is text: no cycle.
    let mut f = Fixture::new();
    let result = f.run(message(reference("shared.loop.self")), None);
    assert_eq!(result.message, "{{ shared.loop.self }}");
    let result = f.run(message(template("<{{ shared.loop.self }}>")), None);
    assert_eq!(result.message, "<{{ shared.loop.self }}>");
}

/// Hands out a project of another profile, as a broken store might.
struct Foreign(ProjectId);

impl VariableResolver for Foreign {
    fn resolve(&self, _: &VariableRef, _: &ResolutionContext) -> Resolution {
        Resolution::Value {
            value: VariableValue::ProjectId(self.0),
            source: ValueSource::Default,
            sensitive: false,
        }
    }
}

#[test]
fn values_never_cross_profiles() {
    let f = Fixture::new();
    let other = with_projects("Home", &["GARDEN"]);
    let foreign = Foreign(other.projects[0].id);
    let resolver = ContextResolver {
        profile: &f.profile,
        os: Os::Linux,
        shared: &foreign,
        outputs: None,
        registry: &REGISTRY,
        live: &crate::variables::NoLiveState,
    };
    let context = Invocation::default().context(&f.profile, None);
    let failure = resolve_parameters(
        Probe::new().definition(),
        &ActionParameters::new().with("project", reference("shared.projects.favorite")),
        &resolver,
        &context,
        &f.profile,
    )
    .unwrap_err();
    let ResolutionFailure::References(errors) = failure else {
        panic!("a reference error");
    };
    assert_eq!(
        errors[0].problem,
        ReferenceProblem::Missing(MissingReason::OtherProfile)
    );
    // A context of another profile resolves nothing of this one.
    let elsewhere = Invocation::default().context(&other, None);
    assert_eq!(
        resolver.resolve(
            &VariableRef::parse("system.profile.current.name").unwrap(),
            &elsewhere
        ),
        Resolution::Missing(MissingReason::OtherProfile)
    );
}

#[test]
fn a_runs_project_is_never_replaced_by_the_active_one() {
    let mut f = Fixture::new();
    let id = f.add(message(template(
        "{{ system.project.current.key }} {{ shared.team.name }}",
    )));
    let engine = f.engine(None);
    let instance = f.profile.action(id).unwrap().clone();
    // The application shows SHOP (FixedProjects); the run is on CMR.
    let mut run = ActiveProjectContext::restore(&f.profile, Some(f.profile.projects[1].id));
    let workflow = crate::WorkflowId::new();
    let result = engine
        .execute(
            &f.profile,
            &f.machine,
            &mut run,
            &instance,
            None,
            None,
            &Invocation::default().with_workflow(workflow),
        )
        .unwrap();
    assert_eq!(result.message, "CMR profile");
    assert_eq!(result.data["runProject"], "CMR");
    assert_eq!(
        result.data["applicationProject"],
        f.profile.projects[0].id.to_string()
    );
    let context = Invocation::default()
        .with_workflow(workflow)
        .context(&f.profile, None);
    assert_eq!(context.project_source, ProjectSource::Execution);
}

#[test]
fn node_outputs_are_injected_and_converted() {
    let f = Fixture::new();
    let node = NodeId::new("status").unwrap();
    let outputs: BTreeMap<NodeId, BTreeMap<String, String>> = BTreeMap::from([(
        node,
        BTreeMap::from([
            ("count".to_owned(), " 3 ".to_owned()),
            ("project".to_owned(), "CMR".to_owned()),
            ("branch".to_owned(), "main".to_owned()),
        ]),
    )]);
    let variables = f.variables();
    let resolver = ContextResolver {
        profile: &f.profile,
        os: Os::Linux,
        shared: &variables,
        outputs: Some(&outputs),
        registry: &REGISTRY,
        live: &crate::variables::NoLiveState,
    };
    let context = Invocation::default()
        .with_workflow(crate::WorkflowId::new())
        .context(&f.profile, None);
    let resolve = |parameters: ActionParameters| {
        resolve_parameters(
            Probe::new().definition(),
            &parameters,
            &resolver,
            &context,
            &f.profile,
        )
    };
    let found = resolve(
        ActionParameters::new()
            .with("count", reference("steps.status.output.count"))
            .with("project", reference("steps.status.output.project"))
            .with("message", template("on {{ steps.status.output.branch }}")),
    )
    .unwrap();
    assert_eq!(found.parameters.integer("count"), Some(3));
    assert_eq!(
        found.parameters.project("project"),
        Some(f.profile.projects[1].id)
    );
    assert_eq!(found.parameters.text("message"), Some("on main"));

    let missing = resolve(message(reference("steps.status.output.nope"))).unwrap_err();
    assert!(
        missing
            .into_result()
            .message
            .contains("the node has not produced it")
    );
    let wrong =
        resolve(ActionParameters::new().with("count", reference("steps.status.output.branch")))
            .unwrap_err();
    assert_eq!(wrong.into_result().code.as_deref(), Some("reference-type"));

    // Outside a run there are no outputs.
    let resolver = ContextResolver {
        outputs: None,
        registry: &REGISTRY,
        live: &crate::variables::NoLiveState,
        ..resolver
    };
    let failure = resolve_parameters(
        Probe::new().definition(),
        &message(reference("steps.status.output.branch")),
        &resolver,
        &Invocation::default().context(&f.profile, None),
        &f.profile,
    )
    .unwrap_err();
    assert_eq!(
        failure.into_result().code.as_deref(),
        Some("reference-unavailable")
    );
}

#[test]
fn runs_know_their_workflow_execution_and_device() {
    let mut f = Fixture::new();
    let id = f.add(message(template(
        "{{ workflow.execution.id }}|{{ workflow.id }}",
    )));
    let workflow = Workflow::new("Report").step(id, OnFailure::Stop);
    let workflow_id = workflow.id;
    f.profile.workflows.push(workflow);
    let engine = f.engine(None);
    let execution = crate::ExecutionId::new();
    let device = DeviceId {
        model: DeviceModelId::new("elgato.streamdeck-mk2"),
        serial: "A1".into(),
    };
    let mut context = ActiveProjectContext::new(&f.profile);
    let report = engine
        .execute_workflow_as(
            &f.profile,
            &f.machine,
            &mut context,
            workflow_id,
            &BTreeMap::new(),
            &Invocation::from_device(device.clone()).with_execution(Some(execution)),
            &CancellationToken::new(),
            &|_| {},
        )
        .unwrap();
    let node = &report.nodes[0];
    assert_eq!(node.message, format!("{execution}|{workflow_id}"));
    assert_eq!(node.output["device"], device.to_string());

    // A key press knows its device but is no run.
    let pressed = f.add(message(template("{{ system.device.current.id }}")));
    let result = engine
        .run_action_from(
            &f.profile,
            &f.machine,
            &mut context,
            pressed,
            &Invocation::from_device(device.clone()),
        )
        .unwrap();
    assert_eq!(result.message, device.to_string());
    let result = f.run(message(template("{{ workflow.execution.id }}")), None);
    assert!(result.message.contains("no run is in the context"));
}

#[test]
fn modules_read_variables_only_with_the_capability() {
    let mut f = Fixture::new();
    let id = f.add(ActionParameters::new());
    let mut context = ActiveProjectContext::restore(&f.profile, Some(f.profile.projects[0].id));
    let run = |engine: &Engine, context: &mut ActiveProjectContext| {
        engine
            .run_action(&f.profile, &f.machine, context, id)
            .unwrap()
    };
    let blind = run(&f.engine(Some(&[Capability::ProfileRead])), &mut context);
    assert_eq!(blind.data["variable"], "denied");
    let reader = run(
        &f.engine(Some(&[Capability::ProfileRead, Capability::VariablesRead])),
        &mut context,
    );
    assert_eq!(reader.data["variable"], "shop team");
    // Without injected services every variable is unavailable.
    let profile = f.profile.clone();
    let machine = MachineConfig::new(Os::Linux);
    let plain = ExecutionContext::new(&profile, None, &machine);
    assert_eq!(
        plain
            .variable(&VariableRef::parse("shared.team.name").unwrap())
            .unwrap(),
        Resolution::UnavailableUntilRuntime
    );
    assert_eq!(plain.device(), None);
}

#[test]
fn cancellation_reaches_the_handler_through_redaction() {
    let mut f = Fixture::new();
    let id = f.add(message(template("{{ shared.vault.pin }}")));
    let engine = f.engine(None);
    let instance = f.profile.action(id).unwrap().clone();
    let token = CancellationToken::new();
    token.cancel();
    let ignore = |_: Progress| {};
    let control = crate::ExecutionControl::new(&token, &ignore);
    let mut context = ActiveProjectContext::new(&f.profile);
    let result = engine
        .execute(
            &f.profile,
            &f.machine,
            &mut context,
            &instance,
            None,
            Some(&control),
            &Invocation::default(),
        )
        .unwrap();
    assert_eq!(result.data["cancelled"], "true");
}

#[test]
fn defaults_that_are_references_are_resolved() {
    let f = Fixture::new();
    let mut definition = Probe::new().definition().clone();
    definition.parameters[0].default = Some(template("hi {{ system.profile.current.name }}"));
    let variables = f.variables();
    let resolver = ContextResolver {
        profile: &f.profile,
        os: Os::Linux,
        shared: &variables,
        outputs: None,
        registry: &REGISTRY,
        live: &crate::variables::NoLiveState,
    };
    let found = resolve_parameters(
        &definition,
        &ActionParameters::new(),
        &resolver,
        &Invocation::default().context(&f.profile, None),
        &f.profile,
    )
    .unwrap();
    assert_eq!(found.parameters.text("message"), Some("hi Work"));
}

#[test]
fn previews_of_a_run_do_not_use_the_active_project() {
    let f = Fixture::new();
    let context = ResolutionContext::preview(f.profile.id)
        .with_project(Some(f.profile.projects[0].id), ProjectSource::Execution);
    let reference = VariableRef::parse("system.project.current.key").unwrap();
    assert_eq!(
        system_value(
            SystemVariable::ProjectCurrentKey,
            &f.profile,
            Os::Linux,
            &context
        ),
        Resolution::UnavailableUntilRuntime
    );
    let explicit = context.with_project(Some(f.profile.projects[0].id), ProjectSource::Explicit);
    let variables = f.variables();
    let resolver = ContextResolver {
        profile: &f.profile,
        os: Os::MacOs,
        shared: &variables,
        outputs: None,
        registry: &REGISTRY,
        live: &crate::variables::NoLiveState,
    };
    assert!(matches!(
        resolver.resolve(&reference, &explicit),
        Resolution::Value { value: VariableValue::String(key), .. } if key == "SHOP"
    ));
    assert!(matches!(
        resolver.resolve(&VariableRef::parse("system.platform").unwrap(), &explicit),
        Resolution::Value { value: VariableValue::String(os), .. } if os == "macOs"
    ));
}

#[test]
fn redactor_removes_longer_values_first() {
    let mut redactor = Redactor::default();
    redactor.add("abc");
    redactor.add("abcdef");
    redactor.add("");
    assert_eq!(redactor.redact("x abcdef abc"), "x [redacted] [redacted]");
    let mut json = Redactor::default();
    json.add_value(&VariableValue::Json(serde_json::json!({"token": "t0k"})));
    assert!(!json.redact(r#"got {"token":"t0k"}"#).contains("t0k"));
}

#[test]
fn key_states_resolve_references_too() {
    let mut f = Fixture::new();
    let id = f.add(message(template("{{ shared.team.size }}")));
    let engine = f.engine(None);
    // Probe has no live state: nothing to show, nothing fails.
    assert!(
        engine
            .button_state(&f.profile, &f.machine, None, id, &CancellationToken::new())
            .is_none()
    );
}

/// The live session the agent would have.
struct Session(crate::ProjectId);

impl crate::variables::LiveState for Session {
    fn work_session(&self) -> Option<crate::work_session::WorkSessionSnapshot> {
        Some(crate::work_session::WorkSessionSnapshot {
            state: crate::work_session::WorkSessionState::Working,
            session: Some(crate::WorkSessionId::new()),
            project: Some(self.0),
            elapsed_ms: 65_000,
            ..crate::work_session::WorkSessionSnapshot::idle(0)
        })
    }
}

/// A registry whose work session variables come from the live state, as
/// the work session module's do.
fn session_registry() -> crate::variables::VariableRegistry {
    use crate::variables::{
        ALIASES, DeclaredScope, VariableDeclaration, VariableEnvironment, VariableSource,
    };
    struct FromLive;
    impl VariableSource for FromLive {
        fn resolve(
            &self,
            declaration: &VariableDeclaration,
            environment: &VariableEnvironment<'_>,
        ) -> Resolution {
            let Some(snapshot) = environment.live.work_session() else {
                return Resolution::Missing(MissingReason::NoValue);
            };
            let value = match declaration.id.as_str() {
                "workSession.projectId" => VariableValue::ProjectId(snapshot.project.unwrap()),
                "workSession.elapsedSeconds" => {
                    VariableValue::Integer(i64::try_from(snapshot.elapsed_ms / 1000).unwrap())
                }
                _ => return Resolution::Missing(MissingReason::NoValue),
            };
            Resolution::Value {
                value,
                source: ValueSource::System,
                sensitive: false,
            }
        }
    }
    let mut registry = crate::variables::VariableRegistry::new();
    let declarations: Vec<VariableDeclaration> = ALIASES
        .iter()
        .filter(|alias| alias.owner == "work-session")
        .map(|alias| {
            VariableDeclaration::new(
                alias.canonical,
                alias.canonical,
                alias.alias.value_type(),
                DeclaredScope::WorkSession,
            )
        })
        .collect();
    registry
        .register_module(
            &ModuleId::new("work-session").unwrap(),
            &declarations,
            Some(Arc::new(FromLive)),
        )
        .unwrap();
    registry
}

#[test]
fn work_session_variables_come_from_the_live_state() {
    use crate::work_session::WorkSessionVariable;

    let f = Fixture::new();
    let context = Invocation::default().context(&f.profile, None);
    for variable in WorkSessionVariable::ALL {
        let system = SystemVariable::from(variable);
        assert_eq!(system.work_session(), Some(variable));
        assert_eq!(system.path(), variable.path());
        assert_eq!(system.value_type(), variable.value_type());
        assert!(system.dynamic() && !system.runtime_only());
        // Nothing in the context knows a session.
        assert_eq!(
            system_value(system, &f.profile, Os::Linux, &context),
            Resolution::Missing(MissingReason::NoValue)
        );
        // Without live state there is no session either.
        assert_eq!(
            f.engine(None)
                .resolver(&f.profile, &f.machine, None)
                .resolve(&VariableRef::System(system), &context),
            Resolution::Missing(MissingReason::NoValue)
        );
    }
    let registry = session_registry();
    let resolve = |session: &Session, text: &str| {
        ContextResolver {
            profile: &f.profile,
            os: Os::Linux,
            shared: &NoSharedVariables,
            outputs: None,
            registry: &registry,
            live: session,
        }
        .resolve(&VariableRef::parse(text).unwrap(), &context)
    };
    let own = Session(f.profile.projects[0].id);
    // The legacy id and the canonical id give the same value.
    for text in [
        "system.work.session.elapsedSeconds",
        "workSession.elapsedSeconds",
    ] {
        assert!(matches!(
            resolve(&own, text),
            Resolution::Value {
                value: VariableValue::Integer(65),
                ..
            }
        ));
    }
    assert!(matches!(
        resolve(&own, "system.work.session.projectId"),
        Resolution::Value { value: VariableValue::ProjectId(id), .. } if id == own.0
    ));
    // A session started in another profile names none of these projects.
    let foreign = Session(crate::ProjectId::new());
    assert_eq!(
        resolve(&foreign, "workSession.projectId"),
        Resolution::Missing(MissingReason::OtherProfile)
    );
    // In a template, through the whole pipeline.
    let found = resolve_parameters(
        Probe::new().definition(),
        &ActionParameters::new().with(
            "message",
            template("{{ system.work.session.elapsedSeconds }}s"),
        ),
        &ContextResolver {
            profile: &f.profile,
            os: Os::Linux,
            shared: &NoSharedVariables,
            outputs: None,
            registry: &registry,
            live: &own,
        },
        &context,
        &f.profile,
    )
    .unwrap();
    assert_eq!(found.parameters.text("message"), Some("65s"));
}
