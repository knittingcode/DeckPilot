//! The action types of the `git` module.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::launch::{self, Launcher, OpenTarget};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, ButtonState,
    Capability, ExecutionContext, Os, ParameterKind, ParameterSpec, ParameterValue, Profile,
    RefreshPolicy, ResourceKind, StateError,
};

use crate::codes;
use crate::porcelain::{self, Status};
use crate::repo::{self, Failure, GitSettings, Repository, fail};

pub const STATUS: &str = "git.status";
pub const BRANCH: &str = "git.branch";
pub const CHANGES: &str = "git.changes";
pub const LAST_COMMIT: &str = "git.last-commit";
pub const REMOTE_STATUS: &str = "git.remote-status";
pub const OPEN_REPOSITORY: &str = "git.open-repository";

/// Time limit of local Git commands.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Most changed files listed by `git.changes`.
pub const MAX_LISTED: i64 = 500;

/// # Panics
///
/// Never for the constants of this module.
pub fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("the action type id is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Status,
    Branch,
    Changes,
    LastCommit,
    RemoteStatus,
    Open,
}

pub fn handlers(
    settings: &Arc<GitSettings>,
    launcher: &Arc<dyn Launcher>,
) -> Vec<Box<dyn ActionHandler>> {
    [
        Kind::Status,
        Kind::Branch,
        Kind::Changes,
        Kind::LastCommit,
        Kind::RemoteStatus,
        Kind::Open,
    ]
    .into_iter()
    .map(|kind| {
        Box::new(GitAction {
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

/// Where the repository comes from; see [`repo::resolve`].
fn repository_parameters() -> Vec<ParameterSpec> {
    vec![
        parameter(
            repo::REPOSITORY,
            "Repository",
            ParameterKind::Resource {
                kind: Some(ResourceKind::Directory),
            },
            "A folder resource. Empty: the project's repository resource.",
        ),
        parameter(
            repo::PROJECT,
            "Project",
            ParameterKind::Project,
            "Whose repository, when no repository is set. Empty: the workflow's current \
             project. Choosing one never changes the active project.",
        ),
    ]
}

fn out(key: &str, label: &str, kind: ValueKind) -> OutputSpec {
    OutputSpec::new(key, label, kind)
}

fn location_outputs() -> Vec<OutputSpec> {
    vec![
        out("repository", "Repository folder", ValueKind::Path),
        out("projectKey", "Project", ValueKind::ProjectKey).optional(),
    ]
}

fn branch_outputs() -> Vec<OutputSpec> {
    vec![
        out("branch", "Branch", ValueKind::Text).optional(),
        out("head", "Commit", ValueKind::Text).optional(),
        out("detached", "Detached HEAD", ValueKind::Boolean),
        out("unborn", "No commits yet", ValueKind::Boolean),
        out("upstream", "Upstream", ValueKind::Text).optional(),
    ]
}

fn count_outputs() -> Vec<OutputSpec> {
    vec![
        out("staged", "Staged", ValueKind::Integer),
        out("unstaged", "Not staged", ValueKind::Integer),
        out("untracked", "Untracked", ValueKind::Integer),
        out("conflicts", "Conflicts", ValueKind::Integer),
    ]
}

fn node(icon: &str, outputs: Vec<OutputSpec>, timeout_ms: u32) -> NodeSpec {
    let mut all = outputs;
    all.extend(location_outputs());
    NodeSpec {
        icon: Some(icon.into()),
        outputs: all,
        timeout_ms,
        retry_safe: true,
        idempotent: true,
        cancellable: true,
        effect: ActionEffect::ReadOnly,
        ..NodeSpec::default()
    }
}

#[allow(clippy::too_many_lines)] // One table of the six action types.
fn definition(kind: Kind) -> ActionType {
    let mut parameters = repository_parameters();
    let (id_value, name, description, spec) = match kind {
        Kind::Status => {
            let mut outputs = branch_outputs();
            outputs.push(out("clean", "Clean", ValueKind::Boolean));
            outputs.extend(count_outputs());
            outputs.push(out("ahead", "Ahead", ValueKind::Integer).optional());
            outputs.push(out("behind", "Behind", ValueKind::Integer).optional());
            (
                STATUS,
                "Repository status",
                "Branch, whether the work tree is clean, counts of changes and how far the \
                 branch is ahead of or behind its upstream as last fetched. Local only.",
                node("git-branch", outputs, 30_000),
            )
        }
        Kind::Branch => (
            BRANCH,
            "Current branch",
            "The checked-out branch and commit; reports a detached HEAD and a branch without \
             commits. Local only.",
            node("git-branch", branch_outputs(), 30_000),
        ),
        Kind::Changes => {
            let mut limit = parameter(
                "limit",
                "Files listed",
                ParameterKind::Integer {
                    min: Some(1),
                    max: Some(MAX_LISTED),
                },
                "At most this many paths in the list.",
            );
            limit.required = true;
            limit.default = Some(ParameterValue::Integer(100));
            parameters.push(limit);
            let mut outputs = vec![out("changed", "Changed paths", ValueKind::Integer)];
            outputs.extend(count_outputs());
            outputs.push(out("files", "Files", ValueKind::Text));
            outputs.push(out("truncated", "List truncated", ValueKind::Boolean));
            (
                CHANGES,
                "Changed files",
                "The paths with changes, one per line with their status (as `git status \
                 --short`), and counts. Local only.",
                node("list-checks", outputs, 30_000),
            )
        }
        Kind::LastCommit => (
            LAST_COMMIT,
            "Last commit",
            "The commit HEAD points to: hash, author name, date and subject. Local only.",
            node(
                "git-commit",
                vec![
                    out("hash", "Hash", ValueKind::Text),
                    out("shortHash", "Short hash", ValueKind::Text),
                    out("author", "Author", ValueKind::Text),
                    out("date", "Date", ValueKind::Text),
                    out("subject", "Subject", ValueKind::Text),
                ],
                30_000,
            ),
        ),
        Kind::RemoteStatus => {
            let mut refresh = parameter(
                "refresh",
                "Ask the remote",
                ParameterKind::Boolean,
                "Off: what was last fetched, without the network. On: also asks the remote \
                 for its branch (git ls-remote), which changes nothing locally.",
            );
            refresh.required = true;
            refresh.default = Some(ParameterValue::Boolean(false));
            parameters.push(refresh);
            (
                REMOTE_STATUS,
                "Remote status",
                "How the branch compares with its upstream: ahead and behind as last \
                 fetched, and on request whether the remote has moved since.",
                node(
                    "cloud",
                    vec![
                        out("hasUpstream", "Has an upstream", ValueKind::Boolean),
                        out("source", "Source (cached or network)", ValueKind::Text),
                        out("branch", "Branch", ValueKind::Text).optional(),
                        out("upstream", "Upstream", ValueKind::Text).optional(),
                        out("remote", "Remote", ValueKind::Text).optional(),
                        out("ahead", "Ahead", ValueKind::Integer).optional(),
                        out("behind", "Behind", ValueKind::Integer).optional(),
                        out("trackingHead", "Last fetched commit", ValueKind::Text).optional(),
                        out("remoteHead", "Remote commit", ValueKind::Text).optional(),
                        out(
                            "stale",
                            "Remote moved since the last fetch",
                            ValueKind::Boolean,
                        )
                        .optional(),
                    ],
                    // The network limit is at most 120 s.
                    150_000,
                ),
            )
        }
        Kind::Open => {
            parameters.push(parameter(
                "application",
                "Open with",
                ParameterKind::Resource {
                    kind: Some(ResourceKind::Application),
                },
                "An application resource such as an editor. Empty: the file manager.",
            ));
            let mut spec = node("folder-open", Vec::new(), 15_000);
            spec.idempotent = false;
            spec.cancellable = false;
            spec.effect = ActionEffect::Opens;
            (
                OPEN_REPOSITORY,
                "Open repository",
                "Opens the repository folder in the file manager or an application.",
                spec,
            )
        }
    };
    ActionType {
        id: id(id_value),
        name: name.into(),
        description: description.into(),
        category: "Git".into(),
        parameters,
        requires_project: false,
        node: spec,
    }
}

#[derive(Debug)]
struct GitAction {
    definition: ActionType,
    kind: Kind,
    settings: Arc<GitSettings>,
    launcher: Arc<dyn Launcher>,
}

impl ActionHandler for GitAction {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        self.run(parameters, context)
            .unwrap_or_else(|failure| failure)
    }

    fn check(&self, parameters: &ActionParameters, profile: &Profile) -> Vec<String> {
        match parameters.project(repo::PROJECT) {
            Some(id) if profile.project(id).is_none() => {
                vec!["the project does not belong to this profile".into()]
            }
            _ => Vec::new(),
        }
    }

    /// Status and branch keys show the branch (and for status whether the
    /// work tree is clean), read at most every
    /// [`STATE_INTERVAL`](crate::state::STATE_INTERVAL).
    fn state_policy(&self, _parameters: &ActionParameters) -> Option<RefreshPolicy> {
        matches!(self.kind, Kind::Status | Kind::Branch)
            .then(|| RefreshPolicy::every(crate::state::STATE_INTERVAL))
    }

    fn button_state(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ButtonState, StateError> {
        let repository = repo::resolve(parameters, context, &self.settings)?;
        let changes = self.kind == Kind::Status;
        // Untracked files count as changes, but their folders are not
        // listed file by file.
        let untracked = if changes {
            "--untracked-files=normal"
        } else {
            "--untracked-files=no"
        };
        let output = self.git(
            context,
            &repository,
            &["status", "--porcelain=v2", "--branch", "-z", untracked],
        )?;
        let status = porcelain::parse(&output)
            .map_err(|e| StateError::new(codes::GIT_FAILED, e.to_string()))?;
        Ok(crate::state::state_of(&status, changes))
    }
}

impl GitAction {
    fn run(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ActionResult, Failure> {
        let repository = repo::resolve(parameters, context, &self.settings)?;
        let result = match self.kind {
            Kind::Status => self.status(context, &repository, true)?,
            Kind::Branch => self.status(context, &repository, false)?,
            Kind::Changes => self.changes(parameters, context, &repository)?,
            Kind::LastCommit => self.last_commit(context, &repository)?,
            Kind::RemoteStatus => self.remote_status(parameters, context, &repository)?,
            Kind::Open => self.open(parameters, context, &repository)?,
        };
        let mut result = result.with_data("repository", repository.path.display().to_string());
        if let Some(project) = repository.project {
            result = result.with_data("projectKey", project.key.as_str());
        }
        Ok(result)
    }

    fn git(
        &self,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
        args: &[&str],
    ) -> Result<String, Failure> {
        repo::git(
            context,
            &self.settings,
            &repository.path,
            args,
            LOCAL_TIMEOUT,
        )
        .map(|output| output.stdout)
    }

    fn read_status(
        &self,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
        with_files: bool,
    ) -> Result<Status, Failure> {
        let untracked = if with_files {
            "--untracked-files=all"
        } else {
            "--untracked-files=no"
        };
        let output = self.git(
            context,
            repository,
            &["status", "--porcelain=v2", "--branch", "-z", untracked],
        )?;
        porcelain::parse(&output).map_err(|e| fail(codes::GIT_FAILED, e.to_string()))
    }

    fn status(
        &self,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
        with_files: bool,
    ) -> Result<ActionResult, Failure> {
        let status = self.read_status(context, repository, with_files)?;
        let branch = &status.branch;
        let mut message = describe_branch(branch);
        let mut result = branch_data(ActionResult::success(String::new()), branch);
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
                result = result
                    .with_data("ahead", ahead.to_string())
                    .with_data("behind", behind.to_string());
            }
            result = result
                .with_data("clean", status.clean().to_string())
                .with_data("staged", counts.staged.to_string())
                .with_data("unstaged", counts.unstaged.to_string())
                .with_data("untracked", counts.untracked.to_string())
                .with_data("conflicts", counts.conflicts.to_string());
        }
        result.message = message;
        Ok(result)
    }

    fn changes(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
    ) -> Result<ActionResult, Failure> {
        let status = self.read_status(context, repository, true)?;
        let limit = usize::try_from(
            parameters
                .integer("limit")
                .unwrap_or(100)
                .clamp(1, MAX_LISTED),
        )
        .unwrap_or(100);
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
        Ok(ActionResult::success(if changed == 0 {
            "no changes".to_owned()
        } else {
            format!("{changed} changed paths")
        })
        .with_data("changed", changed.to_string())
        .with_data("staged", counts.staged.to_string())
        .with_data("unstaged", counts.unstaged.to_string())
        .with_data("untracked", counts.untracked.to_string())
        .with_data("conflicts", counts.conflicts.to_string())
        .with_data("files", files.join("\n"))
        .with_data("truncated", (changed > limit).to_string()))
    }

    fn last_commit(
        &self,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
    ) -> Result<ActionResult, Failure> {
        let status = self.read_status(context, repository, false)?;
        if status.branch.unborn() {
            return Err(fail(
                codes::NO_COMMITS,
                format!(
                    "{} has no commits yet",
                    status.branch.name.as_deref().unwrap_or("the branch")
                ),
            ));
        }
        let output = self.git(
            context,
            repository,
            &[
                "log",
                "-1",
                "--no-show-signature",
                "--format=%H%x00%h%x00%an%x00%aI%x00%s",
            ],
        )?;
        let fields: Vec<&str> = output.trim_end_matches(['\n', '\r']).split('\0').collect();
        let [hash, short, author, date, subject] = fields.as_slice() else {
            return Err(fail(codes::GIT_FAILED, "unexpected output from git log"));
        };
        Ok(ActionResult::success(format!("{short} {subject}"))
            .with_data("hash", *hash)
            .with_data("shortHash", *short)
            .with_data("author", *author)
            .with_data("date", *date)
            .with_data("subject", *subject))
    }

    fn config(
        &self,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
        key: &str,
    ) -> Result<Option<String>, Failure> {
        // `git config --get` exits with 1 when the key is not set.
        match repo::git(
            context,
            &self.settings,
            &repository.path,
            &["config", "--get", key],
            LOCAL_TIMEOUT,
        ) {
            Ok(output) => Ok(Some(output.stdout.trim().to_owned())),
            Err(failure) if failure.code.as_deref() == Some(codes::GIT_FAILED) => Ok(None),
            Err(failure) => Err(failure),
        }
    }

    /// The remote `branch` is configured to track, when that remote does
    /// not exist.
    fn missing_remote(
        &self,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
        branch: &str,
    ) -> Result<Option<String>, Failure> {
        let Some(remote) = self
            .config(context, repository, &format!("branch.{branch}.remote"))?
            .filter(|r| r != ".")
        else {
            return Ok(None);
        };
        let remotes = self.git(context, repository, &["remote"])?;
        Ok((!remotes.lines().any(|line| line.trim() == remote)).then_some(remote))
    }

    fn remote_status(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
    ) -> Result<ActionResult, Failure> {
        let refresh = parameters.boolean("refresh").unwrap_or(false);
        let status = self.read_status(context, repository, false)?;
        let branch = &status.branch;
        let source = if refresh { "network" } else { "cached" };
        let (Some(name), Some(upstream)) = (&branch.name, &branch.upstream) else {
            let mut result = ActionResult::success(String::new())
                .with_data("hasUpstream", "false")
                .with_data("source", "cached");
            result.message = match &branch.name {
                None => "HEAD is detached, so there is no upstream".to_owned(),
                Some(name) => {
                    result = result.with_data("branch", name.clone());
                    // An upstream whose remote was removed is not reported
                    // by `git status`; the branch configuration still
                    // names the remote.
                    match self.missing_remote(context, repository, name)? {
                        Some(remote) if refresh => {
                            return Err(fail(
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
            return Ok(result);
        };
        let mut result = ActionResult::success(String::new())
            .with_data("hasUpstream", "true")
            .with_data("source", source)
            .with_data("branch", name.clone())
            .with_data("upstream", upstream.clone());
        let mut message = format!("{name} → {upstream}");
        match branch.ahead_behind {
            Some((ahead, behind)) => {
                let _ = write!(message, ": {ahead} ahead, {behind} behind as last fetched");
                result = result
                    .with_data("ahead", ahead.to_string())
                    .with_data("behind", behind.to_string());
            }
            None => message.push_str(": the upstream branch is gone"),
        }
        let remote = self.config(context, repository, &format!("branch.{name}.remote"))?;
        if let Some(remote) = remote.as_deref().filter(|r| *r != ".") {
            result = result.with_data("remote", remote);
        }
        if branch.ahead_behind.is_some() {
            let tracking = self.git(
                context,
                repository,
                &["rev-parse", "--verify", "-q", upstream],
            )?;
            result = result.with_data("trackingHead", tracking.trim());
        }
        if refresh {
            let remote = remote.filter(|r| r != ".").ok_or_else(|| {
                fail(
                    codes::NO_REMOTE,
                    format!("{upstream} does not belong to a remote"),
                )
            })?;
            let (remote_head, merge) = self.ask_remote(context, repository, name, &remote)?;
            match remote_head {
                Some(head) => {
                    let stale = result.data.get("trackingHead") != Some(&head);
                    message.push_str(if stale {
                        "; the remote has moved since"
                    } else {
                        "; the remote has not moved since"
                    });
                    result = result
                        .with_data("remoteHead", head)
                        .with_data("stale", stale.to_string());
                }
                None => {
                    let _ = write!(message, "; {merge} does not exist on the remote");
                }
            }
        }
        result.message = message;
        Ok(result)
    }

    /// The commit `remote` has for the branch `name` merges from, by
    /// `git ls-remote`, which changes nothing locally; and that ref.
    fn ask_remote(
        &self,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
        name: &str,
        remote: &str,
    ) -> Result<(Option<String>, String), Failure> {
        let remotes = self.git(context, repository, &["remote"])?;
        if !remotes.lines().any(|line| line.trim() == remote) {
            return Err(fail(
                codes::NO_REMOTE,
                format!("the remote {remote} is not configured"),
            ));
        }
        let merge = self
            .config(context, repository, &format!("branch.{name}.merge"))?
            .unwrap_or_else(|| format!("refs/heads/{name}"));
        context.report_progress(None, format!("asking {remote}"));
        let output = repo::git(
            context,
            &self.settings,
            &repository.path,
            &["ls-remote", "--", remote, &merge],
            self.settings.network_timeout,
        )
        .map_err(|failure| match failure.code.as_deref() {
            Some(codes::GIT_FAILED) => fail(codes::NETWORK_FAILED, failure.message),
            _ => failure,
        })?;
        let head = output
            .stdout
            .lines()
            .find_map(|line| line.split_once('\t').filter(|(_, r)| *r == merge))
            .map(|(hash, _)| hash.to_owned());
        Ok((head, merge))
    }

    fn open(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
        repository: &Repository<'_>,
    ) -> Result<ActionResult, Failure> {
        context
            .permit(Capability::OpenExternal)
            .map_err(ActionResult::from)?;
        let target = match parameters.resource("application") {
            Some(reference) => OpenTarget::PathWith {
                path: repository.path.clone(),
                application: context
                    .resolve(reference, Some(ResourceKind::Application))
                    .map_err(|e| fail(e.code(), e.to_string()))?
                    .value
                    .clone(),
            },
            None => OpenTarget::Path(repository.path.clone()),
        };
        let plan =
            launch::plan(Os::current(), &target).map_err(|e| fail(e.code(), e.to_string()))?;
        self.launcher
            .launch(&plan)
            .map_err(|e| fail(e.code(), e.to_string()))?;
        Ok(ActionResult::success(format!(
            "opened {}",
            repository.path.display()
        )))
    }
}

fn describe_branch(branch: &porcelain::Branch) -> String {
    match (&branch.name, &branch.head) {
        (Some(name), None) => format!("{name} (no commits yet)"),
        (Some(name), Some(_)) => name.clone(),
        (None, Some(head)) => format!("detached at {}", &head[..head.len().min(12)]),
        (None, None) => "no branch".to_owned(),
    }
}

fn branch_data(mut result: ActionResult, branch: &porcelain::Branch) -> ActionResult {
    if let Some(name) = &branch.name {
        result = result.with_data("branch", name.clone());
    }
    if let Some(head) = &branch.head {
        result = result.with_data("head", head.clone());
    }
    if let Some(upstream) = &branch.upstream {
        result = result.with_data("upstream", upstream.clone());
    }
    result
        .with_data("detached", branch.detached().to_string())
        .with_data("unborn", branch.unborn().to_string())
}

#[cfg(test)]
mod tests;
