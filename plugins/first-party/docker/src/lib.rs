//! `deckpilot.docker`: the first-party Docker plugin, the replacement of
//! the built-in `docker` module (`docs/marketplace/module-migration.md` §7
//! X3b, X3c, §11 D1): Docker availability, containers, Compose projects
//! (status, up, down, restart) and bounded logs.
//!
//! **The same actions.** Every action of the built-in under the same name
//! (`docker.<action>` → `deckpilot.docker.<action>`), with the same
//! parameters (the project parameter is `projectId`, since a manifest key
//! is camel case), outputs, messages and error codes; `README.md` lists
//! the differences.
//!
//! **No process of its own.** Docker runs through `tool.run` (plugin API
//! 1.9): the agent runs the user's own `docker`, adds the Docker context
//! (of a bound Docker environment, or DeckPilot's setting) and, for
//! Compose, the bound folder or file itself, with a cleared environment
//! and its time and output limits (logs at most 64 KiB). The plugin only
//! chooses the subcommands and options its manifest declares and parses
//! the output. Starting, stopping and restarting a Compose project need
//! their own grant (`tools:docker:modify`, which replaces the built-in's
//! `allowStateChanges`) and a run the user (or their work session)
//! started; `compose down` never removes volumes or images.
//!
//! **Key state.** The agent allows no `tool.run` in a state call: keys
//! show what the last run found ([`state`]).
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;
use deckpilot_plugin_sdk::{
    PRIVILEGED_API_LEVEL, ResourceHandle, ResourceKind, ResourceResolve, Tool, ToolOutput,
};

pub mod cli;
pub mod state;

use cli::Container;

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.docker";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Compose project resource parameter.
pub const COMPOSE: &str = "compose";
/// The Docker environment resource parameter (new: where Docker runs
/// when no Compose project is involved).
pub const ENVIRONMENT: &str = "environment";
/// The project parameter (`project_id` of the built-in).
pub const PROJECT: &str = "projectId";
/// The action's Docker context name.
pub const CONTEXT: &str = "context";
/// Setting: the project resource that is the Compose project.
pub const COMPOSE_ROLE: &str = "composeRole";
/// Setting: time limit of Compose changes, in seconds (the agent's own
/// limit still applies; this can only shorten it).
pub const TIMEOUT: &str = "timeoutSeconds";
/// Most log lines asked for (the built-in's bound; the agent tails at
/// most [`MAX_TAIL`]).
pub const MAX_LOG_LINES: i64 = 2000;
/// The agent's highest `--tail`.
pub const MAX_TAIL: i64 = 1000;
const JSON: &str = "--format={{json .}}";

/// Stable error codes: the built-in's, and the agent's the plugin passes
/// on.
pub mod codes {
    pub const DOCKER_NOT_FOUND: &str = "docker-not-found";
    pub const DAEMON_UNAVAILABLE: &str = "daemon-unavailable";
    pub const PERMISSION_DENIED: &str = "permission-denied";
    pub const CONTEXT_NOT_FOUND: &str = "context-not-found";
    pub const COMPOSE_UNAVAILABLE: &str = "compose-unavailable";
    pub const COMPOSE_FILE_NOT_FOUND: &str = "compose-file-not-found";
    pub const NOT_FOUND: &str = "not-found";
    pub const INVALID_NAME: &str = "invalid-name";
    /// Starting and stopping is not granted (`tools:docker:modify`).
    pub const CHANGES_DISABLED: &str = "changes-disabled";
    pub const PROJECT_NOT_IN_PROFILE: &str = "project-not-in-profile";
    pub const DOCKER_FAILED: &str = "docker-failed";
    /// A state change outside a run the user started.
    pub const NOT_USER_INITIATED: &str = "not-user-initiated";
    /// The action's `context` names another context than the one the
    /// agent runs Docker in.
    pub const CONTEXT_UNSUPPORTED: &str = "context-unsupported";
    /// `projectName`: the agent decides the Compose project name.
    pub const PROJECT_NAME_UNSUPPORTED: &str = "project-name-unsupported";
    /// Neither a Docker environment, a Compose project nor a project's
    /// Compose resource says where Docker runs.
    pub const ENVIRONMENT_MISSING: &str = "docker-environment-missing";
    pub const NO_ACTIVE_PROJECT: &str = "no-active-project";
    pub const CANCELLED: &str = "cancelled";
}

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches`; the `resource` parameters arrive as
/// handles, so they are text here).
pub fn plugin() -> Plugin {
    let context = || Param::text(CONTEXT).max_length(128);
    let environment = || Param::text(ENVIRONMENT);
    let compose = |action: Action| {
        action
            .param(Param::text(COMPOSE))
            .param(Param::text(PROJECT))
            .param(Param::text("projectName").max_length(64))
            .param(context())
    };
    let services = || Param::text("services");
    Plugin::new(ID, VERSION)
        .requires_api(PRIVILEGED_API_LEVEL)
        .action(
            Action::new("status", status)
                .param(context())
                .param(environment())
                .state(state::status),
        )
        .action(
            Action::new("containers", containers)
                .param(Param::boolean("all").default(false))
                .param(Param::text("name").max_length(128))
                .param(context())
                .param(environment())
                .state(state::containers),
        )
        .action(compose(Action::new("compose-status", compose_status)).state(state::compose_status))
        .action(compose(Action::new("compose-up", compose_up)).param(services()))
        .action(
            compose(Action::new("compose-down", compose_down))
                .param(Param::boolean("removeOrphans").default(false)),
        )
        .action(compose(Action::new("compose-restart", compose_restart)).param(services()))
        .action(
            compose(
                Action::new("logs", logs)
                    .param(Param::text("container").max_length(128))
                    .param(Param::text("service").max_length(128))
                    .param(
                        Param::integer("lines")
                            .min(1.0)
                            .max(2000.0)
                            .default(100_i64),
                    )
                    .param(Param::text("since").max_length(40)),
            )
            .param(environment()),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);

// ------------------------------------------------------------ the agent

/// The reason code at the start of the agent's refusal (`no-active-project:
/// …`), when there is one.
pub fn reason(error: &CapabilityError) -> Option<&str> {
    let (code, _) = error.detail().split_once(": ")?;
    (!code.is_empty()
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    .then_some(code)
}

fn reason_message(error: &CapabilityError) -> &str {
    error
        .detail()
        .split_once(": ")
        .map_or(error.detail(), |(_, rest)| rest)
}

fn fail(code: &str, message: impl Into<String>) -> PluginError {
    PluginError::new(code, message)
}

/// A text parameter, trimmed, `None` when empty or not one of the
/// action's.
pub(crate) fn optional(args: &Args<'_>, key: &str) -> Option<String> {
    match args.values().get(key) {
        Some(deckpilot_plugin_sdk::ParamValue::String(text)) => {
            Some(text.trim().to_owned()).filter(|v| !v.is_empty())
        }
        _ => None,
    }
}

fn services(args: &Args<'_>) -> Vec<String> {
    optional(args, "services")
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The built-in's `names`: every name the action passes to Docker, before
/// anything runs.
pub fn names(args: &Args<'_>) -> Result<(), PluginError> {
    for key in [CONTEXT, "container", "service"] {
        if let Some(value) = optional(args, key)
            && !cli::valid_name(&value)
        {
            return Err(fail(
                codes::INVALID_NAME,
                format!("“{value}” is not a valid {key} name"),
            ));
        }
    }
    if let Some(value) = optional(args, "projectName")
        && !cli::valid_project(&value)
    {
        return Err(fail(
            codes::INVALID_NAME,
            format!("“{value}” is not a valid Compose project name (lower case, digits, - and _)"),
        ));
    }
    if let Some(value) = optional(args, "since")
        && !cli::valid_since(&value)
    {
        return Err(fail(
            codes::INVALID_NAME,
            format!("“{value}” is not a valid time"),
        ));
    }
    for service in services(args) {
        if !cli::valid_name(&service) {
            return Err(fail(
                codes::INVALID_NAME,
                format!("“{service}” is not a valid service name"),
            ));
        }
    }
    if let Some(value) = optional(args, "name")
        && !cli::valid_name(&value)
    {
        return Err(fail(
            codes::INVALID_NAME,
            format!("“{value}” is not a valid container name"),
        ));
    }
    Ok(())
}

/// Where Docker runs: a handle of this invocation, its label, and the
/// project the Compose resource came from.
#[derive(Debug, Clone)]
pub struct Target {
    pub handle: String,
    pub label: String,
    pub project_key: Option<String>,
}

impl Target {
    fn of(handle: ResourceHandle, project_key: Option<String>) -> Self {
        Self {
            handle: handle.handle,
            label: handle.label,
            project_key,
        }
    }
}

fn resolve_failure(error: &CapabilityError, explicit_project: bool, compose: bool) -> PluginError {
    match reason(error) {
        Some(codes::NO_ACTIVE_PROJECT) => fail(
            codes::NO_ACTIVE_PROJECT,
            "no project is active and no Compose project is set",
        ),
        Some("not-permitted") if explicit_project => fail(
            codes::PROJECT_NOT_IN_PROFILE,
            "the project does not belong to this profile",
        ),
        Some("not-found") if compose => fail(codes::COMPOSE_FILE_NOT_FOUND, reason_message(error)),
        Some("not-permitted") | None => PluginError::from(error.clone()),
        Some(code) => fail(code, reason_message(error)),
    }
}

/// A handle for the resource parameter `key`, `None` when it is not set.
fn param_handle(
    ctx: &mut Context<'_>,
    key: &str,
    kinds: &[ResourceKind],
) -> Result<Option<ResourceHandle>, PluginError> {
    match ctx.resource_param(key, kinds) {
        Ok(handle) => Ok(Some(handle)),
        Err(e) if reason(&e) == Some("resource-not-set") => Ok(None),
        Err(e) => Err(resolve_failure(&e, false, key == COMPOSE)),
    }
}

/// The Compose project, as the built-in finds it: the `compose` parameter
/// (a folder or a Compose file), else the `projectId` parameter's project
/// or the run's own, and its Compose resource (setting `composeRole`).
/// `with_param`: the action has the `compose` parameter.
pub fn compose_target(
    ctx: &mut Context<'_>,
    args: &Args<'_>,
    with_param: bool,
) -> Result<Target, PluginError> {
    const KINDS: [ResourceKind; 2] = [ResourceKind::Directory, ResourceKind::File];
    if with_param && let Some(handle) = param_handle(ctx, COMPOSE, &KINDS)? {
        return Ok(Target::of(handle, None));
    }
    let project = optional(args, PROJECT);
    let role = ctx
        .settings()
        .ok()
        .and_then(|s| s.text(COMPOSE_ROLE).map(|r| r.trim().to_owned()))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "compose".to_owned());
    let handle = ctx
        .resolve_resource(ResourceResolve {
            param: None,
            role: Some(role),
            project_id: project.clone(),
            kinds: KINDS.to_vec(),
        })
        .map_err(|e| resolve_failure(&e, project.is_some(), true))?;
    let key = project_key(ctx, project.as_deref());
    Ok(Target::of(handle, key))
}

/// Where Docker runs outside Compose: the `environment` parameter (a
/// Docker environment, whose context is used, or a folder), else the
/// Compose project the action names or the project's.
pub fn environment_target(
    ctx: &mut Context<'_>,
    args: &Args<'_>,
    compose_param: bool,
) -> Result<Target, PluginError> {
    if let Some(handle) = param_handle(
        ctx,
        ENVIRONMENT,
        &[ResourceKind::DockerEnvironment, ResourceKind::Directory],
    )? {
        return Ok(Target::of(handle, None));
    }
    compose_target(ctx, args, compose_param).map_err(|e| {
        if [
            codes::NO_ACTIVE_PROJECT,
            "resource-role-missing",
            "resource-unbound",
        ]
        .contains(&e.code())
        {
            fail(
                codes::ENVIRONMENT_MISSING,
                format!(
                    "Docker runs only where you bound it: choose a Docker environment (or a \
                     Compose project) for the action ({})",
                    e.message()
                ),
            )
        } else {
            e
        }
    })
}

fn project_key(ctx: &mut Context<'_>, explicit: Option<&str>) -> Option<String> {
    let project = ctx.profile().ok()?.project?;
    explicit
        .is_none_or(|id| id == project.id)
        .then_some(project.key)
}

/// A refused or failed `tool.run`.
fn tool_failure(error: &CapabilityError, modifies: bool) -> PluginError {
    match reason(error) {
        Some("tool-not-found") => fail(
            codes::DOCKER_NOT_FOUND,
            "Docker was not found; install it or set its location in DeckPilot's settings",
        ),
        Some("timed-out") => fail(
            deckpilot_plugin_sdk::codes::TIMEOUT,
            format!("docker: {}", reason_message(error)),
        ),
        Some(codes::CANCELLED) => fail(
            codes::CANCELLED,
            format!("docker: {}", reason_message(error)),
        ),
        Some(codes::NOT_USER_INITIATED) => fail(codes::NOT_USER_INITIATED, reason_message(error)),
        // The agent finds the compose file in the bound folder.
        Some("tool-invalid-cwd") if reason_message(error).contains("compose") => {
            fail(codes::COMPOSE_FILE_NOT_FOUND, reason_message(error))
        }
        // The broker refuses a change outside a run the user started.
        _ if modifies
            && error.kind() == CapabilityErrorKind::NotPermitted
            && error.detail().contains("the user started") =>
        {
            fail(codes::NOT_USER_INITIATED, error.detail())
        }
        _ if modifies && error.kind() == CapabilityErrorKind::NotPermitted => fail(
            codes::CHANGES_DISABLED,
            "starting and stopping services is not allowed on this computer; allow the Docker \
             plugin to start, stop and restart containers in Settings › Plugins › Docker",
        ),
        _ if error.kind() == CapabilityErrorKind::NotPermitted => PluginError::from(error.clone()),
        _ => fail(codes::DOCKER_FAILED, format!("docker: {}", error.detail())),
    }
}

/// Runs docker in `target`; the output, whatever the exit status.
fn docker(
    ctx: &mut Context<'_>,
    target: &Target,
    args: &[&str],
    timeout_ms: Option<u32>,
    modifies: bool,
) -> Result<ToolOutput, PluginError> {
    ctx.run_tool(Tool::Docker, args, &target.handle, timeout_ms)
        .map_err(|e| tool_failure(&e, modifies))
}

/// Runs docker and turns a failure into its classified error.
fn checked(
    ctx: &mut Context<'_>,
    target: &Target,
    args: &[&str],
    timeout_ms: Option<u32>,
    modifies: bool,
) -> Result<ToolOutput, PluginError> {
    let output = docker(ctx, target, args, timeout_ms, modifies)?;
    if output.exit == Some(0) {
        return Ok(output);
    }
    let first = output
        .stderr
        .trim()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    Err(fail(
        cli::classify(&output.stderr),
        format!("docker: {first}"),
    ))
}

/// The context Docker runs in: what `docker context show` says with the
/// agent's context. The action's `context` must be that one (the agent
/// decides the context: a bound Docker environment's, or DeckPilot's
/// setting).
fn context_name(
    ctx: &mut Context<'_>,
    args: &Args<'_>,
    target: &Target,
) -> Result<String, PluginError> {
    let output = checked(ctx, target, &["context", "show"], None, false)?;
    let name = output.stdout.trim().to_owned();
    if !cli::valid_name(&name) {
        return Err(fail(
            codes::DOCKER_FAILED,
            "Docker reported no current context",
        ));
    }
    match optional(args, CONTEXT) {
        Some(wanted) if wanted != name => Err(fail(
            codes::CONTEXT_UNSUPPORTED,
            format!(
                "Docker runs in the context {name} here; bind a Docker environment for the \
                 context {wanted} and choose it for the action instead"
            ),
        )),
        _ => Ok(name),
    }
}

fn refuse_project_name(args: &Args<'_>) -> Result<(), PluginError> {
    match optional(args, "projectName") {
        Some(name) => Err(fail(
            codes::PROJECT_NAME_UNSUPPORTED,
            format!(
                "the Compose project name {name} cannot be chosen per action: DeckPilot runs \
                 Compose with the project of the bound folder or file"
            ),
        )),
        None => Ok(()),
    }
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn with_compose(output: Output, target: &Target) -> Output {
    match &target.project_key {
        Some(key) => output.with("projectKey", key.clone()),
        None => output,
    }
}

// -------------------------------------------------------------- actions

fn status(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    names(args)?;
    let result = status_of(ctx, args);
    state::remember(
        ctx,
        &state::status_key(args),
        result.as_ref().map(state::availability),
    );
    result
}

/// Never fails because Docker is unavailable: that is the answer.
fn status_of(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let target = environment_target(ctx, args, false)?;
    let unavailable = |reason: &PluginError, context: Option<&str>| {
        let mut output = Output::message(format!("Docker is not available: {}", reason.message()))
            .with("available", false)
            .with("reason", reason.code());
        if let Some(name) = context {
            output = output.with(CONTEXT, name);
        }
        output
    };
    let context = match context_name(ctx, args, &target) {
        Ok(name) => name,
        Err(e)
            if [
                deckpilot_plugin_sdk::codes::NOT_PERMITTED,
                codes::CONTEXT_UNSUPPORTED,
                codes::INVALID_NAME,
            ]
            .contains(&e.code()) =>
        {
            return Err(e);
        }
        Err(e) => return Ok(unavailable(&e, None)),
    };
    let output = match docker(ctx, &target, &["version", JSON], None, false) {
        Ok(output) => output,
        Err(e) => return Ok(unavailable(&e, Some(&context))),
    };
    let versions = cli::parse_versions(&output.stdout);
    let client = versions.as_ref().map(|v| v.client.version.clone());
    let server = versions
        .as_ref()
        .and_then(|v| v.server.clone())
        .filter(|s| !s.version.is_empty());
    let Some(server) = server.filter(|_| output.exit == Some(0)) else {
        let reason = fail(
            cli::classify(&output.stderr),
            output
                .stderr
                .trim()
                .lines()
                .next()
                .unwrap_or("the engine did not answer")
                .to_owned(),
        );
        let mut failed = unavailable(&reason, Some(&context));
        if let Some(version) = client {
            failed = failed.with("clientVersion", version);
        }
        return Ok(failed);
    };
    let compose = docker(
        ctx,
        &target,
        &["compose", "version", "--short"],
        None,
        false,
    )
    .ok()
    .filter(|o| o.exit == Some(0))
    .map(|o| o.stdout.trim().to_owned());
    let mut result = Output::message(format!(
        "Docker {} in context {context}{}",
        server.version,
        compose
            .as_ref()
            .map(|v| format!(", Compose {v}"))
            .unwrap_or_default()
    ))
    .with(CONTEXT, context)
    .with("available", true)
    .with("serverVersion", server.version.clone())
    .with("serverOs", server.os.clone());
    if let Some(version) = client {
        result = result.with("clientVersion", version);
    }
    if let Some(version) = compose {
        result = result.with("composeVersion", version);
    }
    Ok(result)
}

/// Containers from `docker ps` or `compose ps` output; a line cut by the
/// output limit is left out.
fn parse(output: &ToolOutput) -> Result<Vec<Container>, PluginError> {
    let text = if output.truncated && !output.stdout.trim_start().starts_with('[') {
        output.stdout.rsplit_once('\n').map_or("", |(kept, _)| kept)
    } else {
        output.stdout.as_str()
    };
    cli::parse_containers(text).map_err(|e| fail(codes::DOCKER_FAILED, e.to_string()))
}

fn lines(containers: &[Container]) -> String {
    containers
        .iter()
        .map(Container::line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn containers(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    names(args)?;
    let result = containers_of(ctx, args);
    state::remember(
        ctx,
        &state::containers_key(args),
        result.as_ref().map(state::running),
    );
    result
}

fn containers_of(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let target = environment_target(ctx, args, false)?;
    let context = context_name(ctx, args, &target)?;
    let all: bool = args.get("all")?;
    let filter = optional(args, "name").map(|name| format!("--filter=name={name}"));
    let mut command = vec!["ps", "--no-trunc", JSON];
    if all {
        command.push("--all");
    }
    if let Some(filter) = &filter {
        command.push(filter);
    }
    let output = checked(ctx, &target, &command, None, false)?;
    let containers = parse(&output)?;
    let running = containers.iter().filter(|c| c.running()).count();
    Ok(Output::message(format!(
        "{} containers, {running} running, in context {context}",
        containers.len()
    ))
    .with("count", count(containers.len()))
    .with("running", count(running))
    .with(
        "names",
        containers
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(","),
    )
    .with("containers", lines(&containers))
    .with(CONTEXT, context))
}

/// `compose ps` of `target`: the containers.
fn compose_ps(ctx: &mut Context<'_>, target: &Target) -> Result<Vec<Container>, PluginError> {
    let output = checked(
        ctx,
        target,
        &["compose", "ps", "--all", "--format=json"],
        None,
        false,
    )?;
    parse(&output)
}

fn compose_status(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    names(args)?;
    let result = (|| {
        refuse_project_name(args)?;
        let target = compose_target(ctx, args, true)?;
        let context = context_name(ctx, args, &target)?;
        let containers = compose_ps(ctx, &target)?;
        Ok((describe_compose(&containers, &context, &target), containers))
    })();
    state::remember(
        ctx,
        &state::compose_key(args),
        result.as_ref().map(|(_, c)| state::compose_counts(c)),
    );
    result.map(|(output, _)| output)
}

/// The message and outputs of `compose-status`.
pub fn describe_compose(containers: &[Container], context: &str, target: &Target) -> Output {
    let running = containers.iter().filter(|c| c.running()).count();
    let stopped = containers.len() - running;
    let output = Output::message(format!(
        "{running} of {} containers running in context {context}",
        containers.len()
    ))
    .with("services", count(containers.len()))
    .with("running", count(running))
    .with("stopped", count(stopped))
    .with("allRunning", stopped == 0 && running > 0)
    .with("containers", lines(containers))
    .with(CONTEXT, context);
    with_compose(output, target)
}

/// What a Compose change does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Up,
    Down,
    Restart,
}

/// The compose command of a change, without destructive flags: `down`
/// never removes volumes or images (the built-in's `compose_change`).
pub fn compose_change(change: Change, services: &[String], remove_orphans: bool) -> Vec<String> {
    let mut args: Vec<String> = match change {
        Change::Up => vec!["compose".into(), "up".into(), "--detach".into()],
        Change::Down => vec!["compose".into(), "down".into()],
        Change::Restart => vec!["compose".into(), "restart".into()],
    };
    if remove_orphans && change == Change::Down {
        args.push("--remove-orphans".into());
    }
    if change != Change::Down && !services.is_empty() {
        args.push("--".into());
        args.extend(services.iter().cloned());
    }
    args
}

fn compose_up(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    change(ctx, args, Change::Up)
}

fn compose_down(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    change(ctx, args, Change::Down)
}

fn compose_restart(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    change(ctx, args, Change::Restart)
}

fn change(ctx: &mut Context<'_>, args: &Args<'_>, change: Change) -> ActionResult {
    names(args)?;
    refuse_project_name(args)?;
    let target = compose_target(ctx, args, true)?;
    let context = context_name(ctx, args, &target)?;
    let services = if change == Change::Down {
        Vec::new()
    } else {
        services(args)
    };
    let remove_orphans = change == Change::Down && args.get::<bool>("removeOrphans")?;
    let command = compose_change(change, &services, remove_orphans);
    let command: Vec<&str> = command.iter().map(String::as_str).collect();
    let timeout = ctx
        .settings()
        .ok()
        .and_then(|s| s.integer(TIMEOUT))
        .unwrap_or(300)
        .clamp(10, 1800);
    let timeout = u32::try_from(timeout * 1000).unwrap_or(300_000);
    let output = checked(ctx, &target, &command, Some(timeout), true)?;
    // The keys of this project show the new state: one read after the
    // change (best effort).
    if let Ok(containers) = compose_ps(ctx, &target) {
        state::remember(
            ctx,
            &state::compose_key(args),
            Ok(state::compose_counts(&containers)),
        );
    }
    let what = if services.is_empty() || change == Change::Down {
        "the project".to_owned()
    } else {
        services.join(", ")
    };
    let done = match change {
        Change::Up => "started",
        Change::Down => "stopped and removed (volumes kept)",
        Change::Restart => "restarted",
    };
    let result = Output::message(format!("{what} {done} in context {context}"))
        // Compose reports progress on stderr.
        .with(
            "output",
            format!("{}{}", output.stdout, output.stderr)
                .trim()
                .to_owned(),
        )
        .with(CONTEXT, context);
    Ok(with_compose(result, &target))
}

fn logs(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    names(args)?;
    let lines: i64 = args.get("lines")?;
    let tail = format!("--tail={}", lines.clamp(1, MAX_LOG_LINES).min(MAX_TAIL));
    let since = optional(args, "since").map(|s| format!("--since={s}"));
    let (command, target, compose) = match (optional(args, "container"), optional(args, "service"))
    {
        (Some(container), None) => {
            let target = environment_target(ctx, args, true)?;
            let mut command = vec!["logs".to_owned(), tail];
            command.extend(since);
            command.extend(["--".into(), container]);
            (command, target, false)
        }
        (None, Some(service)) => {
            refuse_project_name(args)?;
            let target = compose_target(ctx, args, true)?;
            let mut command = vec![
                "compose".to_owned(),
                "logs".into(),
                "--no-color".into(),
                tail,
            ];
            command.extend(since);
            command.extend(["--".into(), service]);
            (command, target, true)
        }
        _ => {
            return Err(fail(
                deckpilot_plugin_sdk::codes::INVALID_PARAMETERS,
                "set either a container or a Compose service",
            ));
        }
    };
    let context = context_name(ctx, args, &target)?;
    let command: Vec<&str> = command.iter().map(String::as_str).collect();
    let output = checked(ctx, &target, &command, None, false)?;
    let (logs, count_lines) = merge_logs(&output.stdout, &output.stderr);
    let result = Output::message(format!("{count_lines} log lines"))
        .with("lineCount", count(count_lines))
        .with("truncated", output.truncated)
        .with("logs", logs)
        .with(CONTEXT, context);
    Ok(if compose {
        with_compose(result, &target)
    } else {
        result
    })
}

/// A container logs to both streams; both are kept, stdout first.
pub fn merge_logs(stdout: &str, stderr: &str) -> (String, usize) {
    let mut logs = stdout.to_owned();
    if !stderr.is_empty() {
        if !logs.is_empty() && !logs.ends_with('\n') {
            logs.push('\n');
        }
        logs.push_str(stderr);
    }
    let logs = logs.trim_end().to_owned();
    let lines = logs.lines().count();
    (logs, lines)
}

#[cfg(test)]
mod tests;
