//! The action types of the `docker` module.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use deckpilot_core::graph::{ActionEffect, MAX_TIMEOUT_MS, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::process::{ProcessLimits, ProcessOutput, minimal_environment};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, ButtonState,
    ExecutionContext, ParameterKind, ParameterSpec, ParameterValue, Profile, RefreshPolicy,
    ResourceKind, StateError,
};

use crate::cli::{self, Change, ComposeTarget};
use crate::{DockerSettings, codes};

pub const STATUS: &str = "docker.status";
pub const CONTAINERS: &str = "docker.containers";
pub const COMPOSE_STATUS: &str = "docker.compose-status";
pub const COMPOSE_UP: &str = "docker.compose-up";
pub const COMPOSE_DOWN: &str = "docker.compose-down";
pub const COMPOSE_RESTART: &str = "docker.compose-restart";
pub const LOGS: &str = "docker.logs";

const COMPOSE: &str = "compose";
const PROJECT: &str = "project_id";
const CONTEXT: &str = "context";

/// Time limit of commands that only read.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Most log lines fetched.
pub const MAX_LOG_LINES: i64 = 2000;
/// Bytes of log output kept.
const MAX_LOG_BYTES: usize = 64 * 1024;

/// Variables Docker needs besides the minimal environment: which engine
/// and context to use, its configuration and TLS material, and the SSH
/// agent for `ssh://` contexts.
const DOCKER_ENVIRONMENT: &[&str] = &[
    "DOCKER_HOST",
    "DOCKER_CONTEXT",
    "DOCKER_CONFIG",
    "DOCKER_CERT_PATH",
    "DOCKER_TLS_VERIFY",
    "SSH_AUTH_SOCK",
];

type Failure = ActionResult;

fn fail(code: &str, message: impl Into<String>) -> Failure {
    ActionResult::error(code, message)
}

/// # Panics
///
/// Never for the constants of this module.
pub fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("the action type id is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Status,
    Containers,
    ComposeStatus,
    Change(Change),
    Logs,
}

pub fn handlers(settings: &Arc<DockerSettings>) -> Vec<Box<dyn ActionHandler>> {
    [
        Kind::Status,
        Kind::Containers,
        Kind::ComposeStatus,
        Kind::Change(Change::Up),
        Kind::Change(Change::Down),
        Kind::Change(Change::Restart),
        Kind::Logs,
    ]
    .into_iter()
    .map(|kind| {
        Box::new(DockerAction {
            definition: definition(kind),
            kind,
            settings: settings.clone(),
        }) as Box<dyn ActionHandler>
    })
    .collect()
}

fn parameter(key: &str, label: &str, kind: ParameterKind, description: &str) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: label.into(),
        description: Some(description.into()),
        kind,
        required: false,
        default: None,
    }
}

fn text(max_length: usize, multiline: bool) -> ParameterKind {
    ParameterKind::Text {
        multiline,
        max_length: Some(max_length),
    }
}

fn with_default(mut spec: ParameterSpec, value: ParameterValue) -> ParameterSpec {
    spec.required = true;
    spec.default = Some(value);
    spec
}

fn context_parameter() -> ParameterSpec {
    parameter(
        CONTEXT,
        "Docker context",
        text(128, false),
        "Empty: the context set in Settings › Docker, or Docker's current context.",
    )
}

fn compose_parameters() -> Vec<ParameterSpec> {
    vec![
        parameter(
            COMPOSE,
            "Compose project",
            ParameterKind::Resource { kind: None },
            "A resource bound to the Compose file or its folder. Empty: the project's compose \
             resource.",
        ),
        parameter(
            PROJECT,
            "Project",
            ParameterKind::Project,
            "Whose compose resource, when none is set. Empty: the workflow's current project.",
        ),
        parameter(
            "projectName",
            "Compose project name",
            text(64, false),
            "Passed as -p. Empty: Compose's own choice (usually the folder name).",
        ),
        context_parameter(),
    ]
}

fn services_parameter(description: &str) -> ParameterSpec {
    parameter(
        "services",
        "Services",
        ParameterKind::Lines {
            item_label: Some("Service".into()),
            max_items: Some(50),
            max_length: Some(128),
        },
        description,
    )
}

fn out(key: &str, label: &str, kind: ValueKind) -> OutputSpec {
    OutputSpec::new(key, label, kind)
}

/// `docker.status` succeeds without Docker, when the current context cannot
/// be asked; its `context` is then left out.
fn status_node() -> NodeSpec {
    let mut spec = node(
        "container",
        vec![
            out("available", "Available", ValueKind::Boolean),
            out("reason", "Why not available", ValueKind::Text).optional(),
            out("clientVersion", "Client version", ValueKind::Text).optional(),
            out("serverVersion", "Engine version", ValueKind::Text).optional(),
            out("serverOs", "Engine OS", ValueKind::Text).optional(),
            out("composeVersion", "Compose version", ValueKind::Text).optional(),
        ],
        60_000,
        ActionEffect::ReadOnly,
    );
    for output in &mut spec.outputs {
        if output.key == CONTEXT {
            output.always = false;
        }
    }
    spec
}

fn node(icon: &str, outputs: Vec<OutputSpec>, timeout_ms: u32, effect: ActionEffect) -> NodeSpec {
    let mut all = outputs;
    all.push(out(CONTEXT, "Docker context", ValueKind::Text));
    NodeSpec {
        icon: Some(icon.into()),
        outputs: all,
        timeout_ms,
        retry_safe: true,
        idempotent: effect == ActionEffect::ReadOnly,
        cancellable: true,
        effect,
        ..NodeSpec::default()
    }
}

fn compose_outputs() -> Vec<OutputSpec> {
    vec![
        out("project", "Compose project", ValueKind::Text).optional(),
        out("projectKey", "Project", ValueKind::ProjectKey).optional(),
    ]
}

#[allow(clippy::too_many_lines)] // One table of the seven action types.
fn definition(kind: Kind) -> ActionType {
    // A workflow node runs at most MAX_TIMEOUT_MS; the module's longer time
    // limit applies when a change runs outside a workflow.
    let change_timeout = MAX_TIMEOUT_MS;
    let (id_value, name, description, parameters, spec) = match kind {
        Kind::Status => (
            STATUS,
            "Docker status",
            "Whether Docker is installed and its engine reachable in the context, with the \
             client, engine and Compose versions. Succeeds when Docker is unavailable; \
             `available` says so and `reason` why.",
            vec![context_parameter()],
            status_node(),
        ),
        Kind::Containers => (
            CONTAINERS,
            "Containers",
            "The containers of the context: names, states, images and status.",
            vec![
                with_default(
                    parameter(
                        "all",
                        "Include stopped",
                        ParameterKind::Boolean,
                        "Off: running containers only.",
                    ),
                    ParameterValue::Boolean(false),
                ),
                parameter(
                    "name",
                    "Name contains",
                    text(128, false),
                    "Only containers whose name contains this.",
                ),
                context_parameter(),
            ],
            node(
                "server",
                vec![
                    out("count", "Containers", ValueKind::Integer),
                    out("running", "Running", ValueKind::Integer),
                    out("names", "Names", ValueKind::Text),
                    out(
                        "containers",
                        "Containers (name, state, image, status)",
                        ValueKind::Text,
                    ),
                ],
                30_000,
                ActionEffect::ReadOnly,
            ),
        ),
        Kind::ComposeStatus => {
            let mut outputs = vec![
                out("services", "Containers", ValueKind::Integer),
                out("running", "Running", ValueKind::Integer),
                out("stopped", "Not running", ValueKind::Integer),
                out("allRunning", "All running", ValueKind::Boolean),
                out(
                    "containers",
                    "Containers (service, state, image, status)",
                    ValueKind::Text,
                ),
            ];
            outputs.extend(compose_outputs());
            (
                COMPOSE_STATUS,
                "Compose status",
                "The containers of a Compose project and whether they are running.",
                compose_parameters(),
                node("list-checks", outputs, 30_000, ActionEffect::ReadOnly),
            )
        }
        Kind::Change(change) => {
            let mut parameters = compose_parameters();
            let (id_value, name, description) = match change {
                Change::Up => {
                    parameters.push(services_parameter(
                        "One per line. Empty: every service of the project.",
                    ));
                    (
                        COMPOSE_UP,
                        "Compose up",
                        "Starts a Compose project in the background (up --detach). Must be \
                         allowed in Settings › Docker.",
                    )
                }
                Change::Down => {
                    parameters.push(with_default(
                        parameter(
                            "removeOrphans",
                            "Remove orphan containers",
                            ParameterKind::Boolean,
                            "Also removes containers of services no longer in the Compose file.",
                        ),
                        ParameterValue::Boolean(false),
                    ));
                    (
                        COMPOSE_DOWN,
                        "Compose down",
                        "Stops and removes a Compose project's containers and networks. \
                         Volumes and images are kept. Must be allowed in Settings › Docker.",
                    )
                }
                Change::Restart => {
                    parameters.push(services_parameter(
                        "One per line. Empty: every service of the project.",
                    ));
                    (
                        COMPOSE_RESTART,
                        "Compose restart",
                        "Restarts a Compose project's services. Must be allowed in Settings › \
                         Docker.",
                    )
                }
            };
            let mut outputs = vec![out("output", "Compose output", ValueKind::Text).sensitive()];
            outputs.extend(compose_outputs());
            let mut spec = node(
                match change {
                    Change::Up => "play",
                    Change::Down => "power",
                    Change::Restart => "refresh-cw",
                },
                outputs,
                change_timeout,
                ActionEffect::Modifies,
            );
            // Starting or stopping again ends in the same state; another
            // restart restarts again.
            spec.idempotent = change != Change::Restart;
            (id_value, name, description, parameters, spec)
        }
        Kind::Logs => {
            let mut parameters = vec![
                parameter(
                    "container",
                    "Container",
                    text(128, false),
                    "A container name or id. Or leave it empty and choose a Compose service.",
                ),
                parameter(
                    "service",
                    "Compose service",
                    text(128, false),
                    "A service of the Compose project below.",
                ),
                with_default(
                    parameter(
                        "lines",
                        "Lines",
                        ParameterKind::Integer {
                            min: Some(1),
                            max: Some(MAX_LOG_LINES),
                        },
                        "The last lines of the log.",
                    ),
                    ParameterValue::Integer(100),
                ),
                parameter(
                    "since",
                    "Since",
                    text(40, false),
                    "e.g. 10m, 2h or a timestamp. Empty: no limit.",
                ),
            ];
            parameters.extend(compose_parameters());
            (
                LOGS,
                "Container logs",
                "The last lines of a container's or Compose service's log, up to 64 KiB. Never \
                 follows the log.",
                parameters,
                node(
                    "file-text",
                    vec![
                        out("logs", "Logs", ValueKind::Text).sensitive(),
                        out("lineCount", "Lines", ValueKind::Integer),
                        out("truncated", "Truncated", ValueKind::Boolean),
                    ],
                    60_000,
                    ActionEffect::ReadOnly,
                ),
            )
        }
    };
    ActionType {
        id: id(id_value),
        name: name.into(),
        description: description.into(),
        category: "Docker".into(),
        parameters,
        requires_project: false,
        node: spec,
    }
}

#[derive(Debug)]
struct DockerAction {
    definition: ActionType,
    kind: Kind,
    settings: Arc<DockerSettings>,
}

impl ActionHandler for DockerAction {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        self.run(parameters, context)
            .unwrap_or_else(|failure| failure)
    }

    fn check(&self, parameters: &ActionParameters, profile: &Profile) -> Vec<String> {
        let mut problems = Vec::new();
        if let Some(id) = parameters.project(PROJECT)
            && profile.project(id).is_none()
        {
            problems.push("the project does not belong to this profile".into());
        }
        if let Some(problem) = names(parameters).err() {
            problems.push(problem.message);
        }
        problems
    }

    /// Every key but logs shows a live state, read at most every
    /// [`STATE_INTERVAL`](crate::state::STATE_INTERVAL).
    fn state_policy(&self, _parameters: &ActionParameters) -> Option<RefreshPolicy> {
        (self.kind != Kind::Logs).then(|| RefreshPolicy::every(crate::state::STATE_INTERVAL))
    }

    fn button_state(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ButtonState, StateError> {
        names(parameters)?;
        let count = |result: &ActionResult, key: &str| {
            result
                .data
                .get(key)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0)
        };
        if self.kind == Kind::Status {
            let result = self.status(parameters, context);
            if !result.is_success() {
                return Err(result.into());
            }
            let available = result.data.get("available").map(String::as_str) == Some("true");
            return Ok(crate::state::availability(available));
        }
        // Read-only, whatever the key does when pressed.
        let docker_context = self.context_name(parameters, context)?;
        match self.kind {
            Kind::Containers => {
                let result = self.containers(parameters, context, &docker_context)?;
                Ok(crate::state::containers(count(&result, "running")))
            }
            Kind::ComposeStatus | Kind::Change(_) => {
                let result = self.compose_status(parameters, context, &docker_context)?;
                let toggle = matches!(self.kind, Kind::Change(Change::Up | Change::Down));
                Ok(crate::state::compose(
                    count(&result, "running"),
                    count(&result, "services"),
                    toggle,
                ))
            }
            Kind::Status | Kind::Logs => Ok(ButtonState::default()),
        }
    }
}

/// Validates the names an action passes to Docker.
fn names(parameters: &ActionParameters) -> Result<(), Failure> {
    for key in [CONTEXT, "container", "service"] {
        if let Some(value) = parameters
            .text(key)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            && !cli::valid_name(value)
        {
            return Err(fail(
                codes::INVALID_NAME,
                format!("“{value}” is not a valid {key} name"),
            ));
        }
    }
    if let Some(value) = parameters
        .text("projectName")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        && !cli::valid_project(value)
    {
        return Err(fail(
            codes::INVALID_NAME,
            format!("“{value}” is not a valid Compose project name (lower case, digits, - and _)"),
        ));
    }
    if let Some(value) = parameters
        .text("since")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        && !cli::valid_since(value)
    {
        return Err(fail(
            codes::INVALID_NAME,
            format!("“{value}” is not a valid time"),
        ));
    }
    for service in services(parameters) {
        if !cli::valid_name(&service) {
            return Err(fail(
                codes::INVALID_NAME,
                format!("“{service}” is not a valid service name"),
            ));
        }
    }
    Ok(())
}

fn services(parameters: &ActionParameters) -> Vec<String> {
    parameters
        .text("services")
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn optional<'a>(parameters: &'a ActionParameters, key: &str) -> Option<&'a str> {
    parameters
        .text(key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

impl DockerAction {
    fn docker(
        &self,
        context: &ExecutionContext<'_>,
        args: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ProcessOutput, Failure> {
        let mut command = Command::new(&self.settings.executable);
        command.args(args);
        minimal_environment(&mut command, DOCKER_ENVIRONMENT);
        context
            .run_process(
                command,
                timeout,
                ProcessLimits {
                    max_output,
                    ..ProcessLimits::default()
                },
            )
            .map_err(|error| match error.code() {
                "program-not-found" => fail(
                    codes::DOCKER_NOT_FOUND,
                    format!(
                        "Docker ({}) was not found; install it or set its location in Settings › \
                         Docker",
                        self.settings.executable
                    ),
                ),
                code => fail(code, format!("docker: {error}")),
            })
    }

    /// Runs Docker and turns a failure into its classified error.
    fn checked(
        &self,
        context: &ExecutionContext<'_>,
        args: &[String],
        timeout: Duration,
        max_output: usize,
    ) -> Result<ProcessOutput, Failure> {
        let output = self.docker(context, args, timeout, max_output)?;
        if output.success() {
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

    /// The context to name in every command: the action's, the module's,
    /// or the one Docker currently uses.
    fn context_name(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<String, Failure> {
        if let Some(name) = optional(parameters, CONTEXT).or(self.settings.context.as_deref()) {
            return if cli::valid_name(name) {
                Ok(name.to_owned())
            } else {
                Err(fail(
                    codes::INVALID_NAME,
                    format!("“{name}” is not a valid context name"),
                ))
            };
        }
        let output = self.checked(
            context,
            &["context".into(), "show".into()],
            READ_TIMEOUT,
            4096,
        )?;
        let name = output.stdout.trim().to_owned();
        if cli::valid_name(&name) {
            Ok(name)
        } else {
            Err(fail(
                codes::DOCKER_FAILED,
                "Docker reported no current context",
            ))
        }
    }

    /// The Compose project from the action's resource or a project's.
    fn compose_target<'a>(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'a>,
    ) -> Result<(ComposeTarget, Option<&'a deckpilot_core::Project>), Failure> {
        let resolve_error = |e: deckpilot_core::ResolveError| fail(e.code(), e.to_string());
        let (binding, project) = if let Some(reference) = parameters.resource(COMPOSE) {
            (
                context.resolve(reference, None).map_err(resolve_error)?,
                None,
            )
        } else {
            let project = match parameters.project(PROJECT) {
                Some(id) => context
                    .profile()
                    .map_err(ActionResult::from)?
                    .project(id)
                    .ok_or_else(|| {
                        fail(
                            codes::PROJECT_NOT_IN_PROFILE,
                            "the project does not belong to this profile",
                        )
                    })?,
                None => context
                    .project()
                    .map_err(ActionResult::from)?
                    .ok_or_else(|| {
                        fail(
                            "no-active-project",
                            "no project is active and no Compose project is set",
                        )
                    })?,
            };
            (
                context
                    .resolve_project_resource(project, &self.settings.compose_role)
                    .map_err(resolve_error)?,
                Some(project),
            )
        };
        if !matches!(
            binding.kind,
            ResourceKind::Directory | ResourceKind::DockerEnvironment | ResourceKind::Other
        ) {
            return Err(fail(
                "resource-kind-mismatch",
                format!("{} is not a folder or Compose file", binding.key),
            ));
        }
        let name = optional(parameters, "projectName").map(str::to_owned);
        let target =
            cli::compose_target(&PathBuf::from(&binding.value), name).ok_or_else(|| {
                fail(
                    codes::COMPOSE_FILE_NOT_FOUND,
                    format!("{} does not exist", binding.value),
                )
            })?;
        Ok((target, project))
    }

    fn run(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        names(parameters)?;
        if let Kind::Change(_) = self.kind
            && !self.settings.allow_changes
        {
            return Err(fail(
                codes::CHANGES_DISABLED,
                "starting and stopping services is not allowed on this computer; turn on \
                 “Allow starting and stopping services” in Settings › Docker",
            ));
        }
        if self.kind == Kind::Status {
            return Ok(self.status(parameters, context));
        }
        let docker_context = self.context_name(parameters, context)?;
        let result = match self.kind {
            Kind::Containers => self.containers(parameters, context, &docker_context)?,
            Kind::ComposeStatus => self.compose_status(parameters, context, &docker_context)?,
            Kind::Change(change) => self.change(change, parameters, context, &docker_context)?,
            Kind::Logs => self.logs(parameters, context, &docker_context)?,
            Kind::Status => unreachable!("handled above"),
        };
        Ok(result.with_data(CONTEXT, docker_context))
    }

    /// Never fails because Docker is unavailable: that is the answer.
    fn status(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let unavailable = |reason: &Failure, docker_context: Option<&str>| {
            let mut result =
                ActionResult::success(format!("Docker is not available: {}", reason.message))
                    .with_data("available", "false")
                    .with_data("reason", reason.code.clone().unwrap_or_default());
            if let Some(name) = docker_context {
                result = result.with_data(CONTEXT, name);
            }
            result
        };
        let docker_context = match self.context_name(parameters, context) {
            Ok(name) => name,
            Err(failure) if failure.code.as_deref() == Some("not-permitted") => return failure,
            Err(failure) => return unavailable(&failure, None),
        };
        let mut args = cli::base(&docker_context);
        args.extend(["version".into(), "--format".into(), "{{json .}}".into()]);
        let output = match self.docker(context, &args, READ_TIMEOUT, 16 * 1024) {
            Ok(output) => output,
            Err(failure) => return unavailable(&failure, Some(&docker_context)),
        };
        let versions = cli::parse_versions(&output.stdout);
        let mut result =
            ActionResult::success(String::new()).with_data(CONTEXT, docker_context.clone());
        if let Some(client) = versions.as_ref().map(|v| &v.client) {
            result = result.with_data("clientVersion", client.version.clone());
        }
        let server = versions
            .as_ref()
            .and_then(|v| v.server.clone())
            .filter(|s| !s.version.is_empty());
        let Some(server) = server.filter(|_| output.success()) else {
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
            let mut failed = unavailable(&reason, Some(&docker_context));
            if let Some(version) = result.data.get("clientVersion") {
                failed = failed.with_data("clientVersion", version.clone());
            }
            return failed;
        };
        result = result
            .with_data("available", "true")
            .with_data("serverVersion", server.version.clone())
            .with_data("serverOs", server.os.clone());
        let mut compose = cli::base(&docker_context);
        compose.extend(["compose".into(), "version".into(), "--short".into()]);
        let compose = self
            .docker(context, &compose, READ_TIMEOUT, 4096)
            .ok()
            .filter(ProcessOutput::success)
            .map(|o| o.stdout.trim().to_owned());
        result.message = format!(
            "Docker {} in context {docker_context}{}",
            server.version,
            compose
                .as_ref()
                .map(|v| format!(", Compose {v}"))
                .unwrap_or_default()
        );
        if let Some(version) = compose {
            result = result.with_data("composeVersion", version);
        }
        result
    }

    fn containers(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
        docker_context: &str,
    ) -> Result<ActionResult, Failure> {
        let mut args = cli::base(docker_context);
        args.extend([
            "ps".into(),
            "--no-trunc".into(),
            "--format".into(),
            "{{json .}}".into(),
        ]);
        if parameters.boolean("all").unwrap_or(false) {
            args.push("--all".into());
        }
        if let Some(name) = optional(parameters, "name") {
            args.extend(["--filter".into(), format!("name={name}")]);
        }
        let output = self.checked(context, &args, READ_TIMEOUT, 256 * 1024)?;
        let containers = cli::parse_containers(&output.stdout)
            .map_err(|e| fail(codes::DOCKER_FAILED, e.to_string()))?;
        let running = containers.iter().filter(|c| c.running()).count();
        Ok(ActionResult::success(format!(
            "{} containers, {running} running, in context {docker_context}",
            containers.len()
        ))
        .with_data("count", containers.len().to_string())
        .with_data("running", running.to_string())
        .with_data(
            "names",
            containers
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
        .with_data(
            "containers",
            containers
                .iter()
                .map(cli::Container::line)
                .collect::<Vec<_>>()
                .join("\n"),
        ))
    }

    fn compose_status(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
        docker_context: &str,
    ) -> Result<ActionResult, Failure> {
        let (target, project) = self.compose_target(parameters, context)?;
        let args = cli::compose(
            docker_context,
            &target,
            &["ps", "--all", "--format", "json"],
        );
        let output = self.checked(context, &args, READ_TIMEOUT, 256 * 1024)?;
        let containers = cli::parse_containers(&output.stdout)
            .map_err(|e| fail(codes::DOCKER_FAILED, e.to_string()))?;
        let running = containers.iter().filter(|c| c.running()).count();
        let stopped = containers.len() - running;
        let mut result = ActionResult::success(format!(
            "{running} of {} containers running in context {docker_context}",
            containers.len()
        ))
        .with_data("services", containers.len().to_string())
        .with_data("running", running.to_string())
        .with_data("stopped", stopped.to_string())
        .with_data("allRunning", (stopped == 0 && running > 0).to_string())
        .with_data(
            "containers",
            containers
                .iter()
                .map(cli::Container::line)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        result = with_compose(result, &target, project);
        Ok(result)
    }

    fn change(
        &self,
        change: Change,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
        docker_context: &str,
    ) -> Result<ActionResult, Failure> {
        let (target, project) = self.compose_target(parameters, context)?;
        let services = services(parameters);
        let command = cli::compose_change(
            change,
            &services,
            parameters.boolean("removeOrphans").unwrap_or(false),
        );
        let command: Vec<&str> = command.iter().map(String::as_str).collect();
        let args = cli::compose(docker_context, &target, &command);
        let verb = match change {
            Change::Up => "starting",
            Change::Down => "stopping",
            Change::Restart => "restarting",
        };
        context.report_progress(None, format!("{verb} in context {docker_context}"));
        let output = self.checked(context, &args, self.settings.timeout, 64 * 1024)?;
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
        let mut result =
            ActionResult::success(format!("{what} {done} in context {docker_context}"))
                // Compose reports progress on stderr.
                .with_data(
                    "output",
                    format!("{}{}", output.stdout, output.stderr)
                        .trim()
                        .to_owned(),
                );
        result = with_compose(result, &target, project);
        Ok(result)
    }

    fn logs(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
        docker_context: &str,
    ) -> Result<ActionResult, Failure> {
        let lines = parameters
            .integer("lines")
            .unwrap_or(100)
            .clamp(1, MAX_LOG_LINES)
            .to_string();
        let mut options = vec!["--tail".to_owned(), lines];
        if let Some(since) = optional(parameters, "since") {
            options.extend(["--since".into(), since.to_owned()]);
        }
        let (args, target, project) = match (
            optional(parameters, "container"),
            optional(parameters, "service"),
        ) {
            (Some(container), None) => {
                let mut args = cli::base(docker_context);
                args.push("logs".into());
                args.extend(options);
                args.extend(["--".into(), container.to_owned()]);
                (args, None, None)
            }
            (None, Some(service)) => {
                let (target, project) = self.compose_target(parameters, context)?;
                let mut command = vec!["logs".to_owned(), "--no-color".to_owned()];
                command.extend(options);
                command.extend(["--".into(), service.to_owned()]);
                let command: Vec<&str> = command.iter().map(String::as_str).collect();
                (
                    cli::compose(docker_context, &target, &command),
                    Some(target),
                    project,
                )
            }
            _ => {
                return Err(fail(
                    "invalid-parameters",
                    "set either a container or a Compose service",
                ));
            }
        };
        let output = self.checked(context, &args, READ_TIMEOUT, MAX_LOG_BYTES)?;
        // A container logs to both streams; keep both, stdout first.
        let mut logs = output.stdout;
        if !output.stderr.is_empty() {
            if !logs.is_empty() && !logs.ends_with('\n') {
                logs.push('\n');
            }
            logs.push_str(&output.stderr);
        }
        let logs = logs.trim_end().to_owned();
        let count = logs.lines().count();
        let mut result = ActionResult::success(format!("{count} log lines"))
            .with_data("lineCount", count.to_string())
            .with_data(
                "truncated",
                (output.stdout_truncated || output.stderr_truncated).to_string(),
            )
            .with_data("logs", logs);
        if let Some(target) = target {
            result = with_compose(result, &target, project);
        }
        Ok(result)
    }
}

fn with_compose(
    mut result: ActionResult,
    target: &ComposeTarget,
    project: Option<&deckpilot_core::Project>,
) -> ActionResult {
    if let Some(name) = &target.project {
        result = result.with_data("project", name.clone());
    }
    if let Some(project) = project {
        result = result.with_data("projectKey", project.key.as_str());
    }
    result
}

#[cfg(test)]
mod tests;
