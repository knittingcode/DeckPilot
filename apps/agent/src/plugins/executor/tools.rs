//! `tool.run` (plugin API 1.9, X3 in `docs/marketplace/module-migration.md`
//! §7): the user's own `git` or `docker`, with the host's policy, fixed
//! prefix, environment and limits (`crate::plugins::tools`), in the
//! directory of a resource handle of this invocation
//! ([`Executor::bound_inside`]).
//!
//! The broker allowed the call for its tool and sub-grant; here the agent
//! checks again, before anything runs: the plugin's active manifest must
//! declare the subcommand and each option, the host's table must allow
//! them, the broker's classification must match the policy's, and a
//! modifying run must be one the user (or their work session) started.
//! While the process runs it is stopped when the invocation is aborted,
//! past its deadline, or when the grant it used is revoked. Every call is
//! audited: the tool, the subcommand, the option names (never values or
//! positionals), the handle's label, the exit status and byte counts, or
//! the reason it was refused.

use std::time::Instant;

use deckpilot_plugin_abi::capability::{RunCause, RunTrigger};
use deckpilot_plugin_abi::privileged::{ResourceKind, Tool, ToolAccess, ToolOutput, ToolRun};
use deckpilot_plugin_abi::{CallErrorCode, CallResult};
use deckpilot_plugin_broker::PluginId;
use deckpilot_plugin_manifest::tools::ToolGrant;
use deckpilot_plugin_manifest::{Permission, ToolDeclaration};
use deckpilot_plugin_supervisor::CallInterrupt;

use super::{Executor, PrivilegedRecord};
use crate::plugins::tools::{
    self as host, Checked, Denial, Plan, RunSlot, Stop, Target, TargetKind, ToolHost, codes,
};

/// The grant key of the audit.
fn grant_key(tool: Tool, access: ToolAccess) -> &'static str {
    match (tool, access) {
        (Tool::Git, ToolAccess::Network) => "tools:git:network",
        (Tool::Git, _) => "tools:git",
        (Tool::Docker, ToolAccess::Modify) => "tools:docker:modify",
        (Tool::Docker, _) => "tools:docker",
    }
}

fn access_of(grant: ToolGrant) -> ToolAccess {
    match grant {
        ToolGrant::Read => ToolAccess::Read,
        ToolGrant::Network => ToolAccess::Network,
        ToolGrant::Modify => ToolAccess::Modify,
    }
}

/// A refusal as the guest receives it: `<reason code>: <message>`.
fn refusal(denial: &Denial) -> CallResult {
    let call = match denial.code {
        codes::NOT_FOUND | codes::UNAVAILABLE => CallErrorCode::Unavailable,
        codes::TIMED_OUT
        | codes::CANCELLED
        | codes::SPAWN_FAILED
        | codes::INVALID_EXECUTABLE
        | codes::UNKNOWN_REMOTE
        | codes::NOT_A_REPOSITORY => CallErrorCode::Failed,
        codes::INVALID_ARGUMENT => CallErrorCode::BadRequest,
        _ => CallErrorCode::NotPermitted,
    };
    CallResult::error(call, format!("{}: {}", denial.code, denial.message))
}

/// What the audit record says besides the outcome.
#[derive(Default)]
struct Notes {
    details: Vec<(&'static str, String)>,
}

impl Notes {
    fn add(&mut self, name: &'static str, value: impl Into<String>) {
        self.details.push((name, value.into()));
    }
}

impl Executor {
    /// The host-held `git` and `docker` settings (tests; the agent's
    /// default is [`ToolHost::from_agent`]).
    #[cfg(test)]
    pub fn with_tools(mut self, tools: ToolHost) -> Self {
        self.tools = tools;
        self
    }

    /// `tool.run`, which the broker allowed for its tool and `access`.
    pub(super) fn run_tool(
        &self,
        plugin: &PluginId,
        request: &ToolRun,
        access: ToolAccess,
        interrupt: Option<&CallInterrupt>,
    ) -> CallResult {
        let started = Instant::now();
        let mut notes = Notes::default();
        notes.add("tool", request.tool.as_str());
        let outcome = self.tool_run(plugin, request, access, interrupt, &mut notes);
        self.audit_privileged(&PrivilegedRecord {
            plugin: plugin.to_string(),
            op: "tool.run",
            grant: grant_key(request.tool, access),
            outcome: outcome.as_ref().err().map(|d| d.code),
            elapsed: started.elapsed(),
            details: notes.details,
        });
        match outcome {
            Ok(output) => serde_json::to_value(output).map_or_else(
                |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
                CallResult::Ok,
            ),
            Err(denial) => refusal(&denial),
        }
    }

    /// The plugin's active manifest's declaration of `tool`.
    fn declaration(&self, tool: Tool) -> Result<Option<ToolDeclaration>, Denial> {
        let access = self.access.as_ref().ok_or_else(|| {
            Denial::new(codes::UNAVAILABLE, "the plugin's manifest is not available")
        })?;
        let executable = host::executable_of(tool);
        Ok(access
            .manifest
            .permissions
            .iter()
            .filter_map(|p| match p {
                Permission::Tools { tools, .. } => Some(tools),
                _ => None,
            })
            .flatten()
            .find(|t| t.executable == executable)
            .cloned())
    }

    fn tool_run(
        &self,
        plugin: &PluginId,
        request: &ToolRun,
        access: ToolAccess,
        interrupt: Option<&CallInterrupt>,
        notes: &mut Notes,
    ) -> Result<ToolOutput, Denial> {
        let declared = self.declaration(request.tool)?;
        let checked = host::check(request, declared.as_ref())?;
        notes.add("subcommand", checked.spec.subcommand);
        notes.add("options", checked.option_names().join(","));
        if access_of(checked.grant) != access {
            return Err(Denial::new(
                codes::GRANT_MISMATCH,
                "the broker's classification of the run and the host's policy disagree",
            ));
        }
        if checked.grant == ToolGrant::Modify && !self.may_modify() {
            return Err(Denial::new(
                codes::NOT_USER_INITIATED,
                "containers are started and stopped only in runs you or your work session started",
            ));
        }
        let (target, label) = self.target(plugin, &request.cwd, &checked)?;
        notes.add("label", label);
        let plan = host::plan(&self.tools, &checked, &target, request.timeout_ms)?;
        let deadline = interrupt.map(CallInterrupt::deadline);
        let broker = &self.broker;
        let stop = || must_stop(broker, plugin, request.tool, access, interrupt);
        let give_up = || stop().is_some() || deadline.is_some_and(|d| Instant::now() >= d);
        let _slot = RunSlot::acquire(
            plugin.as_str(),
            checked.grant == ToolGrant::Modify,
            &give_up,
        )
        .ok_or_else(|| match stop() {
            Some(Stop::Revoked) => Denial::new(
                codes::REVOKED,
                "the grant was revoked while the run waited for another one",
            ),
            _ => Denial::new(
                codes::CANCELLED,
                "the invocation ended while waiting for another run",
            ),
        })?;
        if request.tool == Tool::Git {
            work_tree_inside(&self.tools, &plan, deadline, &stop)?;
        }
        if let Some(remote) = &plan.remote {
            configured_remote(&self.tools, &plan, remote, deadline, &stop)?;
        }
        let finished = host::execute(&plan, deadline, &stop)?;
        let output = finished.output;
        notes.add(
            "exit",
            output
                .code
                .map_or_else(|| "signal".to_owned(), |c| c.to_string()),
        );
        notes.add("stdoutBytes", output.stdout.len().to_string());
        notes.add("stderrBytes", output.stderr.len().to_string());
        let truncated = output.stdout_truncated || output.stderr_truncated;
        if truncated {
            notes.add("truncated", "true");
        }
        Ok(ToolOutput {
            exit: output.code,
            stdout: host::redact_userinfo(&output.stdout),
            stderr: host::redact_userinfo(&output.stderr),
            truncated,
            duration_ms: u64::try_from(finished.elapsed.as_millis()).unwrap_or(u64::MAX),
        })
    }

    /// A modifying run: one the user started, or a workflow of a work
    /// session the user started (X3c); never a schedule, an event or the
    /// agent's start. The broker decided the same; this is checked again.
    fn may_modify(&self) -> bool {
        self.run.is_user_initiated()
            || (self.run.trigger == RunTrigger::Workflow
                && self.run.cause == Some(RunCause::WorkSession))
    }

    /// The binding of the `cwd` handle, as the subcommand may use it, and
    /// its label.
    fn target(
        &self,
        plugin: &PluginId,
        handle: &str,
        checked: &Checked,
    ) -> Result<(Target, String), Denial> {
        let not_permitted = |result: CallResult| match result {
            CallResult::Error(e) => Denial::new(codes::INVALID_CWD, e.message),
            CallResult::Ok(_) => Denial::new(codes::INVALID_CWD, "the handle is not usable"),
        };
        let issued = self.bound(plugin, handle).map_err(not_permitted)?;
        let kind = match issued.kind {
            ResourceKind::Directory => TargetKind::Directory,
            ResourceKind::File => TargetKind::File,
            ResourceKind::DockerEnvironment => TargetKind::DockerEnvironment,
            ResourceKind::Application | ResourceKind::Url => {
                return Err(Denial::new(
                    codes::INVALID_CWD,
                    format!("`{}` is not a folder", issued.label),
                ));
            }
        };
        if !checked.accepts(kind) {
            return Err(Denial::new(
                codes::INVALID_CWD,
                format!(
                    "`{} {}` cannot run in `{}`",
                    checked.tool.as_str(),
                    checked.spec.subcommand,
                    issued.label
                ),
            ));
        }
        let target = match kind {
            // The folder, by where links lead, still inside the binding.
            TargetKind::Directory => Target::Directory(
                self.bound_inside(plugin, handle, None, true)
                    .map_err(not_permitted)?,
            ),
            TargetKind::File => {
                let path = std::fs::canonicalize(&issued.value)
                    .ok()
                    .filter(|p| p.is_file())
                    .ok_or_else(|| {
                        Denial::new(
                            codes::INVALID_CWD,
                            format!("the file bound as `{}` does not exist", issued.label),
                        )
                    })?;
                Target::File(path)
            }
            TargetKind::DockerEnvironment => Target::DockerEnvironment(issued.value.clone()),
        };
        Ok((target, issued.label))
    }
}

/// Why the running process must stop now: the invocation was aborted or
/// its host exited, or the grant (or sub-grant) the run used is no longer
/// the plugin's.
fn must_stop(
    broker: &deckpilot_plugin_broker::Broker,
    plugin: &PluginId,
    tool: Tool,
    access: ToolAccess,
    interrupt: Option<&CallInterrupt>,
) -> Option<Stop> {
    if interrupt.is_some_and(|i| i.is_aborted() || i.host_exited()) {
        return Some(Stop::Cancelled);
    }
    let granted = broker
        .tool_grant(plugin, tool)
        .is_some_and(|g| match access {
            ToolAccess::Read => true,
            ToolAccess::Network => g.network,
            ToolAccess::Modify => g.modify,
        });
    (!granted).then_some(Stop::Revoked)
}

/// git works only on a repository that is the binding's own: its work
/// tree is the binding or a folder inside it, and so is its git directory
/// (real paths), so a `.git` file (`gitdir: ../secret/.git`) or a `.git`
/// link to another repository is refused. `GIT_CEILING_DIRECTORIES`
/// already keeps discovery from climbing above the binding.
///
/// The one exception is a genuine linked worktree (`git worktree add`):
/// its git directory is `<common>/worktrees/<name>`, which has a
/// `commondir`, and whose own `gitdir` file points back to this work
/// tree's `.git`. A submodule's git directory (`.git/modules/…` of the
/// superproject) is refused: submodules are not supported as bindings yet.
fn work_tree_inside(
    tools: &ToolHost,
    plan: &Plan,
    deadline: Option<Instant>,
    stop: &(dyn Fn() -> Option<Stop> + Sync),
) -> Result<(), Denial> {
    let listing = host::execute(&host::toplevel_listing(tools, plan), deadline, stop)?;
    let not_a_repository = || {
        Denial::new(
            codes::NOT_A_REPOSITORY,
            "the bound folder is not a git work tree",
        )
    };
    let outside = |what: &str| {
        Denial::new(
            codes::OUTSIDE_BINDING,
            format!("the repository's {what} is not inside the bound folder"),
        )
    };
    let binding = std::fs::canonicalize(&plan.cwd).map_err(|_| not_a_repository())?;
    if !listing.output.success() {
        return Err(not_a_repository());
    }
    let lines: Vec<&str> = listing.output.stdout.lines().collect();
    let [top, git_dir] = lines.as_slice() else {
        return Err(not_a_repository());
    };
    let real = |path: &str| {
        (!path.is_empty())
            .then(|| std::fs::canonicalize(path).ok())
            .flatten()
    };
    let (Some(top), Some(git_dir)) = (real(top), real(git_dir)) else {
        return Err(not_a_repository());
    };
    if !top.starts_with(&binding) {
        return Err(outside("work tree"));
    }
    if git_dir.starts_with(&binding) || linked_worktree(&git_dir, &top) {
        Ok(())
    } else {
        Err(outside("git directory"))
    }
}

/// Whether `git_dir` is the git directory of the linked worktree at
/// `top`: `<common>/worktrees/<name>` with a `commondir`, whose `gitdir`
/// file names `top/.git`.
fn linked_worktree(git_dir: &std::path::Path, top: &std::path::Path) -> bool {
    let under_worktrees = git_dir
        .parent()
        .and_then(std::path::Path::file_name)
        .is_some_and(|n| n == "worktrees");
    let back = std::fs::read_to_string(git_dir.join("gitdir"))
        .ok()
        .map(|text| text.trim_end_matches(['\n', '\r']).to_owned())
        .map(|path| {
            let path = std::path::PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                git_dir.join(path)
            }
        })
        .and_then(|path| std::fs::canonicalize(path).ok());
    let ours = std::fs::canonicalize(top.join(".git")).ok();
    under_worktrees && git_dir.join("commondir").is_file() && back.is_some() && back == ours
}

/// `ls-remote` contacts only a remote the repository configures: `git
/// remote` must list it (a plugin can never choose the server).
fn configured_remote(
    tools: &ToolHost,
    ls_remote: &Plan,
    remote: &str,
    deadline: Option<Instant>,
    stop: &(dyn Fn() -> Option<Stop> + Sync),
) -> Result<(), Denial> {
    let listing = host::execute(&host::remote_listing(tools, ls_remote), deadline, stop)?;
    let listed = listing.output.success()
        && listing
            .output
            .stdout
            .lines()
            .any(|line| line.trim() == remote);
    if listed {
        Ok(())
    } else {
        Err(Denial::new(
            codes::UNKNOWN_REMOTE,
            "the repository does not configure that remote",
        ))
    }
}
