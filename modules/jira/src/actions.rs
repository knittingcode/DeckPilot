//! The action types of the `jira` module. All read; `jira.open` opens a
//! page in the browser.

use std::fmt::Write as _;
use std::sync::Arc;

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::launch::{self, Launcher, OpenTarget};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    CredentialId, ExecutionContext, Os, ParameterKind, ParameterSpec, ParameterValue, Profile,
};
use deckpilot_module_http::policy;
use serde_json::Value;

use crate::api::{self, Api, Failure, clean, fail};
use crate::{Deployment, JiraSettings, codes};

pub const MYSELF: &str = "jira.myself";
pub const ISSUE: &str = "jira.issue";
pub const ISSUES: &str = "jira.issues";
pub const OPEN: &str = "jira.open";

/// Where `jira.issues` takes its issues from.
pub const SOURCES: [&str; 2] = ["assigned-to-me", "filter"];
/// Most issues listed.
pub const MAX_LISTED: i64 = 100;
/// The fields read for a list.
const LIST_FIELDS: &str = "summary,status,assignee";

/// # Panics
///
/// Never for the constants of this module.
pub fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("the action type id is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Myself,
    Issue,
    Issues,
    Open,
}

pub fn handlers(
    settings: &Arc<JiraSettings>,
    launcher: &Arc<dyn Launcher>,
) -> Vec<Box<dyn ActionHandler>> {
    [Kind::Myself, Kind::Issue, Kind::Issues, Kind::Open]
        .into_iter()
        .map(|kind| {
            Box::new(JiraAction {
                definition: definition(kind),
                kind,
                settings: settings.clone(),
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

fn text(max_length: usize) -> ParameterKind {
    ParameterKind::Text {
        multiline: false,
        max_length: Some(max_length),
    }
}

fn required(mut spec: ParameterSpec, default: Option<ParameterValue>) -> ParameterSpec {
    spec.required = true;
    spec.default = default;
    spec
}

fn credential_parameter() -> ParameterSpec {
    parameter(
        "credential",
        "Credential",
        text(64),
        "The name of a stored credential. Empty: the module's (Settings › Modules › Jira). \
         Never the secret itself.",
    )
}

fn issue_parameter(required: bool) -> ParameterSpec {
    let mut spec = parameter("issue", "Issue", text(40), "An issue key such as ABC-123.");
    spec.required = required;
    spec
}

fn out(key: &str, label: &str, kind: ValueKind) -> OutputSpec {
    OutputSpec::new(key, label, kind)
}

fn reading(icon: &str, outputs: Vec<OutputSpec>) -> NodeSpec {
    NodeSpec {
        icon: Some(icon.into()),
        outputs,
        timeout_ms: 130_000,
        retry_safe: true,
        idempotent: true,
        cancellable: true,
        effect: ActionEffect::ReadOnly,
        ..NodeSpec::default()
    }
}

#[allow(clippy::too_many_lines)] // One table of the four action types.
fn definition(kind: Kind) -> ActionType {
    let (id_value, name, description, mut parameters, node) = match kind {
        Kind::Myself => (
            MYSELF,
            "Current Jira user",
            "The account the credential belongs to; also checks that it works.",
            Vec::new(),
            reading(
                "user",
                vec![
                    out("displayName", "Name", ValueKind::Text),
                    out("account", "Account id or user name", ValueKind::Text),
                    out("timeZone", "Time zone", ValueKind::Text).optional(),
                ],
            ),
        ),
        Kind::Issue => (
            ISSUE,
            "Jira issue",
            "An issue's summary, status, type, priority and assignee.",
            vec![issue_parameter(true)],
            reading(
                "ticket",
                vec![
                    out("key", "Key", ValueKind::Text),
                    // Written by people and may say anything.
                    out("summary", "Summary", ValueKind::Text).sensitive(),
                    out("status", "Status", ValueKind::Text),
                    out(
                        "statusCategory",
                        "Status category (todo, in-progress, done)",
                        ValueKind::Text,
                    ),
                    out("type", "Type", ValueKind::Text).optional(),
                    out("priority", "Priority", ValueKind::Text).optional(),
                    out("assignee", "Assignee", ValueKind::Text).optional(),
                    out("updated", "Updated", ValueKind::Text).optional(),
                    out("url", "Page", ValueKind::Url),
                ],
            ),
        ),
        Kind::Issues => (
            ISSUES,
            "Jira issues",
            "Counts and lists the issues assigned to you, or those of a saved filter.",
            vec![
                required(
                    parameter(
                        "source",
                        "Issues",
                        ParameterKind::Choice {
                            options: SOURCES.iter().map(|s| (*s).to_owned()).collect(),
                        },
                        "assigned-to-me: assigned to you; filter: a saved filter.",
                    ),
                    Some(ParameterValue::Choice("assigned-to-me".into())),
                ),
                parameter(
                    "filterId",
                    "Filter",
                    ParameterKind::Integer {
                        min: Some(1),
                        max: None,
                    },
                    "The saved filter's number (in its address, ?filter=12345).",
                ),
                parameter(
                    "project",
                    "Project",
                    text(255),
                    "Only issues of this project key, e.g. WEB. Empty: every project.",
                ),
                required(
                    parameter(
                        "includeDone",
                        "Include done issues",
                        ParameterKind::Boolean,
                        "Off: only issues whose status is not in the Done category.",
                    ),
                    Some(ParameterValue::Boolean(false)),
                ),
                required(
                    parameter(
                        "limit",
                        "Listed",
                        ParameterKind::Integer {
                            min: Some(1),
                            max: Some(MAX_LISTED),
                        },
                        "At most this many issues in the list.",
                    ),
                    Some(ParameterValue::Integer(20)),
                ),
            ],
            reading(
                "list-checks",
                vec![
                    out("count", "Issues", ValueKind::Integer),
                    out("countExact", "Count complete", ValueKind::Boolean),
                    out("listed", "Listed", ValueKind::Integer),
                    out("items", "Issues", ValueKind::Text).sensitive(),
                    out("truncated", "List truncated", ValueKind::Boolean),
                    out("firstKey", "First issue", ValueKind::Text).optional(),
                    out("firstUrl", "First issue's page", ValueKind::Url).optional(),
                ],
            ),
        ),
        Kind::Open => (
            OPEN,
            "Open in Jira",
            "Opens an issue or a Jira page in the browser.",
            vec![
                parameter(
                    "url",
                    "Page",
                    text(2048),
                    "An https page on the configured Jira site. Empty: the issue below.",
                ),
                issue_parameter(false),
            ],
            NodeSpec {
                icon: Some("external-link".into()),
                outputs: vec![out("url", "Opened", ValueKind::Url)],
                timeout_ms: 15_000,
                retry_safe: true,
                idempotent: false,
                cancellable: false,
                effect: ActionEffect::Opens,
                ..NodeSpec::default()
            },
        ),
    };
    if kind != Kind::Open {
        parameters.push(credential_parameter());
    }
    ActionType {
        id: id(id_value),
        name: name.into(),
        description: description.into(),
        category: "Jira".into(),
        parameters,
        requires_project: false,
        node,
    }
}

#[derive(Debug)]
struct JiraAction {
    definition: ActionType,
    kind: Kind,
    settings: Arc<JiraSettings>,
    launcher: Arc<dyn Launcher>,
}

fn optional<'a>(parameters: &'a ActionParameters, key: &str) -> Option<&'a str> {
    parameters
        .text(key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Text that is not a template, which is only known when the run renders
/// it.
fn literal<'a>(parameters: &'a ActionParameters, key: &str) -> Option<&'a str> {
    optional(parameters, key).filter(|v| !v.contains("{{"))
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// `fields.<field>.<key>` as text.
fn field(issue: &Value, field: &str, key: &str) -> Option<String> {
    issue
        .get("fields")
        .and_then(|f| f.get(field))
        .and_then(|v| string(v, key))
}

fn invalid_response(what: &str) -> Failure {
    fail(
        codes::INVALID_RESPONSE,
        format!("Jira's answer for {what} is not what was expected"),
    )
}

impl ActionHandler for JiraAction {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if self.kind == Kind::Open {
            return self.open(parameters, context).unwrap_or_else(|f| f);
        }
        let api = match Api::connect(&self.settings, parameters, context) {
            Ok(api) => api,
            Err(failure) => return failure,
        };
        let result = match self.kind {
            Kind::Myself => myself(&api),
            Kind::Issue => issue(&api, parameters),
            Kind::Issues => issues(&api, parameters),
            Kind::Open => unreachable!("handled above"),
        };
        api.scrub(result.unwrap_or_else(|failure| failure))
    }

    /// Problems found when the action is saved.
    fn check(&self, parameters: &ActionParameters, _: &Profile) -> Vec<String> {
        let mut problems = Vec::new();
        if let Some(key) = literal(parameters, "issue")
            && let Err(failure) = api::issue_key(key)
        {
            problems.push(failure.message);
        }
        if let Some(key) = literal(parameters, "project")
            && let Err(failure) = api::project_key(key)
        {
            problems.push(failure.message);
        }
        if let Some(name) = literal(parameters, "credential")
            && CredentialId::new(name).is_err()
        {
            problems.push(format!("“{}” is not a credential name", clean(name, 64)));
        }
        if let Some(url) = literal(parameters, "url")
            && let Ok(site) = api::site(&self.settings.site_url)
            && let Err(failure) = api::web_url(url, &site)
        {
            problems.push(failure.message);
        }
        if self.kind == Kind::Issues
            && parameters.text("source") == Some("filter")
            && parameters.integer("filterId").is_none()
        {
            problems.push("choose the saved filter's number".into());
        }
        if self.kind == Kind::Open
            && optional(parameters, "url").is_none()
            && optional(parameters, "issue").is_none()
        {
            problems.push("choose a page or an issue to open".into());
        }
        problems
    }
}

fn myself(api: &Api<'_>) -> Result<ActionResult, Failure> {
    let what = "the signed-in user";
    let user = api.get(api.endpoint(&["myself"], &[]), what)?;
    let name = string(&user, "displayName").ok_or_else(|| invalid_response(what))?;
    let account = string(&user, "accountId")
        .or_else(|| string(&user, "name"))
        .ok_or_else(|| invalid_response(what))?;
    let name = clean(&name, 200);
    let mut result = ActionResult::success(format!("signed in to Jira as {name}"))
        .with_data("displayName", name)
        .with_data("account", account);
    if let Some(zone) = string(&user, "timeZone") {
        result = result.with_data("timeZone", zone);
    }
    Ok(result)
}

fn status_category(issue: &Value) -> &'static str {
    let key = issue
        .get("fields")
        .and_then(|f| f.get("status"))
        .and_then(|s| s.get("statusCategory"))
        .and_then(|c| c.get("key"))
        .and_then(Value::as_str);
    match key {
        Some("new") => "todo",
        Some("indeterminate") => "in-progress",
        Some("done") => "done",
        _ => "unknown",
    }
}

fn issue(api: &Api<'_>, parameters: &ActionParameters) -> Result<ActionResult, Failure> {
    let key = api::issue_key(parameters.text("issue").unwrap_or_default())?;
    let what = format!("the issue {key}");
    let issue = api.get(
        api.endpoint(
            &["issue", &key],
            &[(
                "fields",
                "summary,status,issuetype,priority,assignee,updated",
            )],
        ),
        &what,
    )?;
    let key = string(&issue, "key").ok_or_else(|| invalid_response(&what))?;
    let status = field(&issue, "status", "name").unwrap_or_default();
    let mut result = ActionResult::success(format!("{key}: {}", clean(&status, 60)))
        .with_data("key", key.clone())
        .with_data(
            "summary",
            clean(
                &issue
                    .get("fields")
                    .and_then(|f| string(f, "summary"))
                    .unwrap_or_default(),
                500,
            ),
        )
        .with_data("status", clean(&status, 60))
        .with_data("statusCategory", status_category(&issue))
        .with_data("url", api.browse(&key));
    if let Some(kind) = field(&issue, "issuetype", "name") {
        result = result.with_data("type", clean(&kind, 60));
    }
    if let Some(priority) = field(&issue, "priority", "name") {
        result = result.with_data("priority", clean(&priority, 60));
    }
    if let Some(assignee) = field(&issue, "assignee", "displayName") {
        result = result.with_data("assignee", clean(&assignee, 120));
    }
    if let Some(updated) = issue.get("fields").and_then(|f| string(f, "updated")) {
        result = result.with_data("updated", updated);
    }
    Ok(result)
}

/// The search: built only from fixed parts, a filter number and a
/// validated project key.
fn search_jql(filter: Option<i64>, project: Option<&str>, include_done: bool) -> String {
    let mut jql = match filter {
        Some(id) => format!("filter = {id}"),
        None => "assignee = currentUser()".to_owned(),
    };
    if let Some(project) = project {
        let _ = write!(jql, " AND project = \"{project}\"");
    }
    if !include_done {
        jql.push_str(" AND statusCategory != Done");
    }
    jql.push_str(" ORDER BY updated DESC");
    jql
}

/// `ABC-1 Summary [Status] (@Assignee)`, one line.
fn issue_line(issue: &Value) -> String {
    let key = string(issue, "key").unwrap_or_default();
    let summary = clean(
        &issue
            .get("fields")
            .and_then(|f| string(f, "summary"))
            .unwrap_or_default(),
        200,
    );
    let status = clean(&field(issue, "status", "name").unwrap_or_default(), 60);
    let assignee = field(issue, "assignee", "displayName").map_or_else(
        || "unassigned".to_owned(),
        |a| format!("@{}", clean(&a, 120)),
    );
    format!("{key} {summary} [{status}] ({assignee})")
}

/// One page of a search.
fn search_page(
    api: &Api<'_>,
    jql: &str,
    size: usize,
    fields: &str,
    token: Option<&str>,
    what: &str,
) -> Result<Value, Failure> {
    let size = size.to_string();
    let mut query = vec![
        ("jql", jql),
        ("maxResults", size.as_str()),
        ("fields", fields),
    ];
    let path: &[&str] = match api.deployment() {
        Deployment::Cloud => {
            if let Some(token) = token {
                query.push(("nextPageToken", token));
            }
            &["search", "jql"]
        }
        Deployment::DataCenter => &["search"],
    };
    api.get(api.endpoint(path, &query), what)
}

/// Jira Cloud gives no total: counts by reading up to
/// [`api::MAX_PAGES`] pages of keys. Returns the count and whether it is
/// complete.
fn count_pages(api: &Api<'_>, jql: &str, what: &str) -> Result<(u64, bool), Failure> {
    let mut count = 0;
    let mut token: Option<String> = None;
    for _ in 0..api::MAX_PAGES {
        let page = search_page(api, jql, api::PER_PAGE, "key", token.as_deref(), what)?;
        count += page
            .get("issues")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_response(what))?
            .len() as u64;
        token = next_token(&page);
        if token.is_none() {
            return Ok((count, true));
        }
    }
    Ok((count, false))
}

fn next_token(page: &Value) -> Option<String> {
    if page.get("isLast").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    string(page, "nextPageToken").filter(|t| !t.is_empty())
}

fn issues(api: &Api<'_>, parameters: &ActionParameters) -> Result<ActionResult, Failure> {
    let filter = if parameters.text("source") == Some("filter") {
        Some(
            parameters
                .integer("filterId")
                .ok_or_else(|| fail("invalid-parameters", "choose the saved filter's number"))?,
        )
    } else {
        None
    };
    let project = optional(parameters, "project")
        .map(api::project_key)
        .transpose()?;
    let jql = search_jql(
        filter,
        project.as_deref(),
        parameters.boolean("includeDone").unwrap_or(false),
    );
    let limit = parameters
        .integer("limit")
        .unwrap_or(20)
        .clamp(1, MAX_LISTED);
    let limit = usize::try_from(limit).unwrap_or(20);
    let what = match filter {
        Some(id) => format!("the filter {id}"),
        None => "your issues".to_owned(),
    };
    let page = search_page(api, &jql, limit, LIST_FIELDS, None, &what)?;
    let items = page
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_response(&what))?;
    let listed = items.len() as u64;
    let (count, exact) = match api.deployment() {
        Deployment::DataCenter => (
            page.get("total")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid_response(&what))?,
            true,
        ),
        Deployment::Cloud if next_token(&page).is_none() => (listed, true),
        Deployment::Cloud => count_pages(api, &jql, &what)?,
    };
    let shown = if exact {
        count.to_string()
    } else {
        format!("at least {count}")
    };
    let scope = project
        .as_deref()
        .map_or_else(String::new, |p| format!(" in {p}"));
    let message = match filter {
        Some(id) => format!("{shown} issues in filter {id}{scope}"),
        None => format!("{shown} issues assigned to you{scope}"),
    };
    let lines: Vec<String> = items.iter().map(issue_line).collect();
    let mut result = ActionResult::success(message)
        .with_data("count", count.to_string())
        .with_data("countExact", exact.to_string())
        .with_data("listed", listed.to_string())
        .with_data("items", lines.join("\n"))
        .with_data("truncated", (count > listed || !exact).to_string());
    if let Some(key) = items.first().and_then(|i| string(i, "key")) {
        result = result
            .with_data("firstUrl", api.browse(&key))
            .with_data("firstKey", key);
    }
    Ok(result)
}

impl JiraAction {
    fn open(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        context
            .permit(Capability::OpenExternal)
            .map_err(ActionResult::from)?;
        let site = api::site(&self.settings.site_url)?;
        let target = if let Some(url) = optional(parameters, "url") {
            url.to_owned()
        } else {
            let key = optional(parameters, "issue").ok_or_else(|| {
                fail(
                    codes::INVALID_ISSUE_KEY,
                    "choose a page or an issue to open",
                )
            })?;
            format!("{site}browse/{}", api::issue_key(key)?)
        };
        let url = api::web_url(&target, &site)?;
        let plan = launch::plan(Os::current(), &OpenTarget::Url(url.to_string()))
            .map_err(|e| fail(e.code(), e.to_string()))?;
        self.launcher
            .launch(&plan)
            .map_err(|e| fail(e.code(), e.to_string()))?;
        let shown = policy::shown_url(&url);
        Ok(ActionResult::success(format!("opened {shown}")).with_data("url", shown))
    }
}

#[cfg(test)]
mod tests;
