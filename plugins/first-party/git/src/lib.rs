//! `deckpilot.git`: the first-party Git plugin, the replacement of the
//! built-in `git` module (`docs/marketplace/module-migration.md` §7 X3a,
//! §11 D1). It reads the status, branch, changes, last commit and remote
//! state of a repository, and opens it.
//!
//! **The same actions.** Every action of the built-in under the same name
//! (`git.<action>` → `deckpilot.git.<action>`), with the same parameters
//! (the project parameter is `projectId`, since a manifest key is camel
//! case), outputs, messages and error codes; `README.md` lists the
//! differences.
//!
//! **No process of its own.** Git runs through `tool.run` (plugin API
//! 1.9): the agent runs the user's own `git` with its fixed safety prefix
//! (`-C <folder> --no-pager --no-optional-locks -c core.fsmonitor=false
//! …`), a cleared environment and its time and output limits, in the
//! folder of a resource handle of this invocation. The plugin only chooses
//! the subcommand and options its manifest declares (`status`, `log`,
//! `config --get branch.<b>.remote|merge`, `remote`, `rev-parse`, and
//! `ls-remote` for `remote-status` with `refresh`, the only network use)
//! and parses the output. It never sees a path but the bound folder's
//! (`resources.paths`, for the `repository` output), never an
//! executable, never the environment.
//!
//! **Key state.** The agent allows no `tool.run` in a state call, so a
//! key cannot read the repository by itself any more: `status` and
//! `branch` keys show what the last run of the action (a key press, a
//! workflow) found, kept in the plugin's storage ([`state`]).
#![deny(unsafe_code)]

use std::fmt::Write as _;

use deckpilot_plugin_sdk::prelude::*;
use deckpilot_plugin_sdk::{
    PRIVILEGED_API_LEVEL, ResourceHandle, ResourceKind, ResourceOpen, ResourceResolve, Tool,
};

pub mod porcelain;
pub mod state;

use porcelain::{Branch, Status};

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.git";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The folder resource parameter.
pub const REPOSITORY: &str = "repository";
/// The project parameter (`project_id` of the built-in).
pub const PROJECT: &str = "projectId";
/// Setting: the project resource that is the repository.
pub const REPOSITORY_ROLE: &str = "repositoryRole";
/// Setting: time limit of `ls-remote`, in seconds (the agent's own limit
/// for network runs still applies; this can only shorten it).
pub const NETWORK_TIMEOUT: &str = "networkTimeoutSeconds";
/// The default role.
pub const DEFAULT_ROLE: &str = "repository";
/// Most changed files listed by `changes`.
pub const MAX_LISTED: i64 = 500;
/// `git log -1` as the built-in reads it; `format:` because the agent
/// accepts only explicit formats.
pub const LOG_FORMAT: &str = "--format=format:%H%x00%h%x00%an%x00%aI%x00%s";

/// Stable error codes of the actions: the built-in's, and those of the
/// agent's resource resolution the plugin passes on.
pub mod codes {
    /// The repository folder does not exist.
    pub const NOT_FOUND: &str = "not-found";
    /// The folder is not a Git repository.
    pub const NOT_A_REPOSITORY: &str = "not-a-repository";
    /// Git refuses the repository because another user owns it.
    pub const UNSAFE_REPOSITORY: &str = "unsafe-repository";
    /// The `git` program was not found.
    pub const GIT_NOT_FOUND: &str = "git-not-found";
    /// Git reported an error.
    pub const GIT_FAILED: &str = "git-failed";
    /// The explicit project is not one of the profile's.
    pub const PROJECT_NOT_IN_PROFILE: &str = "project-not-in-profile";
    /// The branch has no commits yet.
    pub const NO_COMMITS: &str = "no-commits";
    /// The upstream's remote is not configured.
    pub const NO_REMOTE: &str = "no-remote";
    /// Contacting the remote failed.
    pub const NETWORK_FAILED: &str = "network-failed";
    /// No project is active and no repository is set (shared).
    pub const NO_ACTIVE_PROJECT: &str = "no-active-project";
    /// The run was stopped.
    pub const CANCELLED: &str = "cancelled";
}

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches` in the unit tests; the `resource`
/// parameters arrive as handles, so they are text here).
pub fn plugin() -> Plugin {
    let with_repository = |action: Action| {
        action
            .param(Param::text(REPOSITORY))
            .param(Param::text(PROJECT))
    };
    Plugin::new(ID, VERSION)
        .requires_api(PRIVILEGED_API_LEVEL)
        .action(with_repository(Action::new("status", status)).state(state::status))
        .action(with_repository(Action::new("branch", branch)).state(state::branch))
        .action(
            with_repository(Action::new("changes", changes))
                .param(Param::integer("limit").min(1.0).max(500.0).default(100_i64)),
        )
        .action(with_repository(Action::new("last-commit", last_commit)))
        .action(
            with_repository(Action::new("remote-status", remote_status))
                .param(Param::boolean("refresh").default(false)),
        )
        .action(
            with_repository(Action::new("open-repository", open_repository))
                .param(Param::text("application")),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);

// ------------------------------------------------------------ the agent

/// The reason code the agent puts at the start of a refusal
/// (`no-active-project: …`), when there is one.
pub fn reason(error: &CapabilityError) -> Option<&str> {
    let (code, _) = error.detail().split_once(": ")?;
    (!code.is_empty()
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    .then_some(code)
}

/// The agent's message after its reason code.
fn reason_message(error: &CapabilityError) -> &str {
    error
        .detail()
        .split_once(": ")
        .map_or(error.detail(), |(_, rest)| rest)
}

/// The repository an action works on.
#[derive(Debug, Clone)]
pub struct Repository {
    /// A handle of this invocation.
    pub handle: String,
    /// The bound folder (`resources.paths`), else the binding's name.
    pub shown: String,
    /// The project it belongs to, when it came from a project.
    pub project_key: Option<String>,
}

impl Repository {
    fn of(handle: ResourceHandle, project_key: Option<String>) -> Self {
        Self {
            shown: handle.display_path.unwrap_or(handle.label),
            handle: handle.handle,
            project_key,
        }
    }
}

/// What a resolution refusal means for the action.
fn resolve_failure(error: &CapabilityError, explicit_project: bool, what: &str) -> PluginError {
    match reason(error) {
        Some(codes::NO_ACTIVE_PROJECT) => PluginError::new(
            codes::NO_ACTIVE_PROJECT,
            format!("no project is active and no {what} is set"),
        ),
        Some("not-permitted") if explicit_project => PluginError::new(
            codes::PROJECT_NOT_IN_PROFILE,
            "the project does not belong to this profile",
        ),
        Some("not-permitted") | None => PluginError::from(error.clone()),
        Some(code) => PluginError::new(code, reason_message(error)),
    }
}

/// The repository, as the built-in finds it:
///
/// 1. the `repository` parameter (a folder resource, fixed or a role of
///    the run's project),
/// 2. otherwise the `projectId` parameter's project and its repository
///    role (the setting `repositoryRole`),
/// 3. otherwise the run's own project and that role.
///
/// The agent resolves all of them; the plugin never sees a path it did
/// not bind. The active project is never changed.
pub fn repository(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<Repository, PluginError> {
    match ctx.resource_param(REPOSITORY, &[ResourceKind::Directory]) {
        Ok(handle) => return Ok(Repository::of(handle, None)),
        Err(e) if reason(&e) == Some("resource-not-set") => {}
        Err(e) => return Err(resolve_failure(&e, false, "repository")),
    }
    let project: Option<String> = args.get(PROJECT)?;
    let role = setting_text(ctx, REPOSITORY_ROLE).unwrap_or_else(|| DEFAULT_ROLE.to_owned());
    let handle = ctx
        .resolve_resource(ResourceResolve {
            param: None,
            role: Some(role),
            project_id: project.clone(),
            kinds: vec![ResourceKind::Directory],
        })
        .map_err(|e| resolve_failure(&e, project.is_some(), "repository"))?;
    let key = project_key(ctx, project.as_deref());
    Ok(Repository::of(handle, key))
}

/// The key of the project a role was resolved in: the active project's
/// (the `profile` read), when no other project was named.
fn project_key(ctx: &mut Context<'_>, explicit: Option<&str>) -> Option<String> {
    let project = ctx.profile().ok()?.project?;
    explicit
        .is_none_or(|id| id == project.id)
        .then_some(project.key)
}

fn setting_text(ctx: &mut Context<'_>, key: &str) -> Option<String> {
    ctx.settings()
        .ok()?
        .text(key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// A refused or failed `tool.run`, as the built-in reports a failed git.
fn tool_failure(error: &CapabilityError, repository: &str) -> PluginError {
    match reason(error) {
        // The agent asks git for the work tree before every run.
        Some(codes::NOT_A_REPOSITORY) => PluginError::new(
            codes::NOT_A_REPOSITORY,
            format!("{repository} is not a Git repository"),
        ),
        Some("tool-outside-binding") => PluginError::new(
            codes::NOT_A_REPOSITORY,
            format!(
                "{repository} is not a Git repository: the repository around it starts in a                  folder above it"
            ),
        ),
        Some("tool-not-found") => PluginError::new(
            codes::GIT_NOT_FOUND,
            "Git was not found; install it or set its location in DeckPilot's settings",
        ),
        Some("timed-out") => PluginError::new(
            deckpilot_plugin_sdk::codes::TIMEOUT,
            format!("git: {}", reason_message(error)),
        ),
        Some(codes::CANCELLED) => {
            PluginError::new(codes::CANCELLED, format!("git: {}", reason_message(error)))
        }
        _ if error.kind() == CapabilityErrorKind::NotPermitted => PluginError::from(error.clone()),
        _ => PluginError::new(codes::GIT_FAILED, format!("git: {}", error.detail())),
    }
}

/// What a git that ran and failed means, from its error output.
pub fn git_failure(repository: &str, subcommand: &str, stderr: &str) -> PluginError {
    let stderr = stderr.trim();
    let first = stderr.lines().next().unwrap_or_default();
    if stderr.contains("not a git repository") {
        PluginError::new(
            codes::NOT_A_REPOSITORY,
            format!("{repository} is not a Git repository"),
        )
    } else if stderr.contains("dubious ownership") {
        PluginError::new(
            codes::UNSAFE_REPOSITORY,
            format!("Git refuses {repository} because another user owns it (safe.directory)"),
        )
    } else {
        PluginError::new(
            codes::GIT_FAILED,
            format!("git {subcommand} failed: {first}"),
        )
    }
}

/// Runs git in the repository; its standard output when it succeeded.
fn git(
    ctx: &mut Context<'_>,
    repository: &Repository,
    args: &[&str],
    timeout_ms: Option<u32>,
) -> Result<String, PluginError> {
    let output = ctx
        .run_tool(Tool::Git, args, &repository.handle, timeout_ms)
        .map_err(|e| tool_failure(&e, &repository.shown))?;
    match output.exit {
        Some(0) => Ok(output.stdout),
        Some(_) => Err(git_failure(
            &repository.shown,
            args.first().copied().unwrap_or_default(),
            &output.stderr,
        )),
        None => Err(PluginError::new(
            codes::GIT_FAILED,
            format!(
                "git {} was stopped",
                args.first().copied().unwrap_or_default()
            ),
        )),
    }
}

/// `git status --porcelain=v2 --branch -z`, with every untracked file
/// (`with_files`) or none.
fn read_status(
    ctx: &mut Context<'_>,
    repository: &Repository,
    with_files: bool,
) -> Result<Status, PluginError> {
    let untracked = if with_files {
        "--untracked-files=all"
    } else {
        "--untracked-files=no"
    };
    let output = git(
        ctx,
        repository,
        &["status", "--porcelain=v2", "--branch", "-z", untracked],
        None,
    )?;
    porcelain::parse(&output).map_err(|e| PluginError::new(codes::GIT_FAILED, e.to_string()))
}

/// The outputs every action has: where the repository is.
fn located(output: Output, repository: &Repository) -> Output {
    let output = output.with("repository", repository.shown.clone());
    match &repository.project_key {
        Some(key) => output.with("projectKey", key.clone()),
        None => output,
    }
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

// -------------------------------------------------------------- actions

fn status(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let project: Option<String> = args.get(PROJECT)?;
    let read = repository(ctx, args).and_then(|repository| {
        let status = read_status(ctx, &repository, true)?;
        Ok((repository, status))
    });
    state::remember_status(ctx, project.as_deref(), read.as_ref().map(|(_, s)| s));
    let (repository, status) = read?;
    Ok(located(describe_status(&status, true), &repository))
}

fn branch(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let project: Option<String> = args.get(PROJECT)?;
    let read = repository(ctx, args).and_then(|repository| {
        let status = read_status(ctx, &repository, false)?;
        Ok((repository, status))
    });
    state::remember_branch(ctx, project.as_deref(), read.as_ref().map(|(_, s)| s));
    let (repository, status) = read?;
    Ok(located(describe_status(&status, false), &repository))
}

/// The message and outputs of `status` (`with_files`) and `branch`.
pub fn describe_status(status: &Status, with_files: bool) -> Output {
    let branch = &status.branch;
    let mut message = describe_branch(branch);
    let mut output = branch_data(Output::ok(), branch);
    if with_files {
        let counts = status.counts();
        let _ = write!(
            message,
            "; {}",
            if status.clean() {
                "clean".to_owned()
            } else {
                format!("{} changed", status.changes.len())
            }
        );
        if let Some((ahead, behind)) = branch.ahead_behind {
            let _ = write!(message, "; {ahead} ahead, {behind} behind");
            output = output.with("ahead", ahead).with("behind", behind);
        }
        output = output
            .with("clean", status.clean())
            .with("staged", count(counts.staged))
            .with("unstaged", count(counts.unstaged))
            .with("untracked", count(counts.untracked))
            .with("conflicts", count(counts.conflicts));
    }
    with_message(output, message)
}

/// `output` with `message`, keeping its data.
fn with_message(output: Output, message: String) -> Output {
    let mut out = Output::message(message);
    for (key, value) in output.data() {
        out = out.with(key.clone(), value.clone());
    }
    out
}

fn changes(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let repository = repository(ctx, args)?;
    let limit: i64 = args.get("limit")?;
    let status = read_status(ctx, &repository, true)?;
    Ok(located(describe_changes(&status, limit), &repository))
}

/// The message and outputs of `changes`.
pub fn describe_changes(status: &Status, limit: i64) -> Output {
    let limit = usize::try_from(limit.clamp(1, MAX_LISTED)).unwrap_or(100);
    let counts = status.counts();
    let files: Vec<String> = status
        .changes
        .iter()
        .take(limit)
        .map(|change| match &change.from {
            Some(from) => format!("{} {from} -> {}", change.code, change.path),
            None => format!("{} {}", change.code, change.path),
        })
        .collect();
    let changed = status.changes.len();
    Output::message(if changed == 0 {
        "no changes".to_owned()
    } else {
        format!("{changed} changed paths")
    })
    .with("changed", count(changed))
    .with("staged", count(counts.staged))
    .with("unstaged", count(counts.unstaged))
    .with("untracked", count(counts.untracked))
    .with("conflicts", count(counts.conflicts))
    .with("files", files.join("\n"))
    .with("truncated", changed > limit)
}

fn last_commit(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let repository = repository(ctx, args)?;
    let status = read_status(ctx, &repository, false)?;
    if status.branch.unborn() {
        return Err(PluginError::new(
            codes::NO_COMMITS,
            format!(
                "{} has no commits yet",
                status.branch.name.as_deref().unwrap_or("the branch")
            ),
        ));
    }
    let output = git(
        ctx,
        &repository,
        &["log", "-1", "--no-show-signature", LOG_FORMAT],
        None,
    )?;
    Ok(located(parse_commit(&output)?, &repository))
}

/// The outputs of `last-commit` from `git log -1` in [`LOG_FORMAT`].
pub fn parse_commit(output: &str) -> ActionResult {
    let fields: Vec<&str> = output.trim_end_matches(['\n', '\r']).split('\0').collect();
    let [hash, short, author, date, subject] = fields.as_slice() else {
        return Err(PluginError::new(
            codes::GIT_FAILED,
            "unexpected output from git log",
        ));
    };
    Ok(Output::message(format!("{short} {subject}"))
        .with("hash", *hash)
        .with("shortHash", *short)
        .with("author", *author)
        .with("date", *date)
        .with("subject", *subject))
}

/// `git config --get <key>`; `None` when the key is not set (git exits
/// with 1).
fn config(
    ctx: &mut Context<'_>,
    repository: &Repository,
    key: &str,
) -> Result<Option<String>, PluginError> {
    match git(ctx, repository, &["config", "--get", key], None) {
        Ok(output) => Ok(Some(output.trim().to_owned())),
        Err(e) if e.code() == codes::GIT_FAILED => Ok(None),
        Err(e) => Err(e),
    }
}

fn remotes(ctx: &mut Context<'_>, repository: &Repository) -> Result<Vec<String>, PluginError> {
    Ok(git(ctx, repository, &["remote"], None)?
        .lines()
        .map(|line| line.trim().to_owned())
        .collect())
}

/// The remote `branch` is configured to track, when that remote does not
/// exist.
fn missing_remote(
    ctx: &mut Context<'_>,
    repository: &Repository,
    branch: &str,
) -> Result<Option<String>, PluginError> {
    let Some(remote) =
        config(ctx, repository, &format!("branch.{branch}.remote"))?.filter(|r| r != ".")
    else {
        return Ok(None);
    };
    let remotes = remotes(ctx, repository)?;
    Ok((!remotes.contains(&remote)).then_some(remote))
}

fn remote_status(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let repository = repository(ctx, args)?;
    let refresh: bool = args.get("refresh")?;
    let status = read_status(ctx, &repository, false)?;
    let branch = &status.branch;
    let (Some(name), Some(upstream)) = (&branch.name, &branch.upstream) else {
        let mut output = Output::ok()
            .with("hasUpstream", false)
            .with("source", "cached");
        let message = match &branch.name {
            None => "HEAD is detached, so there is no upstream".to_owned(),
            Some(name) => {
                output = output.with("branch", name.clone());
                // An upstream whose remote was removed is not reported by
                // `git status`; the branch configuration still names it.
                match missing_remote(ctx, &repository, name)? {
                    Some(remote) if refresh => {
                        return Err(PluginError::new(
                            codes::NO_REMOTE,
                            format!("the remote {remote} of {name} is not configured"),
                        ));
                    }
                    Some(remote) => {
                        format!("{name} tracks the remote {remote}, which is not configured")
                    }
                    None => format!("{name} has no upstream branch"),
                }
            }
        };
        return Ok(located(with_message(output, message), &repository));
    };
    let mut output = Output::ok()
        .with("hasUpstream", true)
        .with("source", if refresh { "network" } else { "cached" })
        .with("branch", name.clone())
        .with("upstream", upstream.clone());
    let mut message = format!("{name} → {upstream}");
    match branch.ahead_behind {
        Some((ahead, behind)) => {
            let _ = write!(message, ": {ahead} ahead, {behind} behind as last fetched");
            output = output.with("ahead", ahead).with("behind", behind);
        }
        None => message.push_str(": the upstream branch is gone"),
    }
    let remote = config(ctx, &repository, &format!("branch.{name}.remote"))?;
    if let Some(remote) = remote.as_deref().filter(|r| *r != ".") {
        output = output.with("remote", remote);
    }
    let mut tracking = None;
    if branch.ahead_behind.is_some() {
        let head = git(
            ctx,
            &repository,
            &["rev-parse", "--verify", "-q", upstream],
            None,
        )?;
        let head = head.trim().to_owned();
        output = output.with("trackingHead", head.clone());
        tracking = Some(head);
    }
    if refresh {
        let remote = remote.filter(|r| r != ".").ok_or_else(|| {
            PluginError::new(
                codes::NO_REMOTE,
                format!("{upstream} does not belong to a remote"),
            )
        })?;
        let (remote_head, merge) = ask_remote(ctx, &repository, name, &remote)?;
        match remote_head {
            Some(head) => {
                let stale = tracking.as_deref() != Some(head.as_str());
                message.push_str(if stale {
                    "; the remote has moved since"
                } else {
                    "; the remote has not moved since"
                });
                output = output.with("remoteHead", head).with("stale", stale);
            }
            None => {
                let _ = write!(message, "; {merge} does not exist on the remote");
            }
        }
    }
    Ok(located(with_message(output, message), &repository))
}

/// The commit `remote` has for the branch `name` merges from, by `git
/// ls-remote` (`tools:git:network`), which changes nothing locally; and
/// that ref.
fn ask_remote(
    ctx: &mut Context<'_>,
    repository: &Repository,
    name: &str,
    remote: &str,
) -> Result<(Option<String>, String), PluginError> {
    if !remotes(ctx, repository)?.iter().any(|r| r == remote) {
        return Err(PluginError::new(
            codes::NO_REMOTE,
            format!("the remote {remote} is not configured"),
        ));
    }
    let merge = config(ctx, repository, &format!("branch.{name}.merge"))?
        .unwrap_or_else(|| format!("refs/heads/{name}"));
    let seconds = ctx
        .settings()
        .ok()
        .and_then(|s| s.integer(NETWORK_TIMEOUT))
        .unwrap_or(20)
        .clamp(5, 120);
    let timeout = u32::try_from(seconds * 1000).unwrap_or(20_000);
    let output = ctx
        .run_tool(
            Tool::Git,
            &["ls-remote", "--", remote, &merge],
            &repository.handle,
            Some(timeout),
        )
        .map_err(|e| match reason(&e) {
            Some("tool-unknown-remote") => PluginError::new(
                codes::NO_REMOTE,
                format!("the remote {remote} is not configured"),
            ),
            _ => tool_failure(&e, &repository.shown),
        })?;
    if output.exit != Some(0) {
        let failure = git_failure(&repository.shown, "ls-remote", &output.stderr);
        return Err(if failure.code() == codes::GIT_FAILED {
            PluginError::new(codes::NETWORK_FAILED, failure.message())
        } else {
            failure
        });
    }
    Ok((parse_ls_remote(&output.stdout, &merge), merge))
}

/// The commit of `merge` in `git ls-remote` output.
pub fn parse_ls_remote(stdout: &str, merge: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.split_once('\t').filter(|(_, r)| *r == merge))
        .map(|(hash, _)| hash.to_owned())
}

fn open_repository(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let repository = repository(ctx, args)?;
    let with = match ctx.resource_param("application", &[ResourceKind::Application]) {
        Ok(handle) => Some(handle.handle),
        Err(e) if reason(&e) == Some("resource-not-set") => None,
        Err(e) => return Err(resolve_failure(&e, false, "application")),
    };
    ctx.open_resource(ResourceOpen {
        handle: repository.handle.clone(),
        path: None,
        with,
        position: None,
        editor: None,
    })
    .map_err(|e| match reason(&e) {
        Some("not-permitted") | None => PluginError::from(e.clone()),
        Some(code) => PluginError::new(code, reason_message(&e)),
    })?;
    Ok(located(
        Output::message(format!("opened {}", repository.shown)),
        &repository,
    ))
}

// ------------------------------------------------------------- messages

pub fn describe_branch(branch: &Branch) -> String {
    match (&branch.name, &branch.head) {
        (Some(name), None) => format!("{name} (no commits yet)"),
        (Some(name), Some(_)) => name.clone(),
        (None, Some(head)) => format!("detached at {}", &head[..head.len().min(12)]),
        (None, None) => "no branch".to_owned(),
    }
}

fn branch_data(mut output: Output, branch: &Branch) -> Output {
    if let Some(name) = &branch.name {
        output = output.with("branch", name.clone());
    }
    if let Some(head) = &branch.head {
        output = output.with("head", head.clone());
    }
    if let Some(upstream) = &branch.upstream {
        output = output.with("upstream", upstream.clone());
    }
    output
        .with("detached", branch.detached())
        .with("unborn", branch.unborn())
}

#[cfg(test)]
mod tests;
