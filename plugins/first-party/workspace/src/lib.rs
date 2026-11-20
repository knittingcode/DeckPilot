//! `deckpilot.workspace`: the first-party Workspace plugin, the replacement
//! of the built-in `workspace` module (`docs/marketplace/module-migration.md`
//! §7, WP-B3). It opens a project's working set: its repository (or a bound
//! file at a line) in an editor, and web pages in the browser.
//!
//! **The same actions.** The three actions of the built-in, with the same
//! name (`workspace.<action>` → `deckpilot.workspace.<action>`), the same
//! parameters, outputs and error codes, on plugin API 1.9:
//!
//! * `open-in-editor` and `open-project` open bound locations in bound
//!   applications with `resource.open` (X4). The plugin never sees a path:
//!   its `resource` parameters and the project's roles reach it as opaque
//!   handles labelled with the binding's name, the agent picks the editor's
//!   arguments from the plan the action names, and checks that the user
//!   started the run.
//! * `open-urls` opens the pages the user typed into the action with
//!   `url.openParam` (X5): the plugin names the parameter and the line, the
//!   agent opens the literal value (http or https only).
//!
//! **What differs from the built-in**: pages always open in the default
//! browser (`browser` is accepted and noted); outputs name the bindings,
//! never the paths (`path` is the binding's name and the relative path).
//! A `path` inside the folder is opened by the agent (`resource.open`'s
//! `path`), which refuses one that leaves the folder, by `..` or through
//! a link, and one that does not exist.
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;
use deckpilot_plugin_sdk::{
    EditorPlan, PRIVILEGED_API_LEVEL, Position, ResourceHandle, ResourceKind, ResourceOpen,
    TranslateArg,
};

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.workspace";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The editor argument plans, as the built-in's `editor` choice.
pub const EDITORS: [&str; 6] = ["vscode", "cursor", "jetbrains", "zed", "sublime", "other"];
/// Most pages one action opens.
pub const MAX_URLS: usize = 10;
/// The highest line or column.
pub const MAX_POSITION: i64 = 1_000_000;
/// The project resource that is the repository, by default.
pub const REPOSITORY_ROLE: &str = "repository";

/// Stable error codes of the plugin's actions (the built-in's), besides the
/// shared ones.
pub mod codes {
    pub const INVALID_POSITION: &str = "invalid-position";
    pub const NO_URLS: &str = "no-urls";
    pub const TOO_MANY_URLS: &str = "too-many-urls";
    pub const NOTHING_OPENED: &str = "nothing-opened";
    /// A `path` outside the folder, or with a file instead of a folder.
    pub const INVALID_PATH: &str = "invalid-path";
    /// Shared codes the actions use.
    pub const NO_ACTIVE_PROJECT: &str = "no-active-project";
    pub const INVALID_URL: &str = "invalid-url";
}

/// The English texts of the messages, as `locales/en.json` has them
/// (checked by the unit tests): the plugin's answer when a translation
/// cannot be had.
pub mod messages {
    pub const OPENED_IN: (&str, &str) = ("opened-in", "opened {{path}} in {{application}}");
    pub const LINE_NEEDS_FILE: (&str, &str) =
        ("line-needs-file", "a line needs a file, not a folder");
    pub const COLUMN_NEEDS_LINE: (&str, &str) = ("column-needs-line", "a column needs a line");
    pub const POSITION_RANGE: (&str, &str) =
        ("position-range", "a line and column are from 1 to 1000000");
    pub const PATH_NEEDS_FOLDER: (&str, &str) =
        ("path-needs-folder", "a path needs a folder, not a file");
    pub const NO_URLS: (&str, &str) = ("no-urls", "no page to open");
    pub const TOO_MANY_URLS: (&str, &str) =
        ("too-many-urls", "at most 10 pages can be opened at once");
    pub const TOO_MANY_ROLES: (&str, &str) = ("too-many-roles", "at most 10 other pages");
    pub const NOT_WEB: (&str, &str) = ("not-web", "“{{url}}” must be an http or https address");
    pub const OPENED_PAGES: (&str, &str) = ("opened-pages", "opened {{count}} pages");
    pub const OPENED_PAGES_FAILED: (&str, &str) = (
        "opened-pages-failed",
        "opened {{count}} pages; {{failed}} could not be opened",
    );
    pub const DEFAULT_BROWSER: (&str, &str) = ("default-browser", " (in the default browser)");
    pub const NO_ACTIVE_PROJECT: (&str, &str) = ("no-active-project", "no project is active");
    pub const NOTHING_CHOSEN: (&str, &str) = ("nothing-chosen", "no resources were chosen");
    pub const NOTHING_OPENED: (&str, &str) = (
        "nothing-opened",
        "nothing of {{project}} could be opened: {{why}}",
    );
    pub const OPENED_PROJECT: (&str, &str) = ("opened-project", "opened {{roles}} of {{project}}");
    pub const OPENED_PROJECT_MISSING: (&str, &str) = (
        "opened-project-missing",
        "opened {{roles}} of {{project}}; missing: {{missing}}",
    );
    pub const NOT_A_FOLDER: (&str, &str) = ("not-a-folder", "is not a folder");
    pub const NOT_AN_APPLICATION: (&str, &str) =
        ("not-an-application", "{{role}} is not an application");

    /// Every message, for the tests.
    pub const ALL: &[(&str, &str)] = &[
        OPENED_IN,
        LINE_NEEDS_FILE,
        COLUMN_NEEDS_LINE,
        POSITION_RANGE,
        PATH_NEEDS_FOLDER,
        NO_URLS,
        TOO_MANY_URLS,
        TOO_MANY_ROLES,
        NOT_WEB,
        OPENED_PAGES,
        OPENED_PAGES_FAILED,
        DEFAULT_BROWSER,
        NO_ACTIVE_PROJECT,
        NOTHING_CHOSEN,
        NOTHING_OPENED,
        OPENED_PROJECT,
        OPENED_PROJECT_MISSING,
        NOT_A_FOLDER,
        NOT_AN_APPLICATION,
    ];
}

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches` in the unit tests). `resource`
/// parameters reach the plugin as optional texts, the handles: the agent
/// checks `required`, and leaves out one whose binding did not resolve so
/// that `resource.resolve` can say why.
pub fn plugin() -> Plugin {
    let editor = || Param::choice("editor", &EDITORS).default("other");
    let role = |key: &str, default: &str| Param::text(key).max_length(64).default(default);
    Plugin::new(ID, VERSION)
        .requires_api(PRIVILEGED_API_LEVEL)
        .action(
            Action::new("open-in-editor", open_in_editor)
                .param(Param::text("application"))
                .param(editor())
                .param(Param::text("location"))
                .param(Param::text("path").max_length(1024))
                .param(Param::integer("line").min(1.0).max(1_000_000.0))
                .param(Param::integer("column").min(1.0).max(1_000_000.0)),
        )
        .action(
            Action::new("open-urls", open_urls)
                .param(Param::text("urls").required())
                .param(Param::text("browser")),
        )
        .action(
            Action::new("open-project", open_project)
                .param(editor())
                .param(role("editorRole", "editor"))
                .param(role("repositoryRole", REPOSITORY_ROLE))
                .param(role("docsRole", "docs"))
                .param(Param::text("urlRoles"))
                .param(Param::text("browser")),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);

// ------------------------------------------------------------- helpers

/// `message` in the app's language with `params`, or its English text when
/// no translation can be had (an older agent, a parameter over 256
/// characters).
fn say(ctx: &mut Context<'_>, message: (&str, &str), params: &[(&str, TranslateArg)]) -> String {
    let (key, english) = message;
    ctx.t(&format!("messages.{key}"), params)
        .unwrap_or_else(|_| fill(english, params))
}

/// `template` with its `{{name}}` placeholders filled.
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

fn arg(value: impl Into<TranslateArg>) -> TranslateArg {
    value.into()
}

fn is_code(text: &str) -> bool {
    text.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A capability's refusal as the action's code and message: the agent's
/// reason code at the start of its message (`no-active-project: …`), or
/// the shared code of the kind of refusal.
pub fn refusal(error: &CapabilityError) -> (String, String) {
    if let Some((code, message)) = error.detail().split_once(": ")
        && is_code(code)
    {
        return (code.to_owned(), message.to_owned());
    }
    let code = match error.kind() {
        CapabilityErrorKind::NotPermitted => "not-permitted",
        CapabilityErrorKind::Throttled => "throttled",
        CapabilityErrorKind::BadRequest => "invalid-parameters",
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

fn optional(args: &Args<'_>, key: &str) -> Result<Option<String>, PluginError> {
    Ok(args
        .get::<Option<String>>(key)?
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty()))
}

/// Non-empty trimmed lines, as the agent counts them (`lines_of`).
fn lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

/// The line and column of `open-in-editor`, checked as the built-in
/// checks them.
fn position(
    ctx: &mut Context<'_>,
    line: Option<i64>,
    column: Option<i64>,
) -> Result<Option<Position>, PluginError> {
    let invalid = |ctx: &mut Context<'_>, message| {
        PluginError::new(codes::INVALID_POSITION, say(ctx, message, &[]))
    };
    let in_range = |n: i64| (1..=MAX_POSITION).contains(&n);
    match (line, column) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(invalid(ctx, messages::COLUMN_NEEDS_LINE)),
        (Some(line), column) if in_range(line) && column.is_none_or(in_range) => {
            Ok(Some(Position {
                line: u32::try_from(line).unwrap_or(1),
                column: column.and_then(|c| u32::try_from(c).ok()),
            }))
        }
        _ => Err(invalid(ctx, messages::POSITION_RANGE)),
    }
}

fn editor_plan(name: &str) -> EditorPlan {
    match name {
        "vscode" => EditorPlan::Vscode,
        "cursor" => EditorPlan::Cursor,
        "jetbrains" => EditorPlan::Jetbrains,
        "zed" => EditorPlan::Zed,
        "sublime" => EditorPlan::Sublime,
        _ => EditorPlan::Other,
    }
}

/// Whether the editor's plan passes a line (every plan but `other`).
fn passes_a_line(plan: EditorPlan) -> bool {
    plan != EditorPlan::Other
}

/// The handle of the `resource` parameter `key`; `None` when it is not
/// set.
fn resource_param(
    ctx: &mut Context<'_>,
    key: &str,
    kinds: &[ResourceKind],
) -> Result<Option<ResourceHandle>, PluginError> {
    match ctx.resource_param(key, kinds) {
        Ok(handle) => Ok(Some(handle)),
        Err(e) if refusal(&e).0 == "resource-not-set" => Ok(None),
        Err(e) => Err(failed(&e)),
    }
}

// ------------------------------------------------------------- actions

fn open_in_editor(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let position = position(ctx, args.get("line")?, args.get("column")?)?;
    let path = optional(args, "path")?;
    let application = resource_param(ctx, "application", &[ResourceKind::Application])?
        .ok_or_else(|| {
            PluginError::new(
                "invalid-parameters",
                "parameter `application` is missing".to_owned(),
            )
        })?;
    let location = match resource_param(
        ctx,
        "location",
        &[ResourceKind::Directory, ResourceKind::File],
    )? {
        Some(location) => location,
        None => ctx
            .resource_role(REPOSITORY_ROLE, &[ResourceKind::Directory])
            .map_err(|e| failed(&e))?,
    };
    if path.is_some() && location.kind != ResourceKind::Directory {
        let message = say(ctx, messages::PATH_NEEDS_FOLDER, &[]);
        return Err(PluginError::new(codes::INVALID_PATH, message));
    }
    // Inside the folder the agent knows whether the path is a file.
    if position.is_some() && path.is_none() && location.kind == ResourceKind::Directory {
        let message = say(ctx, messages::LINE_NEEDS_FILE, &[]);
        return Err(PluginError::new(codes::INVALID_POSITION, message));
    }
    let editor: String = args.get("editor")?;
    let plan = editor_plan(&editor);
    ctx.open_resource(ResourceOpen {
        handle: location.handle.clone(),
        path: path.clone(),
        with: Some(application.handle.clone()),
        position,
        editor: Some(plan),
    })
    .map_err(|e| {
        let refused = failed(&e);
        if refused.code() == codes::INVALID_POSITION {
            let message = say(ctx, messages::LINE_NEEDS_FILE, &[]);
            PluginError::new(codes::INVALID_POSITION, message)
        } else {
            refused
        }
    })?;
    let located = position.is_some() && passes_a_line(plan);
    let shown = match &path {
        Some(path) => format!("{}/{path}", location.label),
        None => location.label.clone(),
    };
    let message = say(
        ctx,
        messages::OPENED_IN,
        &[
            ("path", arg(shown.as_str())),
            ("application", arg(application.label.as_str())),
        ],
    );
    Ok(Output::message(message)
        .with("path", shown)
        .with("application", application.label)
        .with("located", located))
}

/// The pages of `open-urls`, all checked before any is opened: at least
/// one, at most [`MAX_URLS`], each an http or https address.
pub fn urls(ctx: &mut Context<'_>, text: &str) -> Result<Vec<String>, PluginError> {
    let lines = lines(text);
    if lines.is_empty() {
        let message = say(ctx, messages::NO_URLS, &[]);
        return Err(PluginError::new(codes::NO_URLS, message));
    }
    if lines.len() > MAX_URLS {
        let message = say(ctx, messages::TOO_MANY_URLS, &[]);
        return Err(PluginError::new(codes::TOO_MANY_URLS, message));
    }
    lines
        .into_iter()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            let web = ["http://", "https://"]
                .iter()
                .any(|scheme| lower.len() > scheme.len() && lower.starts_with(scheme));
            if web {
                Ok(line.to_owned())
            } else {
                let message = say(ctx, messages::NOT_WEB, &[("url", arg(line))]);
                Err(PluginError::new(codes::INVALID_URL, message))
            }
        })
        .collect()
}

fn open_urls(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let text: String = args.get("urls")?;
    let urls = urls(ctx, &text)?;
    // A browser the user chose, bound: noted, since pages open in the
    // default browser (`resource.open` and `url.openParam` take no
    // application for a page).
    let chose_browser = args.has("browser");
    let mut opened = Vec::new();
    let mut first_failure = None;
    for (index, url) in urls.iter().enumerate() {
        match ctx.open_url_param("urls", u32::try_from(index).ok()) {
            Ok(()) => opened.push(url.clone()),
            Err(e) => {
                first_failure.get_or_insert_with(|| failed(&e));
            }
        }
    }
    if opened.is_empty()
        && let Some(failure) = first_failure
    {
        return Err(failure);
    }
    let failed = urls.len() - opened.len();
    let count = arg(i64::try_from(opened.len()).unwrap_or_default());
    let mut message = if failed == 0 {
        say(ctx, messages::OPENED_PAGES, &[("count", count)])
    } else {
        let failed = arg(i64::try_from(failed).unwrap_or_default());
        say(
            ctx,
            messages::OPENED_PAGES_FAILED,
            &[("count", count), ("failed", failed)],
        )
    };
    if chose_browser {
        message.push_str(&say(ctx, messages::DEFAULT_BROWSER, &[]));
    }
    Ok(Output::message(message)
        .with("opened", opened.len() as i64)
        .with("failed", failed as i64)
        .with("urls", opened.join("\n")))
}

/// What `open-project` found for one role.
enum Found {
    Bound(ResourceHandle),
    Missing(String),
}

/// The role `role` of the active project, of one of `kinds`. A role that
/// is not defined, not bound, of another kind or not there is missing; a
/// refusal of the resources themselves fails the action, as in the
/// built-in.
fn find(ctx: &mut Context<'_>, role: &str, kinds: &[ResourceKind]) -> Result<Found, PluginError> {
    match ctx.resource_role(role, kinds) {
        Ok(handle) => Ok(Found::Bound(handle)),
        Err(e) => {
            let (code, message) = refusal(&e);
            if code == "not-permitted" {
                Err(PluginError::new(code, message))
            } else {
                Ok(Found::Missing(message))
            }
        }
    }
}

/// Opens one resource of `open-project`: its role is opened, or missing
/// with why.
fn attempt(
    ctx: &mut Context<'_>,
    role: &str,
    open: ResourceOpen,
    opened: &mut Vec<String>,
    missing: &mut Vec<String>,
) {
    match ctx.open_resource(open) {
        Ok(()) => opened.push(role.to_owned()),
        Err(e) => missing.push(format!("{role}: {}", refusal(&e).1)),
    }
}

fn open_project(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let project = ctx.profile().map_err(|e| failed(&e))?.project;
    let Some(project) = project else {
        let message = say(ctx, messages::NO_ACTIVE_PROJECT, &[]);
        return Err(PluginError::new(codes::NO_ACTIVE_PROJECT, message));
    };
    let url_text = args.get::<Option<String>>("urlRoles")?.unwrap_or_default();
    let url_roles = lines(&url_text);
    if url_roles.len() > MAX_URLS {
        let message = say(ctx, messages::TOO_MANY_ROLES, &[]);
        return Err(PluginError::new(codes::TOO_MANY_URLS, message));
    }
    let editor: String = args.get("editor")?;
    let plan = editor_plan(&editor);
    let mut opened: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();

    if let Some(role) = optional(args, "repositoryRole")? {
        let editor_role = optional(args, "editorRole")?.unwrap_or_else(|| "editor".to_owned());
        let folder = find(ctx, &role, &[ResourceKind::Directory])?;
        let application = find(ctx, &editor_role, &[ResourceKind::Application])?;
        match (folder, application) {
            (Found::Missing(why), _) => missing.push(format!("{role}: {why}")),
            (_, Found::Missing(why)) => missing.push(format!("{editor_role}: {why}")),
            (Found::Bound(folder), Found::Bound(application)) => {
                let open = ResourceOpen {
                    handle: folder.handle,
                    path: None,
                    with: Some(application.handle),
                    position: None,
                    editor: Some(plan),
                };
                attempt(ctx, &role, open, &mut opened, &mut missing);
            }
        }
    }
    let page = |handle: ResourceHandle| ResourceOpen {
        handle: handle.handle,
        path: None,
        with: None,
        position: None,
        editor: None,
    };
    if let Some(role) = optional(args, "docsRole")? {
        match find(ctx, &role, &[ResourceKind::Url, ResourceKind::Directory])? {
            Found::Missing(why) => missing.push(format!("{role}: {why}")),
            Found::Bound(docs) => attempt(ctx, &role, page(docs), &mut opened, &mut missing),
        }
    }
    for role in url_roles {
        match find(ctx, role, &[ResourceKind::Url])? {
            Found::Missing(why) => missing.push(format!("{role}: {why}")),
            Found::Bound(handle) => {
                attempt(ctx, role, page(handle), &mut opened, &mut missing);
            }
        }
    }

    let key = project.key.clone();
    let summary = |output: Output, opened: &[String], missing: &[String]| {
        output
            .with("projectKey", key.as_str())
            .with("opened", opened.len() as i64)
            .with("openedRoles", opened.join("\n"))
            .with("missingCount", missing.len() as i64)
            .with("missing", missing.join("\n"))
    };
    if opened.is_empty() {
        let why = if missing.is_empty() {
            say(ctx, messages::NOTHING_CHOSEN, &[])
        } else {
            missing.join("; ")
        };
        let message = say(
            ctx,
            messages::NOTHING_OPENED,
            &[("project", arg(key.as_str())), ("why", arg(why))],
        );
        return Err(PluginError::new(codes::NOTHING_OPENED, message));
    }
    let roles = arg(opened.join(", "));
    let message = if missing.is_empty() {
        say(
            ctx,
            messages::OPENED_PROJECT,
            &[("roles", roles), ("project", arg(key.as_str()))],
        )
    } else {
        say(
            ctx,
            messages::OPENED_PROJECT_MISSING,
            &[
                ("roles", roles),
                ("project", arg(key.as_str())),
                ("missing", arg(missing.join("; "))),
            ],
        )
    };
    Ok(summary(Output::message(message), &opened, &missing))
}

#[cfg(test)]
mod tests;
