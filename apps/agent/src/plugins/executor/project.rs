//! The project context of plugin API 1.9 (X6 in
//! `docs/marketplace/module-migration.md` §7: `project.current`,
//! `project.list`, `project.select`, `project.cycle`) and the profile read
//! (X9, `profile.read`).
//!
//! The broker decided (grants, `projectContext:select` for a switch, run
//! origin, 2 switches per second). The executor checks the run again: a
//! switch only in a run the user started, never in one a lifecycle event
//! (`projectChanged` among them), a schedule, the agent's start or a work
//! session started, so a plugin cannot loop on the switch it caused. Then
//! the agent's thread carries the call out on its active profile
//! (`service/plugin_projects.rs`): ids, keys and names only, no bindings,
//! no paths, and only while a project context provider is active
//! (`projects-off` otherwise). `profile.read` passes on only the granted
//! scopes; without an active project its `project` is absent, not an
//! error.
//!
//! **In a workflow run** a switch changes the run's project, as the
//! built-in project actions' `ContextEffect` does, not the active one:
//! the agent only says where the run goes ([`ProjectCall::InRun`]), the
//! target waits in the invocation's [`ProjectSwitch`], and the action's
//! result carries it to the engine, which applies it for the nodes after
//! it (the workflow's project scope then decides whether the active
//! project follows). Only an action whose node declares `changesContext`
//! may; `current` and `profile.read` answer the run's project there.
//! From a key, a gesture or the app a switch changes the active project
//! at once (those runs are detached and carry no effect).
//!
//! Every call is audited ([`super::PrivilegedRecord`]): the operation, the
//! grant, the run's origin, the outcome and, for a switch, the from and to
//! project ids. Refusals carry their reason code at the start of the
//! message (`projects-off: …`).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use deckpilot_core::CycleDirection as CoreDirection;
use deckpilot_plugin_abi::capability::RunTrigger;
use deckpilot_plugin_abi::privileged::{CycleDirection, ProfileInfo, ProjectInfo, ProjectList};
use deckpilot_plugin_abi::{CallErrorCode, CallResult};
use deckpilot_plugin_broker::{Allowed, DenyReason, PluginId, ProfileScope};
use serde_json::Value;
use tracing::warn;

use super::{Executor, PrivilegedRecord};
use crate::service::plugin_projects::{
    ProjectAnswer, ProjectCall, ProjectEntry, ProjectHost, ProjectRefusal,
};

/// The project a plugin action selected in a workflow run, shared by the
/// action's handler (which made it) and its executor (which fills it).
/// Empty until a `project.select` or `project.cycle` of the invocation.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectSwitch(Arc<Mutex<SwitchState>>);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SwitchState {
    /// The action's node declares `changesContext`.
    allowed: bool,
    /// The selected project's id.
    target: Option<String>,
}

impl ProjectSwitch {
    /// The switch of an invocation of an action that changes the run's
    /// project (`allowed`) or not.
    pub fn for_action(allowed: bool) -> Self {
        Self(Arc::new(Mutex::new(SwitchState {
            allowed,
            target: None,
        })))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, SwitchState> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn allowed(&self) -> bool {
        self.state().allowed
    }

    /// The project the invocation selected, if it did.
    pub fn target(&self) -> Option<String> {
        self.state().target.clone()
    }

    fn select(&self, id: String) {
        self.state().target = Some(id);
    }
}

impl PartialEq for ProjectSwitch {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.state() == *other.state()
    }
}

impl Eq for ProjectSwitch {}

impl Executor {
    /// The executor of an invocation that reaches the agent's project
    /// context through `projects`.
    pub fn with_projects(mut self, projects: Option<Arc<dyn ProjectHost>>) -> Self {
        self.projects = projects;
        self
    }

    /// One of the project operations or `profile.read`, which the broker
    /// allowed.
    pub(super) fn project(&self, plugin: &PluginId, allowed: &Allowed) -> CallResult {
        let started = Instant::now();
        let (op, grant, call) = match allowed {
            Allowed::ProjectCurrent => ("project.current", "projectContext", ProjectCall::Current),
            Allowed::ProjectList => ("project.list", "projectContext", ProjectCall::List),
            Allowed::ProjectSelect { id } => (
                "project.select",
                "projectContext:select",
                ProjectCall::Select { id: id.clone() },
            ),
            Allowed::ProjectCycle { request } => (
                "project.cycle",
                "projectContext:select",
                ProjectCall::Cycle {
                    direction: match request.direction {
                        CycleDirection::Next => CoreDirection::Next,
                        CycleDirection::Previous => CoreDirection::Previous,
                    },
                    wrap: request.wrap.unwrap_or(true),
                },
            ),
            Allowed::ProfileRead { .. } => ("profile.read", "profile", ProjectCall::Profile),
            _ => {
                return CallResult::error(CallErrorCode::Failed, "not a project operation");
            }
        };
        let mut record = PrivilegedRecord {
            plugin: plugin.to_string(),
            op,
            grant,
            outcome: None,
            elapsed: std::time::Duration::ZERO,
            details: Vec::new(),
        };
        let switch = matches!(call, ProjectCall::Select { .. } | ProjectCall::Cycle { .. });
        let in_run = self.in_run();
        let answer = if switch && !self.run.is_user_initiated() {
            // The broker refuses it already; checked again so that nothing
            // but a run the user started switches the project (no loop on
            // `projectChanged`).
            warn!(plugin = %plugin, op, trigger = ?self.run.trigger, cause = ?self.run.cause,
                "a plugin project switch was refused: the user did not start the run");
            Err(ProjectRefusal::new(
                "not-permitted",
                DenyReason::PrivilegedNotUserInitiated.to_string(),
            ))
        } else if switch && in_run.as_ref().is_some_and(|(s, _)| !s.allowed()) {
            Err(ProjectRefusal::new(
                "not-permitted",
                "this action does not change the run's project (its node does not declare \
                 `changesContext`)",
            ))
        } else {
            let call = scoped(in_run.as_ref(), call);
            match &self.projects {
                Some(host) => host.call(plugin.as_str(), call),
                None => Err(ProjectRefusal::new(
                    "unavailable",
                    "the project context is not available in this agent",
                )),
            }
        };
        let result = match answer {
            Ok(answer) => {
                if let ProjectAnswer::Switched { from, to } = &answer {
                    record
                        .details
                        .push(("from", from.clone().unwrap_or_default()));
                    record.details.push(("to", to.id.clone()));
                    if let Some((switched, _)) = &in_run {
                        // Applied to the run when the action succeeds.
                        switched.select(to.id.clone());
                        record.details.push(("scope", "run".to_owned()));
                    }
                }
                let scopes = match allowed {
                    Allowed::ProfileRead { scopes } => Some(scopes),
                    _ => None,
                };
                serde_json::to_value(reply(answer, scopes)).map_or_else(
                    |e| CallResult::error(CallErrorCode::Failed, e.to_string()),
                    CallResult::Ok,
                )
            }
            Err(refusal) => {
                record.outcome = Some(refusal.code);
                CallResult::error(
                    code(refusal.code),
                    format!("{}: {}", refusal.code, refusal.message),
                )
            }
        };
        record.elapsed = started.elapsed();
        self.audit_privileged(&record);
        result
    }
}

impl Executor {
    /// In a workflow run: the invocation's switch and the run's project,
    /// which a switch changes (and `current` and `profile.read` answer):
    /// the one this invocation selected, else the run's own.
    fn in_run(&self) -> Option<(ProjectSwitch, Option<String>)> {
        (self.run.trigger == RunTrigger::Workflow).then(|| {
            let table = self.handles();
            let switched = table.switch().clone();
            let project = switched
                .target()
                .or_else(|| table.run_project().map(str::to_owned));
            (switched, project)
        })
    }
}

/// `call` as the agent is asked it: for the run's project in a workflow
/// run (`in_run`), except `project.list`, which is the profile's.
fn scoped(in_run: Option<&(ProjectSwitch, Option<String>)>, call: ProjectCall) -> ProjectCall {
    match (in_run, call) {
        (
            Some((_, project)),
            call @ (ProjectCall::Current
            | ProjectCall::Profile
            | ProjectCall::Select { .. }
            | ProjectCall::Cycle { .. }),
        ) => ProjectCall::InRun {
            project: project.clone(),
            call: Box::new(call),
        },
        (_, call) => call,
    }
}

/// The ABI code of a refusal's reason code.
fn code(reason: &str) -> CallErrorCode {
    match reason {
        "not-permitted" => CallErrorCode::NotPermitted,
        "not-found" | "no-projects" | "at-last-project" | "at-first-project" => {
            CallErrorCode::Failed
        }
        _ => CallErrorCode::Unavailable,
    }
}

fn info(entry: ProjectEntry) -> ProjectInfo {
    ProjectInfo {
        id: entry.id,
        key: entry.key,
        name: entry.name,
    }
}

/// What the plugin receives: a project or `null`, `{projects}`, the new
/// project, or `{profile?, project?}` within the granted `scopes`.
fn reply(
    answer: ProjectAnswer,
    scopes: Option<&std::collections::BTreeSet<ProfileScope>>,
) -> Value {
    let value = match answer {
        ProjectAnswer::Current(project) => serde_json::to_value(project.map(info)),
        ProjectAnswer::List(projects) => serde_json::to_value(ProjectList {
            projects: projects.into_iter().map(info).collect(),
        }),
        ProjectAnswer::Switched { to, .. } => serde_json::to_value(info(to)),
        ProjectAnswer::Profile { name, project } => {
            let granted = |scope| scopes.is_some_and(|s| s.contains(&scope));
            serde_json::to_value(ProfileInfo {
                profile: granted(ProfileScope::Profile).then_some(name),
                project: project.filter(|_| granted(ProfileScope::Project)).map(info),
            })
        }
    };
    value.unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests;
