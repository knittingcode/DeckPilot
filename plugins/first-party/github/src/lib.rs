//! `deckpilot.github`: the first-party GitHub plugin, the replacement of
//! the built-in `github` module (`docs/marketplace/module-migration.md`
//! §6). It reads the signed-in user, repositories, open pull requests,
//! review requests and the checks of a commit from GitHub, and opens
//! GitHub pages in the browser.
//!
//! **The same actions.** Every action of the built-in, with the same name
//! (`github.<action>` → `deckpilot.github.<action>`), the same parameters
//! (except the per-action `credential` override, which the alias drops
//! when it names the module's own credential, §5.3), the same outputs,
//! messages and error codes. `crates/deckpilot-modules/tests/contracts.rs`
//! compares the manifest with the built-in's definitions, and
//! `apps/agent/tests/plugins_github.rs` runs both on the same answers.
//!
//! **Read-only.** Only `GET` requests, to `https://api.github.com` (the
//! manifest's `network` host) or to the GitHub Enterprise Server API the
//! user entered in `apiUrl` and allowed (its user-origin slot, a private
//! network only with its own consent); pages are opened on
//! `https://github.com` (`openUrl`) or on that server.
//!
//! **Token.** The credential slot `token`: the agent injects it as
//! `Authorization: Bearer` into requests to api.github.com or the allowed
//! server and redacts it from everything this module sees; it never
//! enters the module.
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::prelude::*;
use serde_json::Value;

pub mod api;

use api::{Api, Failure, Repo, clean, codes, fail};

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.github";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Pull request filters of `pull-requests`.
pub const FILTERS: [&str; 3] = ["all", "author-me", "review-requested"];
/// Most pull requests listed.
pub const MAX_LISTED: i64 = 100;
/// Longest text one output can carry (ABI 1 `MAX_STRING_LEN`, in bytes).
pub const MAX_OUTPUT_TEXT: usize = 16 * 1024;

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches` in the unit tests).
pub fn plugin() -> Plugin {
    let repository = || Param::text("repository").max_length(140);
    let pull_request = || Param::integer("pullRequest").min(1.0);
    let limit = || Param::integer("limit").min(1.0).max(100.0).default(20_i64);
    Plugin::new(ID, VERSION)
        .action(Action::new("current-user", current_user))
        .action(Action::new("repository", repository_info).param(repository().required()))
        .action(
            Action::new("pull-requests", pull_requests)
                .param(repository())
                .param(Param::choice("filter", &FILTERS).default("all"))
                .param(limit()),
        )
        .action(
            Action::new("review-requests", review_requests)
                .param(repository())
                .param(limit()),
        )
        .action(
            Action::new("checks", checks)
                .param(repository().required())
                .param(Param::text("ref").max_length(250))
                .param(pull_request()),
        )
        .action(
            Action::new("open", open)
                .param(Param::text("url").max_length(2048))
                .param(repository())
                .param(pull_request()),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);

fn optional(args: &Args<'_>, key: &str) -> Result<Option<String>, Failure> {
    Ok(args
        .get::<Option<String>>(key)?
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty()))
}

fn repository(args: &Args<'_>) -> Result<Option<Repo>, Failure> {
    optional(args, "repository")?
        .as_deref()
        .map(Repo::parse)
        .transpose()
}

fn required_repository(args: &Args<'_>) -> Result<Repo, Failure> {
    repository(args)?.ok_or_else(|| {
        fail(
            codes::INVALID_REPOSITORY,
            "choose a repository (owner/name)",
        )
    })
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn number(value: &Value, key: &str) -> i64 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map_or(0, saturating_i64)
}

fn saturating_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn boolean(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn invalid_response(what: &str) -> Failure {
    fail(
        codes::INVALID_RESPONSE,
        format!("GitHub's answer for {what} is not what was expected"),
    )
}

/// `lines` joined by newlines, as many as fit into one output.
fn bounded_lines(lines: &[String]) -> (String, usize) {
    let mut text = String::new();
    let mut kept = 0;
    for line in lines {
        let needed = line.len() + usize::from(kept > 0);
        if text.len() + needed > MAX_OUTPUT_TEXT {
            break;
        }
        if kept > 0 {
            text.push('\n');
        }
        text.push_str(line);
        kept += 1;
    }
    (text, kept)
}

fn current_user(ctx: &mut Context<'_>, _args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let reply = api.get(api.endpoint(&["user"], &[]), "the signed-in user")?;
    let user = &reply.json;
    let login = string(user, "login").ok_or_else(|| invalid_response("the signed-in user"))?;
    let mut output = Output::message(format!("signed in to GitHub as {login}"))
        .with("login", login.clone())
        .with(
            "url",
            string(user, "html_url").unwrap_or_else(|| format!("{}{login}", api.web())),
        );
    if let Some(name) = string(user, "name").filter(|n| !n.is_empty()) {
        output = output.with("name", clean(&name, 200));
    }
    if let Some(remaining) = reply.rate_remaining {
        output = output.with("rateLimitRemaining", saturating_i64(remaining));
    }
    Ok(output)
}

fn repository_info(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let repo = required_repository(args)?;
    let what = format!("the repository {}", repo.full_name());
    let reply = api.get(
        api.endpoint(&["repos", &repo.owner, &repo.name], &[]),
        &what,
    )?;
    let json = &reply.json;
    let full_name = string(json, "full_name").ok_or_else(|| invalid_response(&what))?;
    let mut output = Output::message(format!("read {full_name}"))
        .with("fullName", full_name.clone())
        .with(
            "defaultBranch",
            string(json, "default_branch").unwrap_or_default(),
        )
        .with("private", boolean(json, "private"))
        .with("archived", boolean(json, "archived"))
        .with("openIssues", number(json, "open_issues_count"))
        .with("stars", number(json, "stargazers_count"))
        .with("forks", number(json, "forks_count"))
        .with(
            "url",
            string(json, "html_url").unwrap_or_else(|| format!("{}{full_name}", api.web())),
        );
    if let Some(description) = string(json, "description").filter(|d| !d.is_empty()) {
        output = output.with("description", clean(&description, 500));
    }
    if let Some(pushed) = string(json, "pushed_at") {
        output = output.with("pushedAt", pushed);
    }
    Ok(output)
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
    let number = number(item, "number");
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

fn pull_requests(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    list_pull_requests(ctx, args, false)
}

fn review_requests(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    list_pull_requests(ctx, args, true)
}

fn list_pull_requests(
    ctx: &mut Context<'_>,
    args: &Args<'_>,
    review_requests: bool,
) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let repo = repository(args)?;
    let filter = if review_requests {
        "review-requested".to_owned()
    } else {
        args.get::<Option<String>>("filter")?
            .unwrap_or_else(|| "all".to_owned())
    };
    if !FILTERS.contains(&filter.as_str()) {
        return Err(fail(
            deckpilot_plugin_sdk::codes::INVALID_PARAMETERS,
            format!("“{}” is not a filter", clean(&filter, 40)),
        ));
    }
    if filter == "all" && repo.is_none() {
        return Err(fail(
            codes::INVALID_REPOSITORY,
            "choose a repository to list all its open pull requests",
        ));
    }
    let limit = args
        .get::<Option<i64>>("limit")?
        .unwrap_or(20)
        .clamp(1, MAX_LISTED);
    let limit = usize::try_from(limit).unwrap_or(api::PER_PAGE);
    let query = search_query(repo.as_ref(), &filter);
    // Pages of at most 50: the first page is the built-in's one request
    // whenever `limit` is 50 or less, a second page (GitHub's `next`
    // link) lists the rest up to 100.
    let per_page = limit.min(api::PER_PAGE).to_string();
    let url = api.endpoint(
        &["search", "issues"],
        &[
            ("q", &query),
            ("sort", "updated"),
            ("order", "desc"),
            ("per_page", &per_page),
        ],
    );
    let what = match &repo {
        Some(repo) => format!("the pull requests of {}", repo.full_name()),
        None => "your pull requests".to_owned(),
    };
    let mut reply = api.get(url, &what)?;
    let total = reply
        .json
        .get("total_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_response(&what))?;
    let mut items = reply
        .json
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_response(&what))?
        .clone();
    let mut pages = 1;
    while items.len() < limit
        && pages < limit.div_ceil(api::PER_PAGE)
        && let Some(next) = reply.next.take()
    {
        reply = api.get(next, &what)?;
        pages += 1;
        let more = reply
            .json
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_response(&what))?;
        items.extend(more.iter().cloned());
    }
    items.truncate(limit);
    let lines: Vec<String> = items.iter().map(pull_request_line).collect();
    let (text, listed) = bounded_lines(&lines);
    let listed = listed as u64;
    let scope = repo
        .as_ref()
        .map_or_else(String::new, |r| format!(" in {}", r.full_name()));
    let message = if review_requests {
        format!("{total} pull requests wait for your review{scope}")
    } else {
        format!("{total} open pull requests{scope} ({filter})")
    };
    let mut output = Output::message(message)
        .with("count", saturating_i64(total))
        .with("listed", saturating_i64(listed))
        .with("items", text)
        .with("truncated", total > listed);
    if let Some(first) = items.first().and_then(|i| string(i, "html_url")) {
        output = output.with("firstUrl", first);
    }
    Ok(output)
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
    api: &mut Api<'_, '_>,
    args: &Args<'_>,
    repo: &Repo,
) -> Result<(String, String, Option<String>), Failure> {
    let full = repo.full_name();
    Ok(
        if let Some(number) = args.get::<Option<i64>>("pullRequest")? {
            let number = number.max(1).to_string();
            let what = format!("the pull request {full}#{number}");
            let url = api.endpoint(&["repos", &repo.owner, &repo.name, "pulls", &number], &[]);
            let reply = api.get(url, &what)?;
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
            let reference = optional(args, "ref")?.ok_or_else(|| {
                fail(
                    deckpilot_plugin_sdk::codes::INVALID_PARAMETERS,
                    "choose a branch, tag, commit or pull request",
                )
            })?;
            let reference = api::check_ref(&reference)?.to_owned();
            let shown = format!("{full}@{reference}");
            (reference, shown, None)
        },
    )
}

fn checks(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let repo = required_repository(args)?;
    let full = repo.full_name();
    let (reference, shown, page) = checked_commit(&mut api, args, &repo)?;
    let what = format!("the checks of {shown}");
    let per_page = api::PER_PAGE.to_string();
    let url = api.endpoint(
        &[
            "repos",
            &repo.owner,
            &repo.name,
            "commits",
            &reference,
            "check-runs",
        ],
        &[("per_page", &per_page), ("filter", "latest")],
    );
    let runs = api.get_list(url, &what, Some("check_runs"))?;
    let url = api.endpoint(
        &[
            "repos",
            &repo.owner,
            &repo.name,
            "commits",
            &reference,
            "status",
        ],
        &[("per_page", &per_page)],
    );
    let statuses = api.get_list(url, &what, Some("statuses"))?;
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
    let (mut failing_text, kept) = bounded_lines(&failing);
    if kept < failing.len() {
        // Only when the names exceed one output (16 KiB): say how many
        // more, on a line that still fits.
        let more = format!("… and {} more", failing.len() - kept);
        while failing_text.len() + more.len() + 1 > MAX_OUTPUT_TEXT {
            match failing_text.rfind('\n') {
                Some(at) => failing_text.truncate(at),
                None => failing_text.clear(),
            }
        }
        if !failing_text.is_empty() {
            failing_text.push('\n');
        }
        failing_text.push_str(&more);
    }
    Ok(Output::message(format!(
        "{shown}: {state} ({} passed, {} failed, {} pending)",
        counts.passed, counts.failed, counts.pending
    ))
    .with("state", state)
    .with("sha", sha)
    .with("total", saturating_i64(total))
    .with("passed", saturating_i64(counts.passed))
    .with("failed", saturating_i64(counts.failed))
    .with("pending", saturating_i64(counts.pending))
    .with("failing", failing_text)
    .with("truncated", truncated)
    .with("url", url))
}

fn open(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let settings = api::settings(ctx)?;
    let web = api::web_base(&api::api_base(&settings.api_url)?);
    let target = if let Some(url) = optional(args, "url")? {
        url
    } else {
        let repo = required_repository(args)?;
        let root = web.root();
        match args.get::<Option<i64>>("pullRequest")? {
            Some(number) => format!("{root}{}/pull/{}", repo.full_name(), number.max(1)),
            None => format!("{root}{}", repo.full_name()),
        }
    };
    let page = api::web_url(&target, &web)?;
    ctx.open_url(page.url)?;
    Ok(Output::message(format!("opened {}", page.shown)).with("url", page.shown))
}

#[cfg(test)]
mod tests;
