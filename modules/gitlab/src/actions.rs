//! The action types of the `gitlab` module. All read; `gitlab.open` opens
//! a page in the browser.

use std::sync::Arc;

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::launch::{self, Launcher, OpenTarget};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    CredentialId, ExecutionContext, Os, ParameterKind, ParameterSpec, ParameterValue, Profile,
};
use deckpilot_module_http::policy;
use serde_json::Value;

use crate::api::{self, Api, Failure, Project, clean, fail};
use crate::{GitlabSettings, codes};

pub const CURRENT_USER: &str = "gitlab.current-user";
pub const PROJECT: &str = "gitlab.project";
pub const MERGE_REQUESTS: &str = "gitlab.merge-requests";
pub const REVIEW_REQUESTS: &str = "gitlab.review-requests";
pub const PIPELINE: &str = "gitlab.pipeline";
pub const OPEN: &str = "gitlab.open";

/// Merge request filters of `gitlab.merge-requests`.
pub const FILTERS: [&str; 3] = ["all", "author-me", "review-requested"];
/// Most merge requests listed.
pub const MAX_LISTED: i64 = 100;

/// # Panics
///
/// Never for the constants of this module.
pub fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("the action type id is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    CurrentUser,
    Project,
    MergeRequests,
    ReviewRequests,
    Pipeline,
    Open,
}

pub fn handlers(
    settings: &Arc<GitlabSettings>,
    launcher: &Arc<dyn Launcher>,
) -> Vec<Box<dyn ActionHandler>> {
    [
        Kind::CurrentUser,
        Kind::Project,
        Kind::MergeRequests,
        Kind::ReviewRequests,
        Kind::Pipeline,
        Kind::Open,
    ]
    .into_iter()
    .map(|kind| {
        Box::new(GitlabAction {
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

fn credential_parameter() -> ParameterSpec {
    parameter(
        "credential",
        "Token credential",
        text(64),
        "The name of a stored credential with a GitLab token. Empty: the module's (Settings › \
         Modules › GitLab). Never the token itself.",
    )
}

fn project_parameter(required: bool) -> ParameterSpec {
    let mut spec = parameter(
        "project",
        "Project",
        text(1000),
        if required {
            "group/project (with subgroups), or the project's numeric id."
        } else {
            "group/project (with subgroups), or the project's numeric id. Empty: every project."
        },
    );
    spec.required = required;
    spec
}

fn merge_request_parameter(description: &str) -> ParameterSpec {
    parameter(
        "mergeRequest",
        "Merge request",
        ParameterKind::Integer {
            min: Some(1),
            max: None,
        },
        description,
    )
}

fn limit_parameter() -> ParameterSpec {
    let mut spec = parameter(
        "limit",
        "Listed",
        ParameterKind::Integer {
            min: Some(1),
            max: Some(MAX_LISTED),
        },
        "At most this many merge requests in the list.",
    );
    spec.required = true;
    spec.default = Some(ParameterValue::Integer(20));
    spec
}

fn out(key: &str, label: &str, kind: ValueKind) -> OutputSpec {
    OutputSpec::new(key, label, kind)
}

fn list_outputs() -> Vec<OutputSpec> {
    vec![
        out("count", "Open merge requests", ValueKind::Integer),
        out("countExact", "Count complete", ValueKind::Boolean),
        out("listed", "Listed", ValueKind::Integer),
        // Titles are written by people and may say anything.
        out("items", "Merge requests", ValueKind::Text).sensitive(),
        out("truncated", "List truncated", ValueKind::Boolean),
        out("firstUrl", "First merge request", ValueKind::Url).optional(),
    ]
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

#[allow(clippy::too_many_lines)] // One table of the six action types.
fn definition(kind: Kind) -> ActionType {
    let (id_value, name, description, mut parameters, node) = match kind {
        Kind::CurrentUser => (
            CURRENT_USER,
            "Current GitLab user",
            "The account the token belongs to; also checks that the token works.",
            Vec::new(),
            reading(
                "user",
                vec![
                    out("username", "Username", ValueKind::Text),
                    out("name", "Name", ValueKind::Text).optional(),
                    out("url", "Profile", ValueKind::Url),
                    out("rateLimitRemaining", "Requests left", ValueKind::Integer).optional(),
                ],
            ),
        ),
        Kind::Project => (
            PROJECT,
            "GitLab project",
            "A project's default branch, visibility, counts and last activity.",
            vec![project_parameter(true)],
            reading(
                "book-open",
                vec![
                    out("path", "Project", ValueKind::Text),
                    out("description", "Description", ValueKind::Text)
                        .optional()
                        .sensitive(),
                    out("defaultBranch", "Default branch", ValueKind::Text).optional(),
                    out("visibility", "Visibility", ValueKind::Text),
                    out("archived", "Archived", ValueKind::Boolean),
                    out("openIssues", "Open issues", ValueKind::Integer).optional(),
                    out("stars", "Stars", ValueKind::Integer),
                    out("forks", "Forks", ValueKind::Integer),
                    out("url", "Page", ValueKind::Url),
                    out("lastActivityAt", "Last activity", ValueKind::Text).optional(),
                ],
            ),
        ),
        Kind::MergeRequests => {
            let mut filter = parameter(
                "filter",
                "Which",
                ParameterKind::Choice {
                    options: FILTERS.iter().map(|f| (*f).to_owned()).collect(),
                },
                "all: every open merge request of the project; author-me: opened by you; \
                 review-requested: where you are a reviewer.",
            );
            filter.required = true;
            filter.default = Some(ParameterValue::Choice("all".into()));
            (
                MERGE_REQUESTS,
                "Open merge requests",
                "Counts and lists open merge requests of a project, or yours across projects.",
                vec![project_parameter(false), filter, limit_parameter()],
                reading("git-branch", list_outputs()),
            )
        }
        Kind::ReviewRequests => (
            REVIEW_REQUESTS,
            "Review requests",
            "Open merge requests where you are a reviewer, in every project or one.",
            vec![project_parameter(false), limit_parameter()],
            reading("search", list_outputs()),
        ),
        Kind::Pipeline => (
            PIPELINE,
            "Pipeline status",
            "The latest pipeline of a branch, tag or commit, or a merge request's head \
             pipeline: overall ok, failing or pending, and job counts.",
            vec![
                project_parameter(true),
                parameter(
                    "ref",
                    "Branch, tag or commit",
                    text(250),
                    "Used when no merge request is given.",
                ),
                merge_request_parameter("Its head pipeline is checked (the number after !)."),
            ],
            reading(
                "list-checks",
                vec![
                    out(
                        "state",
                        "State (ok, failing, pending, none)",
                        ValueKind::Text,
                    ),
                    out("status", "GitLab status", ValueKind::Text).optional(),
                    out("pipelineId", "Pipeline", ValueKind::Integer).optional(),
                    out("sha", "Commit", ValueKind::Text).optional(),
                    out("total", "Jobs", ValueKind::Integer),
                    out("passed", "Passed", ValueKind::Integer),
                    out("failed", "Failed", ValueKind::Integer),
                    out("pending", "Pending or running", ValueKind::Integer),
                    out("skipped", "Skipped or manual", ValueKind::Integer),
                    out("failing", "Failing jobs", ValueKind::Text),
                    out("truncated", "Not all read", ValueKind::Boolean),
                    out("url", "Page", ValueKind::Url).optional(),
                ],
            ),
        ),
        Kind::Open => {
            let node = NodeSpec {
                icon: Some("link".into()),
                outputs: vec![out("url", "Opened", ValueKind::Url)],
                timeout_ms: 15_000,
                retry_safe: true,
                idempotent: false,
                cancellable: false,
                effect: ActionEffect::Opens,
                ..NodeSpec::default()
            };
            (
                OPEN,
                "Open on GitLab",
                "Opens a project, a merge request or a GitLab page in the browser.",
                vec![
                    parameter(
                        "url",
                        "Page",
                        text(2048),
                        "An https page on the configured GitLab, e.g. a merge request's URL \
                         from an earlier node. Empty: the project below.",
                    ),
                    project_parameter(false),
                    merge_request_parameter("Opens this merge request of the project."),
                ],
                node,
            )
        }
    };
    if kind != Kind::Open {
        parameters.push(credential_parameter());
    }
    ActionType {
        id: id(id_value),
        name: name.into(),
        description: description.into(),
        category: "GitLab".into(),
        parameters,
        requires_project: false,
        node,
    }
}

#[derive(Debug)]
struct GitlabAction {
    definition: ActionType,
    kind: Kind,
    settings: Arc<GitlabSettings>,
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

fn project(parameters: &ActionParameters) -> Result<Option<Project>, Failure> {
    optional(parameters, "project")
        .map(Project::parse)
        .transpose()
}

fn required_project(parameters: &ActionParameters) -> Result<Project, Failure> {
    project(parameters)?
        .ok_or_else(|| fail(codes::INVALID_PROJECT, "choose a project (group/project)"))
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn integer(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .to_string()
}

fn boolean(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .to_string()
}

fn invalid_response(what: &str) -> Failure {
    fail(
        codes::INVALID_RESPONSE,
        format!("GitLab's answer for {what} is not what was expected"),
    )
}

impl ActionHandler for GitlabAction {
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
            Kind::CurrentUser => current_user(&api),
            Kind::Project => project_info(&api, parameters),
            Kind::MergeRequests => merge_requests(&api, parameters, false),
            Kind::ReviewRequests => merge_requests(&api, parameters, true),
            Kind::Pipeline => pipeline(&api, parameters),
            Kind::Open => unreachable!("handled above"),
        };
        api.scrub(result.unwrap_or_else(|failure| failure))
    }

    /// Problems found when the action is saved.
    fn check(&self, parameters: &ActionParameters, _: &Profile) -> Vec<String> {
        let mut problems = Vec::new();
        if let Some(text) = literal(parameters, "project")
            && let Err(failure) = Project::parse(text)
        {
            problems.push(failure.message);
        }
        if let Some(text) = literal(parameters, "ref")
            && let Err(failure) = api::check_ref(text)
        {
            problems.push(failure.message);
        }
        if let Some(name) = literal(parameters, "credential")
            && CredentialId::new(name).is_err()
        {
            problems.push(format!("“{}” is not a credential name", clean(name, 64)));
        }
        if let Some(url) = literal(parameters, "url")
            && let Ok(base) = api::api_base(&self.settings.api_url)
            && let Err(failure) = api::web_url(url, &api::web_base(&base))
        {
            problems.push(failure.message);
        }
        if self.kind == Kind::Open
            && optional(parameters, "url").is_none()
            && optional(parameters, "project").is_none()
        {
            problems.push("choose a page or a project to open".into());
        }
        problems
    }
}

fn current_user(api: &Api<'_>) -> Result<ActionResult, Failure> {
    let reply = api.get(api.endpoint(&["user"], &[]), "the signed-in user")?;
    let user = &reply.json;
    let username =
        string(user, "username").ok_or_else(|| invalid_response("the signed-in user"))?;
    let mut result = ActionResult::success(format!("signed in to GitLab as {username}"))
        .with_data("username", username.clone())
        .with_data(
            "url",
            string(user, "web_url").unwrap_or_else(|| format!("{}{username}", api.web())),
        );
    if let Some(name) = string(user, "name").filter(|n| !n.is_empty()) {
        result = result.with_data("name", clean(&name, 200));
    }
    if let Some(remaining) = reply.rate_remaining {
        result = result.with_data("rateLimitRemaining", remaining.to_string());
    }
    Ok(result)
}

fn project_info(api: &Api<'_>, parameters: &ActionParameters) -> Result<ActionResult, Failure> {
    let project = required_project(parameters)?;
    let what = format!("the project {}", project.id());
    let reply = api.get(api.endpoint(&["projects", project.id()], &[]), &what)?;
    let json = &reply.json;
    let path = string(json, "path_with_namespace").ok_or_else(|| invalid_response(&what))?;
    let mut result = ActionResult::success(format!("read {path}"))
        .with_data("path", path.clone())
        .with_data(
            "visibility",
            string(json, "visibility").unwrap_or_else(|| "unknown".into()),
        )
        .with_data("archived", boolean(json, "archived"))
        .with_data("stars", integer(json, "star_count"))
        .with_data("forks", integer(json, "forks_count"))
        .with_data(
            "url",
            string(json, "web_url").unwrap_or_else(|| format!("{}{path}", api.web())),
        );
    if let Some(branch) = string(json, "default_branch") {
        result = result.with_data("defaultBranch", branch);
    }
    // Absent when the project's issues are turned off.
    if let Some(issues) = json.get("open_issues_count").and_then(Value::as_u64) {
        result = result.with_data("openIssues", issues.to_string());
    }
    if let Some(description) = string(json, "description").filter(|d| !d.is_empty()) {
        result = result.with_data("description", clean(&description, 500));
    }
    if let Some(activity) = string(json, "last_activity_at") {
        result = result.with_data("lastActivityAt", activity);
    }
    Ok(result)
}

/// `group/project!12 Title (@username)`, one line.
fn merge_request_line(item: &Value) -> String {
    let reference = item
        .get("references")
        .and_then(|r| string(r, "full"))
        .unwrap_or_else(|| format!("!{}", integer(item, "iid")));
    let title = clean(&string(item, "title").unwrap_or_default(), 200);
    let author = item
        .get("author")
        .and_then(|u| u.get("username"))
        .and_then(Value::as_str)
        .unwrap_or("?");
    let draft = ["draft", "work_in_progress"]
        .iter()
        .any(|key| item.get(*key).and_then(Value::as_bool) == Some(true));
    format!(
        "{reference} {title} (@{author}){}",
        if draft { " [draft]" } else { "" }
    )
}

/// `author_id` or `reviewer_id` of the signed-in user, for the filters
/// about you.
fn me_filter(api: &Api<'_>, filter: &str) -> Result<Option<(&'static str, String)>, Failure> {
    let key = match filter {
        "author-me" => "author_id",
        "review-requested" => "reviewer_id",
        _ => return Ok(None),
    };
    let me = api.get(api.endpoint(&["user"], &[]), "the signed-in user")?;
    let id = me
        .json
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_response("the signed-in user"))?;
    Ok(Some((key, id.to_string())))
}

fn merge_requests(
    api: &Api<'_>,
    parameters: &ActionParameters,
    review_requests: bool,
) -> Result<ActionResult, Failure> {
    let project = project(parameters)?;
    let filter = if review_requests {
        "review-requested"
    } else {
        parameters.text("filter").unwrap_or("all")
    };
    if !FILTERS.contains(&filter) {
        return Err(fail(
            "invalid-parameters",
            format!("“{}” is not a filter", clean(filter, 40)),
        ));
    }
    if filter == "all" && project.is_none() {
        return Err(fail(
            codes::INVALID_PROJECT,
            "choose a project to list all its open merge requests",
        ));
    }
    let limit = parameters
        .integer("limit")
        .unwrap_or(20)
        .clamp(1, MAX_LISTED)
        .to_string();
    let mut query: Vec<(&str, String)> = vec![
        ("state", "opened".into()),
        ("order_by", "updated_at".into()),
        ("sort", "desc".into()),
    ];
    if let Some(me) = me_filter(api, filter)? {
        query.push(me);
    }
    let (segments, what): (Vec<&str>, String) = if let Some(project) = &project {
        (
            vec!["projects", project.id(), "merge_requests"],
            format!("the merge requests of {}", project.id()),
        )
    } else {
        query.push(("scope", "all".into()));
        (vec!["merge_requests"], "your merge requests".to_owned())
    };
    let with_size = |size: &str| -> Vec<(&str, String)> {
        let mut all = query.clone();
        all.push(("per_page", size.to_owned()));
        all
    };
    let first: Vec<(&str, String)> = with_size(&limit);
    let first: Vec<(&str, &str)> = first.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let reply = api.get(api.endpoint(&segments, &first), &what)?;
    let items = reply
        .json
        .as_array()
        .ok_or_else(|| invalid_response(&what))?
        .clone();
    let listed = items.len() as u64;
    // GitLab leaves `X-Total` out for very large lists; then count by
    // reading a bounded number of pages.
    let (count, exact) = match (reply.total, &reply.next) {
        (Some(total), _) => (total, true),
        (None, None) => (listed, true),
        (None, Some(_)) => {
            let counting = with_size(&api::PER_PAGE.to_string());
            let counting: Vec<(&str, &str)> =
                counting.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let list = api.get_list(api.endpoint(&segments, &counting), &what, api::MAX_PAGES)?;
            (list.items.len() as u64, !list.truncated)
        }
    };
    let scope = project
        .as_ref()
        .map_or_else(String::new, |p| format!(" in {}", p.id()));
    let shown = if exact {
        count.to_string()
    } else {
        format!("at least {count}")
    };
    let message = if review_requests {
        format!("{shown} merge requests wait for your review{scope}")
    } else {
        format!("{shown} open merge requests{scope} ({filter})")
    };
    let lines: Vec<String> = items.iter().map(merge_request_line).collect();
    let mut result = ActionResult::success(message)
        .with_data("count", count.to_string())
        .with_data("countExact", exact.to_string())
        .with_data("listed", listed.to_string())
        .with_data("items", lines.join("\n"))
        .with_data("truncated", (count > listed || !exact).to_string());
    if let Some(first) = items.first().and_then(|i| string(i, "web_url")) {
        result = result.with_data("firstUrl", first);
    }
    Ok(result)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    passed: u64,
    failed: u64,
    pending: u64,
    skipped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    Pending,
    Skipped,
}

/// A job: success passes; failed fails unless failure is allowed;
/// canceled fails; skipped and manual did not run; the rest are pending.
fn job_outcome(job: &Value) -> Outcome {
    match job.get("status").and_then(Value::as_str) {
        Some("success") => Outcome::Passed,
        Some("failed") if job.get("allow_failure").and_then(Value::as_bool) == Some(true) => {
            Outcome::Passed
        }
        Some("failed" | "canceled") => Outcome::Failed,
        Some("skipped" | "manual") => Outcome::Skipped,
        _ => Outcome::Pending,
    }
}

/// The pipeline's own status in DeckPilot's terms.
fn overall(status: &str) -> &'static str {
    match status {
        "success" => "ok",
        "failed" | "canceled" => "failing",
        "skipped" => "none",
        _ => "pending",
    }
}

/// The pipeline to check, the commit, how it is named in messages and the
/// merge request's page.
fn chosen_pipeline(
    api: &Api<'_>,
    parameters: &ActionParameters,
    project: &Project,
) -> Result<(Option<Value>, String, Option<String>), Failure> {
    if let Some(number) = parameters.integer("mergeRequest") {
        let number = number.max(1).to_string();
        let what = format!("the merge request {}!{number}", project.id());
        let reply = api.get(
            api.endpoint(&["projects", project.id(), "merge_requests", &number], &[]),
            &what,
        )?;
        let pipeline = reply
            .json
            .get("head_pipeline")
            .filter(|p| p.is_object())
            .cloned();
        return Ok((
            pipeline,
            format!("{}!{number}", project.id()),
            string(&reply.json, "web_url"),
        ));
    }
    let reference = optional(parameters, "ref").ok_or_else(|| {
        fail(
            "invalid-parameters",
            "choose a branch, tag, commit or merge request",
        )
    })?;
    let reference = api::check_ref(reference)?;
    let is_sha = reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit());
    let what = format!("the pipelines of {}@{reference}", project.id());
    let reply = api.get(
        api.endpoint(
            &["projects", project.id(), "pipelines"],
            &[
                (if is_sha { "sha" } else { "ref" }, reference),
                ("order_by", "id"),
                ("sort", "desc"),
                ("per_page", "1"),
            ],
        ),
        &what,
    )?;
    let pipeline = reply
        .json
        .as_array()
        .ok_or_else(|| invalid_response(&what))?
        .first()
        .cloned();
    Ok((pipeline, format!("{}@{reference}", project.id()), None))
}

fn pipeline(api: &Api<'_>, parameters: &ActionParameters) -> Result<ActionResult, Failure> {
    let project = required_project(parameters)?;
    let (pipeline, shown, page) = chosen_pipeline(api, parameters, &project)?;
    let empty = |result: ActionResult| {
        result
            .with_data("state", "none")
            .with_data("total", "0")
            .with_data("passed", "0")
            .with_data("failed", "0")
            .with_data("pending", "0")
            .with_data("skipped", "0")
            .with_data("failing", "")
            .with_data("truncated", "false")
    };
    let Some(pipeline) = pipeline else {
        let mut result = empty(ActionResult::success(format!("{shown}: no pipeline")));
        if let Some(page) = page {
            result = result.with_data("url", page);
        }
        return Ok(result);
    };
    let what = format!("the pipeline of {shown}");
    let id = pipeline
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_response(&what))?
        .to_string();
    let status = string(&pipeline, "status").unwrap_or_else(|| "unknown".into());
    let jobs = api.get_list(
        api.endpoint(
            &["projects", project.id(), "pipelines", &id, "jobs"],
            &[("per_page", &api::PER_PAGE.to_string())],
        ),
        &format!("the jobs of {shown}"),
        api::MAX_PAGES,
    )?;
    let mut counts = Counts::default();
    let mut failing = Vec::new();
    for job in &jobs.items {
        match job_outcome(job) {
            Outcome::Passed => counts.passed += 1,
            Outcome::Pending => counts.pending += 1,
            Outcome::Skipped => counts.skipped += 1,
            Outcome::Failed => {
                counts.failed += 1;
                failing.push(clean(&string(job, "name").unwrap_or_default(), 120));
            }
        }
    }
    let state = overall(&status);
    let truncated = jobs.truncated || jobs.total.is_some_and(|t| t > jobs.items.len() as u64);
    let mut result = ActionResult::success(format!(
        "{shown}: {state} ({} passed, {} failed, {} pending)",
        counts.passed, counts.failed, counts.pending
    ))
    .with_data("state", state)
    .with_data("status", clean(&status, 40))
    .with_data("pipelineId", id)
    .with_data("total", jobs.items.len().to_string())
    .with_data("passed", counts.passed.to_string())
    .with_data("failed", counts.failed.to_string())
    .with_data("pending", counts.pending.to_string())
    .with_data("skipped", counts.skipped.to_string())
    .with_data("failing", failing.join("\n"))
    .with_data("truncated", truncated.to_string());
    if let Some(sha) = string(&pipeline, "sha") {
        result = result.with_data("sha", sha);
    }
    if let Some(url) = page.or_else(|| string(&pipeline, "web_url")) {
        result = result.with_data("url", url);
    }
    Ok(result)
}

impl GitlabAction {
    fn open(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        context
            .permit(Capability::OpenExternal)
            .map_err(ActionResult::from)?;
        let web = api::web_base(&api::api_base(&self.settings.api_url)?);
        let target = if let Some(url) = optional(parameters, "url") {
            url.to_owned()
        } else {
            let project = required_project(parameters)?;
            if project.is_numeric() {
                return Err(fail(
                    codes::INVALID_PROJECT,
                    "a project is opened by its path (group/project), not its id",
                ));
            }
            match parameters.integer("mergeRequest") {
                Some(number) => format!("{web}{}/-/merge_requests/{}", project.id(), number.max(1)),
                None => format!("{web}{}", project.id()),
            }
        };
        let url = api::web_url(&target, &web)?;
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
