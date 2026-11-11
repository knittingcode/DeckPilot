//! Contract tests over every built-in module and action type.
//!
//! The checks are generic: they read manifests, action type definitions and
//! parameter schemas, and run every action twice on a fixture profile that
//! has no machine resources bound — once without any capability and once
//! with the module's own. Nothing real is started: modules that open things
//! get a recording launcher, program settings point to a file that does not
//! exist, and every address is a closed port on this computer that the
//! network policy refuses anyway.
//!
//! A finding is either fixed or listed in [`KNOWN`] with the reason it is
//! accepted. A listed finding that no longer occurs fails the test too, so
//! the list stays current.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_core::graph::{ActionEffect, MAX_TIMEOUT_MS};
use deckpilot_core::launch::{Launcher, RecordingLauncher};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, CancellationToken,
    Capability, CredentialId, ExecutionContext, ExecutionControl, FeatureModule, MachineConfig,
    MemoryCredentials, ModuleId, ModuleManifest, Os, ParameterKind, ParameterSpec, ParameterValue,
    Profile, Progress, Project, ProjectId, ProjectKey, ResourceKey, ResourceRef, Secret,
    SettingScope,
};
use deckpilot_modules::{
    builtin, docker, general, git, github, gitlab, http, jira, notifications, projects, system,
    workspace,
};

/// The planted secret: no result may ever contain it.
const SECRET: &str = "contract-PLANTED-s3cret-7f3a9c";
/// The credential every action that takes one is given.
const CREDENTIAL: &str = "contract.token";
/// A closed port on this computer, refused by the network policies.
const CLOSED_URL: &str = "http://127.0.0.1:9/";
/// A program that does not exist.
const MISSING_PROGRAM: &str = "/nonexistent/deckpilot-contract/program";
/// The workspace version every module reports.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Accepted findings: subject (module or action type id), check, reason.
const KNOWN: &[(&str, &str, &str)] = &[
    // Heuristic limits.
    (
        "gitlab.merge-requests",
        "action-effect",
        "“merge requests” is a noun here; the action only lists them",
    ),
    (
        "general.wait",
        "run-denied-succeeds",
        "waiting needs no capability",
    ),
    (
        "system.process-status",
        "action-cancellable",
        "reads the process list in-process; it starts no program and has nothing to cancel",
    ),
    // Findings in modules outside the integration modules, kept until their
    // owners decide.
    (
        "projects.next",
        "action-retry",
        "changes the active project but is declared read-only",
    ),
    (
        "projects.previous",
        "action-retry",
        "changes the active project but is declared read-only",
    ),
    (
        "notifications.show",
        "run-denied-succeeds",
        "by design a notification that cannot be shown does not fail the node (`shown` is \
         false) unless it is critical, even when the capability is missing",
    ),
    (
        "notifications.workflow-summary",
        "run-denied-succeeds",
        "as notifications.show",
    ),
    (
        "projects.select",
        "run-failure-without-code",
        "failures have no ActionResult.code",
    ),
    (
        "projects.repository",
        "run-failure-without-code",
        "failures have no ActionResult.code",
    ),
    (
        "projects.documentation",
        "run-failure-without-code",
        "failures have no ActionResult.code",
    ),
    (
        "projects.resolve-resource",
        "run-failure-without-code",
        "failures have no ActionResult.code",
    ),
];

/// Findings of one test, compared with [`KNOWN`] for the checks it owns.
struct Report {
    checks: &'static [&'static str],
    found: BTreeSet<(String, &'static str)>,
    details: Vec<String>,
}

impl Report {
    fn new(checks: &'static [&'static str]) -> Self {
        Self {
            checks,
            found: BTreeSet::new(),
            details: Vec::new(),
        }
    }

    fn add(&mut self, subject: &str, check: &'static str, detail: impl Into<String>) {
        assert!(self.checks.contains(&check), "{check} is not declared");
        if self.found.insert((subject.to_owned(), check)) {
            self.details
                .push(format!("{subject}: {check}: {}", detail.into()));
        } else {
            self.details
                .push(format!("{subject}: {check} (again): {}", detail.into()));
        }
    }

    fn finish(self) {
        let known: BTreeSet<(String, &str)> = KNOWN
            .iter()
            .filter(|(_, check, _)| self.checks.contains(check))
            .map(|(subject, check, _)| ((*subject).to_owned(), *check))
            .collect();
        let unexpected: Vec<&String> = self
            .details
            .iter()
            .filter(|detail| {
                !known
                    .iter()
                    .any(|(s, c)| detail.starts_with(&format!("{s}: {c}")))
            })
            .collect();
        let stale: Vec<_> = known
            .iter()
            .filter(|(s, c)| !self.found.contains(&(s.clone(), *c)))
            .collect();
        assert!(
            unexpected.is_empty() && stale.is_empty(),
            "contract violations:\n  {}\nknown findings that no longer occur: {stale:?}",
            unexpected
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
}

/// Every built-in module, with a recording launcher where it opens things.
fn modules(launcher: &Arc<RecordingLauncher>) -> Vec<Box<dyn FeatureModule>> {
    let opener: Arc<dyn Launcher> = launcher.clone();
    vec![
        Box::new(docker::DockerModule::new()),
        Box::new(general::GeneralModule::new()),
        Box::new(git::GitModule::with_launcher(opener.clone())),
        Box::new(github::GithubModule::with_launcher(opener.clone())),
        Box::new(gitlab::GitlabModule::with_launcher(opener.clone())),
        Box::new(http::HttpModule::new()),
        Box::new(jira::JiraModule::with_launcher(opener.clone())),
        Box::new(notifications::NotificationsModule::new()),
        Box::new(projects::ProjectsModule::new()),
        Box::new(system::SystemModule::with_launcher(opener.clone())),
        Box::new(workspace::WorkspaceModule::with_launcher(opener)),
    ]
}

fn schema(id: &str, parameters: Vec<ParameterSpec>) -> ActionType {
    ActionType {
        id: ActionTypeId::new(format!("{id}.config")).unwrap(),
        name: id.into(),
        description: String::new(),
        category: String::new(),
        parameters,
        requires_project: false,
        node: deckpilot_core::graph::NodeSpec::default(),
    }
}

/// The module's settings with every program missing and every address a
/// closed local port. Permissions stay at their defaults (off).
fn safe_config(manifest: &ModuleManifest) -> ActionParameters {
    let mut config = ActionParameters::new();
    for spec in &manifest.config {
        let key = spec.key.to_ascii_lowercase();
        if key.contains("executable") || key.contains("program") {
            config = config.with(&spec.key, ParameterValue::Text(MISSING_PROGRAM.into()));
        } else if key.ends_with("url") {
            config = config.with(&spec.key, ParameterValue::Text(CLOSED_URL.into()));
        }
    }
    schema(manifest.id.as_str(), manifest.config.clone())
        .normalize(&config)
        .unwrap_or_else(|e| panic!("{}: the safe settings do not validate: {e:?}", manifest.id))
}

/// A value for a required parameter without a default.
fn sample(spec: &ParameterSpec) -> ParameterValue {
    let key = spec.key.to_ascii_lowercase();
    match &spec.kind {
        ParameterKind::Text { .. } => ParameterValue::Text(
            if key.contains("url") {
                CLOSED_URL
            } else {
                match key.as_str() {
                    "repository" => "octo/site",
                    "project" => "group/site",
                    "issue" => "ABC-1",
                    "path" => "README.md",
                    _ => "contract",
                }
            }
            .into(),
        ),
        ParameterKind::Integer { min, max } => {
            ParameterValue::Integer(min.unwrap_or(1).max(1).min(max.unwrap_or(i64::MAX)))
        }
        ParameterKind::Boolean => ParameterValue::Boolean(false),
        ParameterKind::Choice { options } => ParameterValue::Choice(options[0].clone()),
        ParameterKind::Resource { .. } => ParameterValue::Resource(ResourceRef::Fixed {
            key: ResourceKey::new("contract.unbound").unwrap(),
        }),
        // A project that is not in the profile.
        ParameterKind::Project => ParameterValue::Project(ProjectId::new()),
    }
}

/// Required parameters filled in, waits as short as possible, and the
/// planted credential wherever one is taken.
fn minimal(action: &ActionType) -> ActionParameters {
    let mut parameters = ActionParameters::new();
    for spec in &action.parameters {
        if spec.required && spec.default.is_none() {
            parameters = parameters.with(&spec.key, sample(spec));
        }
        if spec.key == "seconds" {
            parameters = parameters.with(&spec.key, sample(spec));
        }
        if spec.key == "credential" {
            parameters = parameters.with(&spec.key, ParameterValue::Text(CREDENTIAL.into()));
        }
    }
    parameters
}

/// A profile with one active project whose resources are bound nowhere,
/// and credentials that all hold the planted secret.
struct World {
    profile: Profile,
    machine: MachineConfig,
    credentials: MemoryCredentials,
}

fn world() -> World {
    let mut profile = Profile::new("Contract");
    let mut project = Project::new(ProjectKey::new("CONTRACT").unwrap(), "Contract");
    for role in ["repository", "docs", "editor", "compose"] {
        project.resources.insert(
            role.into(),
            ResourceKey::new(format!("contract.{role}")).unwrap(),
        );
    }
    profile.projects.push(project);
    let mut credentials = MemoryCredentials::new();
    for (id, secret) in [
        (CREDENTIAL, SECRET.to_owned()),
        ("github.token", SECRET.to_owned()),
        ("gitlab.token", SECRET.to_owned()),
        // Jira Cloud expects email:api-token.
        ("jira.token", format!("contract@example.com:{SECRET}")),
    ] {
        credentials.insert(CredentialId::new(id).unwrap(), Secret::new(secret));
    }
    World {
        profile,
        machine: MachineConfig::new(Os::current()),
        credentials,
    }
}

/// Runs `handler` with `capabilities` only, stopping it after a few
/// seconds; `None` when it panicked.
fn execute(
    handler: &dyn ActionHandler,
    parameters: &ActionParameters,
    world: &World,
    capabilities: &BTreeSet<Capability>,
) -> Option<ActionResult> {
    let token =
        CancellationToken::new().child_with_deadline(Instant::now() + Duration::from_secs(5));
    let progress = |_: Progress| {};
    let control = ExecutionControl::new(&token, &progress);
    let context = ExecutionContext::new(
        &world.profile,
        Some(&world.profile.projects[0]),
        &world.machine,
    )
    .with_credentials(&world.credentials)
    .with_control(&control)
    .restricted(capabilities);
    std::panic::catch_unwind(AssertUnwindSafe(|| handler.execute(parameters, &context))).ok()
}

/// The frontend's parameter kinds; a kind it cannot draw is a violation.
const FRONTEND_TYPES: &str = include_str!("../../../apps/desktop/src/lib/types.ts");

fn frontend_kind(kind: &ParameterKind) -> &'static str {
    // Exhaustive on purpose: a new kind must be added here and in the app.
    match kind {
        ParameterKind::Text { .. } => "text",
        ParameterKind::Integer { .. } => "integer",
        ParameterKind::Boolean => "boolean",
        ParameterKind::Choice { .. } => "choice",
        ParameterKind::Resource { .. } => "resource",
        ParameterKind::Project => "project",
    }
}

fn renderable(kind: &ParameterKind) -> bool {
    let union = FRONTEND_TYPES
        .split("export type ParameterKind =")
        .nth(1)
        .and_then(|rest| rest.split("export type").next())
        .unwrap_or_default();
    union.contains(&format!("type: \"{}\"", frontend_kind(kind)))
}

/// Problems with a parameter's schema and default.
fn schema_problems(spec: &ParameterSpec) -> Vec<(&'static str, String)> {
    let mut problems = Vec::new();
    if spec.key.is_empty() || spec.label.trim().is_empty() {
        problems.push(("parameter-label", format!("{} has no label", spec.key)));
    }
    if !renderable(&spec.kind) {
        problems.push((
            "parameter-kind-renderable",
            format!("{} has a kind the app cannot draw", spec.key),
        ));
    }
    match &spec.kind {
        ParameterKind::Choice { options } => {
            let unique: BTreeSet<_> = options.iter().collect();
            if options.is_empty() || unique.len() != options.len() {
                problems.push((
                    "parameter-schema",
                    format!("{}: choice options are empty or repeated", spec.key),
                ));
            }
        }
        ParameterKind::Integer {
            min: Some(min),
            max: Some(max),
        } if min > max => problems.push((
            "parameter-schema",
            format!("{}: min {min} is above max {max}", spec.key),
        )),
        ParameterKind::Text {
            max_length: Some(0),
            ..
        } => problems.push(("parameter-schema", format!("{}: max length 0", spec.key))),
        _ => {}
    }
    if let Some(default) = &spec.default {
        let fits = match (&spec.kind, default) {
            (ParameterKind::Text { max_length, .. }, ParameterValue::Text(text)) => {
                max_length.is_none_or(|max| text.chars().count() <= max)
                    && !(spec.required && text.trim().is_empty())
            }
            (ParameterKind::Integer { min, max }, ParameterValue::Integer(value)) => {
                min.is_none_or(|min| *value >= min) && max.is_none_or(|max| *value <= max)
            }
            (ParameterKind::Boolean, ParameterValue::Boolean(_))
            | (ParameterKind::Resource { .. }, ParameterValue::Resource(_)) => true,
            (ParameterKind::Choice { options }, ParameterValue::Choice(choice)) => {
                options.contains(choice)
            }
            _ => false,
        };
        if !fits {
            problems.push((
                "parameter-default",
                format!(
                    "{}: the default {default:?} does not fit its kind",
                    spec.key
                ),
            ));
        }
    }
    let key = spec.key.to_ascii_lowercase();
    if [
        "secret",
        "token",
        "password",
        "apikey",
        "api_key",
        "passphrase",
    ]
    .iter()
    .any(|word| key.contains(word))
    {
        problems.push((
            "secret-parameter",
            format!(
                "{} looks like it holds a secret; secrets go through a stored credential",
                spec.key
            ),
        ));
    }
    if key.contains("credential")
        && !matches!(
            spec.kind,
            ParameterKind::Text {
                multiline: false,
                max_length: Some(1..=64)
            }
        )
    {
        problems.push((
            "credential-parameter",
            format!(
                "{} must be a one-line credential name of at most 64 characters",
                spec.key
            ),
        ));
    }
    problems
}

#[test]
fn the_contract_covers_every_builtin_module() {
    let launcher = Arc::new(RecordingLauncher::new());
    let listed: Vec<String> = modules(&launcher)
        .iter()
        .map(|m| m.manifest().id.to_string())
        .collect();
    let registered: Vec<String> = builtin(Os::current())
        .modules()
        .iter()
        .map(|m| m.manifest.id.to_string())
        .collect();
    assert_eq!(listed, registered, "add new modules to `modules()`");
    // Same action types as the registry builds.
    for module in modules(&launcher) {
        let ours: Vec<_> = module
            .action_handlers(&safe_config(module.manifest()))
            .iter()
            .map(|h| h.definition().id.clone())
            .collect();
        let info = builtin(Os::current())
            .modules()
            .into_iter()
            .find(|m| m.manifest.id == module.manifest().id)
            .unwrap();
        let theirs: Vec<_> = info.action_types.iter().map(|t| t.id.clone()).collect();
        assert_eq!(ours, theirs, "{}", module.manifest().id);
    }
    // Every known finding names a real module or action type.
    let subjects: BTreeSet<String> = builtin(Os::current())
        .modules()
        .iter()
        .flat_map(|m| {
            std::iter::once(m.manifest.id.to_string())
                .chain(m.action_types.iter().map(|t| t.id.to_string()))
        })
        .collect();
    for (subject, _, reason) in KNOWN {
        assert!(subjects.contains(*subject), "unknown subject {subject}");
        assert!(!reason.is_empty(), "{subject} needs a reason");
    }
}

const AUDIT_MANIFESTS_CHECKS: &[&str] = &[
    "manifest-fields",
    "manifest-version",
    "manifest-platforms",
    "manifest-capabilities",
];

#[test]
fn manifests_are_complete() {
    let launcher = Arc::new(RecordingLauncher::new());
    let mut report = Report::new(AUDIT_MANIFESTS_CHECKS);
    audit_manifests(&modules(&launcher), &mut report);
    report.finish();
}

fn audit_manifests(subjects: &[Box<dyn FeatureModule>], report: &mut Report) {
    for module in subjects {
        let manifest = module.manifest();
        let id = manifest.id.as_str();
        if manifest.name.trim().is_empty() || manifest.description.trim().is_empty() {
            report.add(id, "manifest-fields", "name or description is empty");
        }
        if manifest.version != VERSION {
            report.add(
                id,
                "manifest-version",
                format!("version {} is not {VERSION}", manifest.version),
            );
        }
        let repeated = manifest
            .platforms
            .iter()
            .enumerate()
            .any(|(i, os)| manifest.platforms[..i].contains(os));
        if repeated {
            report.add(id, "manifest-platforms", "platforms are repeated");
        }
        let capabilities: BTreeSet<_> = manifest.capabilities.iter().collect();
        if capabilities.len() != manifest.capabilities.len() {
            report.add(id, "manifest-capabilities", "capabilities are repeated");
        }
        let actions = module.action_handlers(&safe_config(manifest));
        if !actions.is_empty() && manifest.capabilities.is_empty() {
            report.add(
                id,
                "manifest-capabilities",
                "has action types but declares no capability",
            );
        }
    }
}

fn local_name(id: &ActionTypeId) -> &str {
    id.as_str().split_once('.').map_or("", |(_, name)| name)
}

/// What the action's name says about its effect.
fn effect_problem(action: &ActionType) -> Option<String> {
    let name = local_name(&action.id);
    let segments: Vec<&str> = name.split('-').collect();
    let description = action.description.to_ascii_lowercase();
    let effect = action.node.effect;
    if name.starts_with("open") && effect != ActionEffect::Opens {
        return Some(format!("“{name}” opens something but is {effect:?}"));
    }
    let changing = [
        "up", "down", "restart", "run", "request", "delete", "merge", "create", "set", "write",
        "kill", "stop", "start",
    ];
    if effect == ActionEffect::ReadOnly && segments.iter().any(|s| changing.contains(s)) {
        return Some(format!("“{name}” sounds like a change but is read-only"));
    }
    let says_it = ["open", "show", "start"]
        .iter()
        .any(|word| description.contains(word));
    if effect == ActionEffect::Opens && !says_it {
        return Some("opens, but its description does not say what it opens or shows".into());
    }
    None
}

const AUDIT_ACTION_TYPES_CHECKS: &[&str] = &[
    "action-namespace",
    "action-fields",
    "action-icon",
    "action-outputs",
    "action-timeout",
    "action-effect",
    "action-cancellable",
    "action-retry",
    "parameter-label",
    "parameter-kind-renderable",
    "parameter-schema",
    "parameter-default",
    "parameter-keys",
    "secret-parameter",
    "credential-parameter",
    "minimal-parameters",
];

#[test]
fn action_types_are_described() {
    let launcher = Arc::new(RecordingLauncher::new());
    let mut report = Report::new(AUDIT_ACTION_TYPES_CHECKS);
    audit_action_types(&modules(&launcher), &mut report);
    report.finish();
}

fn audit_action_types(subjects: &[Box<dyn FeatureModule>], report: &mut Report) {
    for module in subjects {
        let manifest = module.manifest();
        let processes = manifest
            .capabilities
            .contains(&Capability::ProcessExecution)
            || manifest.capabilities.contains(&Capability::Network);
        for handler in module.action_handlers(&safe_config(manifest)) {
            let action = handler.definition();
            let id = action.id.as_str();
            if ModuleId::owner_of(&action.id).as_ref() != Some(&manifest.id) {
                report.add(id, "action-namespace", "not in its module's namespace");
            }
            if action.name.trim().is_empty()
                || action.description.trim().is_empty()
                || action.category.trim().is_empty()
            {
                report.add(
                    id,
                    "action-fields",
                    "name, description or category is empty",
                );
            }
            if action
                .node
                .icon
                .as_deref()
                .is_none_or(|i| i.trim().is_empty())
            {
                report.add(id, "action-icon", "no icon");
            }
            let keys: BTreeSet<_> = action.node.outputs.iter().map(|o| &o.key).collect();
            if keys.len() != action.node.outputs.len()
                || action
                    .node
                    .outputs
                    .iter()
                    .any(|o| o.key.is_empty() || o.label.trim().is_empty())
            {
                report.add(id, "action-outputs", "outputs are repeated or unlabelled");
            }
            if action.node.timeout_ms == 0 || action.node.timeout_ms > MAX_TIMEOUT_MS {
                report.add(
                    id,
                    "action-timeout",
                    format!(
                        "timeout {} ms is outside 1..={MAX_TIMEOUT_MS}",
                        action.node.timeout_ms
                    ),
                );
            }
            if let Some(problem) = effect_problem(action) {
                report.add(id, "action-effect", problem);
            }
            if processes && action.node.effect != ActionEffect::Opens && !action.node.cancellable {
                report.add(
                    id,
                    "action-cancellable",
                    "runs programs or uses the network but cannot be cancelled",
                );
            }
            if action.node.effect == ActionEffect::ReadOnly
                && !(action.node.retry_safe && action.node.idempotent)
            {
                report.add(
                    id,
                    "action-retry",
                    "read-only but not retry-safe and idempotent",
                );
            }
            let keys: BTreeSet<_> = action.parameters.iter().map(|p| &p.key).collect();
            if keys.len() != action.parameters.len() {
                report.add(id, "parameter-keys", "parameter keys are repeated");
            }
            for spec in &action.parameters {
                for (check, problem) in schema_problems(spec) {
                    report.add(id, check, problem);
                }
            }
            if let Err(errors) = action.normalize(&minimal(action)) {
                report.add(
                    id,
                    "minimal-parameters",
                    format!("defaults and required values do not validate: {errors:?}"),
                );
            }
        }
    }
}

/// Names of settings that only make sense on one computer.
fn machine_specific_name(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("allow")
        || [
            "path",
            "executable",
            "program",
            "host",
            "url",
            "directory",
            "folder",
            "private",
            "context",
        ]
        .iter()
        .any(|word| key.contains(word))
}

const AUDIT_SETTINGS_CHECKS: &[&str] = &[
    "setting-scope",
    "setting-machine-specific",
    "setting-permission",
    "parameter-label",
    "parameter-kind-renderable",
    "parameter-schema",
    "parameter-default",
    "secret-parameter",
    "credential-parameter",
];

#[test]
fn module_settings_declare_portability() {
    let launcher = Arc::new(RecordingLauncher::new());
    let mut report = Report::new(AUDIT_SETTINGS_CHECKS);
    audit_settings(&modules(&launcher), &mut report);
    report.finish();
}

fn audit_settings(subjects: &[Box<dyn FeatureModule>], report: &mut Report) {
    for module in subjects {
        let manifest = module.manifest();
        let id = manifest.id.as_str();
        for spec in &manifest.config {
            let declared = manifest.portability.settings.get(&spec.key);
            if declared.is_none() {
                report.add(
                    id,
                    "setting-scope",
                    format!("{} has no declared portability", spec.key),
                );
            }
            if machine_specific_name(&spec.key) && declared == Some(&SettingScope::Portable) {
                report.add(
                    id,
                    "setting-machine-specific",
                    format!("{} sounds machine-specific but is portable", spec.key),
                );
            }
            if spec.key.to_ascii_lowercase().starts_with("allow")
                && (spec.kind != ParameterKind::Boolean
                    || spec.default != Some(ParameterValue::Boolean(false))
                    || declared != Some(&SettingScope::MachineSpecific))
            {
                report.add(
                    id,
                    "setting-permission",
                    format!(
                        "{} must be a boolean, off by default and machine-specific",
                        spec.key
                    ),
                );
            }
            for (check, problem) in schema_problems(spec) {
                report.add(id, check, problem);
            }
        }
        // Scopes for settings that do not exist are leftovers.
        let keys: BTreeSet<&String> = manifest.config.iter().map(|s| &s.key).collect();
        for key in manifest.portability.settings.keys() {
            if !keys.contains(key) {
                report.add(
                    id,
                    "setting-scope",
                    format!("scope for unknown setting {key}"),
                );
            }
        }
    }
}

const AUDIT_RUNS_CHECKS: &[&str] = &[
    "run-panics",
    "run-denied-succeeds",
    "run-denied-opens",
    "run-failure-without-code",
    "run-undeclared-output",
    "run-missing-output",
    "run-secret-in-result",
    "run-credential-name-unchecked",
];

#[test]
fn actions_hold_their_contract_at_run_time() {
    let launcher = Arc::new(RecordingLauncher::new());
    let mut report = Report::new(AUDIT_RUNS_CHECKS);
    audit_runs(&modules(&launcher), &launcher, &mut report);
    report.finish();
}

fn audit_runs(
    subjects: &[Box<dyn FeatureModule>],
    launcher: &RecordingLauncher,
    report: &mut Report,
) {
    let world = world();
    let nothing = BTreeSet::new();
    for module in subjects {
        let manifest = module.manifest();
        let granted: BTreeSet<Capability> = manifest.capabilities.iter().copied().collect();
        for handler in module.action_handlers(&safe_config(manifest)) {
            let action = handler.definition().clone();
            let id = action.id.as_str();
            let Ok(parameters) = action.normalize(&minimal(&action)) else {
                // Reported by `action_types_are_described`.
                continue;
            };

            let before = launcher.plans().len();
            let denied = execute(handler.as_ref(), &parameters, &world, &nothing);
            let after = launcher.plans().len();
            let full = execute(handler.as_ref(), &parameters, &world, &granted);

            for (when, result) in [
                ("without capabilities", &denied),
                ("with capabilities", &full),
            ] {
                let Some(result) = result else {
                    report.add(id, "run-panics", format!("panicked {when}"));
                    continue;
                };
                if !result.is_success()
                    && result.status != deckpilot_core::ActionStatus::Skipped
                    && result.code.as_deref().is_none_or(str::is_empty)
                {
                    report.add(
                        id,
                        "run-failure-without-code",
                        format!("failed {when} without a code: {}", result.message),
                    );
                }
                let shown = format!("{} {:?}", result.message, result.data);
                if shown.contains(SECRET) {
                    report.add(id, "run-secret-in-result", format!("{when}: {shown}"));
                }
            }
            if let Some(result) = &denied
                && result.is_success()
            {
                report.add(
                    id,
                    "run-denied-succeeds",
                    format!("succeeded without any capability: {}", result.message),
                );
            }
            if after != before {
                report.add(
                    id,
                    "run-denied-opens",
                    "opened something without any capability",
                );
            }
            if let Some(result) = &full
                && result.is_success()
            {
                let declared: BTreeMap<&str, bool> = action
                    .node
                    .outputs
                    .iter()
                    .map(|o| (o.key.as_str(), o.always))
                    .collect();
                for key in result.data.keys() {
                    if !declared.contains_key(key.as_str()) {
                        report.add(
                            id,
                            "run-undeclared-output",
                            format!("returns `{key}`, which is not a declared output"),
                        );
                    }
                }
                for (key, always) in &declared {
                    if *always && !result.data.contains_key(*key) {
                        report.add(
                            id,
                            "run-missing-output",
                            format!("`{key}` is declared always present but is missing"),
                        );
                    }
                }
            }
            check_credential_name(handler.as_ref(), &parameters, &world, &granted, report);
        }
    }
}

/// A credential parameter must refuse something that is not a credential
/// name, when saved or when run.
fn check_credential_name(
    handler: &dyn ActionHandler,
    parameters: &ActionParameters,
    world: &World,
    granted: &BTreeSet<Capability>,
    report: &mut Report,
) {
    let action = handler.definition();
    if !action.parameters.iter().any(|p| p.key == "credential") {
        return;
    }
    let id = action.id.as_str();
    let bad = parameters
        .clone()
        .with("credential", ParameterValue::Text("Not A Name".into()));
    let saved = handler.check(&bad, &world.profile);
    let run = execute(handler, &bad, world, granted);
    let refused_when_run =
        run.is_some_and(|r| !r.is_success() && r.code.as_deref() == Some("invalid-parameters"));
    if saved.is_empty() && !refused_when_run {
        report.add(
            id,
            "run-credential-name-unchecked",
            "accepts a credential name that is not valid",
        );
    }
}

/// A module that breaks the contracts on purpose, to show that the checks
/// find what they claim to find.
mod broken {
    use super::*;
    use deckpilot_core::graph::{NodeSpec, OutputSpec, ValueKind};

    #[derive(Debug, Clone, Copy)]
    pub enum Mode {
        /// Opens, leaks the secret and returns undeclared data, with or
        /// without capabilities.
        Leaks,
        Panics,
        FailsWithoutCode,
        /// Only a broken schema.
        Schema,
    }

    pub struct Handler {
        pub definition: ActionType,
        pub mode: Mode,
        pub launcher: Arc<RecordingLauncher>,
    }

    impl ActionHandler for Handler {
        fn definition(&self) -> &ActionType {
            &self.definition
        }

        fn execute(&self, _: &ActionParameters, context: &ExecutionContext<'_>) -> ActionResult {
            match self.mode {
                Mode::Leaks => {
                    let plan = deckpilot_core::launch::LaunchPlan {
                        program: "contract".into(),
                        args: Vec::new(),
                        wait: false,
                    };
                    let _ = self.launcher.launch(&plan);
                    let secret = context
                        .credential(&CredentialId::new(CREDENTIAL).unwrap())
                        .map(|s| s.expose().to_owned())
                        .unwrap_or_default();
                    ActionResult::success(format!("token {secret}")).with_data("leak", secret)
                }
                Mode::Panics => panic!("broken on purpose"),
                Mode::FailsWithoutCode | Mode::Schema => ActionResult::failure("no code"),
            }
        }
    }

    pub struct Module {
        pub manifest: ModuleManifest,
        pub launcher: Arc<RecordingLauncher>,
    }

    fn text(key: &str, label: &str, multiline: bool) -> ParameterSpec {
        ParameterSpec {
            key: key.into(),
            label: label.into(),
            description: None,
            kind: ParameterKind::Text {
                multiline,
                max_length: Some(100),
            },
            required: false,
            default: None,
        }
    }

    fn action(id: &str, mode: Mode) -> ActionType {
        let mut node = NodeSpec {
            icon: Some("x".into()),
            outputs: vec![OutputSpec::new("declared", "Declared", ValueKind::Text)],
            timeout_ms: 1000,
            ..NodeSpec::default()
        };
        let mut parameters = vec![text("credential", "Credential", false)];
        let mut category = "Broken".to_owned();
        let mut description = "Opens something.".to_owned();
        if let Mode::Schema = mode {
            node.icon = None;
            node.timeout_ms = 0;
            node.outputs
                .push(OutputSpec::new("declared", "", ValueKind::Text));
            node.effect = ActionEffect::ReadOnly;
            node.retry_safe = false;
            category.clear();
            description = "Does things.".into();
            parameters = vec![
                text("apiToken", "Token", false),
                text("credential", "Credential", true),
                text("same", "", false),
                text("same", "Same", false),
                ParameterSpec {
                    key: "count".into(),
                    label: "Count".into(),
                    description: None,
                    kind: ParameterKind::Integer {
                        min: Some(5),
                        max: Some(1),
                    },
                    required: true,
                    default: Some(ParameterValue::Integer(9)),
                },
            ];
        }
        ActionType {
            id: ActionTypeId::new(id).unwrap(),
            name: "Broken".into(),
            description,
            category,
            parameters,
            requires_project: false,
            node,
        }
    }

    impl FeatureModule for Module {
        fn manifest(&self) -> &ModuleManifest {
            &self.manifest
        }

        fn action_handlers(&self, _: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
            [
                ("broken.open-thing", Mode::Schema),
                ("elsewhere.thing", Mode::Leaks),
                ("broken.crash", Mode::Panics),
                ("broken.silent", Mode::FailsWithoutCode),
            ]
            .into_iter()
            .map(|(id, mode)| {
                Box::new(Handler {
                    definition: action(id, mode),
                    mode,
                    launcher: self.launcher.clone(),
                }) as Box<dyn ActionHandler>
            })
            .collect()
        }
    }

    pub fn module(launcher: &Arc<RecordingLauncher>) -> Box<dyn FeatureModule> {
        Box::new(Module {
            manifest: ModuleManifest {
                id: ModuleId::new("broken").unwrap(),
                version: "0.0.0".into(),
                name: "Broken".into(),
                description: String::new(),
                platforms: vec![Os::Linux, Os::Linux],
                pages: Vec::new(),
                capabilities: vec![
                    Capability::Network,
                    Capability::Network,
                    Capability::Credentials,
                ],
                config: vec![
                    ParameterSpec {
                        key: "allowEverything".into(),
                        label: "Allow everything".into(),
                        description: None,
                        kind: ParameterKind::Boolean,
                        required: true,
                        default: Some(ParameterValue::Boolean(true)),
                    },
                    text("serverHost", "Server", false),
                ],
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: deckpilot_core::Portability {
                    version: 1,
                    settings: BTreeMap::from([
                        ("allowEverything".to_owned(), SettingScope::Portable),
                        ("ghost".to_owned(), SettingScope::Portable),
                    ]),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
            },
            launcher: launcher.clone(),
        })
    }
}

#[test]
fn the_checks_find_what_they_claim_to_find() {
    let launcher = Arc::new(RecordingLauncher::new());
    let subjects = vec![broken::module(&launcher)];
    let mut found: BTreeSet<&str> = BTreeSet::new();
    let mut run = |checks: &'static [&'static str], audit: &dyn Fn(&mut Report)| {
        let mut report = Report::new(checks);
        audit(&mut report);
        found.extend(report.found.iter().map(|(_, check)| *check));
    };
    run(AUDIT_MANIFESTS_CHECKS, &|r| audit_manifests(&subjects, r));
    run(AUDIT_ACTION_TYPES_CHECKS, &|r| {
        audit_action_types(&subjects, r);
    });
    run(AUDIT_SETTINGS_CHECKS, &|r| audit_settings(&subjects, r));
    run(AUDIT_RUNS_CHECKS, &|r| audit_runs(&subjects, &launcher, r));
    let every: BTreeSet<&str> = [
        AUDIT_MANIFESTS_CHECKS,
        AUDIT_ACTION_TYPES_CHECKS,
        AUDIT_SETTINGS_CHECKS,
        AUDIT_RUNS_CHECKS,
    ]
    .concat()
    .into_iter()
    .collect();
    // Every kind the app knows is drawable, so this one cannot be shown
    // with a real `ParameterKind`.
    let missed: Vec<_> = every
        .iter()
        .filter(|check| **check != "parameter-kind-renderable" && !found.contains(**check))
        .collect();
    assert!(missed.is_empty(), "checks that found nothing: {missed:?}");
}
