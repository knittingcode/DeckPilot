//! The action types of the `github` module. All read; `github.open` opens
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

use crate::api::{self, Api, Failure, Repo, clean, fail};
use crate::{GithubSettings, codes};

pub const CURRENT_USER: &str = "github.current-user";
pub const REPOSITORY: &str = "github.repository";
pub const PULL_REQUESTS: &str = "github.pull-requests";
pub const REVIEW_REQUESTS: &str = "github.review-requests";
pub const CHECKS: &str = "github.checks";
pub const OPEN: &str = "github.open";

/// Pull request filters of `github.pull-requests`.
pub const FILTERS: [&str; 3] = ["all", "author-me", "review-requested"];
/// Most pull requests listed.
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
    Repository,
    PullRequests,
    ReviewRequests,
    Checks,
    Open,
}

pub fn handlers(
    settings: &Arc<GithubSettings>,
    launcher: &Arc<dyn Launcher>,
) -> Vec<Box<dyn ActionHandler>> {
    [
        Kind::CurrentUser,
        Kind::Repository,
        Kind::PullRequests,
        Kind::ReviewRequests,
        Kind::Checks,
        Kind::Open,
    ]
    .into_iter()
    .map(|kind| {
        Box::new(GithubAction {
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
        ParameterKind::Credential,
        "The name of a stored credential with a GitHub token. Empty: the module's (Settings › \
         Modules › GitHub). Never the token itself.",
    )
}

fn repository_parameter(required: bool) -> ParameterSpec {
    let mut spec = parameter(
        "repository",
        "Repository",
        text(140),
        if required {
            "owner/name, e.g. octo-org/website."
        } else {
            "owner/name, e.g. octo-org/website. Empty: every repository."
        },
    );
    spec.required = required;
    spec
}

fn pull_request_parameter(description: &str) -> ParameterSpec {
    parameter(
        "pullRequest",
        "Pull request",
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
        "At most this many pull requests in the list; the count is always complete.",
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
        out("count", "Open pull requests", ValueKind::Integer),
        out("listed", "Listed", ValueKind::Integer),
        // Titles are written by people and may say anything.
        out("items", "Pull requests", ValueKind::Text).sensitive(),
        out("truncated", "List truncated", ValueKind::Boolean),
        out("firstUrl", "First pull request", ValueKind::Url).optional(),
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
            "Current GitHub user",
            "The account the token belongs to; also checks that the token works.",
            Vec::new(),
            reading(
                "user",
                vec![
                    out("login", "Login", ValueKind::Text),
                    out("name", "Name", ValueKind::Text).optional(),
                    out("url", "Profile", ValueKind::Url),
                    out("rateLimitRemaining", "Requests left", ValueKind::Integer).optional(),
                ],
            ),
        ),
        Kind::Repository => (
            REPOSITORY,
            "GitHub repository",
            "A repository's default branch, visibility, counts and when it was last pushed.",
            vec![repository_parameter(true)],
            reading(
                "book-open",
                vec![
                    out("fullName", "Repository", ValueKind::Text),
                    out("description", "Description", ValueKind::Text)
                        .optional()
                        .sensitive(),
                    out("defaultBranch", "Default branch", ValueKind::Text),
                    out("private", "Private", ValueKind::Boolean),
                    out("archived", "Archived", ValueKind::Boolean),
                    out(
                        "openIssues",
                        "Open issues and pull requests",
                        ValueKind::Integer,
                    ),
                    out("stars", "Stars", ValueKind::Integer),
                    out("forks", "Forks", ValueKind::Integer),
                    out("url", "Page", ValueKind::Url),
                    out("pushedAt", "Last push", ValueKind::Text).optional(),
                ],
            ),
        ),
        Kind::PullRequests => {
            let mut filter = parameter(
                "filter",
                "Which",
                ParameterKind::Choice {
                    options: FILTERS.iter().map(|f| (*f).to_owned()).collect(),
                },
                "all: every open pull request of the repository; author-me: opened by you; \
                 review-requested: waiting for your review (also of your teams).",
            );
            filter.required = true;
            filter.default = Some(ParameterValue::Choice("all".into()));
            (
                PULL_REQUESTS,
                "Open pull requests",
                "Counts and lists open pull requests of a repository, or yours across \
                 repositories.",
                vec![repository_parameter(false), filter, limit_parameter()],
                reading("git-branch", list_outputs()),
            )
        }
        Kind::ReviewRequests => (
            REVIEW_REQUESTS,
            "Review requests",
            "Open pull requests waiting for your review (directly or through a team), in every \
             repository or one.",
            vec![repository_parameter(false), limit_parameter()],
            reading("search", list_outputs()),
        ),
        Kind::Checks => (
            CHECKS,
            "Checks",
            "The checks and commit statuses of a branch, tag, commit or pull request: overall \
             ok, failing or pending, and counts.",
            vec![
                repository_parameter(true),
                parameter(
                    "ref",
                    "Branch, tag or commit",
                    text(250),
                    "Used when no pull request is given.",
                ),
                pull_request_parameter("Its latest commit is checked."),
            ],
            reading(
                "list-checks",
                vec![
                    out(
                        "state",
                        "State (ok, failing, pending, none)",
                        ValueKind::Text,
                    ),
                    out("sha", "Commit", ValueKind::Text),
                    out("total", "Checks", ValueKind::Integer),
                    out("passed", "Passed", ValueKind::Integer),
                    out("failed", "Failed", ValueKind::Integer),
                    out("pending", "Pending", ValueKind::Integer),
                    out("failing", "Failing checks", ValueKind::Text),
                    out("truncated", "Not all read", ValueKind::Boolean),
                    out("url", "Page", ValueKind::Url),
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
                "Open on GitHub",
                "Opens a repository, a pull request or a GitHub page in the browser.",
                vec![
                    parameter(
                        "url",
                        "Page",
                        text(2048),
                        "An https page on the configured GitHub, e.g. a pull request's URL \
                         from an earlier node. Empty: the repository below.",
                    ),
                    repository_parameter(false),
                    pull_request_parameter("Opens this pull request of the repository."),
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
        category: "GitHub".into(),
        parameters,
        requires_project: false,
        node,
    }
}

#[derive(Debug)]
struct GithubAction {
    definition: ActionType,
    kind: Kind,
    settings: Arc<GithubSettings>,
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

fn repository(parameters: &ActionParameters) -> Result<Option<Repo>, Failure> {
    optional(parameters, "repository")
        .map(Repo::parse)
        .transpose()
}

fn required_repository(parameters: &ActionParameters) -> Result<Repo, Failure> {
    repository(parameters)?.ok_or_else(|| {
        fail(
            codes::INVALID_REPOSITORY,
            "choose a repository (owner/name)",
        )
    })
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
        format!("GitHub's answer for {what} is not what was expected"),
    )
}

impl ActionHandler for GithubAction {
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
            Kind::Repository => repository_info(&api, parameters),
            Kind::PullRequests => pull_requests(&api, parameters, false),
            Kind::ReviewRequests => pull_requests(&api, parameters, true),
            Kind::Checks => checks(&api, parameters),
            Kind::Open => unreachable!("handled above"),
        };
        api.scrub(result.unwrap_or_else(|failure| failure))
    }

    /// Problems found when the action is saved.
    fn check(&self, parameters: &ActionParameters, _: &Profile) -> Vec<String> {
        let mut problems = Vec::new();
        if let Some(text) = literal(parameters, "repository")
            && let Err(failure) = Repo::parse(text)
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
            && optional(parameters, "repository").is_none()
        {
            problems.push("choose a page or a repository to open".into());
        }
        problems
    }
}

fn current_user(api: &Api<'_>) -> Result<ActionResult, Failure> {
    let reply = api.get(api.endpoint(&["user"], &[]), "the signed-in user")?;
    let user = &reply.json;
    let login = string(user, "login").ok_or_else(|| invalid_response("the signed-in user"))?;
    let mut result = ActionResult::success(format!("signed in to GitHub as {login}"))
        .with_data("login", login.clone())
        .with_data(
            "url",
            string(user, "html_url").unwrap_or_else(|| format!("{}{login}", api.web())),
        );
    if let Some(name) = string(user, "name").filter(|n| !n.is_empty()) {
        result = result.with_data("name", clean(&name, 200));
    }
    if let Some(remaining) = reply.rate_remaining {
        result = result.with_data("rateLimitRemaining", remaining.to_string());
    }
    Ok(result)
}

fn repository_info(api: &Api<'_>, parameters: &ActionParameters) -> Result<ActionResult, Failure> {
    let repo = required_repository(parameters)?;
    let what = format!("the repository {}", repo.full_name());
    let reply = api.get(
        api.endpoint(&["repos", &repo.owner, &repo.name], &[]),
        &what,
    )?;
    let json = &reply.json;
    let full_name = string(json, "full_name").ok_or_else(|| invalid_response(&what))?;
    let mut result = ActionResult::success(format!("read {full_name}"))
        .with_data("fullName", full_name.clone())
        .with_data(
            "defaultBranch",
            string(json, "default_branch").unwrap_or_default(),
        )
        .with_data("private", boolean(json, "private"))
        .with_data("archived", boolean(json, "archived"))
        .with_data("openIssues", integer(json, "open_issues_count"))
        .with_data("stars", integer(json, "stargazers_count"))
        .with_data("forks", integer(json, "forks_count"))
        .with_data(
            "url",
            string(json, "html_url").unwrap_or_else(|| format!("{}{full_name}", api.web())),
        );
    if let Some(description) = string(json, "description").filter(|d| !d.is_empty()) {
        result = result.with_data("description", clean(&description, 500));
    }
    if let Some(pushed) = string(json, "pushed_at") {
        result = result.with_data("pushedAt", pushed);
    }
    Ok(result)
}

/// The search for open pull requests.
fn search_query(repo: Option<&Repo>, filter: &str) -> String {
    let mut query = "is:pr is:open".to_owned();
    match repo {
        Some(repo) => {
            query.push_str(" repo:");
            query.push_str(&repo.full_name());
        }
        None => query.push_str(" archived:false"),
    }
    match filter {
        "author-me" => query.push_str(" author:@me"),
        "review-requested" => query.push_str(" review-requested:@me"),
        _ => {}
    }
    query
}

/// `owner/name#12 Title (@login)`, one line.
fn pull_request_line(item: &Value) -> String {
    let repository = item
        .get("repository_url")
        .and_then(Value::as_str)
        .and_then(|url| {
            let mut parts = url.rsplit('/');
            let name = parts.next()?;
            let owner = parts.next()?;
            Some(format!("{owner}/{name}"))
        })
        .unwrap_or_default();
    let number = integer(item, "number");
    let title = clean(&string(item, "title").unwrap_or_default(), 200);
    let author = item
        .get("user")
        .and_then(|u| u.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("?");
    let draft = if item.get("draft").and_then(Value::as_bool) == Some(true) {
        " [draft]"
    } else {
        ""
    };
    format!("{repository}#{number} {title} (@{author}){draft}")
}

fn pull_requests(
    api: &Api<'_>,
    parameters: &ActionParameters,
    review_requests: bool,
) -> Result<ActionResult, Failure> {
    let repo = repository(parameters)?;
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
    if filter == "all" && repo.is_none() {
        return Err(fail(
            codes::INVALID_REPOSITORY,
            "choose a repository to list all its open pull requests",
        ));
    }
    let limit = parameters
        .integer("limit")
        .unwrap_or(20)
        .clamp(1, MAX_LISTED)
        .to_string();
    let query = search_query(repo.as_ref(), filter);
    let url = api.endpoint(
        &["search", "issues"],
        &[
            ("q", &query),
            ("sort", "updated"),
            ("order", "desc"),
            ("per_page", &limit),
        ],
    );
    let what = match &repo {
        Some(repo) => format!("the pull requests of {}", repo.full_name()),
        None => "your pull requests".to_owned(),
    };
    // One page: the count comes from `total_count`, the list is bounded.
    let reply = api.get(url, &what)?;
    let total = reply
        .json
        .get("total_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_response(&what))?;
    let items = reply
        .json
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_response(&what))?;
    let lines: Vec<String> = items.iter().map(pull_request_line).collect();
    let listed = lines.len() as u64;
    let scope = repo
        .as_ref()
        .map_or_else(String::new, |r| format!(" in {}", r.full_name()));
    let message = if review_requests {
        format!("{total} pull requests wait for your review{scope}")
    } else {
        format!("{total} open pull requests{scope} ({filter})")
    };
    let mut result = ActionResult::success(message)
        .with_data("count", total.to_string())
        .with_data("listed", listed.to_string())
        .with_data("items", lines.join("\n"))
        .with_data("truncated", (total > listed).to_string());
    if let Some(first) = items.first().and_then(|i| string(i, "html_url")) {
        result = result.with_data("firstUrl", first);
    }
    Ok(result)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    passed: u64,
    failed: u64,
    pending: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    Pending,
}

/// A check run: pending until completed; success, neutral and skipped
/// pass; everything else (failure, cancelled, timed out, action required,
/// stale, …) fails.
fn check_run_outcome(run: &Value) -> Outcome {
    if run.get("status").and_then(Value::as_str) != Some("completed") {
        return Outcome::Pending;
    }
    match run.get("conclusion").and_then(Value::as_str) {
        Some("success" | "neutral" | "skipped") => Outcome::Passed,
        _ => Outcome::Failed,
    }
}

/// A commit status: success, pending, or failure and error.
fn status_outcome(status: &Value) -> Outcome {
    match status.get("state").and_then(Value::as_str) {
        Some("success") => Outcome::Passed,
        Some("pending") => Outcome::Pending,
        _ => Outcome::Failed,
    }
}

fn overall(counts: Counts) -> &'static str {
    if counts.failed > 0 {
        "failing"
    } else if counts.pending > 0 {
        "pending"
    } else if counts.passed > 0 {
        "ok"
    } else {
        "none"
    }
}

/// The commit to check: the ref, or a pull request's latest commit; how
/// it is named in messages; and the pull request's page.
fn checked_commit(
    api: &Api<'_>,
    parameters: &ActionParameters,
    repo: &Repo,
) -> Result<(String, String, Option<String>), Failure> {
    let full = repo.full_name();
    Ok(if let Some(number) = parameters.integer("pullRequest") {
        let number = number.max(1).to_string();
        let what = format!("the pull request {full}#{number}");
        let reply = api.get(
            api.endpoint(&["repos", &repo.owner, &repo.name, "pulls", &number], &[]),
            &what,
        )?;
        let sha = reply
            .json
            .get("head")
            .and_then(|h| string(h, "sha"))
            .ok_or_else(|| invalid_response(&what))?;
        api::check_ref(&sha)?;
        (
            sha,
            format!("{full}#{number}"),
            string(&reply.json, "html_url"),
        )
    } else {
        let reference = optional(parameters, "ref").ok_or_else(|| {
            fail(
                "invalid-parameters",
                "choose a branch, tag, commit or pull request",
            )
        })?;
        let reference = api::check_ref(reference)?.to_owned();
        let shown = format!("{full}@{reference}");
        (reference, shown, None)
    })
}

fn checks(api: &Api<'_>, parameters: &ActionParameters) -> Result<ActionResult, Failure> {
    let repo = required_repository(parameters)?;
    let full = repo.full_name();
    let (reference, shown, page) = checked_commit(api, parameters, &repo)?;
    let what = format!("the checks of {shown}");
    let runs = api.get_list(
        api.endpoint(
            &[
                "repos",
                &repo.owner,
                &repo.name,
                "commits",
                &reference,
                "check-runs",
            ],
            &[
                ("per_page", &api::PER_PAGE.to_string()),
                ("filter", "latest"),
            ],
        ),
        &what,
        Some("check_runs"),
    )?;
    let statuses = api.get_list(
        api.endpoint(
            &[
                "repos",
                &repo.owner,
                &repo.name,
                "commits",
                &reference,
                "status",
            ],
            &[("per_page", &api::PER_PAGE.to_string())],
        ),
        &what,
        Some("statuses"),
    )?;
    let mut counts = Counts::default();
    let mut failing = Vec::new();
    let named = runs
        .items
        .iter()
        .map(|r| (check_run_outcome(r), string(r, "name")))
        .chain(
            statuses
                .items
                .iter()
                .map(|s| (status_outcome(s), string(s, "context"))),
        );
    for (outcome, name) in named {
        match outcome {
            Outcome::Passed => counts.passed += 1,
            Outcome::Pending => counts.pending += 1,
            Outcome::Failed => {
                counts.failed += 1;
                failing.push(clean(&name.unwrap_or_default(), 120));
            }
        }
    }
    let truncated = runs.truncated
        || statuses.truncated
        || runs.total.is_some_and(|t| t > runs.items.len() as u64);
    let sha = string(&statuses.first, "sha").unwrap_or_else(|| reference.clone());
    let state = overall(counts);
    let total = counts.passed + counts.failed + counts.pending;
    let url = page.unwrap_or_else(|| format!("{}{full}/commit/{sha}", api.web()));
    Ok(ActionResult::success(format!(
        "{shown}: {state} ({} passed, {} failed, {} pending)",
        counts.passed, counts.failed, counts.pending
    ))
    .with_data("state", state)
    .with_data("sha", sha)
    .with_data("total", total.to_string())
    .with_data("passed", counts.passed.to_string())
    .with_data("failed", counts.failed.to_string())
    .with_data("pending", counts.pending.to_string())
    .with_data("failing", failing.join("\n"))
    .with_data("truncated", truncated.to_string())
    .with_data("url", url))
}

impl GithubAction {
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
            let repo = required_repository(parameters)?;
            match parameters.integer("pullRequest") {
                Some(number) => format!("{web}{}/pull/{}", repo.full_name(), number.max(1)),
                None => format!("{web}{}", repo.full_name()),
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
