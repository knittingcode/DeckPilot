//! `deckpilot.projects`: the first-party Projects plugin, the user-facing
//! part of the built-in `projects` module (`docs/marketplace/module-migration.md`
//! §7 X6 to X8, §11, WP-B4).
//!
//! **What moves and what stays.** The project data model (`Project`, the
//! active project context, the storage tables, portability and IPC) stays
//! in the core as a neutral primitive. The plugin owns every user-facing
//! part of it:
//!
//! * **The actions**, with the built-in's names (`projects.<action>` →
//!   `deckpilot.projects.<action>`), parameters, outputs, messages and
//!   error codes: `cycle`, `select`, `current`, `repository`,
//!   `documentation`, `resolve-resource`, and the deprecated `next`,
//!   `previous` and `active`. Switching goes through `project.select` and
//!   `project.cycle` (X6): the agent applies it to its active project
//!   context, announces `projectChanged` naming the plugin, and allows it
//!   only in runs the user started. Reading goes through `project.current`
//!   and `project.list`; the resources through `resource.resolve` (X2).
//! * **`wrapAround`**, its setting, read when a cycle runs.
//! * **The variables** `current.id`, `.key` and `.name` (the built-in's
//!   `projects.current.*`): host-published (X7), computed by the agent
//!   from the project in the context while the plugin is enabled; the
//!   plugin never writes them.
//! * **The Projects screen, the active project in the status bar and its
//!   dashboard card** (`hostViews`, X8): drawn by the app from core data,
//!   shown while the plugin is enabled.
//!
//! **What differs from the built-in** (host gaps, the report of WP-B4):
//! the project parameter's key is `projectId` (manifest keys are camel
//! case; the built-in's is `project_id`); `current` has no `profileId`
//! (no operation gives the profile's id); the resource actions report the
//! bound path of folders and files only (`resources.paths`), and only of
//! bindings that exist; key faces show no project detail.
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;
use deckpilot_plugin_sdk::{
    CycleDirection, PRIVILEGED_API_LEVEL, ProjectInfo, ResourceHandle, ResourceKind,
    ResourceResolve, TranslateArg,
};

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.projects";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The setting: whether next and previous wrap around at the ends.
pub const WRAP_AROUND: &str = "wrapAround";
/// The project parameter (the built-in's `project_id`).
pub const PROJECT_ID: &str = "projectId";
/// The resource name of `repository` and `documentation`.
pub const REPOSITORY_ROLE: &str = "repository";
pub const DOCUMENTATION_ROLE: &str = "docs";

/// Stable error codes of the plugin's actions (the built-in's), besides
/// the shared ones.
pub mod codes {
    pub const NO_PROJECTS: &str = "no-projects";
    pub const AT_LAST: &str = "at-last-project";
    pub const AT_FIRST: &str = "at-first-project";
    pub const NOT_IN_PROFILE: &str = "project-not-in-profile";
    /// Shared.
    pub const NO_ACTIVE_PROJECT: &str = "no-active-project";
    pub const INVALID_PARAMETERS: &str = "invalid-parameters";
}

/// The English texts of the messages, as `locales/en.json` has them
/// (checked by the unit tests).
pub mod messages {
    pub const ONLY_PROJECT: (&str, &str) = ("only-project", "{{key}} is the only project");
    pub const SWITCHED: (&str, &str) = ("switched", "switched to {{key}}");
    pub const SELECTED: (&str, &str) = ("selected", "selected {{key}}");
    pub const IS_ACTIVE: (&str, &str) = ("is-active", "{{key}} is active");
    pub const CURRENT: (&str, &str) = ("current", "{{key}} ({{name}})");
    pub const NO_ACTIVE_PROJECT: (&str, &str) = ("no-active-project", "no project is active");
    pub const NOT_IN_PROFILE: (&str, &str) = (
        "not-in-profile",
        "the project does not belong to the active profile",
    );
    pub const NO_RESOURCE: (&str, &str) = ("no-resource", "no resource configured");
    pub const RESOLVED: (&str, &str) = ("resolved", "{{key}} {{role}}: {{value}}");

    pub const ALL: &[(&str, &str)] = &[
        ONLY_PROJECT,
        SWITCHED,
        SELECTED,
        IS_ACTIVE,
        CURRENT,
        NO_ACTIVE_PROJECT,
        NOT_IN_PROFILE,
        NO_RESOURCE,
        RESOLVED,
    ];
}

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches` in the unit tests).
pub fn plugin() -> Plugin {
    // One project per event: a key, or a dial's detent or press.
    let steps = || {
        Controls::new()
            .key([
                Input::Press,
                Input::Release,
                Input::Click,
                Input::DoubleClick,
                Input::LongPress,
            ])
            .encoder([
                Input::RotateClockwise,
                Input::RotateCounterClockwise,
                Input::Press,
                Input::Click,
            ])
    };
    let project = || Param::text(PROJECT_ID);
    Plugin::new(ID, VERSION)
        .requires_api(PRIVILEGED_API_LEVEL)
        .action(
            Action::new("cycle", cycle)
                .param(Param::choice("direction", &["next", "previous"]).default("next"))
                .controls(steps()),
        )
        .action(Action::new("select", select).param(project()))
        .action(Action::new("current", current))
        .action(Action::new("repository", repository).param(project()))
        .action(Action::new("documentation", documentation).param(project()))
        .action(
            Action::new("resolve-resource", resolve_resource)
                .param(project())
                .param(Param::text("resource").max_length(64).required()),
        )
        .action(Action::new("next", next).controls(steps()))
        .action(Action::new("previous", previous).controls(steps()))
        .action(Action::new("active", active))
}

deckpilot_plugin_sdk::plugin!(plugin);

// ------------------------------------------------------------- helpers

/// `message` in the app's language with `params`, or its English text.
fn say(ctx: &mut Context<'_>, message: (&str, &str), params: &[(&str, TranslateArg)]) -> String {
    let (key, english) = message;
    ctx.t(&format!("messages.{key}"), params)
        .unwrap_or_else(|_| fill(english, params))
}

fn fill(template: &str, params: &[(&str, TranslateArg)]) -> String {
    let mut text = template.to_owned();
    for (name, value) in params {
        let value = match value {
            TranslateArg::Text(text) => text.clone(),
            TranslateArg::Number(n) => n.to_string(),
        };
        text = text.replace(&format!("{{{{{name}}}}}"), &value);
    }
    text
}

fn arg(value: &str) -> TranslateArg {
    TranslateArg::from(value)
}

fn is_code(text: &str) -> bool {
    text.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A capability's refusal as the action's code and message: the agent's
/// reason code at the start of its message (`at-last-project: …`), or the
/// shared code of the kind of refusal.
pub fn refusal(error: &CapabilityError) -> (String, String) {
    if let Some((code, message)) = error.detail().split_once(": ")
        && is_code(code)
    {
        return (code.to_owned(), message.to_owned());
    }
    let code = match error.kind() {
        CapabilityErrorKind::NotPermitted => "not-permitted",
        CapabilityErrorKind::Throttled => "throttled",
        CapabilityErrorKind::BadRequest => codes::INVALID_PARAMETERS,
        CapabilityErrorKind::Unsupported => "unsupported",
        CapabilityErrorKind::Unavailable => "plugin-unavailable",
        _ => "plugin-error",
    };
    (code.to_owned(), error.detail().to_owned())
}

fn failed(error: &CapabilityError) -> PluginError {
    let (code, message) = refusal(error);
    PluginError::new(code, message)
}

/// The project's identity as the action's outputs.
fn with_project(output: Output, project: &ProjectInfo) -> Output {
    output
        .with("projectId", project.id.as_str())
        .with("projectKey", project.key.as_str())
        .with("projectName", project.name.as_str())
}

fn no_active_project(ctx: &mut Context<'_>) -> PluginError {
    let message = say(ctx, messages::NO_ACTIVE_PROJECT, &[]);
    PluginError::new(codes::NO_ACTIVE_PROJECT, message)
}

fn current_project(ctx: &mut Context<'_>) -> Result<Option<ProjectInfo>, PluginError> {
    ctx.current_project().map_err(|e| failed(&e))
}

/// The `projectId` parameter, when set.
fn named_project(args: &Args<'_>) -> Result<Option<String>, PluginError> {
    Ok(args
        .get::<Option<String>>(PROJECT_ID)?
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty()))
}

/// The project an action refers to: its `projectId` parameter (a project
/// of the active profile), or the active project.
fn target_project(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<ProjectInfo, PluginError> {
    match named_project(args)? {
        Some(id) => {
            let projects = ctx.projects().map_err(|e| failed(&e))?;
            match projects.into_iter().find(|p| p.id == id) {
                Some(project) => Ok(project),
                None => {
                    let message = say(ctx, messages::NOT_IN_PROFILE, &[]);
                    Err(PluginError::new(codes::NOT_IN_PROFILE, message))
                }
            }
        }
        None => current_project(ctx)?.ok_or_else(|| no_active_project(ctx)),
    }
}

// ----------------------------------------------------------- switching

/// One step through the profile's project order, as `cycle`, `next` and
/// `previous` report it: the project the step led to. Without projects it
/// fails with `no-projects`; with a single project that is already active
/// it succeeds without changing anything; without wrap-around the ends
/// fail with `at-last-project` and `at-first-project` (the agent's step).
fn step(ctx: &mut Context<'_>, direction: CycleDirection) -> ActionResult {
    let wrap = ctx
        .settings()
        .map_err(|e| failed(&e))?
        .boolean(WRAP_AROUND)
        .unwrap_or(true);
    let projects = ctx.projects().map_err(|e| failed(&e))?;
    let current = current_project(ctx)?;
    if let ([only], Some(current)) = (projects.as_slice(), &current)
        && only.id == current.id
    {
        let message = say(ctx, messages::ONLY_PROJECT, &[("key", arg(&current.key))]);
        return Ok(with_project(Output::message(message), current));
    }
    ctx.cycle_project(direction, Some(wrap))
        .map_err(|e| failed(&e))?;
    let now = current_project(ctx)?.ok_or_else(|| no_active_project(ctx))?;
    let message = say(ctx, messages::SWITCHED, &[("key", arg(&now.key))]);
    Ok(with_project(Output::message(message), &now))
}

fn cycle(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let direction = match args.get::<String>("direction")?.as_str() {
        "previous" => CycleDirection::Previous,
        _ => CycleDirection::Next,
    };
    step(ctx, direction)
}

/// Deprecated: `cycle` with direction `next`.
fn next(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    step(ctx, CycleDirection::Next)
}

/// Deprecated: `cycle` with direction `previous`.
fn previous(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    step(ctx, CycleDirection::Previous)
}

/// Makes the named project active; without one, reports the active
/// project and changes nothing.
fn select(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let Some(id) = named_project(args)? else {
        let project = current_project(ctx)?.ok_or_else(|| no_active_project(ctx))?;
        let message = say(ctx, messages::IS_ACTIVE, &[("key", arg(&project.key))]);
        return Ok(with_project(Output::message(message), &project));
    };
    if let Err(e) = ctx.select_project(&id) {
        let (code, message) = refusal(&e);
        if code == "not-found" || code == codes::INVALID_PARAMETERS {
            let message = say(ctx, messages::NOT_IN_PROFILE, &[]);
            return Err(PluginError::new(codes::NOT_IN_PROFILE, message));
        }
        return Err(PluginError::new(code, message));
    }
    let project = current_project(ctx)?.ok_or_else(|| no_active_project(ctx))?;
    let message = say(ctx, messages::SELECTED, &[("key", arg(&project.key))]);
    Ok(with_project(Output::message(message), &project))
}

// ------------------------------------------------------------- reading

fn report(ctx: &mut Context<'_>) -> Result<(ProjectInfo, String), PluginError> {
    let project = current_project(ctx)?.ok_or_else(|| no_active_project(ctx))?;
    let message = say(
        ctx,
        messages::CURRENT,
        &[("key", arg(&project.key)), ("name", arg(&project.name))],
    );
    Ok((project, message))
}

/// The current project: its id, key and name. The built-in's `profileId`
/// is left out (no operation gives the profile's id).
fn current(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let (project, message) = report(ctx)?;
    Ok(with_project(Output::message(message), &project))
}

/// Deprecated: `current`.
fn active(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let (project, message) = report(ctx)?;
    Ok(with_project(Output::message(message), &project))
}

fn repository(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    resolve(ctx, args, REPOSITORY_ROLE)
}

fn documentation(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    resolve(ctx, args, DOCUMENTATION_ROLE)
}

fn resolve_resource(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let role: String = args.get("resource")?;
    resolve(ctx, args, role.trim())
}

fn kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Directory => "directory",
        ResourceKind::File => "file",
        ResourceKind::Application => "application",
        ResourceKind::DockerEnvironment => "dockerEnvironment",
        ResourceKind::Url => "url",
    }
}

/// The handle of role `role` of `project` (the `projectId` parameter's, or
/// the run's), of any kind the agent issues handles for.
fn lookup(
    ctx: &mut Context<'_>,
    role: &str,
    project: Option<String>,
) -> Result<ResourceHandle, CapabilityError> {
    let request = |kinds: Vec<ResourceKind>| ResourceResolve {
        param: None,
        role: Some(role.to_owned()),
        project_id: project.clone(),
        kinds,
    };
    let first = ctx.resolve_resource(request(vec![
        ResourceKind::Directory,
        ResourceKind::File,
        ResourceKind::Application,
        ResourceKind::Url,
    ]));
    match first {
        Err(e) if refusal(&e).0 == "resource-kind-mismatch" => ctx
            .resolve_resource(request(vec![ResourceKind::DockerEnvironment]))
            .map_err(|_| e),
        other => other,
    }
}

/// Resolves resource `role` of the target project and reports it: the
/// project, the resource name, its binding's key, kind and, for a folder or
/// a file, the bound path.
fn resolve(ctx: &mut Context<'_>, args: &Args<'_>, role: &str) -> ActionResult {
    if role.is_empty() {
        let message = say(ctx, messages::NO_RESOURCE, &[]);
        return Err(PluginError::new(codes::INVALID_PARAMETERS, message));
    }
    let project = target_project(ctx, args)?;
    let named = named_project(args)?;
    let handle = lookup(ctx, role, named).map_err(|e| failed(&e))?;
    let value = handle.display_path.clone().unwrap_or_default();
    let shown = if value.is_empty() {
        handle.label.as_str()
    } else {
        value.as_str()
    };
    let message = say(
        ctx,
        messages::RESOLVED,
        &[
            ("key", arg(&project.key)),
            ("role", arg(role)),
            ("value", arg(shown)),
        ],
    );
    Ok(with_project(Output::message(message), &project)
        .with("resource", role)
        .with("resourceKey", handle.label.as_str())
        .with("kind", kind_name(handle.kind))
        .with("value", value))
}

#[cfg(test)]
mod tests;
