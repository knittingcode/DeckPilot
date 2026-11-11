//! Workflows running in the background.
//!
//! A run gets an [`ExecutionId`] and executes on its own thread against a
//! snapshot of the active profile, the machine configuration and the
//! project selection, so the agent keeps serving devices and clients while
//! it runs, and no configuration app has to be open. Progress comes back to
//! the agent task as [`Command::Execution`] and goes out to subscribers as
//! execution events.
//!
//! - **Project context.** A run starts with the active project of the moment
//!   as its own context and changes only that (see `deckpilot_core::graph`);
//!   other runs never see its changes. Only a workflow whose project scope is
//!   `application` (a permission per workflow) changes the active project,
//!   when it ends, and only by compare-and-set: if the active profile or
//!   project changed during the run — by the user, a device or another run —
//!   the change is not made and the run records why. The Projects module
//!   must be enabled. What happened is recorded as the run's
//!   `ProjectOutcome`.
//! - **Concurrency.** A workflow runs at most once at a time; at most
//!   [`MAX_RUNNING`] workflows run together.
//! - **Cancellation** stops a run between nodes (see
//!   [`deckpilot_core::graph`]); locking the configuration cancels every run.
//! - **History.** Records are written when a run starts and when it ends.
//!   A record still marked running when the database is opened belongs to
//!   an agent that stopped: it is marked interrupted. Records written while
//!   the database is locked are kept in memory until it opens again.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use deckpilot_core::graph::{CancellationToken, ExecutionEvent, GraphReport};
use deckpilot_core::{
    ActiveProjectContext, Engine, EngineError, ExecutionId, ExecutionRecord, ExecutionState,
    ExecutionSummary, ExecutionTrigger, Profile, ProjectId, ProjectOutcome, ProjectScope,
    WorkflowId, safe_event, safe_report,
};
use deckpilot_ipc::{AgentEvent, ErrorCode, IpcError, MAX_EXECUTION_LIST, Response, SecurityState};
use deckpilot_storage::executions::{ExecutionQuery, Retention};
use tokio::sync::oneshot;
use tracing::{info, warn};

use super::{Agent, Command, WORKFLOW_SUMMARY_WINDOW, engine_error, failed, locked, storage_error};

/// How long a cancelled run may take to stop by default.
pub(crate) const CANCEL_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// Workflows that may run at the same time.
pub(crate) const MAX_RUNNING: usize = 4;
/// Executions listed when a client does not ask for a number.
const DEFAULT_LIST: usize = 50;
/// Records kept in memory while the database is locked.
const MAX_UNSAVED: usize = 100;
const INTERRUPTED: &str = "the agent stopped before the workflow finished";

type Waiter = oneshot::Sender<Result<Response, IpcError>>;

/// Sent from a run's thread to the agent task.
#[derive(Debug)]
pub(crate) enum ExecutionUpdate {
    Event {
        execution: ExecutionId,
        event: ExecutionEvent,
    },
    Finished {
        execution: ExecutionId,
        outcome: Result<GraphReport, IpcError>,
        /// The run's project selection at the end.
        context: ActiveProjectContext,
    },
}

struct Running {
    record: ExecutionRecord,
    cancel: CancellationToken,
    /// `runWorkflow` requests, answered when the run ends.
    waiters: Vec<Waiter>,
    /// What the run executes with, to make its reports safe.
    profile: Arc<Profile>,
    engine: Arc<Engine>,
    /// The project selection when the run started.
    start_project: Option<ProjectId>,
    /// Whether the run may change the active project.
    scope: ProjectScope,
    /// When cancellation was requested.
    cancel_requested: Option<std::time::Instant>,
}

#[derive(Default)]
pub(crate) struct Executions {
    running: Vec<(ExecutionId, Running)>,
    /// Records not written yet because the database was locked.
    unsaved: Vec<ExecutionRecord>,
    retention: Retention,
}

impl std::fmt::Debug for Executions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Executions")
            .field("running", &self.running.len())
            .field("unsaved", &self.unsaved.len())
            .finish_non_exhaustive()
    }
}

impl Executions {
    fn get(&self, id: ExecutionId) -> Option<&Running> {
        self.running.iter().find(|(i, _)| *i == id).map(|(_, r)| r)
    }

    fn get_mut(&mut self, id: ExecutionId) -> Option<&mut Running> {
        self.running
            .iter_mut()
            .find(|(i, _)| *i == id)
            .map(|(_, r)| r)
    }

    fn take(&mut self, id: ExecutionId) -> Option<Running> {
        let index = self.running.iter().position(|(i, _)| *i == id)?;
        Some(self.running.remove(index).1)
    }

    fn keep_unsaved(&mut self, record: &ExecutionRecord) {
        self.unsaved.retain(|r| r.summary.id != record.summary.id);
        self.unsaved.push(record.clone());
        if self.unsaved.len() > MAX_UNSAVED {
            // Drop the oldest finished record; running ones stay.
            if let Some(index) = self
                .unsaved
                .iter()
                .position(|r| r.summary.state.is_finished())
            {
                self.unsaved.remove(index);
            }
        }
    }

    /// Ids of runs that are not over yet, or whose final record is not
    /// written yet.
    fn pending_ids(&self) -> Vec<ExecutionId> {
        self.running
            .iter()
            .map(|(id, _)| *id)
            .chain(self.unsaved.iter().map(|r| r.summary.id))
            .collect()
    }
}

impl Agent {
    fn now_ms(&self) -> u64 {
        (self.options.clock)()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    /// Starts `workflow` of the active profile in the background.
    pub(super) fn start_execution(
        &mut self,
        workflow: WorkflowId,
        trigger: ExecutionTrigger,
    ) -> Result<ExecutionSummary, IpcError> {
        if self.security != SecurityState::Unlocked {
            return Err(locked());
        }
        let profile = self
            .runtime
            .profile()
            .ok_or_else(|| engine_error(&EngineError::NoActiveProfile))?;
        let definition = profile
            .workflow(workflow)
            .ok_or_else(|| engine_error(&EngineError::UnknownWorkflow(workflow)))?;
        let engine = self.runtime.shared_engine();
        engine
            .check_runnable(profile, workflow)
            .map_err(|e| engine_error(&e))?;
        if self
            .executions
            .running
            .iter()
            .any(|(_, r)| r.record.summary.workflow == workflow)
        {
            return Err(failed("this workflow is already running".into()));
        }
        if self.executions.running.len() >= MAX_RUNNING {
            return Err(failed(format!(
                "{MAX_RUNNING} workflows are already running; wait for one to finish"
            )));
        }
        let id = ExecutionId::new();
        let scope = definition.effective_project_scope(profile, engine.registry());
        let mut record = ExecutionRecord::start(id, definition, profile, trigger, self.now_ms());
        let profile = Arc::new(profile.clone());
        let context = self
            .runtime
            .project_context()
            .cloned()
            .unwrap_or_else(|| ActiveProjectContext::new(&profile));
        let start_project = context.project_id();
        record.summary.initial_project = start_project;
        record.summary.project_scope = Some(scope);
        let cancel = CancellationToken::new();
        spawn_run(
            id,
            workflow,
            engine.clone(),
            profile.clone(),
            self.runtime.machine().clone(),
            context,
            cancel.clone(),
            self.handle.commands.clone(),
        )
        .map_err(|error| failed(format!("cannot start the workflow: {error}")))?;

        info!(workflow = %record.summary.workflow_name, execution = %id, ?trigger, "workflow started");
        self.persist_execution(&record);
        let summary = record.summary.clone();
        self.executions.running.push((
            id,
            Running {
                record,
                cancel,
                waiters: Vec::new(),
                profile,
                engine,
                start_project,
                scope,
                cancel_requested: None,
            },
        ));
        let _ = self.handle.events.send(AgentEvent::ExecutionStarted {
            execution: summary.clone(),
        });
        Ok(summary)
    }

    /// `runWorkflow`: starts the run and answers `reply` when it ends.
    pub(super) fn run_execution(&mut self, workflow: WorkflowId, reply: Waiter) {
        match self.start_execution(workflow, ExecutionTrigger::App) {
            Ok(summary) => {
                if let Some(run) = self.executions.get_mut(summary.id) {
                    run.waiters.push(reply);
                }
            }
            Err(error) => {
                let _ = reply.send(Err(error));
            }
        }
    }

    pub(super) fn handle_execution(&mut self, update: ExecutionUpdate) {
        match update {
            ExecutionUpdate::Event { execution, event } => {
                let Some(run) = self.executions.get_mut(execution) else {
                    return;
                };
                let event = safe_event(event, &run.profile, run.engine.registry());
                run.record.apply(&event);
                let (completed, total) = (
                    run.record.summary.completed_nodes,
                    run.record.summary.total_nodes,
                );
                let _ = self.handle.events.send(AgentEvent::ExecutionProgress {
                    execution,
                    event,
                    completed,
                    total,
                });
            }
            ExecutionUpdate::Finished {
                execution,
                outcome,
                context,
            } => self.finish_execution(execution, outcome, &context),
        }
    }

    fn finish_execution(
        &mut self,
        execution: ExecutionId,
        outcome: Result<GraphReport, IpcError>,
        context: &ActiveProjectContext,
    ) {
        let Some(mut run) = self.executions.take(execution) else {
            return;
        };
        let now = self.now_ms();
        let answer = match outcome {
            Ok(mut report) => {
                for node in &mut report.nodes {
                    *node = safe_report(node, &run.profile, run.engine.registry());
                }
                let steps = report.clone().into_steps();
                let conflict = report.context_conflict.clone();
                run.record.finish(report, now);
                run.record.summary.project =
                    Some(self.settle_project(&run, context, conflict.as_deref()));
                for step in &steps.steps {
                    self.report_action(step.action, &step.result);
                }
                self.last_workflow = Some(steps.clone());
                Ok(Response::WorkflowReport { report: steps })
            }
            Err(error) => {
                run.record.end(ExecutionState::Failed, &error.message, now);
                Err(error)
            }
        };
        let summary = run.record.summary.clone();
        info!(workflow = %summary.workflow_name, %execution, state = ?summary.state, "workflow finished");
        for waiter in run.waiters.drain(..) {
            let _ = waiter.send(answer.clone());
        }
        self.persist_execution(&run.record);
        // A workflow started from the deck has no window of its own; show
        // what failed.
        if summary.trigger == ExecutionTrigger::Device
            && summary.state == ExecutionState::Failed
            && self.security == SecurityState::Unlocked
            && let Err(error) = self.request_window(WORKFLOW_SUMMARY_WINDOW)
        {
            warn!(%error, "cannot show the workflow summary");
        }
        let _ = self
            .handle
            .events
            .send(AgentEvent::ExecutionFinished { execution: summary });
        self.after_change();
    }

    /// Decides what the project a run ended with means for the active
    /// project, and makes the change where the run may.
    fn settle_project(
        &mut self,
        run: &Running,
        context: &ActiveProjectContext,
        conflict: Option<&str>,
    ) -> ProjectOutcome {
        let after = context.project_id();
        let not_applied = |project, reason: &str| ProjectOutcome::NotApplied {
            project,
            reason: reason.to_owned(),
        };
        if after == run.start_project {
            return match (run.scope, conflict) {
                (ProjectScope::Application, Some(conflict)) => not_applied(None, conflict),
                _ => ProjectOutcome::Unchanged,
            };
        }
        if run.scope == ProjectScope::Execution {
            return ProjectOutcome::KeptInRun { project: after };
        }
        let Some(project) = after else {
            return not_applied(after, "the run ended without a project");
        };
        let outcome = if self.runtime.profile().map(|p| p.id) != Some(run.profile.id) {
            not_applied(after, "the active profile changed during the run")
        } else if self.runtime.active_project_id() != run.start_project {
            // Someone else changed the active project meanwhile: their
            // change stands.
            not_applied(after, "the active project changed during the run")
        } else if self.require_projects().is_err() {
            not_applied(after, "the Projects module is disabled")
        } else if self.runtime.select_project(project).is_err() {
            not_applied(after, "the project no longer exists")
        } else {
            ProjectOutcome::Applied { project }
        };
        info!(workflow = %run.record.summary.workflow_name, outcome = ?outcome, "project after a workflow");
        outcome
    }

    pub(super) fn cancel_execution(&mut self, id: ExecutionId) -> Result<(), IpcError> {
        let profile = self.runtime.profile().map(|p| p.id);
        match self.executions.get_mut(id) {
            Some(run) if Some(run.record.summary.profile) == profile => {
                info!(execution = %id, "cancelling a workflow");
                run.cancel.cancel();
                run.cancel_requested
                    .get_or_insert_with(std::time::Instant::now);
                Ok(())
            }
            _ => Err(IpcError::new(
                ErrorCode::NotFound,
                "no such running workflow",
            )),
        }
    }

    /// Cancels every run, e.g. when the configuration is locked. The runs
    /// end shortly after; their records are written once the database is
    /// open again.
    pub(super) fn cancel_all_executions(&mut self) {
        for (_, run) in &mut self.executions.running {
            run.cancel.cancel();
            run.cancel_requested
                .get_or_insert_with(std::time::Instant::now);
        }
    }

    /// Records runs that were cancelled more than the grace period ago as
    /// cancelled, although an action that cannot be interrupted is still
    /// finishing. Its result arrives later and is discarded; nothing of it
    /// is applied.
    pub(super) fn expire_cancelled_executions(&mut self) {
        let grace = self.options.cancel_grace;
        let expired: Vec<ExecutionId> = self
            .executions
            .running
            .iter()
            .filter(|(_, run)| run.cancel_requested.is_some_and(|at| at.elapsed() >= grace))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            let Some(mut run) = self.executions.take(id) else {
                continue;
            };
            let now = self.now_ms();
            run.record.end(
                ExecutionState::Cancelled,
                "cancelled; an action that cannot be interrupted was still running, and its result is discarded",
                now,
            );
            warn!(execution = %id, "a cancelled workflow did not stop in time");
            for waiter in run.waiters.drain(..) {
                let _ = waiter.send(Err(failed("the workflow was cancelled".into())));
            }
            self.persist_execution(&run.record);
            let _ = self.handle.events.send(AgentEvent::ExecutionFinished {
                execution: run.record.summary.clone(),
            });
        }
    }

    /// The agent stops: runs still going are recorded as interrupted.
    pub(super) fn interrupt_executions(&mut self) {
        let now = self.now_ms();
        for (_, mut run) in std::mem::take(&mut self.executions.running) {
            run.cancel.cancel();
            run.record
                .end(ExecutionState::Interrupted, INTERRUPTED, now);
            for waiter in run.waiters.drain(..) {
                let _ = waiter.send(Err(failed("agent is shutting down".into())));
            }
            self.persist_execution(&run.record);
        }
    }

    /// Writes `record`, or keeps it until the database is open.
    fn persist_execution(&mut self, record: &ExecutionRecord) {
        let Some(store) = self.store.as_ref() else {
            self.executions.keep_unsaved(record);
            return;
        };
        let mut unsaved = std::mem::take(&mut self.executions.unsaved);
        unsaved.retain(|r| r.summary.id != record.summary.id);
        unsaved.push(record.clone());
        let mut failed_records = Vec::new();
        for record in unsaved {
            if let Err(error) = store.save_execution(&record) {
                warn!(%error, "cannot record a workflow run");
                failed_records.push(record);
            }
        }
        if record.summary.state.is_finished()
            && let Err(error) = store.prune_executions(self.executions.retention)
        {
            warn!(%error, "cannot prune the execution history");
        }
        for record in &failed_records {
            self.executions.keep_unsaved(record);
        }
    }

    /// After the database opened: marks runs of an agent that stopped as
    /// interrupted and writes records kept while it was locked.
    pub(super) fn recover_executions(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        match store.interrupt_stale_executions(&self.executions.pending_ids(), self.now_ms()) {
            Ok(0) => {}
            Ok(count) => info!(count, "marked interrupted workflow runs"),
            Err(error) => warn!(%error, "cannot check for interrupted workflow runs"),
        }
        if let Some(record) = self.executions.unsaved.pop() {
            self.persist_execution(&record);
        }
    }

    /// Running and recorded executions of the active profile, newest first.
    pub(super) fn list_executions(
        &self,
        workflow: Option<WorkflowId>,
        limit: Option<u32>,
    ) -> Result<Vec<ExecutionSummary>, IpcError> {
        let store = self.store()?;
        let profile = self
            .runtime
            .profile()
            .ok_or_else(|| engine_error(&EngineError::NoActiveProfile))?
            .id;
        let limit = limit.map_or(DEFAULT_LIST, |l| {
            usize::try_from(l.min(MAX_EXECUTION_LIST)).unwrap_or(DEFAULT_LIST)
        });
        let matches = |record: &ExecutionRecord| {
            record.summary.profile == profile
                && workflow.is_none_or(|w| record.summary.workflow == w)
        };
        let mut records: Vec<ExecutionSummary> = self
            .executions
            .running
            .iter()
            .map(|(_, r)| &r.record)
            .chain(&self.executions.unsaved)
            .filter(|r| matches(r))
            .map(|r| r.summary.clone())
            .collect();
        let stored = store
            .executions(ExecutionQuery {
                profile,
                workflow,
                limit,
            })
            .map_err(|e| storage_error(&e))?;
        for record in stored {
            if !records.iter().any(|r| r.id == record.summary.id) {
                records.push(record.summary);
            }
        }
        records.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(a.id.cmp(&b.id)));
        records.truncate(limit);
        Ok(records)
    }

    /// One execution of the active profile.
    pub(super) fn execution(&self, id: ExecutionId) -> Result<ExecutionRecord, IpcError> {
        let store = self.store()?;
        let profile = self.runtime.profile().map(|p| p.id);
        let record = match self
            .executions
            .get(id)
            .map(|r| &r.record)
            .or_else(|| self.executions.unsaved.iter().find(|r| r.summary.id == id))
        {
            Some(record) => Some(record.clone()),
            None => store.execution(id).map_err(|e| storage_error(&e))?,
        };
        record
            .filter(|r| Some(r.summary.profile) == profile)
            .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "no such execution"))
    }

    pub(super) fn clear_execution_history(
        &mut self,
        workflow: Option<WorkflowId>,
    ) -> Result<(), IpcError> {
        let profile = self
            .runtime
            .profile()
            .ok_or_else(|| engine_error(&EngineError::NoActiveProfile))?
            .id;
        let removed = self
            .store()?
            .clear_executions(profile, workflow)
            .map_err(|e| storage_error(&e))?;
        self.executions.unsaved.retain(|r| {
            !(r.summary.state.is_finished()
                && r.summary.profile == profile
                && workflow.is_none_or(|w| r.summary.workflow == w))
        });
        info!(removed, "execution history cleared");
        Ok(())
    }
}

/// Runs a workflow on its own thread and reports back through `commands`.
#[allow(clippy::too_many_arguments)]
fn spawn_run(
    id: ExecutionId,
    workflow: WorkflowId,
    engine: Arc<Engine>,
    profile: Arc<Profile>,
    machine: deckpilot_core::MachineConfig,
    mut context: ActiveProjectContext,
    cancel: CancellationToken,
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(format!("workflow-{id}"))
        .spawn(move || {
            let events = commands.clone();
            let on_event = move |event| {
                let _ = events.send(Command::Execution(ExecutionUpdate::Event {
                    execution: id,
                    event,
                }));
            };
            // A panicking action must not leave the run open forever.
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                engine.execute_workflow(
                    &profile,
                    &machine,
                    &mut context,
                    workflow,
                    &cancel,
                    &on_event,
                )
            }));
            let outcome = match outcome {
                Ok(Ok(report)) => Ok(report),
                Ok(Err(error)) => Err(engine_error(&error)),
                Err(_) => Err(failed("the workflow stopped unexpectedly".into())),
            };
            let _ = commands.send(Command::Execution(ExecutionUpdate::Finished {
                execution: id,
                outcome,
                context,
            }));
        })
        .map(|_| ())
}
