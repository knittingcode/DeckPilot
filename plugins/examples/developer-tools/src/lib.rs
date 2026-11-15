//! Developer Tools: an example DeckPilot plugin.
//!
//! Pure utilities, which ask the agent for nothing:
//!
//! * `timestamp`: Unix seconds or milliseconds to RFC 3339 and back;
//! * `base64`: encode or decode text (standard or URL-safe alphabet);
//! * `json-format`: pretty-print or compact JSON;
//! * `uuid-v5`: a name-based UUID (RFC 9562 version 5).
//!
//! GitHub checks, through capabilities the user approved:
//!
//! * `open-prs`: how many pull requests of a repository are open;
//! * `build-status`: the latest GitHub Actions run of a branch;
//! * `last-values`: what the last checks found (storage only).
//!
//! Every request goes to `https://api.github.com` through the agent's
//! permission broker (`network` permission, GET only). A GitHub token, if
//! the user stored one, is the credential slot `githubToken`: the plugin
//! names the slot, the agent injects the secret, and the secret never
//! enters this module. The checks publish the plugin's own variables
//! `openPrs` and `buildStatus`, keep the last results in the plugin's
//! storage, read the default repository and branch from the settings, and
//! (when the user turns it on) notify when the build status changes.
//!
//! There is no clock and no random source in a guest, so there is no "now"
//! and no random (version 4) UUID or ULID; the time of a check is the
//! `Date` header of GitHub's answer.
#![deny(unsafe_code)]

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use deckpilot_plugin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

pub mod util;

/// The only origin this plugin talks to: the manifest's `network`
/// permission (`hosts: ["api.github.com"]`, `methods: ["GET"]`).
pub const GITHUB_API: &str = "https://api.github.com";
/// The credential slot (`contributes.credentials[0].key`).
pub const TOKEN_SLOT: &str = "githubToken";
/// GitHub refuses requests without a User-Agent; the agent sets none.
const USER_AGENT: &str = "deckpilot-developer-tools/0.1.0";

/// Longest text an output value can carry (ABI 1 `MAX_STRING_LEN`).
const MAX_OUTPUT_TEXT: usize = 16 * 1024;

/// Storage keys.
const LAST_OPEN_PRS: &str = "last.openPrs";
const LAST_BUILD: &str = "last.buildStatus";

/// Codes besides the shared ones; each is declared in `manifest.json`
/// `contributes.errorCodes`.
pub const PLUGIN_CODES: &[&str] = &[
    INVALID_INPUT,
    REPO_REQUIRED,
    REPO_NOT_FOUND,
    GITHUB_UNAUTHORIZED,
    GITHUB_ERROR,
    INVALID_RESPONSE,
    OUTPUT_TOO_LARGE,
];
const INVALID_INPUT: &str = "invalid-input";
const REPO_REQUIRED: &str = "repo-required";
const REPO_NOT_FOUND: &str = "repo-not-found";
const GITHUB_UNAUTHORIZED: &str = "github-unauthorized";
const GITHUB_ERROR: &str = "github-error";
const INVALID_RESPONSE: &str = "invalid-response";
const OUTPUT_TOO_LARGE: &str = "output-too-large";

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches` in the unit tests).
pub fn plugin() -> Plugin {
    Plugin::new("acme.developer-tools", "0.1.0")
        .action(
            Action::new("timestamp", timestamp)
                .param(Param::text("value").required().max_length(40))
                .param(
                    Param::choice("from", &["auto", "seconds", "milliseconds", "rfc3339"])
                        .default("auto"),
                ),
        )
        .action(
            Action::new("base64", base64_action)
                .param(Param::text("text").required().max_length(4096))
                .param(Param::choice("mode", &["encode", "decode"]).default("encode"))
                .param(Param::choice("alphabet", &["standard", "url-safe"]).default("standard")),
        )
        .action(
            Action::new("json-format", json_format)
                .param(Param::text("json").required().max_length(4096))
                .param(Param::choice("style", &["pretty", "compact"]).default("pretty")),
        )
        .action(
            Action::new("uuid-v5", uuid_v5)
                .param(Param::text("name").required().max_length(1024))
                .param(Param::choice("namespace", &["dns", "url", "oid", "x500"]).default("dns")),
        )
        .action(Action::new("open-prs", open_prs).param(Param::text("repo").max_length(140)))
        .action(
            Action::new("build-status", build_status)
                .param(Param::text("repo").max_length(140))
                .param(Param::text("branch").max_length(100)),
        )
        .action(Action::new("last-values", last_values))
}

deckpilot_plugin_sdk::plugin!(plugin);

fn invalid(message: impl Into<String>) -> PluginError {
    PluginError::new(INVALID_INPUT, message)
}

/// An output text, refused when it is longer than ABI 1 lets a value be.
fn output_text(text: String) -> Result<String, PluginError> {
    if text.len() > MAX_OUTPUT_TEXT {
        return Err(PluginError::new(
            OUTPUT_TOO_LARGE,
            format!(
                "the result is {} bytes; an output can hold at most {MAX_OUTPUT_TEXT}",
                text.len()
            ),
        ));
    }
    Ok(text)
}

// ------------------------------------------------------------ pure actions

fn timestamp(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let value: String = args.get("value")?;
    let from: String = args.get("from")?;
    let (millis, unit) = util::parse_timestamp(&value, &from).map_err(invalid)?;
    let iso = util::format_rfc3339(millis);
    let read_as = match unit {
        util::TimeUnit::Seconds => "seconds",
        util::TimeUnit::Millis => "milliseconds",
        util::TimeUnit::Rfc3339 => "rfc3339",
    };
    Ok(
        Output::message(format!("{iso} ({} s)", millis.div_euclid(1000)))
            .with("iso", iso)
            .with("unixSeconds", millis.div_euclid(1000))
            .with("unixMillis", millis)
            .with("readAs", read_as),
    )
}

fn base64_action(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let text: String = args.get("text")?;
    let mode: String = args.get("mode")?;
    let url_safe = args.get::<String>("alphabet")? == "url-safe";
    let result = if mode == "decode" {
        let input = text.trim();
        let decoded = if url_safe {
            // Padding is optional in the URL-safe form.
            URL_SAFE_NO_PAD
                .decode(input.trim_end_matches('='))
                .map_err(|e| invalid(format!("not URL-safe base64: {e}")))?
        } else {
            STANDARD
                .decode(input)
                .map_err(|e| invalid(format!("not standard base64: {e}")))?
        };
        String::from_utf8(decoded).map_err(|_| invalid("the decoded bytes are not UTF-8 text"))?
    } else if url_safe {
        URL_SAFE.encode(text.as_bytes())
    } else {
        STANDARD.encode(text.as_bytes())
    };
    let result = output_text(result)?;
    let verb = if mode == "decode" {
        "Decoded"
    } else {
        "Encoded"
    };
    Ok(Output::message(format!("{verb} {} bytes", text.len())).with("result", result))
}

fn json_format(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let text: String = args.get("json")?;
    let style: String = args.get("style")?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        invalid(format!(
            "not JSON: {} at line {}, column {}",
            match e.classify() {
                serde_json::error::Category::Eof => "unexpected end",
                _ => "syntax error",
            },
            e.line(),
            e.column()
        ))
    })?;
    let formatted = if style == "compact" {
        serde_json::to_string(&value)
    } else {
        serde_json::to_string_pretty(&value)
    }
    .map_err(|_| invalid("the JSON does not serialize"))?;
    let formatted = output_text(formatted)?;
    Ok(Output::message(format!("{} bytes of JSON", formatted.len())).with("result", formatted))
}

fn uuid_v5(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let name: String = args.get("name")?;
    let namespace: String = args.get("namespace")?;
    let Some(ns) = util::namespace(&namespace) else {
        return Err(PluginError::new(
            codes::PLUGIN_ERROR,
            "namespace table out of sync",
        ));
    };
    let uuid = util::uuid_v5(ns, &name);
    Ok(Output::message(uuid.clone()).with("uuid", uuid))
}

// ----------------------------------------------------------- GitHub checks

/// The repository from the parameter, else the `defaultRepo` setting.
fn repo_of(args: &Args<'_>, settings: &Settings) -> Result<String, PluginError> {
    let repo = match args.get::<Option<String>>("repo")? {
        Some(r) if !r.trim().is_empty() => r.trim().to_owned(),
        _ => match settings.text("defaultRepo") {
            Some(r) if !r.trim().is_empty() => r.trim().to_owned(),
            _ => {
                return Err(PluginError::new(
                    REPO_REQUIRED,
                    "name a repository (owner/name) or set a default repository in the \
                     plugin's settings",
                ));
            }
        },
    };
    if !util::is_repo(&repo) {
        return Err(invalid(format!(
            "`{}` is not a GitHub repository (owner/name)",
            util::cut(&repo, 150)
        )));
    }
    Ok(repo)
}

/// A GET to the GitHub API, with the token's handle when the user stored
/// a token. The secret is added by the agent, never seen here.
fn github_get(ctx: &mut Context<'_>, path_and_query: &str) -> Result<Response, PluginError> {
    let mut request = Request::get(format!("{GITHUB_API}{path_and_query}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", USER_AGENT);
    let token = SecretHandle::slot(TOKEN_SLOT)?;
    if ctx.has_credential(&token)? {
        request = request.bearer(&token);
    }
    let response = ctx.fetch(request)?;
    match response.status {
        200..=299 => Ok(response),
        401 => Err(PluginError::new(
            GITHUB_UNAUTHORIZED,
            "GitHub refused the token; store a new one in the plugin's credentials",
        )),
        403 | 429 => Err(PluginError::new(
            codes::THROTTLED,
            format!(
                "GitHub refused the request (HTTP {}), usually its rate limit{}",
                response.status,
                response
                    .header("retry-after")
                    .map(|s| format!("; retry after {} s", util::cut(s, 10)))
                    .unwrap_or_default()
            ),
        )
        .retryable()),
        404 | 422 => Err(PluginError::new(
            REPO_NOT_FOUND,
            "GitHub does not know this repository or branch (or the token cannot see it)",
        )),
        s @ 500..=599 => {
            Err(PluginError::new(GITHUB_ERROR, format!("GitHub failed (HTTP {s})")).retryable())
        }
        s => Err(PluginError::new(
            GITHUB_ERROR,
            format!("GitHub answered HTTP {s}"),
        )),
    }
}

fn unexpected() -> PluginError {
    PluginError::new(INVALID_RESPONSE, "GitHub sent an unexpected answer")
}

/// When the check happened: the answer's `Date` header (the guest has no
/// clock).
fn checked_at(response: &Response) -> String {
    response
        .header("date")
        .map(|d| util::cut(d, 64))
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct SearchResult {
    total_count: i64,
}

/// What `open-prs` keeps in storage.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastOpenPrs {
    pub repo: String,
    pub count: i64,
    pub checked_at: String,
}

fn open_prs(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let settings = ctx.settings()?;
    let repo = repo_of(args, &settings)?;
    let query = util::percent_encode(&format!("repo:{repo} is:pr is:open"));
    let response = github_get(ctx, &format!("/search/issues?q={query}&per_page=1"))?;
    let result: SearchResult = response.json().map_err(|_| unexpected())?;
    if result.total_count < 0 {
        return Err(unexpected());
    }
    let last = LastOpenPrs {
        repo: repo.clone(),
        count: result.total_count,
        checked_at: checked_at(&response),
    };
    ctx.storage_set(LAST_OPEN_PRS, &last)?;
    ctx.publish_variable("openPrs", last.count)?;
    ctx.debug(format!("{repo}: {} open pull requests", last.count));
    let noun = if last.count == 1 {
        "pull request"
    } else {
        "pull requests"
    };
    Ok(
        Output::message(format!("{repo}: {} open {noun}", last.count))
            .with("count", last.count)
            .with("repo", repo)
            .with("checkedAt", last.checked_at),
    )
}

#[derive(Deserialize)]
struct Runs {
    workflow_runs: Vec<Run>,
}

#[derive(Deserialize)]
struct Run {
    #[serde(default)]
    name: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

/// What `build-status` keeps in storage.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastBuild {
    pub repo: String,
    pub branch: String,
    pub status: String,
    pub checked_at: String,
}

/// `success`, `failure`, `cancelled`, ... for a finished run, the run's
/// status (`queued`, `in_progress`, ...) otherwise, `none` without runs.
fn status_of(run: Option<&Run>) -> String {
    let word = |s: &str| {
        s.chars()
            .take(32)
            .all(|c| c.is_ascii_lowercase() || c == '_')
            .then(|| s.to_owned())
    };
    match run {
        None => "none".into(),
        Some(r) if r.status.as_deref() == Some("completed") => r
            .conclusion
            .as_deref()
            .and_then(word)
            .unwrap_or_else(|| "unknown".into()),
        Some(r) => r
            .status
            .as_deref()
            .and_then(word)
            .unwrap_or_else(|| "unknown".into()),
    }
}

fn build_status(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let settings = ctx.settings()?;
    let repo = repo_of(args, &settings)?;
    let branch = match args.get::<Option<String>>("branch")? {
        Some(b) if !b.trim().is_empty() => b.trim().to_owned(),
        _ => settings.text("defaultBranch").unwrap_or("main").to_owned(),
    };
    if !util::is_branch(&branch) {
        return Err(invalid(format!(
            "`{}` is not a branch name",
            util::cut(&branch, 100)
        )));
    }
    let response = github_get(
        ctx,
        &format!(
            "/repos/{repo}/actions/runs?branch={}&per_page=1",
            util::percent_encode(&branch)
        ),
    )?;
    let runs: Runs = response.json().map_err(|_| unexpected())?;
    let run = runs.workflow_runs.first();
    let status = status_of(run);
    let last = LastBuild {
        repo: repo.clone(),
        branch: branch.clone(),
        status: status.clone(),
        checked_at: checked_at(&response),
    };
    let previous: Option<LastBuild> = ctx.storage_get_as(LAST_BUILD).unwrap_or(None);
    ctx.storage_set(LAST_BUILD, &last)?;
    ctx.publish_variable("buildStatus", &status)?;
    let changed = previous
        .as_ref()
        .is_some_and(|p| p.repo == last.repo && p.branch == last.branch && p.status != last.status);
    if changed && settings.boolean("notifyOnChange") == Some(true) {
        let was = previous.map(|p| p.status).unwrap_or_default();
        // A notification that cannot be shown does not fail the check.
        if let Err(e) = ctx.notify(
            format!("Build {status}"),
            util::cut(&format!("{repo} {branch}: {was} -> {status}"), 300),
        ) {
            ctx.warn(format!("notification not shown: {e}"));
        }
    }
    let mut out = Output::message(format!("{repo} {branch}: {status}"))
        .with("status", status)
        .with("repo", repo)
        .with("branch", branch)
        .with("changed", changed)
        .with("checkedAt", last.checked_at);
    if let Some(run) = run {
        if let Some(name) = &run.name {
            out = out.with("workflow", util::cut(name, 200));
        }
        if let Some(url) = run
            .html_url
            .as_deref()
            .filter(|u| u.starts_with("https://github.com/"))
        {
            out = out.with("url", util::cut(url, 500));
        }
    }
    Ok(out)
}

fn last_values(ctx: &mut Context<'_>, _args: &Args<'_>) -> ActionResult {
    let prs: Option<LastOpenPrs> = ctx.storage_get_as(LAST_OPEN_PRS)?;
    let build: Option<LastBuild> = ctx.storage_get_as(LAST_BUILD)?;
    let usage = ctx.storage_usage()?;
    let mut parts = Vec::new();
    if let Some(p) = &prs {
        parts.push(format!("{}: {} open", p.repo, p.count));
    }
    if let Some(b) = &build {
        parts.push(format!("{} {}: {}", b.repo, b.branch, b.status));
    }
    if parts.is_empty() {
        return Ok(Output::skipped("nothing checked yet"));
    }
    let mut out = Output::message(util::cut(&parts.join("; "), 1000)).with(
        "storageBytes",
        i64::try_from(usage.used_bytes).unwrap_or(i64::MAX),
    );
    if let Some(p) = prs {
        out = out
            .with("openPrsRepo", p.repo)
            .with("openPrs", p.count)
            .with("openPrsCheckedAt", p.checked_at);
    }
    if let Some(b) = build {
        out = out
            .with("buildRepo", b.repo)
            .with("buildBranch", b.branch)
            .with("buildStatus", b.status)
            .with("buildCheckedAt", b.checked_at);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
