//! The action types of the `workspace` module. They only open things.

use std::path::PathBuf;
use std::sync::Arc;

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::launch::{self, LaunchPlan, Launcher, OpenTarget};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    ExecutionContext, Os, ParameterKind, ParameterSpec, ParameterValue, Profile, ResolveError,
    ResourceKind, ResourceRef,
};
use deckpilot_module_system::codes as system_codes;
use deckpilot_module_system::support::{self, Failure, fail};

use crate::codes;
use crate::editor::{self, EDITORS, Editor, MAX_POSITION, Position};

pub const OPEN_IN_EDITOR: &str = "workspace.open-in-editor";
pub const OPEN_URLS: &str = "workspace.open-urls";
pub const OPEN_PROJECT: &str = "workspace.open-project";

/// Most pages opened by one action.
pub const MAX_URLS: usize = 10;
/// The project resource that is the repository, by default.
pub const REPOSITORY_ROLE: &str = "repository";

/// # Panics
///
/// Never for the constants of this module.
pub fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("the action type id is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Editor,
    Urls,
    Project,
}

pub fn handlers(launcher: &Arc<dyn Launcher>) -> Vec<Box<dyn ActionHandler>> {
    [Kind::Editor, Kind::Urls, Kind::Project]
        .into_iter()
        .map(|kind| {
            Box::new(WorkspaceAction {
                definition: definition(kind),
                kind,
                launcher: launcher.clone(),
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

fn required(mut spec: ParameterSpec, default: Option<ParameterValue>) -> ParameterSpec {
    spec.required = true;
    spec.default = default;
    spec
}

fn text(max_length: usize, multiline: bool) -> ParameterKind {
    ParameterKind::Text {
        multiline,
        max_length: Some(max_length),
    }
}

/// A list of at most [`MAX_URLS`] items, one per line.
fn list_kind(item: &str, max_length: usize) -> ParameterKind {
    ParameterKind::Lines {
        item_label: Some(item.into()),
        max_items: Some(MAX_URLS),
        max_length: Some(max_length),
    }
}

fn resource(kind: ResourceKind) -> ParameterKind {
    ParameterKind::Resource { kind: Some(kind) }
}

fn editor_parameter() -> ParameterSpec {
    required(
        parameter(
            "editor",
            "Editor kind",
            ParameterKind::Choice {
                options: EDITORS.iter().map(|e| (*e).to_owned()).collect(),
            },
            "How to pass a line: vscode and cursor (--goto file:line), jetbrains (--line N \
             file), zed and sublime (file:line). other opens the file without a line.",
        ),
        Some(ParameterValue::Choice("other".into())),
    )
}

fn browser_parameter() -> ParameterSpec {
    parameter(
        "browser",
        "Browser",
        resource(ResourceKind::Application),
        "An application resource. Empty: the default browser.",
    )
}

fn role_parameter(key: &str, label: &str, default: &str, description: &str) -> ParameterSpec {
    required(
        parameter(key, label, text(64, false), description),
        Some(ParameterValue::Text(default.into())),
    )
}

fn out(key: &str, label: &str, kind: ValueKind) -> OutputSpec {
    OutputSpec::new(key, label, kind)
}

fn opens(icon: &str, outputs: Vec<OutputSpec>) -> NodeSpec {
    NodeSpec {
        icon: Some(icon.into()),
        outputs,
        timeout_ms: 15_000,
        retry_safe: true,
        idempotent: false,
        cancellable: false,
        effect: ActionEffect::Opens,
        ..NodeSpec::default()
    }
}

#[allow(clippy::too_many_lines)] // One table of the three action types.
fn definition(kind: Kind) -> ActionType {
    let (id_value, name, description, parameters, node) = match kind {
        Kind::Editor => (
            OPEN_IN_EDITOR,
            "Open in editor",
            "Opens the project's repository, or a file in it at a line, in an editor.",
            vec![
                required(
                    parameter(
                        "application",
                        "Editor",
                        resource(ResourceKind::Application),
                        "An application resource: the editor (bind its command-line tool, e.g. \
                         /usr/local/bin/code, to open at a line in an editor that is already \
                         running on macOS).",
                    ),
                    None,
                ),
                editor_parameter(),
                parameter(
                    "location",
                    "Folder",
                    resource(ResourceKind::Directory),
                    "A folder resource. Empty: the project's repository.",
                ),
                parameter(
                    "path",
                    "File",
                    text(1024, false),
                    "Relative to the folder, e.g. src/main.rs. It cannot leave the folder. \
                     Empty: the folder itself.",
                ),
                parameter(
                    "line",
                    "Line",
                    ParameterKind::Integer {
                        min: Some(1),
                        max: Some(MAX_POSITION),
                    },
                    "Needs a file.",
                ),
                parameter(
                    "column",
                    "Column",
                    ParameterKind::Integer {
                        min: Some(1),
                        max: Some(MAX_POSITION),
                    },
                    "Needs a line.",
                ),
            ],
            opens(
                "code",
                vec![
                    out("path", "Opened", ValueKind::Path),
                    out("application", "Editor", ValueKind::Text),
                    out("located", "Opened at the line", ValueKind::Boolean),
                ],
            ),
        ),
        Kind::Urls => (
            OPEN_URLS,
            "Open pages",
            "Opens up to ten web pages (http, https) in the default browser or a chosen one.",
            vec![
                required(
                    parameter(
                        "urls",
                        "Pages",
                        list_kind("Page", 2048),
                        "One http or https address per line, at most 10.",
                    ),
                    None,
                ),
                browser_parameter(),
            ],
            opens(
                "globe",
                vec![
                    out("opened", "Opened", ValueKind::Integer),
                    out("failed", "Not opened", ValueKind::Integer),
                    // Addresses may carry tokens in their query.
                    out("urls", "Pages opened", ValueKind::Text).sensitive(),
                ],
            ),
        ),
        Kind::Project => (
            OPEN_PROJECT,
            "Open project workspace",
            "Opens the current project's repository in its editor, its documentation and \
             other pages, from the project's resources. Missing ones are reported.",
            vec![
                editor_parameter(),
                role_parameter(
                    "editorRole",
                    "Editor resource",
                    "editor",
                    "The project resource that is the editor (an application).",
                ),
                role_parameter(
                    "repositoryRole",
                    "Repository resource",
                    REPOSITORY_ROLE,
                    "The project resource that is the repository (a folder). Empty: skip.",
                ),
                role_parameter(
                    "docsRole",
                    "Documentation resource",
                    "docs",
                    "A page (URL) or folder. Empty: skip.",
                ),
                parameter(
                    "urlRoles",
                    "Other pages",
                    list_kind("Resource", 64),
                    "Project resources that are pages, one name per line, e.g. dashboard, at \
                     most 10.",
                ),
                browser_parameter(),
            ],
            opens(
                "app-window",
                vec![
                    out("projectKey", "Project", ValueKind::ProjectKey),
                    out("opened", "Opened", ValueKind::Integer),
                    out("openedRoles", "Opened resources", ValueKind::Text),
                    out("missingCount", "Missing", ValueKind::Integer),
                    out("missing", "Missing resources and why", ValueKind::Text),
                ],
            ),
        ),
    };
    ActionType {
        id: id(id_value),
        name: name.into(),
        description: description.into(),
        category: "Workspace".into(),
        parameters,
        requires_project: false,
        deprecated: false,
        node,
    }
}

#[derive(Debug)]
struct WorkspaceAction {
    definition: ActionType,
    kind: Kind,
    launcher: Arc<dyn Launcher>,
}

fn optional<'a>(parameters: &'a ActionParameters, key: &str) -> Option<&'a str> {
    parameters
        .text(key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Non-empty trimmed lines.
fn lines(text: Option<&str>) -> Vec<&str> {
    text.unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

/// The pages of `workspace.open-urls`, all checked before any is opened.
pub fn urls(text: Option<&str>) -> Result<Vec<String>, Failure> {
    let lines = lines(text);
    if lines.is_empty() {
        return Err(fail(codes::NO_URLS, "no page to open"));
    }
    if lines.len() > MAX_URLS {
        return Err(fail(
            codes::TOO_MANY_URLS,
            format!("at most {MAX_URLS} pages can be opened at once"),
        ));
    }
    lines.into_iter().map(editor::browser_url).collect()
}

fn launch_error(error: &launch::LaunchError) -> Failure {
    fail(error.code(), error.to_string())
}

impl ActionHandler for WorkspaceAction {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let result = context
            .permit(Capability::OpenExternal)
            .map_err(ActionResult::from)
            .and_then(|()| match self.kind {
                Kind::Editor => self.open_in_editor(parameters, context),
                Kind::Urls => self.open_urls(parameters, context),
                Kind::Project => self.open_project(parameters, context),
            });
        result.unwrap_or_else(|failure| failure)
    }

    /// Problems found when the action is saved.
    fn check(&self, parameters: &ActionParameters, _: &Profile) -> Vec<String> {
        let mut problems = Vec::new();
        if let Some(path) = optional(parameters, "path")
            && !path.contains("{{")
            && let Err(failure) = support::join_inside("/", Some(path))
        {
            problems.push(failure.message);
        }
        if let Err(failure) =
            Position::new(parameters.integer("line"), parameters.integer("column"))
        {
            problems.push(failure.message);
        }
        if self.kind == Kind::Urls {
            let text = parameters.text("urls");
            if !text.unwrap_or_default().contains("{{")
                && let Err(failure) = urls(text)
            {
                problems.push(failure.message);
            }
        }
        if self.kind == Kind::Project && lines(parameters.text("urlRoles")).len() > MAX_URLS {
            problems.push(format!("at most {MAX_URLS} other pages"));
        }
        problems
    }
}

/// What `workspace.open-project` found for one resource.
enum Found {
    Value(deckpilot_core::MachineResourceBinding),
    Missing(String),
}

impl WorkspaceAction {
    fn launch(&self, plan: &LaunchPlan) -> Result<(), Failure> {
        self.launcher.launch(plan).map_err(|e| launch_error(&e))
    }

    fn open_in_editor(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        let position = Position::new(parameters.integer("line"), parameters.integer("column"))?;
        let application = support::resolve(
            context,
            parameters.resource("application"),
            Some(ResourceKind::Application),
            "editor",
        )?;
        if application.contains(['/', '\\']) {
            support::existing(application.clone().into(), false)?;
        }
        let repository = ResourceRef::ProjectRole {
            role: REPOSITORY_ROLE.into(),
        };
        let base = support::resolve(
            context,
            Some(parameters.resource("location").unwrap_or(&repository)),
            Some(ResourceKind::Directory),
            "folder",
        )?;
        let relative = optional(parameters, "path");
        let path = support::existing_inside(&base, relative, relative.is_none())?;
        if position.is_some() && path.is_dir() {
            return Err(fail(
                codes::INVALID_POSITION,
                "a line needs a file, not a folder",
            ));
        }
        let editor = Editor::parse(parameters.text("editor").unwrap_or("other"));
        let (plan, located) =
            editor::editor_plan(Os::current(), editor, &application, &path, position)
                .map_err(|e| launch_error(&e))?;
        self.launch(&plan)?;
        Ok(
            ActionResult::success(format!("opened {} in {application}", path.display()))
                .with_data("path", path.display().to_string())
                .with_data("application", application)
                .with_data("located", located.to_string()),
        )
    }

    fn browser(
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<Option<String>, Failure> {
        parameters
            .resource("browser")
            .map(|reference| {
                support::resolve(
                    context,
                    Some(reference),
                    Some(ResourceKind::Application),
                    "browser",
                )
            })
            .transpose()
    }

    fn open_urls(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        let urls = urls(parameters.text("urls"))?;
        let browser = Self::browser(parameters, context)?;
        let mut opened = Vec::new();
        let mut first_failure = None;
        for url in &urls {
            let outcome = editor::url_plan(Os::current(), url, browser.as_deref())
                .map_err(|e| launch_error(&e))
                .and_then(|plan| self.launch(&plan));
            match outcome {
                Ok(()) => opened.push(url.clone()),
                Err(failure) => {
                    first_failure.get_or_insert(failure);
                }
            }
        }
        if opened.is_empty()
            && let Some(failure) = first_failure
        {
            return Err(failure);
        }
        let failed = urls.len() - opened.len();
        Ok(ActionResult::success(if failed == 0 {
            format!("opened {} pages", opened.len())
        } else {
            format!(
                "opened {} pages; {failed} could not be opened",
                opened.len()
            )
        })
        .with_data("opened", opened.len().to_string())
        .with_data("failed", failed.to_string())
        .with_data("urls", opened.join("\n")))
    }

    #[allow(clippy::too_many_lines)] // Each resource and its report.
    fn open_project(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        let project = context
            .project()
            .map_err(ActionResult::from)?
            .ok_or_else(|| fail("no-active-project", "no project is active"))?;
        let url_roles = lines(parameters.text("urlRoles"));
        if url_roles.len() > MAX_URLS {
            return Err(fail(
                codes::TOO_MANY_URLS,
                format!("at most {MAX_URLS} other pages"),
            ));
        }
        let find = |role: &str| -> Result<Found, Failure> {
            match context.resolve_project_resource(project, role) {
                Ok(binding) => Ok(Found::Value(binding.clone())),
                Err(ResolveError::NotPermitted) => Err(fail(
                    "not-permitted",
                    ResolveError::NotPermitted.to_string(),
                )),
                Err(error) => Ok(Found::Missing(error.to_string())),
            }
        };
        let browser = Self::browser(parameters, context)?;
        let os = Os::current();
        let mut opened: Vec<String> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        let mut attempt =
            |role: &str, plan: Result<LaunchPlan, Failure>, missing: &mut Vec<String>| match plan
                .and_then(|plan| self.launch(&plan))
            {
                Ok(()) => opened.push(role.to_owned()),
                Err(failure) => missing.push(format!("{role}: {}", failure.message)),
            };

        if let Some(role) = optional(parameters, "repositoryRole") {
            let editor_role = optional(parameters, "editorRole").unwrap_or("editor");
            match (find(role)?, find(editor_role)?) {
                (Found::Missing(why), _) => missing.push(format!("{role}: {why}")),
                (_, Found::Missing(why)) => missing.push(format!("{editor_role}: {why}")),
                (Found::Value(folder), Found::Value(application)) => {
                    let plan = if folder.kind != ResourceKind::Directory {
                        Err(fail(system_codes::NOT_FOUND, "is not a folder"))
                    } else if application.kind != ResourceKind::Application {
                        Err(fail(
                            system_codes::NOT_FOUND,
                            format!("{editor_role} is not an application"),
                        ))
                    } else {
                        support::existing(PathBuf::from(&folder.value), true).and_then(|path| {
                            let editor =
                                Editor::parse(parameters.text("editor").unwrap_or("other"));
                            editor::editor_plan(os, editor, &application.value, &path, None)
                                .map(|(plan, _)| plan)
                                .map_err(|e| launch_error(&e))
                        })
                    };
                    attempt(role, plan, &mut missing);
                }
            }
        }
        if let Some(role) = optional(parameters, "docsRole") {
            match find(role)? {
                Found::Missing(why) => missing.push(format!("{role}: {why}")),
                Found::Value(docs) if docs.kind == ResourceKind::Directory => {
                    let plan =
                        support::existing(PathBuf::from(&docs.value), true).and_then(|path| {
                            launch::plan(os, &OpenTarget::Path(path)).map_err(|e| launch_error(&e))
                        });
                    attempt(role, plan, &mut missing);
                }
                Found::Value(docs) => attempt(
                    role,
                    page_plan(os, &docs.value, browser.as_deref()),
                    &mut missing,
                ),
            }
        }
        for role in url_roles {
            match find(role)? {
                Found::Missing(why) => missing.push(format!("{role}: {why}")),
                Found::Value(page) => attempt(
                    role,
                    page_plan(os, &page.value, browser.as_deref()),
                    &mut missing,
                ),
            }
        }

        let summary = |result: ActionResult| {
            result
                .with_data("projectKey", project.key.as_str())
                .with_data("opened", opened.len().to_string())
                .with_data("openedRoles", opened.join("\n"))
                .with_data("missingCount", missing.len().to_string())
                .with_data("missing", missing.join("\n"))
        };
        if opened.is_empty() {
            let why = if missing.is_empty() {
                "no resources were chosen".to_owned()
            } else {
                missing.join("; ")
            };
            return Err(summary(fail(
                codes::NOTHING_OPENED,
                format!("nothing of {} could be opened: {why}", project.key),
            )));
        }
        let message = if missing.is_empty() {
            format!("opened {} of {}", opened.join(", "), project.key)
        } else {
            format!(
                "opened {} of {}; missing: {}",
                opened.join(", "),
                project.key,
                missing.join("; ")
            )
        };
        Ok(summary(ActionResult::success(message)))
    }
}

/// How a page resource is opened.
fn page_plan(os: Os, value: &str, browser: Option<&str>) -> Result<LaunchPlan, Failure> {
    let url = editor::browser_url(value)?;
    editor::url_plan(os, &url, browser).map_err(|e| launch_error(&e))
}

#[cfg(test)]
mod tests;
