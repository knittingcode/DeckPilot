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
//! Every call is audited ([`super::PrivilegedRecord`]): the operation, the
//! grant, the run's origin, the outcome and, for a switch, the from and to
//! project ids. Refusals carry their reason code at the start of the
//! message (`projects-off: …`).

use std::sync::Arc;
use std::time::Instant;

use deckpilot_core::CycleDirection as CoreDirection;
use deckpilot_plugin_abi::privileged::{CycleDirection, ProfileInfo, ProjectInfo, ProjectList};
use deckpilot_plugin_abi::{CallErrorCode, CallResult};
use deckpilot_plugin_broker::{Allowed, DenyReason, PluginId, ProfileScope};
use serde_json::Value;
use tracing::warn;

use super::{Executor, PrivilegedRecord};
use crate::service::plugin_projects::{
    ProjectAnswer, ProjectCall, ProjectEntry, ProjectHost, ProjectRefusal,
};

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
        } else {
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
