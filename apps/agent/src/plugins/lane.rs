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
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::{ActionOutcome, Params};
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
    /// The invocation was aborted before it started.
    Aborted,
    /// The broker or the executor panicked; the plugin is disabled.
    Panicked,
    /// The host could not be prepared (files modified, module unreadable).
    Refused(deckpilot_ipc::IpcError),
}

/// Called with the outcome on the lane's thread.
pub(super) type Done = Box<dyn FnOnce(Result<ActionOutcome, LaneError>) + Send>;

pub(super) struct Job {
    pub action: String,
    pub params: Params,
    pub timeout: Duration,
    pub executor: Executor,
    pub abort: AbortSignal,
    pub done: Done,
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
        let thread = {
            let current = current.clone();
            let stopping = stopping.clone();
            std::thread::Builder::new()
                .name(format!("plugin {plugin}"))
                .spawn(move || run(&plugin, id, supervisor, &rx, &current, &stopping, &ended))
                .map_err(|error| warn!(%error, "the plugin lane could not start"))
                .ok()
        };
        Self {
            id,
            tx: thread.is_some().then_some(tx),
            current,
            stopping,
            thread,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Queues `job`; refused at once (with its `done` called) when the
    /// queue is full or the lane ended.
    pub fn submit(&self, job: Job) {
        let Some(tx) = &self.tx else {
            (job.done)(Err(LaneError::Stopped));
            return;
        };
        match tx.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(job)) => (job.done)(Err(LaneError::Busy)),
            Err(TrySendError::Disconnected(job)) => (job.done)(Err(LaneError::Stopped)),
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

fn run(
    plugin: &str,
    id: u64,
    mut supervisor: Supervisor,
    rx: &Receiver<Job>,
    current: &Mutex<Option<AbortSignal>>,
    stopping: &AbortSignal,
    ended: &Ended,
) {
    let mut end = None;
    while let Ok(job) = rx.recv() {
        if end.is_some() || stopping.is_aborted() {
            (job.done)(Err(LaneError::Stopped));
            continue;
        }
        if job.abort.is_aborted() {
            (job.done)(Err(LaneError::Aborted));
            continue;
        }
        let Job {
            action,
            params,
            timeout,
            mut executor,
            abort,
            done,
        } = job;
        if let Ok(mut slot) = current.lock() {
            *slot = Some(abort.clone());
        }
        // A stop raised between the check above and now.
        if stopping.is_aborted() {
            abort.abort();
        }
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            supervisor.invoke_abortable(&action, &params, timeout, &mut executor, Some(&abort))
        }));
        if let Ok(mut slot) = current.lock() {
            *slot = None;
        }
        let result = match outcome {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(e)) => {
                match &e {
                    SupervisorError::Disabled => end = Some(LaneEnd::CrashLoop),
                    SupervisorError::Host(HostError::SandboxMissing(_)) => {
                        end = Some(LaneEnd::WeakSandbox);
                    }
                    _ => {}
                }
                Err(LaneError::Supervisor(e))
            }
            Err(_) => {
                warn!(plugin, action, "a plugin invocation panicked in the agent");
                end = Some(LaneEnd::Panicked);
                Err(LaneError::Panicked)
            }
        };
        // The worker hears of the end first, so a request that follows
        // the answer sees the plugin's new state.
        if let Some(why) = end {
            ended(plugin, id, why);
        }
        done(result);
    }
    let status = supervisor.shutdown();
    info!(plugin, ?status, "plugin host stopped");
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
