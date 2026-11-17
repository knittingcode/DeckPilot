//! `deckpilot.jira`: the first-party Jira plugin, the replacement of the
//! built-in `jira` module (`docs/marketplace/module-migration.md` §9,
//! P11). It reads the signed-in user, single issues, the issues assigned
//! to you and those of a saved filter from Jira Cloud or Data Center, and
//! opens Jira pages in the browser.
//!
//! **The same actions.** Every action of the built-in, with the same name
//! (`jira.<action>` → `deckpilot.jira.<action>`), the same parameters
//! (except the per-action `credential` override, which the alias drops
//! when it names the module's own credential, §5.3), the same outputs,
//! messages and error codes. `crates/deckpilot-modules/tests/contracts.rs`
//! compares the manifest with the built-in's definitions, and
//! `apps/agent/tests/plugins_jira.rs` runs both on the same answers.
//!
//! **Read-only.** Only `GET` requests, only to the Jira site the user
//! entered in the `siteUrl` setting and allowed (the manifest's one
//! user-origin slot, within the path entered); searches are fixed
//! templates, never free JQL; pages are opened on the same site only.
//!
//! **Credential.** The slot `credential`: the agent injects it as Basic
//! (Jira Cloud) or Bearer (Data Center) into requests to that site and
//! redacts it from everything this module sees; it never enters the
//! module.
#![deny(unsafe_code)]

use std::fmt::Write as _;

use deckpilot_first_party_support::{bounded_lines, saturating_i64};
use deckpilot_plugin_sdk::prelude::*;
use serde_json::Value;

pub mod api;

use api::{Api, Deployment, Failure, clean, codes, fail};

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.jira";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where `issues` takes its issues from.
pub const SOURCES: [&str; 2] = ["assigned-to-me", "filter"];
/// Most issues listed.
pub const MAX_LISTED: i64 = 100;
/// The fields read for a list.
const LIST_FIELDS: &str = "summary,status,assignee";

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches` in the unit tests).
pub fn plugin() -> Plugin {
    let issue = || Param::text("issue").max_length(40);
    Plugin::new(ID, VERSION)
        .action(Action::new("myself", myself))
        .action(Action::new("issue", issue_info).param(issue().required()))
        .action(
            Action::new("issues", issues)
                .param(Param::choice("source", &SOURCES).default("assigned-to-me"))
                .param(Param::integer("filterId").min(1.0))
                .param(Param::text("project").max_length(255))
                .param(Param::boolean("includeDone").default(false))
                .param(Param::integer("limit").min(1.0).max(100.0).default(20_i64)),
        )
        .action(
            Action::new("open", open)
                .param(Param::text("url").max_length(2048))
                .param(issue()),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);

fn optional(args: &Args<'_>, key: &str) -> Result<Option<String>, Failure> {
    Ok(args
        .get::<Option<String>>(key)?
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty()))
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

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

fn myself(ctx: &mut Context<'_>, _args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let what = "the signed-in user";
    let user = api.get(api.endpoint(&["myself"], &[]), what)?;
    let name = string(&user, "displayName").ok_or_else(|| invalid_response(what))?;
    let account = string(&user, "accountId")
        .or_else(|| string(&user, "name"))
        .ok_or_else(|| invalid_response(what))?;
    let name = clean(&name, 200);
    let mut output = Output::message(format!("signed in to Jira as {name}"))
        .with("displayName", name)
        .with("account", account);
    if let Some(zone) = string(&user, "timeZone") {
        output = output.with("timeZone", zone);
    }
    Ok(output)
}

/// The status category in DeckPilot's terms.
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

fn issue_info(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let key = api::issue_key(&args.get::<Option<String>>("issue")?.unwrap_or_default())?;
    let what = format!("the issue {key}");
    let url = api.endpoint(
        &["issue", &key],
        &[(
            "fields",
            "summary,status,issuetype,priority,assignee,updated",
        )],
    );
    let issue = api.get(url, &what)?;
    let key = string(&issue, "key").ok_or_else(|| invalid_response(&what))?;
    let status = field(&issue, "status", "name").unwrap_or_default();
    let mut output = Output::message(format!("{key}: {}", clean(&status, 60)))
        .with("key", key.clone())
        .with(
            "summary",
            clean(
                &issue
                    .get("fields")
                    .and_then(|f| string(f, "summary"))
                    .unwrap_or_default(),
                500,
            ),
        )
        .with("status", clean(&status, 60))
        .with("statusCategory", status_category(&issue))
        .with("url", api.browse(&key));
    if let Some(kind) = field(&issue, "issuetype", "name") {
        output = output.with("type", clean(&kind, 60));
    }
    if let Some(priority) = field(&issue, "priority", "name") {
        output = output.with("priority", clean(&priority, 60));
    }
    if let Some(assignee) = field(&issue, "assignee", "displayName") {
        output = output.with("assignee", clean(&assignee, 120));
    }
    if let Some(updated) = issue.get("fields").and_then(|f| string(f, "updated")) {
        output = output.with("updated", updated);
    }
    Ok(output)
}

/// The search for a list: a saved filter or the issues assigned to the
/// signed-in user, optionally in one project and without done issues.
/// `project` is a checked project key ([`api::project_key`]) and the
/// filter a number, so nothing the user types reaches the search as JQL.
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

/// One page of a search: Cloud's token-paged `search/jql`, Data Center's
/// `search`.
fn search_page(
    api: &mut Api<'_, '_>,
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
    let url = api.endpoint(path, &query);
    api.get(url, what)
}

/// Cloud gives no total: count by reading a bounded number of pages of
/// keys.
fn count_pages(api: &mut Api<'_, '_>, jql: &str, what: &str) -> Result<(u64, bool), Failure> {
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

fn issues(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let mut api = Api::connect(ctx)?;
    let source = args.get::<Option<String>>("source")?;
    let filter = if source.as_deref() == Some("filter") {
        Some(args.get::<Option<i64>>("filterId")?.ok_or_else(|| {
            fail(
                deckpilot_plugin_sdk::codes::INVALID_PARAMETERS,
                "choose the saved filter's number",
            )
        })?)
    } else {
        None
    };
    let project = optional(args, "project")?
        .as_deref()
        .map(api::project_key)
        .transpose()?;
    let jql = search_jql(
        filter,
        project.as_deref(),
        args.get::<Option<bool>>("includeDone")?.unwrap_or(false),
    );
    let limit = args
        .get::<Option<i64>>("limit")?
        .unwrap_or(20)
        .clamp(1, MAX_LISTED);
    let limit = usize::try_from(limit).unwrap_or(20);
    let what = match filter {
        Some(id) => format!("the filter {id}"),
        None => "your issues".to_owned(),
    };
    let page = search_page(&mut api, &jql, limit, LIST_FIELDS, None, &what)?;
    let items = page
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_response(&what))?;
    let (count, exact) = match api.deployment() {
        Deployment::DataCenter => (
            page.get("total")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid_response(&what))?,
            true,
        ),
        Deployment::Cloud if next_token(&page).is_none() => (items.len() as u64, true),
        Deployment::Cloud => count_pages(&mut api, &jql, &what)?,
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
    let (text, listed) = bounded_lines(&lines);
    let listed = listed as u64;
    let mut output = Output::message(message)
        .with("count", saturating_i64(count))
        .with("countExact", exact)
        .with("listed", saturating_i64(listed))
        .with("items", text)
        .with("truncated", count > listed || !exact);
    if let Some(key) = items.first().and_then(|i| string(i, "key")) {
        output = output
            .with("firstUrl", api.browse(&key))
            .with("firstKey", key);
    }
    Ok(output)
}

fn open(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let settings = api::settings(ctx)?;
    let site = api::site(&settings.site_url)?;
    let target = if let Some(url) = optional(args, "url")? {
        url
    } else {
        let key = optional(args, "issue")?.ok_or_else(|| {
            fail(
                codes::INVALID_ISSUE_KEY,
                "choose a page or an issue to open",
            )
        })?;
        format!("{}browse/{}", site.root(), api::issue_key(&key)?)
    };
    let page = api::web_url(&target, &site)?;
    ctx.open_url(page.url)?;
    Ok(Output::message(format!("opened {}", page.shown)).with("url", page.shown))
}

#[cfg(test)]
mod tests;
