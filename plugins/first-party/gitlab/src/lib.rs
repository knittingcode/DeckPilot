//! `deckpilot.gitlab`: the first-party GitLab plugin, the replacement of
//! the built-in `gitlab` module (`docs/marketplace/module-migration.md`
//! §9, P11). It reads the signed-in user, projects, open merge requests,
//! review requests and pipelines from GitLab.com or a self-managed GitLab,
//! and opens GitLab pages in the browser.
//!
//! **The same actions.** Every action of the built-in, with the same name
//! (`gitlab.<action>` → `deckpilot.gitlab.<action>`), the same parameters
//! (except the per-action `credential` override, which the alias drops
//! when it names the module's own credential, §5.3), the same outputs,
//! messages and error codes. `crates/deckpilot-modules/tests/contracts.rs`
//! compares the manifest with the built-in's definitions, and
//! `apps/agent/tests/plugins_gitlab.rs` runs both on the same answers.
//!
//! **Read-only.** Only `GET` requests, to `https://gitlab.com` (the
//! manifest's `network` host) or to the server the user entered in the
//! `apiUrl` setting and allowed (its user-origin slot, within the path
//! entered); pages are opened on the same server only.
//!
//! **Token.** The credential slot `token`: the agent injects it in the
//! `PRIVATE-TOKEN` header into requests to that server and redacts it from
//! everything this module sees; it never enters the module.
#![deny(unsafe_code)]

use deckpilot_first_party_support::{bounded_lines, bounded_lines_with_rest, saturating_i64};
use deckpilot_plugin_sdk::prelude::*;
use serde_json::Value;

pub mod api;

use api::{Api, Failure, Project, clean, codes, fail};

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.gitlab";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Merge request filters of `merge-requests`.
pub const FILTERS: [&str; 3] = ["all", "author-me", "review-requested"];
/// Most merge requests listed.
pub const MAX_LISTED: i64 = 100;

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches` in the unit tests).
pub fn plugin() -> Plugin {
    let project = || Param::text("project").max_length(1000);
    let merge_request = || Param::integer("mergeRequest").min(1.0);
    let limit = || Param::integer("limit").min(1.0).max(100.0).default(20_i64);
    Plugin::new(ID, VERSION)
        .action(Action::new("current-user", current_user))
        .action(Action::new("project", project_info).param(project().required()))
        .action(
            Action::new("merge-requests", merge_requests)
                .param(project())
                .param(Param::choice("filter", &FILTERS).default("all"))
                .param(limit()),
        )
        .action(
            Action::new("review-requests", review_requests)
                .param(project())
                .param(limit()),
        )
        .action(
            Action::new("pipeline", pipeline)
                .param(project().required())
                .param(Param::text("ref").max_length(250))
                .param(merge_request()),
        )
        .action(
            Action::new("open", open)
                .param(Param::text("url").max_length(2048))
                .param(project())
                .param(merge_request()),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);

fn optional(args: &Args<'_>, key: &str) -> Result<Option<String>, Failure> {
    Ok(args
        .get::<Option<String>>(key)?
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty()))
}

fn project(args: &Args<'_>) -> Result<Option<Project>, Failure> {
    optional(args, "project")?
        .as_deref()
        .map(Project::parse)
        .transpose()
}

fn required_project(args: &Args<'_>) -> Result<Project, Failure> {
    project(args)?.ok_or_else(|| fail(codes::INVALID_PROJECT, "choose a project (group/project)"))
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

fn boolean(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn invalid_response(what: &str) -> Failure {
    fail(
        codes::INVALID_RESPONSE,
        format!("GitLab's answer for {what} is not what was expected"),
    )
}

fn current_user(ctx: &mut Context<'_>, _args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let reply = api.get(api.endpoint(&["user"], &[]), "the signed-in user")?;
    let user = &reply.json;
    let username =
        string(user, "username").ok_or_else(|| invalid_response("the signed-in user"))?;
    let mut output = Output::message(format!("signed in to GitLab as {username}"))
        .with("username", username.clone())
        .with(
            "url",
            string(user, "web_url").unwrap_or_else(|| format!("{}{username}", api.web())),
        );
    if let Some(name) = string(user, "name").filter(|n| !n.is_empty()) {
        output = output.with("name", clean(&name, 200));
    }
    if let Some(remaining) = reply.rate_remaining {
        output = output.with("rateLimitRemaining", saturating_i64(remaining));
    }
    Ok(output)
}

fn project_info(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let project = required_project(args)?;
    let what = format!("the project {}", project.id());
    let reply = api.get(api.endpoint(&["projects", project.id()], &[]), &what)?;
    let json = &reply.json;
    let path = string(json, "path_with_namespace").ok_or_else(|| invalid_response(&what))?;
    let mut output = Output::message(format!("read {path}"))
        .with("path", path.clone())
        .with(
            "visibility",
            string(json, "visibility").unwrap_or_else(|| "unknown".into()),
        )
        .with("archived", boolean(json, "archived"))
        .with("stars", number(json, "star_count"))
        .with("forks", number(json, "forks_count"))
        .with(
            "url",
            string(json, "web_url").unwrap_or_else(|| format!("{}{path}", api.web())),
        );
    if let Some(branch) = string(json, "default_branch") {
        output = output.with("defaultBranch", branch);
    }
    // Absent when the project's issues are turned off.
    if let Some(issues) = json.get("open_issues_count").and_then(Value::as_u64) {
        output = output.with("openIssues", saturating_i64(issues));
    }
    if let Some(description) = string(json, "description").filter(|d| !d.is_empty()) {
        output = output.with("description", clean(&description, 500));
    }
    if let Some(activity) = string(json, "last_activity_at") {
        output = output.with("lastActivityAt", activity);
    }
    Ok(output)
}

/// `group/project!12 Title (@username)`, one line.
fn merge_request_line(item: &Value) -> String {
    let reference = item
        .get("references")
        .and_then(|r| string(r, "full"))
        .unwrap_or_else(|| format!("!{}", number(item, "iid")));
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
fn me_filter(
    api: &mut Api<'_, '_>,
    filter: &str,
) -> Result<Option<(&'static str, String)>, Failure> {
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

fn merge_requests(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    list_merge_requests(ctx, args, false)
}

fn review_requests(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    list_merge_requests(ctx, args, true)
}

fn list_merge_requests(
    ctx: &mut Context<'_>,
    args: &Args<'_>,
    review_requests: bool,
) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let project = project(args)?;
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
    if filter == "all" && project.is_none() {
        return Err(fail(
            codes::INVALID_PROJECT,
            "choose a project to list all its open merge requests",
        ));
    }
    let limit = args
        .get::<Option<i64>>("limit")?
        .unwrap_or(20)
        .clamp(1, MAX_LISTED);
    let limit = usize::try_from(limit).unwrap_or(api::PER_PAGE);
    let mut query: Vec<(&str, String)> = vec![
        ("state", "opened".into()),
        ("order_by", "updated_at".into()),
        ("sort", "desc".into()),
    ];
    if let Some(me) = me_filter(&mut api, &filter)? {
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
    let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
    // The built-in's one request whenever `limit` is 50 or less; a second
    // page lists the rest up to 100.
    let list = api.get_list(&segments, &query, &what, limit)?;
    let items = list.items;
    // GitLab leaves `X-Total` out for very large lists; then count by
    // reading a bounded number of pages.
    let (count, exact) = match (list.total, list.more) {
        (Some(total), _) => (total, true),
        (None, false) => (items.len() as u64, true),
        (None, true) => {
            let counted = api.get_list(&segments, &query, &what, api::MAX_ITEMS)?;
            (counted.items.len() as u64, !counted.more)
        }
    };
    let lines: Vec<String> = items.iter().map(merge_request_line).collect();
    let (text, listed) = bounded_lines(&lines);
    let listed = listed as u64;
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
    let mut output = Output::message(message)
        .with("count", saturating_i64(count))
        .with("countExact", exact)
        .with("listed", saturating_i64(listed))
        .with("items", text)
        .with("truncated", count > listed || !exact);
    if let Some(first) = items.first().and_then(|i| string(i, "web_url")) {
        output = output.with("firstUrl", first);
    }
    Ok(output)
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

/// The pipeline to check, how it is named in messages and the merge
/// request's page.
fn chosen_pipeline(
    api: &mut Api<'_, '_>,
    args: &Args<'_>,
    project: &Project,
) -> Result<(Option<Value>, String, Option<String>), Failure> {
    if let Some(number) = args.get::<Option<i64>>("mergeRequest")? {
        let number = number.max(1).to_string();
        let what = format!("the merge request {}!{number}", project.id());
        let url = api.endpoint(&["projects", project.id(), "merge_requests", &number], &[]);
        let reply = api.get(url, &what)?;
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
    let reference = optional(args, "ref")?.ok_or_else(|| {
        fail(
            deckpilot_plugin_sdk::codes::INVALID_PARAMETERS,
            "choose a branch, tag, commit or merge request",
        )
    })?;
    let reference = api::check_ref(&reference)?;
    let is_sha = reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit());
    let what = format!("the pipelines of {}@{reference}", project.id());
    let url = api.endpoint(
        &["projects", project.id(), "pipelines"],
        &[
            (if is_sha { "sha" } else { "ref" }, reference),
            ("order_by", "id"),
            ("sort", "desc"),
            ("per_page", "1"),
        ],
    );
    let reply = api.get(url, &what)?;
    let pipeline = reply
        .json
        .as_array()
        .ok_or_else(|| invalid_response(&what))?
        .first()
        .cloned();
    Ok((pipeline, format!("{}@{reference}", project.id()), None))
}

fn pipeline(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let project = required_project(args)?;
    let (pipeline, shown, page) = chosen_pipeline(&mut api, args, &project)?;
    let Some(pipeline) = pipeline else {
        let mut output = Output::message(format!("{shown}: no pipeline"))
            .with("state", "none")
            .with("total", 0_i64)
            .with("passed", 0_i64)
            .with("failed", 0_i64)
            .with("pending", 0_i64)
            .with("skipped", 0_i64)
            .with("failing", "")
            .with("truncated", false);
        if let Some(page) = page {
            output = output.with("url", page);
        }
        return Ok(output);
    };
    let what = format!("the pipeline of {shown}");
    let id = pipeline
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_response(&what))?;
    let status = string(&pipeline, "status").unwrap_or_else(|| "unknown".into());
    let jobs = api.get_list(
        &[
            "projects",
            project.id(),
            "pipelines",
            &id.to_string(),
            "jobs",
        ],
        &[],
        &format!("the jobs of {shown}"),
        api::MAX_ITEMS,
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
    let truncated = jobs.more || jobs.total.is_some_and(|t| t > jobs.items.len() as u64);
    let mut output = Output::message(format!(
        "{shown}: {state} ({} passed, {} failed, {} pending)",
        counts.passed, counts.failed, counts.pending
    ))
    .with("state", state)
    .with("status", clean(&status, 40))
    .with("pipelineId", saturating_i64(id))
    .with("total", saturating_i64(jobs.items.len() as u64))
    .with("passed", saturating_i64(counts.passed))
    .with("failed", saturating_i64(counts.failed))
    .with("pending", saturating_i64(counts.pending))
    .with("skipped", saturating_i64(counts.skipped))
    .with("failing", bounded_lines_with_rest(&failing))
    .with("truncated", truncated);
    if let Some(sha) = string(&pipeline, "sha") {
        output = output.with("sha", sha);
    }
    if let Some(url) = page.or_else(|| string(&pipeline, "web_url")) {
        output = output.with("url", url);
    }
    Ok(output)
}

fn open(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let settings = api::settings(ctx)?;
    let web = api::web_base(&api::api_base(&settings.api_url)?);
    let target = if let Some(url) = optional(args, "url")? {
        url
    } else {
        let project = required_project(args)?;
        if project.is_numeric() {
            return Err(fail(
                codes::INVALID_PROJECT,
                "a project is opened by its path (group/project), not its id",
            ));
        }
        match args.get::<Option<i64>>("mergeRequest")? {
            Some(number) => format!(
                "{}{}/-/merge_requests/{}",
                web.root(),
                project.id(),
                number.max(1)
            ),
            None => format!("{}{}", web.root(), project.id()),
        }
    };
    let page = api::web_url(&target, &web)?;
    ctx.open_url(page.url)?;
    Ok(Output::message(format!("opened {}", page.shown)).with("url", page.shown))
}

#[cfg(test)]
mod tests;
