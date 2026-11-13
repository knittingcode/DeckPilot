//! Shared variables end to end: the agent's resolver for runs and
//! previews over real IPC (`listVariables`, `previewReferences`), the
//! context events of protocol 7, and older clients that know none of it.

mod support;

use std::time::Duration;

use deckpilot_core::variables::{
    CatalogFilter, PreviewItem, ProjectSource, ReferenceAvailability, ResolutionContext, ScopeKey,
    VariableDefinition, VariableKey, VariableScope, VariableType, VariableValue,
};
use deckpilot_core::{
    ActionInstanceId, ActionParameters, ModuleId, ModuleSettings, OnFailure, ParameterKind,
    ParameterValue, Profile, ProfileId, Project, ProjectKey, Workflow,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::models;
use deckpilot_ipc::AgentPaths;
use deckpilot_ipc::{
    AgentClient, AgentEvent, Capability, ClientError, ClientKind, ErrorCode, Request, Response,
};
use deckpilot_modules::general;
use deckpilot_storage::{ConfigStore, DatabaseKey, SqliteStore};
use serde_json::{Value, json};
use support::{TIMEOUT, TempDir, TestAgent, data_dir, registry};

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

struct Setup {
    agent: TestAgent,
    work: Profile,
    home: Profile,
    greet: ActionInstanceId,
    pin: ActionInstanceId,
    _dir: TempDir,
}

fn log(profile: &mut Profile, name: &str, message: ParameterValue) -> ActionInstanceId {
    let instance = registry()
        .create_instance(
            name,
            general::log_id(),
            &ActionParameters::new().with("message", message),
        )
        .unwrap();
    let id = instance.id;
    profile.actions.push(instance);
    id
}

/// The shared variables of the tests, written to the database as the
/// configuration app does.
fn store_variables(store: &SqliteStore, work: &Profile) {
    let shop = work.projects[0].id;
    let define = |definition: VariableDefinition| store.save_shared_variable(&definition).unwrap();
    let set = |name: &str, scope: ScopeKey, value: VariableValue| {
        store
            .set_shared_variable_value(&key(name), &scope, &value)
            .unwrap();
    };
    define(VariableDefinition::new(
        key("team.name"),
        VariableType::String,
        &[VariableScope::Global, VariableScope::Project],
    ));
    set(
        "team.name",
        ScopeKey::Global,
        VariableValue::String("crew".into()),
    );
    set(
        "team.name",
        ScopeKey::Project {
            profile: work.id,
            project: shop,
        },
        VariableValue::String("shop crew".into()),
    );
    define(VariableDefinition {
        default: Some(VariableValue::Integer(4)),
        ..VariableDefinition::new(
            key("team.size"),
            VariableType::Integer,
            &[VariableScope::Global],
        )
    });
    define(VariableDefinition {
        sensitive: true,
        label: "Vault PIN".into(),
        ..VariableDefinition::new(
            key("vault.pin"),
            VariableType::String,
            &[VariableScope::Global],
        )
    });
    set(
        "vault.pin",
        ScopeKey::Global,
        VariableValue::String("s3cr3t".into()),
    );
    define(VariableDefinition {
        default: Some(VariableValue::ProjectId(shop)),
        ..VariableDefinition::new(
            key("projects.favorite"),
            VariableType::ProjectId,
            &[VariableScope::Profile],
        )
    });
    // Declared by the Docker module, which is disabled.
    let conflicts = store
        .declare_module_variables(
            &ModuleId::new("docker").unwrap(),
            &[VariableDefinition {
                default: Some(VariableValue::Boolean(true)),
                ..VariableDefinition::new(
                    key("docker.flag"),
                    VariableType::Boolean,
                    &[VariableScope::Global],
                )
            }],
        )
        .unwrap();
    assert!(conflicts.is_empty(), "{conflicts:?}");
}

/// A data directory with `work` active, `home` beside it, the test
/// variables and the Docker module disabled.
fn prepare(dir: &TempDir, work: &Profile, home: Option<&Profile>) -> (AgentPaths, DatabaseKey) {
    let (paths, db_key) = data_dir(dir, work, &[]);
    let store = support::open_store(&paths, &db_key);
    if let Some(home) = home {
        store.save_profile(home).unwrap();
    }
    store
        .set_module_state(
            &ModuleId::new("docker").unwrap(),
            &ModuleSettings {
                enabled: false,
                config: ActionParameters::new(),
            },
        )
        .unwrap();
    store_variables(&store, work);
    (paths, db_key)
}

async fn setup() -> Setup {
    let dir = TempDir::new("deckpilot-variables");
    let mut work = Profile::new("Work");
    for project in ["SHOP", "CMR"] {
        work.projects
            .push(Project::new(ProjectKey::new(project).unwrap(), project));
    }
    let greet = log(
        &mut work,
        "Greet",
        ParameterValue::Template(
            "hi {{ shared.team.name }} on {{ system.project.current.key }}".into(),
        ),
    );
    let pin = log(
        &mut work,
        "Pin",
        ParameterValue::Template("pin {{ shared.vault.pin }}".into()),
    );
    let workflow = Workflow::new("Greet all").step(greet, OnFailure::Stop);
    work.workflows.push(workflow);
    let mut home = Profile::new("Home");
    home.projects
        .push(Project::new(ProjectKey::new("GARDEN").unwrap(), "Garden"));
    let (paths, db_key) = prepare(&dir, &work, Some(&home));
    let mock = MockBackend::new();
    mock.plug(&models::stream_deck_mk2(), "VARIABLES-1");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    Setup {
        agent,
        work,
        home,
        greet,
        pin,
        _dir: dir,
    }
}

impl Setup {
    fn context(&self, project: usize, source: ProjectSource) -> ResolutionContext {
        ResolutionContext::preview(self.work.id)
            .with_project(Some(self.work.projects[project].id), source)
    }

    async fn preview(
        &self,
        context: ResolutionContext,
        items: &[(&str, Option<ParameterKind>)],
    ) -> Value {
        let items = items
            .iter()
            .map(|(reference, expected)| PreviewItem {
                reference: (*reference).to_owned(),
                expected: expected.clone(),
            })
            .collect();
        let response = self
            .agent
            .request(Request::PreviewReferences {
                context: Box::new(context),
                items,
            })
            .await
            .unwrap();
        let Response::ReferencePreviews { previews } = &response else {
            panic!("previews expected, got {response:?}");
        };
        let json = serde_json::to_value(previews).unwrap();
        assert!(
            !json.to_string().contains("s3cr3t"),
            "a sensitive value left the agent: {json}"
        );
        json
    }

    async fn list(&self, filter: CatalogFilter) -> Vec<deckpilot_core::variables::CatalogEntry> {
        let context = self.context(0, ProjectSource::Application);
        match self
            .agent
            .request(Request::ListVariables {
                context: Box::new(context),
                filter: Box::new(filter),
            })
            .await
            .unwrap()
        {
            Response::Variables { entries } => entries,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn run(&self, action: ActionInstanceId) -> String {
        match self
            .agent
            .request(Request::RunAction { action })
            .await
            .unwrap()
        {
            Response::ActionResult { result } => result.message,
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previews_report_every_status_without_secrets() {
    let s = setup().await;
    let previews = s
        .preview(
            s.context(0, ProjectSource::Explicit),
            &[
                ("shared.team.name", None),
                ("shared.team.size", Some(ParameterKind::Project)),
                ("shared.team.nope", None),
                ("shared.docker.flag", None),
                ("shared.vault.pin", None),
                ("workflow.execution.id", None),
                ("{{ env.home }}", None),
                (" system.project.current.key ", None),
            ],
        )
        .await;
    let status = |index: usize| {
        previews[index]["status"]["status"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(status(0), "resolved");
    assert_eq!(
        previews[0]["status"]["value"],
        json!({"type": "string", "value": "shop crew"})
    );
    assert_eq!(previews[0]["status"]["source"]["scope"]["kind"], "project");
    assert_eq!(status(1), "typeMismatch");
    assert_eq!(status(2), "missing");
    assert_eq!(previews[2]["status"]["reason"], "unknownVariable");
    assert_eq!(status(3), "permissionDenied");
    assert_eq!(status(4), "redacted");
    // The type, as types.ts names it, and never a value.
    assert!(previews[4]["status"].get("value").is_none());
    assert!(previews[4]["status"].get("value_type").is_none());
    assert_eq!(
        previews[4]["status"]["valueType"],
        json!({"type": "string"})
    );
    assert_eq!(status(5), "unavailableUntilRuntime");
    assert_eq!(status(6), "invalid");
    assert_eq!(status(7), "resolved");
    assert_eq!(previews[7]["reference"], "system.project.current.key");

    // The project of a run is not known before it runs: the active project
    // never stands in for it.
    let previews = s
        .preview(
            s.context(0, ProjectSource::Execution),
            &[
                ("system.project.current.key", None),
                ("shared.team.name", None),
            ],
        )
        .await;
    assert_eq!(previews[0]["status"]["status"], "unavailableUntilRuntime");
    // Another project of the profile has no project value: the global one.
    let previews = s
        .preview(
            s.context(1, ProjectSource::Explicit),
            &[("shared.team.name", None)],
        )
        .await;
    assert_eq!(previews[0]["status"]["value"]["value"], "crew");
    // A client cannot claim to be a run.
    let mut run = s
        .context(0, ProjectSource::Explicit)
        .for_execution(deckpilot_core::ExecutionId::new());
    run.workflow = Some(s.work.workflows[0].id);
    let previews = s.preview(run, &[("workflow.execution.id", None)]).await;
    assert_eq!(previews[0]["status"]["status"], "unavailableUntilRuntime");
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn values_never_cross_profiles() {
    let s = setup().await;
    // Home with a project of Work: that project is not Home's.
    let foreign = ResolutionContext::preview(s.home.id)
        .with_project(Some(s.work.projects[0].id), ProjectSource::Explicit);
    let previews = s
        .preview(
            foreign,
            &[
                ("system.project.current.id", None),
                ("shared.team.name", None),
                ("shared.projects.favorite", None),
                ("system.profile.current.name", None),
            ],
        )
        .await;
    assert_eq!(previews[0]["status"]["reason"], "otherProfile");
    // Work's project value does not apply in Home.
    assert_eq!(previews[1]["status"]["value"]["value"], "crew");
    // The default names a project of Work: missing in Home.
    assert_eq!(previews[2]["status"]["reason"], "otherProfile");
    assert_eq!(previews[3]["status"]["value"]["value"], "Home");
    let unknown = s
        .agent
        .request(Request::PreviewReferences {
            context: Box::new(ResolutionContext::preview(ProfileId::new())),
            items: Vec::new(),
        })
        .await;
    assert!(
        matches!(&unknown, Err(ClientError::Agent(e)) if e.code == ErrorCode::NotFound),
        "{unknown:?}"
    );
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_catalog_is_metadata_filtered_for_the_field() {
    let s = setup().await;
    let projects = s
        .list(CatalogFilter {
            compatible_with: Some(ParameterKind::Project),
            ..CatalogFilter::default()
        })
        .await;
    let references: Vec<String> = projects.iter().map(|e| e.reference.to_string()).collect();
    assert!(references.contains(&"shared.projects.favorite".to_owned()));
    assert!(references.contains(&"system.project.current.id".to_owned()));
    assert!(!references.contains(&"shared.team.name".to_owned()));
    let vault = s
        .list(CatalogFilter {
            query: "vault pin".into(),
            ..CatalogFilter::default()
        })
        .await;
    assert_eq!(vault.len(), 1);
    assert!(vault[0].sensitive);
    assert_eq!(vault[0].availability, ReferenceAvailability::Available);
    let everything = serde_json::to_string(&s.list(CatalogFilter::default()).await).unwrap();
    assert!(!everything.contains("s3cr3t"));
    assert!(!everything.contains("shop crew"));
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actions_resolve_references_when_they_run() {
    let s = setup().await;
    // The active project is SHOP: its value wins over the global one.
    assert_eq!(s.run(s.greet).await, "hi shop crew on SHOP");
    s.agent
        .request(Request::SelectProject {
            project: s.work.projects[1].id,
        })
        .await
        .unwrap();
    assert_eq!(s.run(s.greet).await, "hi crew on CMR");
    // Sensitive values never come back.
    assert_eq!(s.run(s.pin).await, "pin [redacted]");
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_value_is_used_by_the_next_run_and_announced() {
    let s = setup().await;
    assert_eq!(s.run(s.greet).await, "hi shop crew on SHOP");
    let mut events = s.agent.events().await;
    // The configuration app writes the database, then asks for a reload.
    s.agent
        .store()
        .set_shared_variable_value(
            &key("team.name"),
            &ScopeKey::Project {
                profile: s.work.id,
                project: s.work.projects[0].id,
            },
            &VariableValue::String("night shift".into()),
        )
        .unwrap();
    s.agent.request(Request::ReloadConfiguration).await.unwrap();
    let changed = next_matching(&mut events, "sharedVariablesChanged", |e| {
        matches!(e, AgentEvent::SharedVariablesChanged { .. })
    })
    .await;
    assert_eq!(
        changed,
        AgentEvent::SharedVariablesChanged {
            keys: vec!["team.name".into()]
        }
    );
    assert_eq!(s.run(s.greet).await, "hi night shift on SHOP");
    // Previews read the database too, and a reload without changes
    // announces nothing.
    let previews = s
        .preview(
            s.context(0, ProjectSource::Explicit),
            &[("shared.team.name", None)],
        )
        .await;
    assert_eq!(previews[0]["status"]["value"]["value"], "night shift");
    s.agent.request(Request::ReloadConfiguration).await.unwrap();
    s.agent
        .request(Request::SelectProject {
            project: s.work.projects[1].id,
        })
        .await
        .unwrap();
    let next = next_matching(&mut events, "projectChanged", |e| {
        matches!(
            e,
            AgentEvent::SharedVariablesChanged { .. } | AgentEvent::ProjectChanged { .. }
        )
    })
    .await;
    assert!(
        matches!(next, AgentEvent::ProjectChanged { .. }),
        "{next:?}"
    );
    s.agent.shutdown().await;
}

async fn next_matching(
    events: &mut AgentClient,
    what: &str,
    matches: impl Fn(&AgentEvent) -> bool,
) -> AgentEvent {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .unwrap_or_else(|_| panic!("no {what} in time"))
            .unwrap();
        if matches(&event) {
            return event;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_changes_are_announced_to_new_clients_only() {
    let s = setup().await;
    let mut current = s.agent.events().await;
    assert!(current.supports(Capability::Variables));
    let mut old = AgentClient::connect_with_protocol(&s.agent.paths, ClientKind::Desktop, 6)
        .await
        .unwrap();
    assert!(!old.supports(Capability::Variables));
    old.subscribe().await.unwrap();

    let cmr = s.work.projects[1].id;
    s.agent
        .request(Request::SelectProject { project: cmr })
        .await
        .unwrap();
    let changed = next_matching(&mut current, "projectChanged", |e| {
        matches!(e, AgentEvent::ProjectChanged { .. })
    })
    .await;
    assert_eq!(
        changed,
        AgentEvent::ProjectChanged {
            profile: s.work.id,
            project: Some(cmr),
            source: ProjectSource::Application,
        }
    );

    s.agent
        .request(Request::SetActiveProfile { profile: s.home.id })
        .await
        .unwrap();
    let changed = next_matching(&mut current, "profileChanged", |e| {
        matches!(e, AgentEvent::ProfileChanged { .. })
    })
    .await;
    assert_eq!(changed, AgentEvent::ProfileChanged { profile: s.home.id });

    let device = s.agent.mock.plug(&models::stream_deck_mk2(), "VARIABLES-2");
    let changed = next_matching(&mut current, "deviceContextChanged", |e| {
        matches!(e, AgentEvent::DeviceContextChanged { .. })
    })
    .await;
    assert_eq!(
        changed,
        AgentEvent::DeviceContextChanged {
            device: Some(device)
        }
    );

    // The old client saw the state changes but none of the new events.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while let Ok(event) = tokio::time::timeout_at(deadline, old.next_event()).await {
        let event = event.unwrap();
        assert!(event.since_protocol() <= 6, "{event:?}");
    }
    // Nor can it ask for variables.
    let refused = old
        .request(Request::ListVariables {
            context: Box::new(ResolutionContext::preview(s.work.id)),
            filter: Box::default(),
        })
        .await;
    assert!(matches!(
        refused,
        Err(ClientError::Unsupported { needs: 7, .. })
    ));
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runs_announce_their_own_context_and_resolve_in_it() {
    let s = setup().await;
    let mut events = s.agent.events().await;
    let workflow = s.work.workflows[0].id;
    let execution = match s
        .agent
        .request(Request::StartWorkflow { workflow })
        .await
        .unwrap()
    {
        Response::ExecutionStarted { execution } => execution.id,
        other => panic!("unexpected {other:?}"),
    };
    for _ in 0..2 {
        let event = next_matching(&mut events, "executionContextChanged", |e| {
            matches!(e, AgentEvent::ExecutionContextChanged { .. })
        })
        .await;
        assert_eq!(
            event,
            AgentEvent::ExecutionContextChanged {
                execution,
                project: Some(s.work.projects[0].id),
            }
        );
    }
    let record = s.agent.wait_finished(execution).await;
    assert_eq!(record.nodes[0].message, "hi shop crew on SHOP");
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graph_runs_resolve_mappings_and_instance_references() {
    let dir = TempDir::new("deckpilot-variables-graph");
    let mut work = Profile::new("Work");
    work.projects
        .push(Project::new(ProjectKey::new("SHOP").unwrap(), "SHOP"));
    // The instance itself refers to a node output and a shared variable;
    // the second node maps its message from shared and system variables.
    let own = log(
        &mut work,
        "Own references",
        ParameterValue::Template("{{ steps.vars.output.stage }} for {{ shared.team.name }}".into()),
    );
    let mapped = log(&mut work, "Mapped", ParameterValue::Text("unused".into()));
    let secret = log(&mut work, "Secret", ParameterValue::Text("unused".into()));
    let mut workflow = support::graph_workflow(
        "Greet",
        support::graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "remember", "type": "setVariable", "values": {
                    "stage": {"type": "template", "template": "ready"},
                }},
                {"id": "own", "type": "action", "action": own},
                {"id": "mapped", "type": "action", "action": mapped, "mappings": {
                    "message": {"type": "template",
                        "template": "{{ shared.team.name }}/{{ system.project.current.key }}"},
                }},
                {"id": "secret", "type": "action", "action": secret, "mappings": {
                    "message": {"type": "template", "template": "pin {{ shared.vault.pin }}"},
                }},
            ]),
            &json!([
                {"from": "start", "to": "remember"},
                {"from": "remember", "to": "own"},
                {"from": "own", "to": "mapped"},
                {"from": "mapped", "to": "secret"},
            ]),
        ),
    );
    workflow.variables = vec![deckpilot_core::graph::WorkflowVariable {
        initial: Some("start".into()),
        ..deckpilot_core::graph::WorkflowVariable::new(
            "stage",
            deckpilot_core::graph::ValueKind::Text,
        )
    }];
    let workflow_id = workflow.id;
    work.workflows.push(workflow);
    let (paths, db_key) = prepare(&dir, &work, None);
    let mock = MockBackend::new();
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    let execution = match agent
        .request(Request::StartWorkflow {
            workflow: workflow_id,
        })
        .await
        .unwrap()
    {
        Response::ExecutionStarted { execution } => execution.id,
        other => panic!("unexpected {other:?}"),
    };
    let record = agent.wait_finished(execution).await;
    let message = |node: &str| {
        record
            .nodes
            .iter()
            .find(|n| n.node.as_str() == node)
            .map(|n| (n.status, n.message.clone(), n.error.clone()))
            .unwrap()
    };
    assert_eq!(
        message("own").1,
        "ready for shop crew",
        "{:?}",
        message("own")
    );
    assert_eq!(
        message("mapped").1,
        "shop crew/SHOP",
        "{:?}",
        message("mapped")
    );
    // The node that used the sensitive variable: its message is redacted,
    // and failure messages never print it (it could hold the value if the
    // redaction broke).
    let (status, sensitive_message, _) = message("secret");
    assert!(
        !format!("{record:?}").contains("s3cr3t"),
        "the value of the sensitive variable is in the history"
    );
    assert_eq!(status, deckpilot_core::graph::NodeStatus::Succeeded);
    assert!(
        sensitive_message == "[redacted]",
        "the message of the node that used the sensitive variable is not redacted"
    );
    agent.shutdown().await;
}

/// The unified registry over IPC: owners and canonical ids for protocol 9,
/// the old catalog for older clients, and a disabled module's variables
/// unavailable (never lost) until it is enabled again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // One module, disabled and enabled again.
async fn module_variables_have_owners_and_follow_the_module() {
    use deckpilot_core::variables::{VariableGroup, VariableOwner};
    let s = setup().await;
    let projects = ModuleId::new("projects").unwrap();
    let everything = CatalogFilter {
        include_unavailable: true,
        ..CatalogFilter::default()
    };
    let entries = s.list(everything.clone()).await;
    let entry = |reference: &str| {
        entries
            .iter()
            .find(|e| e.reference.to_string() == reference)
            .unwrap_or_else(|| panic!("{reference} is listed"))
            .clone()
    };
    let canonical = entry("projects.current.key");
    assert_eq!(canonical.group, VariableGroup::Module);
    assert_eq!(
        canonical.owner,
        Some(VariableOwner::Module {
            module: projects.clone()
        })
    );
    assert!(canonical.read_only && canonical.deprecated.is_none());
    assert_eq!(
        canonical.declared_scope,
        Some(deckpilot_core::variables::DeclaredScope::Project)
    );
    let legacy = entry("system.project.current.key");
    assert_eq!(
        legacy.deprecated.unwrap().canonical.as_str(),
        "projects.current.key"
    );
    assert_eq!(entry("system.platform").owner, Some(VariableOwner::Core));
    let team = entry("shared.team.name");
    assert_eq!(team.owner, Some(VariableOwner::User));
    assert!(!team.read_only);
    assert_eq!(
        entry("shared.docker.flag").owner,
        Some(VariableOwner::Module {
            module: ModuleId::new("docker").unwrap()
        })
    );

    // Both spellings resolve alike.
    let both = [
        ("projects.current.key", None),
        ("system.project.current.key", None),
    ];
    let previews = s
        .preview(s.context(0, ProjectSource::Explicit), &both)
        .await;
    assert_eq!(previews[0]["status"], previews[1]["status"]);
    assert_eq!(previews[0]["status"]["value"]["value"], "SHOP");
    // A preview of the application's project without naming one sees the
    // active project, as a key press would.
    let active = s
        .preview(ResolutionContext::preview(s.work.id), &both)
        .await;
    assert_eq!(active[0]["status"]["value"]["value"], "SHOP");

    // A protocol 8 client gets the catalog it knew.
    let mut old = AgentClient::connect_with_protocol(&s.agent.paths, ClientKind::Desktop, 8)
        .await
        .unwrap();
    assert!(!old.supports(Capability::VariableRegistry));
    let response = old
        .request(Request::ListVariables {
            context: Box::new(s.context(0, ProjectSource::Application)),
            filter: Box::new(everything.clone()),
        })
        .await
        .unwrap();
    let Response::Variables { entries: listed } = response else {
        panic!("variables");
    };
    let json = serde_json::to_value(&listed).unwrap();
    for entry in json.as_array().unwrap() {
        let reference = entry["reference"].as_str().unwrap();
        assert!(
            ["system.", "workflow.", "shared."]
                .iter()
                .any(|root| reference.starts_with(root)),
            "{reference}"
        );
        assert!(entry.get("owner").is_none(), "{entry}");
        assert!(entry.get("deprecated").is_none(), "{entry}");
        assert!(entry.get("declaredScope").is_none(), "{entry}");
    }
    assert!(
        listed
            .iter()
            .any(|e| e.reference.to_string() == "system.project.current.key")
    );

    // Disabling the projects module makes its canonical ids unavailable
    // and says so. Legacy ids keep resolving exactly as before, and the
    // configuration that uses them keeps running.
    let mut events = s.agent.events().await;
    s.agent
        .store()
        .set_module_state(
            &projects,
            &ModuleSettings {
                enabled: false,
                config: ActionParameters::new(),
            },
        )
        .unwrap();
    s.agent.request(Request::ReloadConfiguration).await.unwrap();
    let changed = next_matching(&mut events, "sharedVariablesChanged", |e| {
        matches!(e, AgentEvent::SharedVariablesChanged { keys } if keys.iter().any(|k| k.starts_with("projects.")))
    })
    .await;
    let AgentEvent::SharedVariablesChanged { keys } = changed else {
        unreachable!();
    };
    assert!(keys.iter().any(|k| k == "projects.current.key"), "{keys:?}");
    let previews = s
        .preview(s.context(0, ProjectSource::Explicit), &both)
        .await;
    assert_eq!(
        previews[0]["status"],
        json!({"status": "unavailable", "owner": "projects"})
    );
    assert_eq!(previews[1]["status"]["value"]["value"], "SHOP");
    let listed = s.list(everything.clone()).await;
    assert_eq!(
        entry_of(&listed, "projects.current.key").availability,
        ReferenceAvailability::Unavailable
    );
    assert_eq!(
        entry_of(&listed, "system.project.current.key").availability,
        ReferenceAvailability::Available
    );
    let response = old
        .request(Request::PreviewReferences {
            context: Box::new(s.context(0, ProjectSource::Explicit)),
            items: vec![PreviewItem {
                reference: "system.project.current.key".into(),
                expected: None,
            }],
        })
        .await
        .unwrap();
    let Response::ReferencePreviews { previews } = response else {
        panic!("previews");
    };
    assert_eq!(
        serde_json::to_value(&previews[0].status).unwrap()["value"]["value"],
        "SHOP"
    );
    assert_eq!(s.run(s.greet).await, "hi shop crew on SHOP");
    let stored = s.agent.store().load_profile(s.work.id).unwrap().unwrap();
    assert_eq!(
        stored.action(s.greet).unwrap().parameters.get("message"),
        Some(&ParameterValue::Template(
            "hi {{ shared.team.name }} on {{ system.project.current.key }}".into()
        ))
    );

    // Enabled again, the same references resolve.
    s.agent
        .store()
        .set_module_state(
            &projects,
            &ModuleSettings {
                enabled: true,
                config: ActionParameters::new(),
            },
        )
        .unwrap();
    s.agent.request(Request::ReloadConfiguration).await.unwrap();
    assert_eq!(s.run(s.greet).await, "hi shop crew on SHOP");
    s.agent.shutdown().await;
}

fn entry_of(
    entries: &[deckpilot_core::variables::CatalogEntry],
    reference: &str,
) -> deckpilot_core::variables::CatalogEntry {
    entries
        .iter()
        .find(|e| e.reference.to_string() == reference)
        .unwrap()
        .clone()
}
