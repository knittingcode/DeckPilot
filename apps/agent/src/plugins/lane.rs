//! One thread per running plugin host (design §3.7). The worker thread
//! prepares an invocation (state, verdict, grants, the declaration) and
//! hands it to the plugin's lane, so a slow or hung plugin blocks neither
//! the worker (installs, other plugins) nor the agent's service task. The
//! lane owns the plugin's [`Supervisor`]; its queue is bounded, and a full
//! queue refuses new invocations at once.
//!
//! Every invocation runs inside `catch_unwind`: the broker's
//! authorization and the capability executor run here, and a panic in
//! them ends the lane and disables the plugin (the worker is told), never
//! the agent.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::{ActionOutcome, Params, StateOutcome};
use deckpilot_plugin_supervisor::{AbortSignal, HostError, Supervisor, SupervisorError};
use tracing::{info, warn};

use super::executor::Executor;

/// Invocations that may wait for a plugin's host; more are refused.
pub(super) const QUEUE: usize = 8;
/// How long stopping a lane waits for its thread: the running
/// invocation's abort grace plus the host's shutdown grace, and a margin.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// Why an invocation did not produce an outcome.
#[derive(Debug)]
pub(super) enum LaneError {
    Supervisor(SupervisorError),
    /// The queue was full.
    Busy,
    /// The lane stopped (the plugin was disabled, updated, uninstalled…)
    /// before or while the invocation ran.
    Stopped,
    /// The invocation was aborted (cancelled, past its deadline, or its
    /// plugin stopped) before it started or before its answer arrived; a
    /// late answer is discarded.
    Aborted,
    /// The broker or the executor panicked; the plugin is disabled.
    Panicked,
    /// The host could not be prepared (files modified, module unreadable).
    Refused(deckpilot_ipc::IpcError),
}

/// Called with the outcome on the lane's thread.
pub(super) type Done = Box<dyn FnOnce(Result<ActionOutcome, LaneError>) + Send>;
/// Called with a key's state on the lane's thread.
pub(super) type StateDone = Box<dyn FnOnce(Result<StateOutcome, LaneError>) + Send>;

/// What a job asks the plugin for, and who hears the answer.
pub(super) enum Reply {
    /// One run of the action (`dp_invoke`).
    Action(Done),
    /// The state of a key bound to the action (`dp_state`).
    State(StateDone),
}

impl Reply {
    fn fail(self, error: LaneError) {
        match self {
            Self::Action(done) => done(Err(error)),
            Self::State(done) => done(Err(error)),
        }
    }
}

pub(super) struct Job {
    pub action: String,
    pub params: Params,
    pub timeout: Duration,
    pub executor: Executor,
    pub abort: AbortSignal,
    pub reply: Reply,
}

/// Why a lane ended by itself; the worker updates the plugin's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LaneEnd {
    /// The supervisor's crash-loop breaker tripped.
    CrashLoop,
    /// A host reported less than a full sandbox: not a crash, the next
    /// invocation checks a fresh host.
    WeakSandbox,
    /// The broker or the executor panicked.
    Panicked,
}

/// Tells the worker that the lane `id` of `plugin` ended.
pub(super) type Ended = Arc<dyn Fn(&str, u64, LaneEnd) + Send + Sync>;

pub(super) struct Lane {
    id: u64,
    tx: Option<SyncSender<Job>>,
    /// The running invocation's abort signal, raised when the lane stops.
    current: Arc<Mutex<Option<AbortSignal>>>,
    stopping: AbortSignal,
    thread: Option<JoinHandle<()>>,
    /// Jobs queued or running.
    pending: Arc<AtomicUsize>,
}

impl std::fmt::Debug for Lane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lane")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Lane {
    pub fn start(plugin: String, id: u64, supervisor: Supervisor, ended: Ended) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Job>(QUEUE);
        let current = Arc::new(Mutex::new(None));
        let stopping = AbortSignal::new();
        let pending = Arc::new(AtomicUsize::new(0));
        let thread = {
            let current = current.clone();
            let stopping = stopping.clone();
            let pending = pending.clone();
            std::thread::Builder::new()
                .name(format!("plugin {plugin}"))
                .spawn(move || {
                    run(
                        &plugin, id, supervisor, &rx, &current, &stopping, &ended, &pending,
                    );
                })
                .map_err(|error| warn!(%error, "the plugin lane could not start"))
                .ok()
        };
        Self {
            id,
            tx: thread.is_some().then_some(tx),
            current,
            stopping,
            thread,
            pending,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Queues `job`; refused at once (with its `done` called) when the
    /// queue is full or the lane ended.
    pub fn submit(&self, job: Job) {
        let Some(tx) = &self.tx else {
            job.reply.fail(LaneError::Stopped);
            return;
        };
        self.pending.fetch_add(1, Ordering::SeqCst);
        let refused = match tx.try_send(job) {
            Ok(()) => return,
            Err(TrySendError::Full(job)) => (job, LaneError::Busy),
            Err(TrySendError::Disconnected(job)) => (job, LaneError::Stopped),
        };
        self.pending.fetch_sub(1, Ordering::SeqCst);
        refused.0.reply.fail(refused.1);
    }

    /// [`Lane::submit`] only while nothing is queued or running; otherwise
    /// `job` is refused at once as [`LaneError::Busy`]. For key state: it
    /// never waits behind an action and never fills the queue of key
    /// presses (a press may find one state call ahead of it, no more).
    pub fn submit_idle(&self, job: Job) {
        if self.pending.load(Ordering::SeqCst) == 0 {
            self.submit(job);
        } else {
            job.reply.fail(LaneError::Busy);
        }
    }

    /// Aborts the running invocation, refuses the queued ones and stops
    /// the host; waits for the thread a few seconds at most.
    pub fn stop(mut self) {
        self.stopping.abort();
        if let Ok(current) = self.current.lock()
            && let Some(abort) = current.as_ref()
        {
            abort.abort();
        }
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let deadline = Instant::now() + STOP_WAIT;
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                warn!("a plugin lane did not stop in time");
            }
        }
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        self.stopping.abort();
        if let Ok(current) = self.current.lock()
            && let Some(abort) = current.as_ref()
        {
            abort.abort();
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the lane's thread borrows each of its owner's handles"
)]
fn run(
    plugin: &str,
    id: u64,
    mut supervisor: Supervisor,
    rx: &Receiver<Job>,
    current: &Mutex<Option<AbortSignal>>,
    stopping: &AbortSignal,
    ended: &Ended,
    pending: &AtomicUsize,
) {
    let mut end = None;
    while let Ok(job) = rx.recv() {
        // Counted until its answer was given.
        let _counted = Counted(pending);
        if end.is_some() || stopping.is_aborted() {
            job.reply.fail(LaneError::Stopped);
            continue;
        }
        if job.abort.is_aborted() {
            job.reply.fail(LaneError::Aborted);
            continue;
        }
        let Job {
            action,
            params,
            timeout,
            mut executor,
            abort,
            reply,
        } = job;
        if let Ok(mut slot) = current.lock() {
            *slot = Some(abort.clone());
        }
        // A stop raised between the check above and now.
        if stopping.is_aborted() {
            abort.abort();
        }
        // The same supervisor, sandbox and breaker for both; a state call
        // only reads (the broker's `authorize_state`).
        let answer = match reply {
            Reply::Action(done) => {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    supervisor.invoke_abortable(
                        &action,
                        &params,
                        timeout,
                        &mut executor,
                        Some(&abort),
                    )
                }));
                let result = settle(plugin, &action, outcome, &abort, &mut end);
                Box::new(move || done(result)) as Box<dyn FnOnce()>
            }
            Reply::State(done) => {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    supervisor.key_state(&action, &params, timeout, &mut executor, Some(&abort))
                }));
                let result = settle(plugin, &action, outcome, &abort, &mut end);
                Box::new(move || done(result)) as Box<dyn FnOnce()>
            }
        };
        if let Ok(mut slot) = current.lock() {
            *slot = None;
        }
        // The worker hears of the end first, so a request that follows
        // the answer sees the plugin's new state.
        if let Some(why) = end {
            ended(plugin, id, why);
        }
        answer();
    }
    let status = supervisor.shutdown();
    info!(plugin, ?status, "plugin host stopped");
}

/// One job less is pending when this is dropped.
struct Counted<'a>(&'a AtomicUsize);

impl Drop for Counted<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What the lane answers for what the supervisor (or a panic in the
/// broker or the executor) returned; `end` is set when the lane must end.
fn settle<T>(
    plugin: &str,
    action: &str,
    outcome: std::thread::Result<Result<T, SupervisorError>>,
    abort: &AbortSignal,
    end: &mut Option<LaneEnd>,
) -> Result<T, LaneError> {
    match outcome {
        // An answer to an aborted invocation (cancelled, or its plugin
        // stopped meanwhile) is late: never applied.
        Ok(Ok(_)) if abort.is_aborted() => Err(LaneError::Aborted),
        Ok(Ok(outcome)) => Ok(outcome),
        Ok(Err(e)) => {
            match &e {
                SupervisorError::Disabled => *end = Some(LaneEnd::CrashLoop),
                SupervisorError::Host(HostError::SandboxMissing(_)) => {
                    *end = Some(LaneEnd::WeakSandbox);
                }
                _ => {}
            }
            Err(LaneError::Supervisor(e))
        }
        Err(_) => {
            warn!(plugin, action, "a plugin invocation panicked in the agent");
            *end = Some(LaneEnd::Panicked);
            Err(LaneError::Panicked)
        }
    }
}

/// The outputs an outcome may carry: those the action declares.
pub(super) fn declared_outputs(action: &deckpilot_plugin_manifest::Action) -> BTreeSet<String> {
    action
        .node
        .as_ref()
        .and_then(|node| node.outputs.as_ref())
        .map(|outputs| outputs.iter().map(|o| o.key.clone()).collect())
        .unwrap_or_default()
}
