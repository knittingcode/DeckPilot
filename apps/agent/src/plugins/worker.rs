//! The plugin worker thread. It owns the [`PluginService`] while the
//! configuration is unlocked and answers plugin requests one at a time, so
//! installs and verifications never run on the agent's service task.
//! Invocations are prepared here and run on the plugin's lane
//! (`lane.rs`, one thread per host), so a slow plugin blocks neither this
//! thread nor other plugins.
//!
//! After every change the worker publishes the installed plugins
//! ([`CatalogPlugin`]) for the agent's module registry and tells the
//! service task, which registers them as modules (`engine.rs`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use deckpilot_core::alias::LegacyRouting;
use deckpilot_ipc::plugins::PluginEvent;
use deckpilot_ipc::{AgentEvent, IpcError, Request, Response};
use deckpilot_plugin_abi::capability::RunContext;
use deckpilot_plugin_abi::{ActionOutcome, Params};
use deckpilot_plugin_store::StoreKey;
use deckpilot_plugin_supervisor::AbortSignal;
use tokio::sync::{broadcast, oneshot};
use tracing::{info, warn};

use super::PluginSettings;
use super::engine::CatalogPlugin;
use super::lane::LaneEnd;
use super::migration::LegacySnapshot;
use super::service::{PluginService, closed_status, open_problem, store_unavailable};
use super::variables::PublishedVariables;

/// How long stopping waits for the worker (hosts get a grace period each).
const STOP_WAIT: Duration = Duration::from_secs(10);

/// Where the plugin runtime reports: agent events, plugin events
/// (progress, state), and the agent's notifier for plugin notifications
/// (the status bar of open app windows and the desktop).
#[derive(Debug, Clone)]
pub(crate) struct Events {
    pub agent: broadcast::Sender<AgentEvent>,
    pub plugins: broadcast::Sender<PluginEvent>,
    pub notifier: Arc<dyn deckpilot_core::notify::Notifier>,
}

/// Why an engine invocation produced no outcome: a stable code for the
/// action result, and a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EngineFailure {
    pub code: String,
    pub message: String,
}

pub(super) type EngineReply = Result<ActionOutcome, EngineFailure>;
/// The answer to a state call for the engine.
pub(super) type StateReply = Result<deckpilot_plugin_abi::StateOutcome, EngineFailure>;

pub(super) enum Message {
    Open(StoreKey),
    Close,
    Rekey(StoreKey, StoreKey),
    Request(Request, oneshot::Sender<Result<Response, IpcError>>),
    /// What `deckpilot.db` says about the modules that can migrate, after
    /// the agent loaded its modules (`migration.rs`).
    Migrate(Box<LegacySnapshot>),
    /// A plugin action from the workflow engine.
    Invoke {
        plugin: String,
        action: String,
        params: Params,
        /// What started the run (G8).
        run: RunContext,
        abort: AbortSignal,
        reply: mpsc::SyncSender<EngineReply>,
    },
    /// The state of a key bound to a plugin action (`dp_state`).
    KeyState {
        plugin: String,
        action: String,
        params: Params,
        abort: AbortSignal,
        reply: mpsc::SyncSender<StateReply>,
    },
    /// A lane ended by itself (crash loop, weak sandbox, panic).
    LaneEnded {
        plugin: String,
        lane: u64,
        why: LaneEnd,
    },
    Stop,
}

/// Hands engine invocations to the worker. Cheap to clone; held by every
/// plugin action handler.
#[derive(Debug, Clone)]
pub(crate) struct Invoker(mpsc::Sender<Message>, super::key_state::StateBook);

impl Invoker {
    /// Queues an invocation of a run that `run` describes; `None` when the
    /// worker is gone. The answer arrives on the returned channel;
    /// dropping it discards a late one.
    pub fn invoke(
        &self,
        plugin: String,
        action: String,
        params: Params,
        run: RunContext,
        abort: AbortSignal,
    ) -> Option<Receiver<EngineReply>> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.0
            .send(Message::Invoke {
                plugin,
                action,
                params,
                run,
                abort,
                reply,
            })
            .ok()?;
        Some(answer)
    }

    /// Queues a state call for a key bound to `action` with `params`;
    /// `None` when the worker is gone. Refused at once (`plugin-busy`)
    /// while the plugin runs or waits for anything else.
    pub fn key_state(
        &self,
        plugin: String,
        action: String,
        params: Params,
        abort: AbortSignal,
    ) -> Option<Receiver<StateReply>> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.0
            .send(Message::KeyState {
                plugin,
                action,
                params,
                abort,
                reply,
            })
            .ok()?;
        Some(answer)
    }

    /// What the agent remembers about plugin key states between calls.
    pub fn book(&self) -> &super::key_state::StateBook {
        &self.1
    }
}

/// Called after the installed plugins changed (on the worker thread).
pub(crate) type Notify = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug)]
pub(super) struct PluginWorker {
    tx: mpsc::Sender<Message>,
    thread: Option<JoinHandle<()>>,
    catalog: Arc<Mutex<Vec<CatalogPlugin>>>,
    routing: Arc<Mutex<LegacyRouting>>,
    values: PublishedVariables,
    /// Plugin key states between calls, shared with every invoker.
    book: super::key_state::StateBook,
}

struct State {
    settings: PluginSettings,
    data_dir: PathBuf,
    events: Events,
    service: Option<PluginService>,
    /// Why the store did not open (wrong key, newer schema…).
    problem: Option<String>,
    unlocked: bool,
    /// For lanes to report their end.
    tx: mpsc::Sender<Message>,
    /// The installed plugins as last published, and whom to tell.
    published: Arc<Mutex<Vec<CatalogPlugin>>>,
    notify: Notify,
    /// The plugins' published variable values, shared with their modules.
    values: PublishedVariables,
    /// The routing of migrated modules as last published.
    routing: Arc<Mutex<LegacyRouting>>,
    /// Whether the migration pass of this opening of the store ran (it
    /// runs once, with the first snapshot after the store opened).
    migrated: bool,
}

impl State {
    fn open(&mut self, key: &StoreKey) {
        self.close();
        self.unlocked = true;
        self.migrated = false;
        let tx = Mutex::new(self.tx.clone());
        let ended = Arc::new(move |plugin: &str, lane: u64, why: LaneEnd| {
            if let Ok(tx) = tx.lock() {
                let _ = tx.send(Message::LaneEnded {
                    plugin: plugin.to_owned(),
                    lane,
                    why,
                });
            }
        });
        match PluginService::open(
            &self.settings,
            &self.data_dir,
            key,
            self.events.clone(),
            ended,
            self.values.clone(),
        ) {
            Ok(service) => {
                info!("plugin store opened");
                self.service = Some(service);
                self.problem = None;
            }
            Err(error) => {
                let problem = open_problem(&error);
                warn!(%problem, "plugins are unavailable");
                self.problem = Some(problem);
            }
        }
    }

    fn close(&mut self) {
        if let Some(mut service) = self.service.take() {
            service.shutdown();
            info!("plugin store closed");
        }
        self.unlocked = false;
        self.problem = None;
    }

    fn rekey(&mut self, old: &StoreKey, new: &StoreKey) {
        self.close();
        let path = deckpilot_plugin_store::Layout::new(&self.data_dir).database();
        if !path.exists() {
            return;
        }
        match deckpilot_plugin_store::Database::rekey(&path, old, new) {
            Ok(()) => info!("plugins.db re-encrypted for the new configuration key"),
            Err(error) => warn!(%error, "plugins.db could not be re-encrypted"),
        }
    }

    fn unavailable(&self) -> IpcError {
        match &self.problem {
            Some(problem) => store_unavailable(problem),
            None => super::locked(),
        }
    }

    fn request(&mut self, request: Request, reply: oneshot::Sender<Result<Response, IpcError>>) {
        match (&mut self.service, request) {
            // Answered from the plugin's lane when the action finished.
            (
                Some(service),
                Request::InvokePluginAction {
                    plugin,
                    action,
                    params,
                },
            ) => service.invoke(&plugin, &action, &params, reply),
            (Some(service), request) => {
                let answer = service.handle(request);
                // Before the answer: the agent then registers the new
                // state before the client's next request arrives.
                self.publish();
                let _ = reply.send(answer);
            }
            (None, Request::GetPluginRuntime) => {
                let _ = reply.send(Ok(Response::PluginRuntime {
                    status: Box::new(closed_status(self.problem.as_deref(), self.unlocked)),
                }));
            }
            (None, _) => {
                let _ = reply.send(Err(self.unavailable()));
            }
        }
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Open(key) => self.open(&key),
            Message::Close => self.close(),
            Message::Rekey(old, new) => self.rekey(&old, &new),
            Message::Request(request, reply) => self.request(request, reply),
            Message::Migrate(snapshot) => {
                if let Some(service) = &mut self.service {
                    // The pass runs once per opening; every snapshot is
                    // kept for the user's decisions.
                    let first = !self.migrated;
                    self.migrated = true;
                    service.migrate(*snapshot, first);
                }
            }
            Message::Invoke {
                plugin,
                action,
                params,
                run,
                abort,
                reply,
            } => {
                if let Some(service) = &mut self.service {
                    service.invoke_for_engine(&plugin, &action, params, run, abort, reply);
                } else {
                    let error = self.unavailable();
                    let _ = reply.try_send(Err(EngineFailure {
                        code: "plugin-unavailable".into(),
                        message: error.message,
                    }));
                }
            }
            Message::KeyState {
                plugin,
                action,
                params,
                abort,
                reply,
            } => {
                if let Some(service) = &mut self.service {
                    service.key_state_for_engine(&plugin, &action, params, abort, reply);
                } else {
                    let error = self.unavailable();
                    let _ = reply.try_send(Err(EngineFailure {
                        code: "plugin-unavailable".into(),
                        message: error.message,
                    }));
                }
            }
            Message::LaneEnded { plugin, lane, why } => {
                if let Some(service) = &mut self.service {
                    service.lane_ended(&plugin, lane, why);
                }
            }
            Message::Stop => {}
        }
    }

    fn catalog(&self) -> Vec<CatalogPlugin> {
        self.service
            .as_ref()
            .map(PluginService::catalog)
            .unwrap_or_default()
    }

    /// Publishes the installed plugins and the routing of migrated modules
    /// when they changed, and says so. Without an open store every module
    /// is routed to its built-in.
    fn publish(&self) {
        let catalog = catch_unwind(AssertUnwindSafe(|| self.catalog())).unwrap_or_else(|_| {
            warn!("the plugin catalog could not be read");
            Vec::new()
        });
        let routing = catch_unwind(AssertUnwindSafe(|| {
            self.service
                .as_ref()
                .map(PluginService::legacy_routing)
                .unwrap_or_default()
        }))
        .unwrap_or_default();
        let mut changed = match self.published.lock() {
            Ok(mut current) if *current != catalog => {
                *current = catalog;
                true
            }
            _ => false,
        };
        if let Ok(mut current) = self.routing.lock()
            && *current != routing
        {
            info!(?routing, "module routing changed");
            *current = routing;
            changed = true;
        }
        if changed {
            (self.notify)();
        }
    }
}

impl PluginWorker {
    pub fn start(
        settings: PluginSettings,
        data_dir: &Path,
        events: Events,
        notify: Notify,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Message>();
        let catalog = Arc::new(Mutex::new(Vec::new()));
        let routing = Arc::new(Mutex::new(LegacyRouting::default()));
        let values = PublishedVariables::default();
        let mut state = State {
            routing: routing.clone(),
            migrated: false,
            values: values.clone(),
            settings,
            data_dir: data_dir.to_path_buf(),
            events,
            service: None,
            problem: None,
            unlocked: false,
            tx: tx.clone(),
            published: catalog.clone(),
            notify,
        };
        let thread = std::thread::Builder::new()
            .name("plugins".into())
            .spawn(move || {
                while let Ok(message) = rx.recv() {
                    if matches!(message, Message::Stop) {
                        break;
                    }
                    let checks =
                        !matches!(message, Message::Invoke { .. } | Message::KeyState { .. });
                    // A panic in the plugin backend (store, broker,
                    // registry) is logged and answered by dropping the
                    // reply; it never reaches the agent.
                    if catch_unwind(AssertUnwindSafe(|| state.handle(message))).is_err() {
                        warn!("the plugin worker caught a panic; the request failed");
                    }
                    if checks {
                        state.publish();
                    }
                }
                state.close();
                state.publish();
            })
            .map_err(|error| warn!(%error, "the plugin worker could not start"))
            .ok();
        Self {
            tx,
            thread,
            catalog,
            routing,
            values,
            book: super::key_state::StateBook::default(),
        }
    }

    /// The routing of migrated modules, as last published.
    pub fn legacy_routing(&self) -> LegacyRouting {
        self.routing.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Hands the worker what `deckpilot.db` says about the modules that
    /// can migrate.
    pub fn migrate(&self, snapshot: LegacySnapshot) {
        let _ = self.tx.send(Message::Migrate(Box::new(snapshot)));
    }

    /// The plugins' published variable values (for their modules'
    /// variable sources).
    pub fn values(&self) -> PublishedVariables {
        self.values.clone()
    }

    pub fn request(&self, request: Request, reply: oneshot::Sender<Result<Response, IpcError>>) {
        if let Err(mpsc::SendError(Message::Request(_, reply))) =
            self.tx.send(Message::Request(request, reply))
        {
            let _ = reply.send(Err(super::unavailable(&PluginSettings::default())));
        }
    }

    pub fn invoker(&self) -> Invoker {
        Invoker(self.tx.clone(), self.book.clone())
    }

    /// The installed plugins, as last published.
    pub fn catalog(&self) -> Vec<CatalogPlugin> {
        self.catalog.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn open(&self, key: StoreKey) {
        let _ = self.tx.send(Message::Open(key));
    }

    pub fn close(&self) {
        let _ = self.tx.send(Message::Close);
    }

    pub fn rekey(&self, old: StoreKey, new: StoreKey) {
        let _ = self.tx.send(Message::Rekey(old, new));
    }

    pub fn stop(mut self) {
        let _ = self.tx.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            let deadline = std::time::Instant::now() + STOP_WAIT;
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                warn!("the plugin worker did not stop in time");
            }
        }
    }
}
