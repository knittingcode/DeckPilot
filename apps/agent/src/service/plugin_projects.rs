//! The agent's side of plugin API 1.9's project context (X6 in
//! `docs/marketplace/module-migration.md` §7: `project.current`,
//! `project.list`, `project.select`, `project.cycle`) and the profile read
//! (X9, `profile.read`).
//!
//! The broker allowed the call and the executor checked the run again
//! (`plugins/executor/project.rs`); the executor then asks the agent's
//! thread ([`AgentProjects`]), which answers from the runtime's active
//! profile and its [`ActiveProjectContext`](deckpilot_core::ActiveProjectContext):
//!
//! * Only the **active profile**: ids, keys and names of its projects, no
//!   bindings, no paths, never another profile's.
//! * Only while a **project context provider** is active
//!   ([`Agent::project_context_active`]): otherwise `projects-off`.
//! * `project.select` and `project.cycle` switch the active project as a
//!   request from the app does (persisted, keys redrawn), and the
//!   `projectChanged` event names the plugin. The lifecycle trigger
//!   `projectChanged` then fires as for every switch; a run it starts is
//!   not one the user started, so it cannot switch again (no loop).
//! * In a **workflow run** the executor asks [`ProjectCall::InRun`] with
//!   the run's project: `current` and `profile.read` answer that project,
//!   and `select` and `cycle` only say where the run goes, switching
//!   nothing. The executor keeps the target for the action, whose result
//!   carries it as the built-in project actions' `ContextEffect`; the
//!   engine applies it to the run's project, and the workflow's project
//!   scope decides at its end whether the active project follows
//!   (`docs/workflow-graphs.md`).
//! * `profile.read` answers the profile's name and the active project
//!   (`None` without one, and while no provider is active); the executor
//!   passes on only the granted scopes.

use std::sync::mpsc::sync_channel;
use std::time::Duration;

use deckpilot_core::{CycleDirection, Project, ProjectId};
use tokio::sync::mpsc::UnboundedSender;
use tracing::info;

use super::{Agent, Command, SecurityState};

/// How long an executor waits for the agent's thread.
const ANSWER_WAIT: Duration = Duration::from_secs(2);

/// What a plugin asks of the project context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectCall {
    Current,
    List,
    Select {
        id: String,
    },
    Cycle {
        direction: CycleDirection,
        /// Past either end, start again at the other.
        wrap: bool,
    },
    /// `profile.read`: the profile's name and the active project.
    Profile,
    /// `call` in a workflow run whose project is `project` (its id;
    /// `None`: the run has none): answered for that project, and a
    /// `select` or `cycle` switches nothing, it says where the run goes.
    InRun {
        project: Option<String>,
        call: Box<ProjectCall>,
    },
}

/// A project as plugins see it: no bindings, no paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectEntry {
    pub id: String,
    pub key: String,
    pub name: String,
}

impl From<&Project> for ProjectEntry {
    fn from(project: &Project) -> Self {
        Self {
            id: project.id.to_string(),
            key: project.key.to_string(),
            name: project.name.clone(),
        }
    }
}

/// The agent's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectAnswer {
    Current(Option<ProjectEntry>),
    List(Vec<ProjectEntry>),
    /// `select` or `cycle` made `to` the active project.
    Switched {
        from: Option<String>,
        to: ProjectEntry,
    },
    Profile {
        name: String,
        project: Option<ProjectEntry>,
    },
}

/// Why the agent refused: a stable reason code and a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectRefusal {
    pub code: &'static str,
    pub message: String,
}

impl ProjectRefusal {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub(crate) type ProjectReply = Result<ProjectAnswer, ProjectRefusal>;

/// Whose project a call is about.
#[derive(Debug, Clone, Copy)]
enum Scope<'a> {
    /// The active project, which a switch changes.
    Active,
    /// A workflow run's (its id; `None`: the run has none), which a switch
    /// only plans.
    Run(Option<&'a str>),
}

/// The project context as the executors reach it.
pub(crate) trait ProjectHost: Send + Sync + std::fmt::Debug {
    /// Carries out `call` for `plugin` (already allowed by the broker and
    /// checked by the executor).
    fn call(&self, plugin: &str, call: ProjectCall) -> ProjectReply;
}

/// [`ProjectHost`] through the agent's command channel, answered on the
/// agent's thread. Called on a plugin's lane thread, never on the agent's.
#[derive(Debug, Clone)]
pub(crate) struct AgentProjects(UnboundedSender<Command>);

impl AgentProjects {
    pub fn new(commands: UnboundedSender<Command>) -> Self {
        Self(commands)
    }
}

impl ProjectHost for AgentProjects {
    fn call(&self, plugin: &str, call: ProjectCall) -> ProjectReply {
        let unavailable = || ProjectRefusal::new("unavailable", "the agent is not answering");
        let (reply, answer) = sync_channel(1);
        self.0
            .send(Command::PluginProject {
                plugin: plugin.to_owned(),
                call,
                reply,
            })
            .map_err(|_| unavailable())?;
        answer
            .recv_timeout(ANSWER_WAIT)
            .map_err(|_| unavailable())?
    }
}

impl Agent {
    /// A plugin's project operation or profile read, on the agent's
    /// thread.
    pub(super) fn plugin_project(&mut self, plugin: &str, call: ProjectCall) -> ProjectReply {
        match call {
            ProjectCall::InRun { project, call } => match *call {
                ProjectCall::InRun { .. } => Err(ProjectRefusal::new(
                    "unavailable",
                    "a run's call names one run",
                )),
                call => self.project_call(plugin, call, Scope::Run(project.as_deref())),
            },
            call => self.project_call(plugin, call, Scope::Active),
        }
    }

    /// `call` for the active project or for a workflow run's project,
    /// which a switch only plans.
    fn project_call(&mut self, plugin: &str, call: ProjectCall, scope: Scope<'_>) -> ProjectReply {
        if self.security != SecurityState::Unlocked {
            return Err(ProjectRefusal::new("locked", "DeckPilot is locked"));
        }
        let active = self.project_context_active();
        let profile = self
            .runtime
            .profile()
            .ok_or_else(|| ProjectRefusal::new("no-active-profile", "no profile is active"))?;
        let current = match scope {
            // Never another profile's: an id the active profile does not
            // have is no project.
            Scope::Run(project) => {
                project.and_then(|id| profile.projects.iter().find(|p| p.id.to_string() == id))
            }
            Scope::Active => self
                .runtime
                .active_project_id()
                .and_then(|id| profile.project(id)),
        };
        if call == ProjectCall::Profile {
            return Ok(ProjectAnswer::Profile {
                name: profile.name.clone(),
                project: current.filter(|_| active).map(ProjectEntry::from),
            });
        }
        if !active {
            return Err(ProjectRefusal::new(
                "projects-off",
                "no project context is available: the Projects feature is off",
            ));
        }
        let target = match call {
            ProjectCall::Current => {
                return Ok(ProjectAnswer::Current(current.map(ProjectEntry::from)));
            }
            ProjectCall::List => {
                return Ok(ProjectAnswer::List(
                    profile.projects.iter().map(ProjectEntry::from).collect(),
                ));
            }
            ProjectCall::Profile => unreachable!("answered above"),
            ProjectCall::InRun { .. } => {
                return Err(ProjectRefusal::new(
                    "unavailable",
                    "a run's call names one run",
                ));
            }
            ProjectCall::Select { id } => profile
                .projects
                .iter()
                .find(|p| p.id.to_string() == id)
                .map(|p| p.id)
                .ok_or_else(|| {
                    ProjectRefusal::new("not-found", "the active profile has no such project")
                })?,
            ProjectCall::Cycle { direction, wrap } => deckpilot_core::ProjectSwitcher::step(
                &profile.projects,
                current.map(|p| p.id),
                direction,
                wrap,
            )
            .map_err(|e| ProjectRefusal::new(e.code(), e.to_string()))?,
        };
        if let Scope::Run(_) = scope {
            // The run's switch, which the action's result carries: the
            // active project stays as it is.
            let from = current.map(|p| p.id.to_string());
            let to = profile
                .project(target)
                .map(ProjectEntry::from)
                .ok_or_else(|| ProjectRefusal::new("not-found", "the project is gone"))?;
            info!(plugin, from = ?from, to = %to.id, "a plugin selected the run's project");
            return Ok(ProjectAnswer::Switched { from, to });
        }
        self.switch_project_for(plugin, target)
    }

    /// Makes `target` (a project of the active profile) the active project
    /// for `plugin`, as a request from the app would.
    fn switch_project_for(&mut self, plugin: &str, target: ProjectId) -> ProjectReply {
        let from = self.runtime.active_project_id();
        self.runtime
            .select_project(target)
            .map_err(|e| ProjectRefusal::new("not-found", e.to_string()))?;
        self.context_watch.changed_by(plugin);
        self.after_change();
        let to = self
            .runtime
            .profile()
            .and_then(|profile| profile.project(target))
            .map(ProjectEntry::from)
            .ok_or_else(|| ProjectRefusal::new("not-found", "the project is gone"))?;
        info!(plugin, from = ?from, to = %to.id, "a plugin switched the active project");
        Ok(ProjectAnswer::Switched {
            from: from.map(|id| id.to_string()),
            to,
        })
    }
}
