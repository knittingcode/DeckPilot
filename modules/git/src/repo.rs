//! Which repository an action works on, and running Git in it safely.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use deckpilot_core::process::{ProcessLimits, ProcessOutput, minimal_environment};
use deckpilot_core::{ActionParameters, ActionResult, ExecutionContext, Project, ResourceKind};

use crate::codes;

pub const REPOSITORY: &str = "repository";
pub const PROJECT: &str = "project_id";

/// A failure, as the action's result.
pub type Failure = ActionResult;

pub fn fail(code: &str, message: impl Into<String>) -> Failure {
    ActionResult::error(code, message)
}

/// The repository an action works on, and the project it belongs to.
#[derive(Debug)]
pub struct Repository<'a> {
    pub path: PathBuf,
    pub project: Option<&'a Project>,
}

/// Settings that apply to every Git action.
#[derive(Debug, Clone)]
pub struct GitSettings {
    /// The `git` program: a path, or a name found on `PATH`.
    pub executable: String,
    /// The project resource used when an action names no repository.
    pub role: String,
    /// Limit for operations that use the network.
    pub network_timeout: Duration,
}

/// Resolves the repository:
///
/// 1. the `repository` parameter (a folder resource, fixed or a role of
///    the run's project),
/// 2. otherwise the `project_id` parameter's project — which must belong to
///    the action's profile — and its repository resource,
/// 3. otherwise the run's own project (the execution-local project
///    context, not necessarily DeckPilot's active project).
///
/// The global active project is never changed.
pub fn resolve<'a>(
    parameters: &ActionParameters,
    context: &ExecutionContext<'a>,
    settings: &GitSettings,
) -> Result<Repository<'a>, Failure> {
    let resolve_error = |e: deckpilot_core::ResolveError| fail(e.code(), e.to_string());
    if let Some(reference) = parameters.resource(REPOSITORY) {
        let binding = context
            .resolve(reference, Some(ResourceKind::Directory))
            .map_err(resolve_error)?;
        let project = context.project().ok().flatten();
        let project = matches!(reference, deckpilot_core::ResourceRef::ProjectRole { .. })
            .then_some(project)
            .flatten();
        return existing(PathBuf::from(&binding.value)).map(|path| Repository { path, project });
    }
    let project = match parameters.project(PROJECT) {
        Some(id) => context
            .profile()
            .map_err(ActionResult::from)?
            .project(id)
            .ok_or_else(|| {
                fail(
                    codes::PROJECT_NOT_IN_PROFILE,
                    "the project does not belong to this profile",
                )
            })?,
        None => context
            .project()
            .map_err(ActionResult::from)?
            .ok_or_else(|| {
                fail(
                    "no-active-project",
                    "no project is active and no repository is set",
                )
            })?,
    };
    let binding = context
        .resolve_project_resource(project, &settings.role)
        .map_err(resolve_error)?;
    existing(PathBuf::from(&binding.value)).map(|path| Repository {
        path,
        project: Some(project),
    })
}

fn existing(path: PathBuf) -> Result<PathBuf, Failure> {
    if path.is_dir() {
        Ok(path)
    } else {
        Err(fail(
            codes::NOT_FOUND,
            format!("the repository folder {} does not exist", path.display()),
        ))
    }
}

/// Environment variables Git may need besides the minimal environment:
/// the SSH agent for `ssh` remotes and Git's own location settings.
const GIT_ENVIRONMENT: &[&str] = &["SSH_AUTH_SOCK", "GIT_EXEC_PATH", "XDG_CONFIG_HOME"];

/// Runs `git -C <repository> <args>` without a shell, without prompts and
/// without taking optional locks, so reading never writes to the
/// repository. A repository's `core.fsmonitor` (a program Git would run)
/// is switched off.
pub fn git(
    context: &ExecutionContext<'_>,
    settings: &GitSettings,
    repository: &std::path::Path,
    args: &[&str],
    timeout: Duration,
) -> Result<ProcessOutput, Failure> {
    let mut command = Command::new(&settings.executable);
    command
        .arg("-C")
        .arg(repository)
        .args([
            "--no-pager",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args);
    minimal_environment(&mut command, GIT_ENVIRONMENT);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    let output = context
        .run_process(command, timeout, ProcessLimits::default())
        .map_err(|error| match error.code() {
            "program-not-found" => fail(
                codes::GIT_NOT_FOUND,
                format!(
                    "Git ({}) was not found; install it or set its location in Settings › Git",
                    settings.executable
                ),
            ),
            code => fail(code, format!("git: {error}")),
        })?;
    if output.success() {
        return Ok(output);
    }
    let stderr = output.stderr.trim();
    let first = stderr.lines().next().unwrap_or_default();
    Err(if stderr.contains("not a git repository") {
        fail(
            codes::NOT_A_REPOSITORY,
            format!("{} is not a Git repository", repository.display()),
        )
    } else if stderr.contains("dubious ownership") {
        fail(
            codes::UNSAFE_REPOSITORY,
            format!(
                "Git refuses {} because another user owns it (safe.directory)",
                repository.display()
            ),
        )
    } else {
        fail(
            codes::GIT_FAILED,
            format!(
                "git {} failed: {}",
                args.first().copied().unwrap_or_default(),
                redact(first)
            ),
        )
    })
}

/// Removes credentials from URLs in a message, e.g.
/// `https://user:token@host/repo` becomes `https://***@host/repo`.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        let (head, tail) = rest.split_at(index + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '/' || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&authority[at..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_in_urls_are_redacted() {
        assert_eq!(
            redact("fatal: unable to access 'https://bob:ghp_secret@github.com/a/b.git/': 403"),
            "fatal: unable to access 'https://***@github.com/a/b.git/': 403"
        );
        assert_eq!(redact("ssh://git@host/x"), "ssh://***@host/x");
        assert_eq!(redact("no url here"), "no url here");
        assert_eq!(redact("https://host/x"), "https://host/x");
    }
}
