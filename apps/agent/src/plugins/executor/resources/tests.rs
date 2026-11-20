//! Resource handles: containment, handles bound to their invocation and
//! plugin, declared kinds, project roles without Projects, grants and
//! the audit.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use deckpilot_core::{
    ActionParameters, ExecutionContext, MachineConfig, MachineResourceBinding, Os, ParameterValue,
    Profile, Project, ProjectKey, ResourceKey, ResourceKind as Core, ResourceRef,
};
use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::{RunContext, RunTrigger};
use deckpilot_plugin_abi::privileged::{ResourceHandle, ResourceKind};
use deckpilot_plugin_broker::{Broker, Decision, PluginId};
use deckpilot_plugin_supervisor::CapabilityExecutor;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::super::{
    Executor, Opener, OwnVariables, PrivilegedAudit, PrivilegedEntry, PublishedVariables, Transport,
};
use super::*;

const PLUGIN: &str = "deckpilot.git";

/// A folder tree: `repo/` with `sub/inside.txt`, `outside/secret.txt`
/// and `compose.yaml`; removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-handles-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("repo/sub")).unwrap();
        std::fs::create_dir_all(dir.join("outside")).unwrap();
        std::fs::write(dir.join("repo/sub/inside.txt"), "in").unwrap();
        std::fs::write(dir.join("outside/secret.txt"), "out").unwrap();
        std::fs::write(dir.join("compose.yaml"), "services: {}").unwrap();
        Self(dir)
    }

    fn path(&self, relative: &str) -> String {
        self.0.join(relative).display().to_string()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn binding(key: &str, kind: Core, value: &str) -> MachineResourceBinding {
    MachineResourceBinding {
        key: ResourceKey::new(key).unwrap(),
        kind,
        value: value.into(),
    }
}

fn audited() -> (PrivilegedAudit, Arc<Mutex<Vec<PrivilegedEntry>>>) {
    let records = Arc::new(Mutex::new(Vec::new()));
    let sink = records.clone();
    let audit: PrivilegedAudit = Arc::new(move |entry: &PrivilegedEntry| {
        sink.lock().unwrap().push(entry.clone());
    });
    (audit, records)
}

struct Fixture {
    broker: Broker,
}

impl Fixture {
    fn new(grants: &str) -> Self {
        let broker = Broker::new(
            Arc::new(deckpilot_plugin_broker::MemoryAudit::new(100)),
            Arc::new(deckpilot_plugin_broker::ManualClock::default()),
            deckpilot_plugin_broker::RateLimits::default(),
        );
        for plugin in [PLUGIN, "deckpilot.docker"] {
            broker
                .set_grants(
                    &PluginId::new(plugin).unwrap(),
                    &deckpilot_plugin_broker::ApprovedPermissions::from_json(grants).unwrap(),
                )
                .unwrap();
        }
        Self { broker }
    }

    fn granted() -> Self {
        Self::new(r#"[{"kind":"resources"}]"#)
    }

    fn executor(&self) -> Executor {
        let (events, _) = broadcast::channel(4);
        Executor::new(
            self.broker.clone(),
            "Git".into(),
            events,
            Transport::Network,
            OwnVariables {
                store: PublishedVariables::default(),
                namespace: "plugins.deckpilot.git".into(),
                generation: 0,
                declared: Vec::new(),
            },
            Opener::System,
        )
        .with_run(RunContext::direct(RunTrigger::Key))
    }

    /// An invocation of `plugin` with `resolved`, its action declaring
    /// `repository` (a directory) and `compose` (a file).
    fn invocation(
        &self,
        plugin: &str,
        resolved: InvocationResources,
    ) -> (Executor, BTreeMap<String, String>) {
        let declared = BTreeMap::from([
            ("repository".to_owned(), vec![ResourceKind::Directory]),
            ("compose".to_owned(), vec![ResourceKind::File]),
        ]);
        self.executor()
            .with_resources(plugin, "1.0.0", declared, resolved)
    }

    /// What the supervisor does with one call of `plugin`.
    fn call(&self, executor: &mut Executor, plugin: &str, request: &Value) -> CallResult {
        let plugin = PluginId::new(plugin).unwrap();
        let request: CapabilityRequest = serde_json::from_value(request.clone()).unwrap();
        let session = self.broker.session(plugin.clone());
        match session.authorize_in(&request, executor.run_context()) {
            Decision::Allow(allowed) => executor.execute(&plugin, &allowed),
            Decision::Deny(reason) => {
                CallResult::error(reason.call_error_code(), reason.to_string())
            }
        }
    }
}

fn resolve(args: Value) -> Value {
    let mut request = json!({"op": "resource.resolve"});
    request["args"] = args;
    request
}

fn param(key: &str, kinds: &[&str]) -> Value {
    resolve(json!({"param": key, "kinds": kinds}))
}

fn role(role: &str, kinds: &[&str]) -> Value {
    resolve(json!({"role": role, "kinds": kinds}))
}

fn handle(result: &CallResult) -> ResourceHandle {
    match result {
        CallResult::Ok(value) => serde_json::from_value(value.clone()).unwrap(),
        CallResult::Error(e) => panic!("refused: {e:?}"),
    }
}

/// The ABI code and the reason code of a refusal.
fn refused(result: &CallResult) -> (CallErrorCode, String) {
    match result {
        CallResult::Error(e) => (
            e.code,
            e.message.split(':').next().unwrap_or_default().to_owned(),
        ),
        CallResult::Ok(value) => panic!("carried out: {value}"),
    }
}

fn fixed(binding: MachineResourceBinding) -> ParamResource {
    ParamResource {
        via_role: false,
        binding: Ok(binding),
    }
}

fn project(id: &str, roles: &[(&str, MachineResourceBinding)]) -> ProjectResources {
    ProjectResources {
        id: id.into(),
        roles: roles
            .iter()
            .map(|(role, binding)| ((*role).to_owned(), Ok(binding.clone())))
            .collect(),
    }
}

/// The tree's repository as a fixed parameter and as the run project's
/// `repository` role, its compose file as the `compose` role.
fn resolved(tree: &Tree) -> InvocationResources {
    let repo = binding("shop.repository", Core::Directory, &tree.path("repo"));
    InvocationResources {
        params: BTreeMap::from([("repository".to_owned(), fixed(repo.clone()))]),
        project: Some(project(
            "p-run",
            &[
                ("repository", repo),
                (
                    "compose",
                    binding("shop.compose", Core::Other, &tree.path("compose.yaml")),
                ),
            ],
        )),
        ..InvocationResources::default()
    }
}

/// A resource parameter reaches the guest as a handle, the same one
/// `resource.resolve {param}` answers with the binding's label and kind;
/// the guest-visible values never contain the bound path, which only
/// `resources:paths` adds.
#[test]
fn parameters_become_opaque_handles_with_labels_only() {
    let tree = Tree::new();
    let fixture = Fixture::granted();
    let (mut executor, params) = fixture.invocation(PLUGIN, resolved(&tree));
    let issued = &params["repository"];
    assert_eq!(issued.len(), 32);
    assert!(issued.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    let answer = fixture.call(&mut executor, PLUGIN, &param("repository", &["directory"]));
    let repo = handle(&answer);
    assert_eq!(&repo.handle, issued);
    assert_eq!(repo.kind, ResourceKind::Directory);
    assert_eq!(repo.label, "shop.repository");
    assert_eq!(repo.display_path, None);
    let CallResult::Ok(value) = &answer else {
        unreachable!()
    };
    let visible = format!("{value} {params:?}");
    assert!(!visible.contains(&tree.path("repo")), "{visible}");
    assert!(!visible.contains("deckpilot-handles"), "{visible}");
    // A role of the run's project: its own handle, a file kind for the
    // compose file.
    let compose = handle(&fixture.call(&mut executor, PLUGIN, &role("compose", &["file"])));
    assert_eq!(compose.kind, ResourceKind::File);
    assert_ne!(compose.handle, repo.handle);

    // With `resources:paths` the bound path is added.
    let fixture = Fixture::new(r#"[{"kind":"resources","paths":true}]"#);
    let (mut executor, _) = fixture.invocation(PLUGIN, resolved(&tree));
    let repo = handle(&fixture.call(&mut executor, PLUGIN, &param("repository", &["directory"])));
    assert_eq!(repo.display_path, Some(tree.path("repo")));
}

/// Handles are bound to their invocation and plugin: one replayed in a
/// second invocation, used by another plugin, or forged is
/// `not-permitted`; a plugin cannot resolve in another plugin's
/// invocation either.
#[test]
fn handles_are_not_replayable() {
    let tree = Tree::new();
    let fixture = Fixture::granted();
    let (first, params) = fixture.invocation(PLUGIN, resolved(&tree));
    let issued = params["repository"].clone();
    let git = PluginId::new(PLUGIN).unwrap();
    let docker = PluginId::new("deckpilot.docker").unwrap();
    assert!(first.bound(&git, &issued).is_ok());
    assert!(first.bound_inside(&git, &issued, None, true).is_ok());

    let (second, again) = fixture.invocation(PLUGIN, resolved(&tree));
    assert_ne!(again["repository"], issued, "fresh per invocation");
    let not_permitted = |result: Result<Issued, CallResult>| match result {
        Err(refusal) => refused(&refusal),
        Ok(issued) => panic!("used: {issued:?}"),
    };
    // Another invocation of the same plugin.
    assert_eq!(
        not_permitted(second.bound(&git, &issued)).0,
        CallErrorCode::NotPermitted
    );
    // Another plugin in the same invocation.
    assert_eq!(
        not_permitted(first.bound(&docker, &issued)).0,
        CallErrorCode::NotPermitted
    );
    // A forged handle.
    let forged = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        not_permitted(first.bound(&git, forged)).0,
        CallErrorCode::NotPermitted
    );
    // An executor without resources (a state call, the plugin page).
    assert_eq!(
        not_permitted(fixture.executor().bound(&git, &issued)).0,
        CallErrorCode::NotPermitted
    );

    // Resolving as another plugin than the invocation's.
    let (mut executor, _) = fixture.invocation(PLUGIN, resolved(&tree));
    let answer = fixture.call(
        &mut executor,
        "deckpilot.docker",
        &param("repository", &["directory"]),
    );
    assert_eq!(refused(&answer).0, CallErrorCode::NotPermitted);
}

/// Paths inside a handle stay inside: `..`, absolute paths and links
/// that lead outside are refused, by messages that name the label, never
/// the path.
#[test]
fn paths_stay_inside_their_handle() {
    let tree = Tree::new();
    let fixture = Fixture::granted();
    let (executor, params) = fixture.invocation(PLUGIN, resolved(&tree));
    let git = PluginId::new(PLUGIN).unwrap();
    let repo = &params["repository"];
    assert_eq!(
        executor
            .bound_inside(&git, repo, Some("sub/inside.txt"), false)
            .unwrap(),
        Path::new(&tree.path("repo")).join("sub/inside.txt")
    );
    let code = |relative: &str, directory: bool| match executor.bound_inside(
        &git,
        repo,
        Some(relative),
        directory,
    ) {
        Err(refusal) => {
            let CallResult::Error(e) = &refusal else {
                unreachable!()
            };
            assert!(
                !e.message.contains(&tree.0.display().to_string()),
                "{}",
                e.message
            );
            refused(&refusal)
        }
        Ok(path) => panic!("{relative}: {}", path.display()),
    };
    let absolute = tree.path("outside/secret.txt");
    for relative in [
        "../outside/secret.txt",
        "sub/../../outside",
        absolute.as_str(),
        "/etc/passwd",
        r"C:\Windows",
    ] {
        assert_eq!(
            code(relative, false),
            (CallErrorCode::NotPermitted, "invalid-path".to_owned()),
            "{relative}"
        );
    }
    assert_eq!(code("missing.txt", false).1, "not-found");

    #[cfg(unix)]
    {
        let root = Path::new(&tree.0);
        std::os::unix::fs::symlink(root.join("outside"), root.join("repo/out-dir")).unwrap();
        std::os::unix::fs::symlink(root.join("outside/secret.txt"), root.join("repo/out-file"))
            .unwrap();
        std::os::unix::fs::symlink(root.join("repo/sub"), root.join("repo/in-dir")).unwrap();
        for (relative, directory) in [
            ("out-dir", true),
            ("out-dir/secret.txt", false),
            ("out-file", false),
        ] {
            assert_eq!(
                code(relative, directory),
                (CallErrorCode::NotPermitted, "invalid-path".to_owned()),
                "{relative}"
            );
        }
        assert!(
            executor
                .bound_inside(&git, repo, Some("in-dir/inside.txt"), false)
                .is_ok()
        );
    }

    // A handle of another kind has no inside.
    let mut resolved = resolved(&tree);
    resolved.params.insert(
        "compose".into(),
        fixed(binding(
            "shop.compose",
            Core::Other,
            &tree.path("compose.yaml"),
        )),
    );
    let (executor, params) = fixture.invocation(PLUGIN, resolved);
    let refusal = executor
        .bound_inside(&git, &params["compose"], Some("x"), false)
        .unwrap_err();
    assert_eq!(refused(&refusal).1, "resource-kind-mismatch");
}

/// Only bindings of the declared kinds resolve: a parameter bound to
/// another kind gets no handle and resolves to `resource-kind-mismatch`,
/// as does a role asked for a kind it is not; terminals and URLs are no
/// plugin kind at all. A folder that does not exist is `not-found`.
#[test]
fn only_declared_kinds_resolve() {
    let tree = Tree::new();
    let fixture = Fixture::granted();
    let mut wrong = resolved(&tree);
    wrong.params.insert(
        "repository".into(),
        fixed(binding("editor", Core::Application, "com.example.Editor")),
    );
    wrong.params.insert(
        "compose".into(),
        fixed(binding("shop.terminal", Core::Terminal, "default")),
    );
    let (mut executor, params) = fixture.invocation(PLUGIN, wrong);
    assert!(params.is_empty(), "{params:?}");
    for request in [
        param("repository", &["directory", "application"]),
        param("compose", &["file"]),
        role("repository", &["file"]),
    ] {
        assert_eq!(
            refused(&fixture.call(&mut executor, PLUGIN, &request)),
            (CallErrorCode::NotPermitted, "resource-kind-mismatch".into()),
            "{request}"
        );
    }
    // A parameter the action does not declare.
    assert_eq!(
        refused(&fixture.call(&mut executor, PLUGIN, &param("other", &["directory"]))).0,
        CallErrorCode::NotPermitted
    );

    let mut gone = resolved(&tree);
    gone.params.insert(
        "repository".into(),
        fixed(binding(
            "old.repository",
            Core::Directory,
            &tree.path("gone"),
        )),
    );
    let (mut executor, params) = fixture.invocation(PLUGIN, gone);
    assert!(!params.contains_key("repository"));
    let answer = fixture.call(&mut executor, PLUGIN, &param("repository", &["directory"]));
    assert_eq!(
        refused(&answer),
        (CallErrorCode::Failed, "not-found".into())
    );
    let CallResult::Error(e) = &answer else {
        unreachable!()
    };
    assert!(
        !e.message.contains(&tree.0.display().to_string()),
        "{}",
        e.message
    );

    // Unset, unbound and missing roles say so.
    let mut unset = resolved(&tree);
    unset.params.clear();
    unset.params.insert(
        "compose".into(),
        ParamResource {
            via_role: false,
            binding: Err(ResolveError::Unbound(ResourceKey::new("x.y").unwrap())),
        },
    );
    let (mut executor, _) = fixture.invocation(PLUGIN, unset);
    for (request, reason) in [
        (param("repository", &["directory"]), "resource-not-set"),
        (param("compose", &["file"]), "resource-unbound"),
        (role("docs", &["directory"]), "resource-role-missing"),
    ] {
        assert_eq!(
            refused(&fixture.call(&mut executor, PLUGIN, &request)).1,
            reason
        );
    }
}

/// Without an active project, or with the Projects feature off, a role
/// is `no-active-project`, and so is a parameter taken from a role; a
/// fixed parameter still resolves.
#[test]
fn roles_need_an_active_project_and_projects_on() {
    let tree = Tree::new();
    let fixture = Fixture::granted();
    let mut none = resolved(&tree);
    none.project = None;
    let (mut executor, _) = fixture.invocation(PLUGIN, none);
    assert_eq!(
        refused(&fixture.call(&mut executor, PLUGIN, &role("repository", &["directory"]))),
        (CallErrorCode::Unavailable, "no-active-project".into())
    );

    let mut by_role = resolved(&tree);
    by_role.params.insert(
        "compose".into(),
        ParamResource {
            via_role: true,
            binding: Ok(binding(
                "shop.compose",
                Core::Other,
                &tree.path("compose.yaml"),
            )),
        },
    );
    let (mut executor, params) = fixture.invocation(PLUGIN, by_role.without_projects());
    assert!(params.contains_key("repository"), "a fixed binding stays");
    assert!(!params.contains_key("compose"));
    for request in [
        role("repository", &["directory"]),
        resolve(json!({"role": "repository", "projectId": "p-run", "kinds": ["directory"]})),
        param("compose", &["file"]),
    ] {
        assert_eq!(
            refused(&fixture.call(&mut executor, PLUGIN, &request)),
            (CallErrorCode::Unavailable, "no-active-project".into()),
            "{request}"
        );
    }
    handle(&fixture.call(&mut executor, PLUGIN, &param("repository", &["directory"])));
}

/// `projectId` selects the run's project or one the action's own project
/// parameter names, never another project of the profile.
#[test]
fn another_project_only_when_the_action_names_it() {
    let tree = Tree::new();
    let fixture = Fixture::granted();
    let mut resolved = resolved(&tree);
    resolved.named.push(project(
        "p-named",
        &[(
            "repository",
            binding("other.repository", Core::Directory, &tree.path("outside")),
        )],
    ));
    let (mut executor, _) = fixture.invocation(PLUGIN, resolved);
    let in_project =
        |id: &str| resolve(json!({"role": "repository", "projectId": id, "kinds": ["directory"]}));
    let named = handle(&fixture.call(&mut executor, PLUGIN, &in_project("p-named")));
    assert_eq!(named.label, "other.repository");
    let run = handle(&fixture.call(&mut executor, PLUGIN, &in_project("p-run")));
    assert_eq!(run.label, "shop.repository");
    assert_eq!(
        refused(&fixture.call(&mut executor, PLUGIN, &in_project("p-elsewhere"))).0,
        CallErrorCode::NotPermitted
    );
}

/// Without the `resources` grant the broker denies the call; with it, 20
/// resolves per invocation, then `throttled`. Every call the executor
/// carries out or refuses is audited by label, never by path.
#[test]
fn grants_limits_and_the_audit() {
    let tree = Tree::new();
    let none = Fixture::new("[]");
    let (mut executor, params) = none.invocation(PLUGIN, resolved(&tree));
    assert_eq!(params.len(), 1, "a handle confers nothing by itself");
    assert_eq!(
        refused(&none.call(&mut executor, PLUGIN, &param("repository", &["directory"]))).0,
        CallErrorCode::NotPermitted
    );

    let fixture = Fixture::granted();
    let (audit, records) = audited();
    let (executor, _) = fixture.invocation(PLUGIN, resolved(&tree));
    let mut executor = executor.with_audit(audit);
    for _ in 0..MAX_RESOLVES {
        handle(&fixture.call(&mut executor, PLUGIN, &param("repository", &["directory"])));
    }
    assert_eq!(
        refused(&fixture.call(&mut executor, PLUGIN, &role("repository", &["directory"]))).0,
        CallErrorCode::Throttled
    );
    let records = records.lock().unwrap();
    assert_eq!(records.len(), 21);
    let first = &records[0];
    assert_eq!(first.version, "1.0.0");
    assert_eq!(first.origin, "key");
    assert_eq!(first.record.plugin, PLUGIN);
    assert_eq!(first.record.op, "resource.resolve");
    assert_eq!(first.record.grant, "resources");
    assert_eq!(first.record.outcome, None);
    assert_eq!(
        first.record.details,
        [
            ("param", "repository".to_owned()),
            ("kind", "directory".to_owned()),
            ("label", "shop.repository".to_owned()),
        ]
    );
    assert_eq!(records[20].record.outcome, Some("throttled"));
    assert_eq!(
        records[20].record.details,
        [("role", "repository".to_owned())]
    );
    let all = format!("{records:?}");
    assert!(!all.contains(&tree.path("repo")), "{all}");
}

/// The engine's side: parameters, the run's project roles and a project
/// the action names resolve through the core's references; without the
/// capabilities everything is `not-permitted`.
#[test]
fn the_engine_resolves_references_within_its_capabilities() {
    let mut profile = Profile::new("P");
    let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    shop.resources.insert(
        "repository".into(),
        ResourceKey::new("shop.repository").unwrap(),
    );
    let mut blog = Project::new(ProjectKey::new("BLOG").unwrap(), "Blog");
    blog.resources.insert(
        "repository".into(),
        ResourceKey::new("blog.repository").unwrap(),
    );
    profile.projects = vec![shop.clone(), blog.clone()];
    let mut machine = MachineConfig::new(Os::Linux);
    machine.set_binding(binding("shop.repository", Core::Directory, "/src/shop"));
    machine.set_binding(binding("blog.repository", Core::Directory, "/src/blog"));
    let parameters = ActionParameters::new()
        .with(
            "repository",
            ParameterValue::Resource(ResourceRef::ProjectRole {
                role: "repository".into(),
            }),
        )
        .with(
            "fixed",
            ParameterValue::Resource(ResourceRef::Fixed {
                key: ResourceKey::new("missing.key").unwrap(),
            }),
        )
        .with("project", ParameterValue::Project(blog.id));

    let context = ExecutionContext::new(&profile, Some(&shop), &machine);
    let resolved = InvocationResources::of(&parameters, &context);
    let repo = &resolved.params["repository"];
    assert!(repo.via_role);
    assert_eq!(repo.binding.as_ref().unwrap().value, "/src/shop");
    assert_eq!(
        resolved.params["fixed"].binding,
        Err(ResolveError::Unbound(
            ResourceKey::new("missing.key").unwrap()
        ))
    );
    let run = resolved.project.as_ref().unwrap();
    assert_eq!(run.id, shop.id.to_string());
    assert_eq!(run.roles["repository"].as_ref().unwrap().value, "/src/shop");
    assert_eq!(resolved.named.len(), 1);
    assert_eq!(resolved.named[0].id, blog.id.to_string());
    assert!(!resolved.projects_off);

    let off = resolved.without_projects();
    assert!(off.project.is_none() && off.named.is_empty() && off.projects_off);
    assert_eq!(
        off.params["repository"].binding,
        Err(ResolveError::NoActiveProject)
    );

    let nothing = BTreeSet::new();
    let blind = ExecutionContext::new(&profile, Some(&shop), &machine).restricted(&nothing);
    let resolved = InvocationResources::of(&parameters, &blind);
    assert_eq!(
        resolved.params["repository"].binding,
        Err(ResolveError::NotPermitted)
    );
    assert!(resolved.project.is_none() && resolved.named.is_empty());

    let without_project = ExecutionContext::new(&profile, None, &machine);
    assert_eq!(
        InvocationResources::of(&parameters, &without_project).params["repository"].binding,
        Err(ResolveError::NoActiveProject)
    );
}
