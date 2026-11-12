//! Shared variables end to end: the agent's resolver for runs and
//! previews over real IPC (`listVariables`, `previewReferences`), the
//! context events of protocol 7, and older clients that know none of it.

mod support;

use std::time::Duration;

use deckpilot_agent::SharedVariables;
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
use deckpilot_ipc::{
    AgentClient, AgentEvent, Capability, ClientError, ClientKind, ErrorCode, Request, Response,
};
use deckpilot_modules::general;
use deckpilot_storage::ConfigStore;
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

fn variables(work: &Profile) -> SharedVariables {
    let shop = work.projects[0].id;
    let mut variables = SharedVariables::new();
    variables
        .define(VariableDefinition::new(
            key("team.name"),
            VariableType::String,
            &[VariableScope::Global, VariableScope::Project],
        ))
        .unwrap();
    variables
        .set(
            &key("team.name"),
            ScopeKey::Global,
            VariableValue::String("crew".into()),
        )
        .unwrap();
    variables
        .set(
            &key("team.name"),
            ScopeKey::Project {
                profile: work.id,
                project: shop,
            },
            VariableValue::String("shop crew".into()),
        )
        .unwrap();
    variables
        .define(VariableDefinition {
            default: Some(VariableValue::Integer(4)),
            ..VariableDefinition::new(
                key("team.size"),
                VariableType::Integer,
                &[VariableScope::Global],
            )
        })
        .unwrap();
    variables
        .define(VariableDefinition {
            sensitive: true,
            label: "Vault PIN".into(),
            ..VariableDefinition::new(
                key("vault.pin"),
                VariableType::String,
                &[VariableScope::Global],
            )
        })
        .unwrap();
    variables
        .set(
            &key("vault.pin"),
            ScopeKey::Global,
            VariableValue::String("s3cr3t".into()),
        )
        .unwrap();
    variables
        .define(VariableDefinition {
            module: Some(ModuleId::new("docker").unwrap()),
            default: Some(VariableValue::Boolean(true)),
            ..VariableDefinition::new(
                key("docker.flag"),
                VariableType::Boolean,
                &[VariableScope::Global],
            )
        })
        .unwrap();
    variables
        .define(VariableDefinition {
            default: Some(VariableValue::ProjectId(shop)),
            ..VariableDefinition::new(
                key("projects.favorite"),
                VariableType::ProjectId,
                &[VariableScope::Profile],
            )
        })
        .unwrap();
    variables
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
    let (paths, db_key) = data_dir(&dir, &work, &[]);
    {
        let store = support::open_store(&paths, &db_key);
        store.save_profile(&home).unwrap();
        store
            .set_module_state(
                &ModuleId::new("docker").unwrap(),
                &ModuleSettings {
                    enabled: false,
                    config: ActionParameters::new(),
                },
            )
            .unwrap();
    }
    let mock = MockBackend::new();
    mock.plug(&models::stream_deck_mk2(), "VARIABLES-1");
    let agent =
        TestAgent::start_with_variables(&paths, &db_key, &mock, None, Some(variables(&work)));
    agent.unlock().await;
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
    // The contract serializes this field as `value_type` today (types.ts
    // expects `valueType`); either way it carries the type, never a value.
    assert!(previews[4]["status"].get("value").is_none());
    assert_eq!(
        previews[4]["status"]["valueType"]
            .as_object()
            .or(previews[4]["status"]["value_type"].as_object())
            .cloned()
            .map(Value::Object),
        Some(json!({"type": "string"}))
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
