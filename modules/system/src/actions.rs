//! The action types of the `system` module.

use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::launch::{self, Launcher, OpenTarget};
use deckpilot_core::process::{ProcessLimits, minimal_environment};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    ExecutionContext, Os, ParameterKind, ParameterSpec, ParameterValue, ResourceKind,
};

use crate::support::{self, Failure, fail};
use crate::{ALLOW_COMMANDS, codes};

pub const OPEN_APP: &str = "system.open-app";
pub const OPEN_URL: &str = "system.open-url";
pub const OPEN_FILE: &str = "system.open-file";
pub const OPEN_DIRECTORY: &str = "system.open-directory";
pub const RUN_COMMAND: &str = "system.run-command";
pub const PROCESS_STATUS: &str = "system.process-status";

/// Longest time limit of a command, in seconds.
pub const MAX_COMMAND_SECONDS: i64 = 3600;
/// Output kept per stream of a command.
pub const MAX_COMMAND_OUTPUT: usize = 64 * 1024;

/// # Panics
///
/// Never for the constants of this module.
pub fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("the action type id is valid")
}

pub fn handlers(launcher: &Arc<dyn Launcher>, allow_commands: bool) -> Vec<Box<dyn ActionHandler>> {
    vec![
        Box::new(OpenApp::new(launcher.clone())),
        Box::new(OpenUrl::new(launcher.clone())),
        Box::new(OpenFile::new(launcher.clone())),
        Box::new(OpenDirectory::new(launcher.clone())),
        Box::new(RunCommand::new(allow_commands)),
        Box::new(ProcessStatus::new()),
    ]
}

fn definition(
    id_value: &str,
    name: &str,
    description: &str,
    parameters: Vec<ParameterSpec>,
    node: NodeSpec,
) -> ActionType {
    ActionType {
        id: id(id_value),
        name: name.into(),
        description: description.into(),
        category: "System".into(),
        parameters,
        requires_project: false,
        node,
    }
}

fn parameter(key: &str, label: &str, kind: ParameterKind, required: bool) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: label.into(),
        description: None,
        kind,
        required,
        default: None,
    }
}

fn described(spec: ParameterSpec, description: &str) -> ParameterSpec {
    ParameterSpec {
        description: Some(description.into()),
        ..spec
    }
}

fn resource(kind: Option<ResourceKind>) -> ParameterKind {
    ParameterKind::Resource { kind }
}

fn text(max_length: usize, multiline: bool) -> ParameterKind {
    ParameterKind::Text {
        multiline,
        max_length: Some(max_length),
    }
}

/// Opening: quick, visible, safe to retry when it failed.
fn opens(icon: &str, outputs: Vec<OutputSpec>) -> NodeSpec {
    NodeSpec {
        icon: Some(icon.into()),
        outputs,
        timeout_ms: 15_000,
        retry_safe: true,
        idempotent: false,
        effect: ActionEffect::Opens,
        ..NodeSpec::default()
    }
}

fn open_with(
    launcher: &dyn Launcher,
    context: &ExecutionContext<'_>,
    target: &OpenTarget,
) -> Result<(), Failure> {
    context
        .permit(Capability::OpenExternal)
        .map_err(ActionResult::from)?;
    let plan = launch::plan(Os::current(), target).map_err(|e| fail(e.code(), e.to_string()))?;
    launcher
        .launch(&plan)
        .map_err(|e| fail(e.code(), e.to_string()))
}

fn outcome(result: Result<ActionResult, Failure>) -> ActionResult {
    result.unwrap_or_else(|failure| failure)
}

/// Arguments are a list: each is passed as it is.
fn arguments_kind() -> ParameterKind {
    ParameterKind::Lines {
        item_label: Some("Argument".into()),
        max_items: Some(support::MAX_ARGUMENTS),
        max_length: Some(support::MAX_ARGUMENT_CHARS),
    }
}

const ARGUMENTS_HELP: &str = "Each argument is passed as it is: nothing is split on spaces or \
                              interpreted by a shell.";

/// `system.open-app`: starts an application bound as a resource.
#[derive(Debug)]
pub struct OpenApp {
    definition: ActionType,
    launcher: Arc<dyn Launcher>,
}

impl OpenApp {
    pub fn new(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            definition: definition(
                OPEN_APP,
                "Open application",
                "Starts an application bound on this computer, optionally with arguments.",
                vec![
                    described(
                        parameter(
                            "application",
                            "Application",
                            resource(Some(ResourceKind::Application)),
                            true,
                        ),
                        "An application resource: a path to the application, a macOS bundle \
                         identifier or a Linux .desktop id, bound on each computer.",
                    ),
                    described(
                        parameter("arguments", "Arguments", arguments_kind(), false),
                        ARGUMENTS_HELP,
                    ),
                ],
                opens(
                    "app-window",
                    vec![OutputSpec::new(
                        "application",
                        "Application",
                        ValueKind::Text,
                    )],
                ),
            ),
            launcher,
        }
    }
}

impl ActionHandler for OpenApp {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        outcome((|| {
            let application = support::resolve(
                context,
                parameters.resource("application"),
                Some(ResourceKind::Application),
                "application",
            )?;
            let arguments = support::arguments(parameters.text("arguments"))?;
            if application.contains(['/', '\\']) {
                support::existing(application.clone().into(), false)?;
            }
            open_with(
                self.launcher.as_ref(),
                context,
                &OpenTarget::Application {
                    application: application.clone(),
                    arguments,
                },
            )?;
            Ok(ActionResult::success(format!("opened {application}"))
                .with_data("application", application))
        })())
    }
}

/// `system.open-url`: opens a web page or mail address in the default
/// program.
#[derive(Debug)]
pub struct OpenUrl {
    definition: ActionType,
    launcher: Arc<dyn Launcher>,
}

impl OpenUrl {
    pub fn new(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            definition: definition(
                OPEN_URL,
                "Open URL",
                "Opens a web page (http, https) or a mail address (mailto) in the default \
                 browser or mail program.",
                vec![described(
                    parameter("url", "URL", text(2048, false), true),
                    "Other schemes are refused because they can start programs.",
                )],
                opens("globe", vec![OutputSpec::new("url", "URL", ValueKind::Url)]),
            ),
            launcher,
        }
    }
}

impl ActionHandler for OpenUrl {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        outcome((|| {
            let url = support::web_url(parameters.text("url").unwrap_or_default())?;
            open_with(
                self.launcher.as_ref(),
                context,
                &OpenTarget::Url(url.clone()),
            )?;
            Ok(ActionResult::success(format!("opened {url}")).with_data("url", url))
        })())
    }
}

fn location_parameters(directory: bool) -> Vec<ParameterSpec> {
    vec![
        described(
            parameter(
                "location",
                if directory { "Folder" } else { "Location" },
                resource(directory.then_some(ResourceKind::Directory)),
                true,
            ),
            if directory {
                "A folder resource, e.g. the project's repository."
            } else {
                "A file or folder resource the file is in, e.g. the project's repository."
            },
        ),
        described(
            parameter(
                "path",
                if directory { "Subfolder" } else { "File" },
                text(1024, false),
                !directory,
            ),
            "Relative to the resource, e.g. docs/README.md. It cannot leave the resource.",
        ),
    ]
}

fn resolve_location(
    parameters: &ActionParameters,
    context: &ExecutionContext<'_>,
    directory: bool,
) -> Result<std::path::PathBuf, Failure> {
    let base = support::resolve(
        context,
        parameters.resource("location"),
        directory.then_some(ResourceKind::Directory),
        "location",
    )?;
    let path = support::join_inside(&base, parameters.text("path"))?;
    support::existing(path, directory)
}

/// `system.open-file`: opens a file with its default application or a
/// chosen one.
#[derive(Debug)]
pub struct OpenFile {
    definition: ActionType,
    launcher: Arc<dyn Launcher>,
}

impl OpenFile {
    pub fn new(launcher: Arc<dyn Launcher>) -> Self {
        let mut parameters = location_parameters(false);
        parameters.push(described(
            parameter(
                "application",
                "Open with",
                resource(Some(ResourceKind::Application)),
                false,
            ),
            "An application resource; the file's default application when empty.",
        ));
        Self {
            definition: definition(
                OPEN_FILE,
                "Open file",
                "Opens a file inside a resource with its default application or a chosen one.",
                parameters,
                opens(
                    "file",
                    vec![OutputSpec::new("path", "Path", ValueKind::Path)],
                ),
            ),
            launcher,
        }
    }
}

impl ActionHandler for OpenFile {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        outcome((|| {
            let path = resolve_location(parameters, context, false)?;
            let target = match parameters.resource("application") {
                Some(reference) => OpenTarget::PathWith {
                    path: path.clone(),
                    application: support::resolve(
                        context,
                        Some(reference),
                        Some(ResourceKind::Application),
                        "application",
                    )?,
                },
                None => OpenTarget::Path(path.clone()),
            };
            open_with(self.launcher.as_ref(), context, &target)?;
            let shown = path.display().to_string();
            Ok(ActionResult::success(format!("opened {shown}")).with_data("path", shown))
        })())
    }
}

/// `system.open-directory`: opens a folder in the file manager.
#[derive(Debug)]
pub struct OpenDirectory {
    definition: ActionType,
    launcher: Arc<dyn Launcher>,
}

impl OpenDirectory {
    pub fn new(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            definition: definition(
                OPEN_DIRECTORY,
                "Open folder",
                "Opens a folder resource, or a folder inside it, in the file manager.",
                location_parameters(true),
                opens(
                    "folder-open",
                    vec![OutputSpec::new("path", "Path", ValueKind::Path)],
                ),
            ),
            launcher,
        }
    }
}

impl ActionHandler for OpenDirectory {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        outcome((|| {
            let path = resolve_location(parameters, context, true)?;
            open_with(
                self.launcher.as_ref(),
                context,
                &OpenTarget::Path(path.clone()),
            )?;
            let shown = path.display().to_string();
            Ok(ActionResult::success(format!("opened {shown}")).with_data("path", shown))
        })())
    }
}

/// `system.run-command`: runs a program bound as a resource with an
/// argument list, waits for it and reports its exit code and output.
#[derive(Debug)]
pub struct RunCommand {
    definition: ActionType,
    allowed: bool,
}

impl RunCommand {
    pub fn new(allowed: bool) -> Self {
        let mut timeout = described(
            parameter(
                "timeoutSeconds",
                "Time limit (s)",
                ParameterKind::Integer {
                    min: Some(1),
                    max: Some(MAX_COMMAND_SECONDS),
                },
                true,
            ),
            "The program is stopped after this long.",
        );
        timeout.default = Some(ParameterValue::Integer(60));
        let mut fail_on_error = described(
            parameter(
                "failOnNonZero",
                "Fail on a non-zero exit code",
                ParameterKind::Boolean,
                true,
            ),
            "When off, the exit code is only reported and the node succeeds.",
        );
        fail_on_error.default = Some(ParameterValue::Boolean(true));
        Self {
            definition: definition(
                RUN_COMMAND,
                "Run command",
                "Runs a program bound as a resource with arguments, without a shell, and \
                 reports its exit code and output. Must be allowed in the System settings.",
                vec![
                    described(
                        parameter("program", "Program", resource(None), true),
                        "A resource bound to an absolute path or a program name on PATH.",
                    ),
                    described(
                        parameter("arguments", "Arguments", arguments_kind(), false),
                        ARGUMENTS_HELP,
                    ),
                    described(
                        parameter(
                            "workingDirectory",
                            "Working folder",
                            resource(Some(ResourceKind::Directory)),
                            false,
                        ),
                        "Your home folder when empty.",
                    ),
                    timeout,
                    fail_on_error,
                ],
                NodeSpec {
                    icon: Some("terminal".into()),
                    outputs: vec![
                        OutputSpec::new("exitCode", "Exit code", ValueKind::Integer),
                        OutputSpec::new("stdout", "Output", ValueKind::Text).sensitive(),
                        OutputSpec::new("stderr", "Error output", ValueKind::Text).sensitive(),
                        OutputSpec::new("outputTruncated", "Output truncated", ValueKind::Boolean),
                        OutputSpec::new("durationMs", "Duration (ms)", ValueKind::Integer),
                    ],
                    // The action's own time limit applies; a node can set a
                    // shorter one.
                    timeout_ms: u32::try_from((MAX_COMMAND_SECONDS + 10) * 1000)
                        .unwrap_or(u32::MAX),
                    retry_safe: false,
                    idempotent: false,
                    cancellable: true,
                    effect: ActionEffect::Modifies,
                    ..NodeSpec::default()
                },
            ),
            allowed,
        }
    }

    fn run(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        if !self.allowed {
            return Err(fail(
                codes::COMMANDS_DISABLED,
                format!(
                    "running commands is not allowed on this computer; turn on “Allow running \
                     commands” ({ALLOW_COMMANDS}) in Settings → System"
                ),
            ));
        }
        let value = support::resolve(context, parameters.resource("program"), None, "program")?;
        let program = support::program(&value)?;
        let arguments = support::arguments(parameters.text("arguments"))?;
        let directory = match parameters.resource("workingDirectory") {
            Some(reference) => support::existing(
                support::resolve(
                    context,
                    Some(reference),
                    Some(ResourceKind::Directory),
                    "working folder",
                )?
                .into(),
                true,
            )?,
            None => std::env::home_dir().unwrap_or_else(std::env::temp_dir),
        };
        let seconds = parameters
            .integer("timeoutSeconds")
            .unwrap_or(60)
            .clamp(1, MAX_COMMAND_SECONDS);
        let mut command = std::process::Command::new(&program);
        command.args(&arguments).current_dir(&directory);
        minimal_environment(&mut command, &[]);
        let name = program
            .file_name()
            .map_or_else(|| value.clone(), |n| n.to_string_lossy().into_owned());
        context.report_progress(None, format!("running {name}"));
        let begun = Instant::now();
        let output = context
            .run_process(
                command,
                Duration::from_secs(seconds.unsigned_abs()),
                ProcessLimits {
                    max_output: MAX_COMMAND_OUTPUT,
                    ..ProcessLimits::default()
                },
            )
            .map_err(|error| fail(error.code(), format!("{name}: {error}")))?;
        let elapsed = begun.elapsed().as_millis();
        let code = output.code.unwrap_or(-1);
        let message = format!("{name} exited with code {code} after {elapsed} ms");
        let failed = !output.success() && parameters.boolean("failOnNonZero").unwrap_or(true);
        let result = if failed {
            ActionResult::error(codes::EXIT_NONZERO, message)
        } else {
            ActionResult::success(message)
        };
        let trim = |s: &str| s.trim_end_matches(['\n', '\r']).to_owned();
        Ok(result
            .with_data("exitCode", code.to_string())
            .with_data("stdout", trim(&output.stdout))
            .with_data("stderr", trim(&output.stderr))
            .with_data(
                "outputTruncated",
                (output.stdout_truncated || output.stderr_truncated).to_string(),
            )
            .with_data("durationMs", elapsed.to_string()))
    }
}

impl ActionHandler for RunCommand {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        outcome(self.run(parameters, context))
    }
}

/// `system.process-status`: whether programs with a name are running.
#[derive(Debug)]
pub struct ProcessStatus {
    definition: ActionType,
}

impl ProcessStatus {
    pub fn new() -> Self {
        Self {
            definition: definition(
                PROCESS_STATUS,
                "Program status",
                "Reports whether a program with this name is running, and how many. Reads \
                 only names and process ids, never command lines or environments.",
                vec![described(
                    parameter("name", "Program name", text(255, false), true),
                    "As shown by the system, e.g. Docker, code or slack.exe. Case and a \
                     trailing .exe are ignored.",
                )],
                NodeSpec {
                    icon: Some("activity".into()),
                    outputs: vec![
                        OutputSpec::new("running", "Running", ValueKind::Boolean),
                        OutputSpec::new("count", "Count", ValueKind::Integer),
                        OutputSpec::new("pids", "Process ids", ValueKind::Text),
                    ],
                    timeout_ms: 10_000,
                    retry_safe: true,
                    idempotent: true,
                    ..NodeSpec::default()
                },
            ),
        }
    }
}

impl Default for ProcessStatus {
    fn default() -> Self {
        Self::new()
    }
}

/// A process name as compared: lower case, without `.exe`.
pub fn normalize_process_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    lower
        .strip_suffix(".exe")
        .map_or(lower.clone(), str::to_owned)
}

impl ActionHandler for ProcessStatus {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Err(denied) = context.permit(Capability::ProcessInspect) {
            return denied.into();
        }
        let wanted = normalize_process_name(parameters.text("name").unwrap_or_default());
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        let mut pids: Vec<u32> = system
            .processes()
            .iter()
            .filter(|(_, process)| {
                normalize_process_name(&process.name().to_string_lossy()) == wanted
            })
            .map(|(pid, _)| pid.as_u32())
            .collect();
        pids.sort_unstable();
        let count = pids.len();
        let shown: Vec<String> = pids.iter().take(20).map(ToString::to_string).collect();
        let message = match count {
            0 => format!("{wanted} is not running"),
            1 => format!("{wanted} is running"),
            n => format!("{n} {wanted} processes are running"),
        };
        ActionResult::success(message)
            .with_data("running", (count > 0).to_string())
            .with_data("count", count.to_string())
            .with_data("pids", shown.join(","))
    }
}

#[cfg(test)]
mod tests;
