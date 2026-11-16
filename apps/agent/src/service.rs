//! The agent's core: owns the runtime, devices and runtime state, and
//! processes commands from IPC clients, the tray and device threads on a
//! single task, so no state is shared between threads.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use deckpilot_core::{
    ActionInstanceId, ActionResult, CycleDirection, Engine, EngineError, ExecutionTrigger, KeyFace,
    MachineConfig, ModuleRegistry, Os, ProfileId, ProjectId, Runtime,
};
use deckpilot_device::{
    Capability, CombinedBackend, ControlId, DeviceBackend, DeviceDescriptor, DeviceEvent, DeviceId,
    DeviceModel, ModelCatalog, mock::MockBackend,
};
use deckpilot_ipc::{
    AgentEvent, AgentPaths, AgentStatus, DeviceInfo, DistributionRecord, ErrorCode, IpcError,
    LockReason, PROTOCOL_VERSION, ProfileInfo, ProtectionMode, Request, Response, SecurityState,
    ShutdownReason, token,
};
use deckpilot_keystore::KeyStore;
use deckpilot_storage::policy::AutoLock;
use deckpilot_storage::vault::{DatabaseState, Vault};
use deckpilot_storage::{ConfigStore, DatabaseKey, SqliteStore, StorageError};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use crate::autostart::Autostart;
use crate::devices::{CLOSE_TIMEOUT, DeviceIo, DeviceSignal};
use crate::instance::{InstanceLock, LockError};
use crate::render;

mod deck;
mod executions;
mod gestures;
mod security;
mod stateful;
mod states;
mod triggers;
pub mod variables;
pub mod work_session;
mod work_session_faces;

pub use gestures::GestureHooks;
pub use security::{AUTO_LOCK_CHECK, Clock, system_clock};
pub use work_session::{
    Ignored, PressContext, WorkSessionOperation, WorkSessionOutcome, WorkSessionRequest,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_BRIGHTNESS: u8 = 60;
/// How long stopping waits for the IPC server to close its connections.
const SERVER_STOP: Duration = Duration::from_secs(2);
/// Detached actions (plugin actions from keys and `runAction`) that may run
/// at the same time; more are refused at once. Each plugin also queues at
/// most a few invocations for its host.
const MAX_DETACHED: usize = 16;

/// Rendered key images kept for reuse (about 15 KB each at 72 x 72).
const FRAME_CACHE: usize = 256;

/// Opens the configuration application.
pub type DesktopOpener = Box<dyn Fn() -> Result<(), String> + Send>;
/// Starts the configuration application for one window (by identifier)
/// when no configuration app is connected.
pub type WindowOpener = Box<dyn Fn(&str) -> Result<(), String> + Send>;

/// Window shown when a key is pressed while the configuration is locked.
pub const UNLOCK_WINDOW: &str = "security.unlock";
/// Window with the result of the last workflow.
pub const WORKFLOW_SUMMARY_WINDOW: &str = "workflows.execution-summary";
/// Repeated key presses while locked open the prompt at most this often.
const UNLOCK_PROMPT_INTERVAL: Duration = Duration::from_secs(10);

pub struct AgentOptions {
    pub paths: AgentPaths,
    pub backend: Box<dyn DeviceBackend>,
    /// Present when running with simulated devices; enables `SimulateInput`.
    pub mock: Option<MockBackend>,
    /// How often to look for newly attached or removed devices.
    pub rescan_interval: Duration,
    pub autostart: Option<Autostart>,
    pub open_desktop: Option<DesktopOpener>,
    pub open_window: Option<WindowOpener>,
    /// The operating system's credential store for system-managed keys.
    /// `None`: not available; such databases stay locked.
    pub key_store: Option<Arc<dyn KeyStore>>,
    /// How often auto-lock conditions are checked.
    pub auto_lock_check: Duration,
    /// How long a cancelled workflow may take to stop before it is recorded
    /// as cancelled anyway (an action that cannot be interrupted is still
    /// finishing; its result is discarded).
    pub cancel_grace: Duration,
    pub clock: Clock,
    /// Detects the screen lock, where the platform allows it.
    pub screen_lock: Option<Arc<dyn deckpilot_presence::ScreenLockMonitor>>,
    /// Shows native desktop notifications. `None`: only the app's status
    /// bar shows them.
    pub notifier: Option<Arc<dyn deckpilot_core::notify::Notifier>>,
}

impl std::fmt::Debug for AgentOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOptions")
            .field("paths", &self.paths)
            .field("backend", &self.backend.name())
            .field("rescan_interval", &self.rescan_interval)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("cannot write the client token: {0}")]
    Token(#[source] std::io::Error),
    #[error("cannot listen for clients: {0}")]
    Endpoint(#[source] std::io::Error),
}

/// Commands issued by the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    OpenWindow,
    SelectProfile(ProfileId),
    RefreshDevices,
    Quit,
}

#[derive(Debug)]
pub(crate) enum Command {
    Request {
        request: Request,
        reply: oneshot::Sender<Result<Response, IpcError>>,
    },
    Tray(TrayCommand),
    /// The system key was read (or a configuration set up) on a blocking
    /// thread.
    SystemKey(security::KeyOutcome),
    /// Try to read the system key again after the store was unreachable.
    RetrySystemKey,
    /// Progress or the end of a workflow running in the background.
    Execution(executions::ExecutionUpdate),
    /// A module's answer for a key's live state.
    ButtonState(states::StateUpdate),
    /// The installed plugins or their availability changed: their modules
    /// are registered again.
    PluginsChanged,
    /// A detached action (a plugin's, from a key or `runAction`) finished
    /// on its own thread.
    DetachedFinished {
        action: ActionInstanceId,
        name: String,
        result: Result<ActionResult, deckpilot_core::EngineError>,
        reply: Option<oneshot::Sender<Result<Response, IpcError>>>,
    },
    /// A work session operation from a test (the keys call the service
    /// directly). `press: None` is a press now, in the current state.
    #[cfg(feature = "test-hooks")]
    WorkSession {
        operation: WorkSessionOperation,
        gesture: deckpilot_core::gesture::Gesture,
        press: Option<PressContext>,
        reply: oneshot::Sender<WorkSessionOutcome>,
    },
    /// A tagged run "ended" with `canFinish` (`None`: it failed), e.g. a
    /// late result of an older verification.
    /// When the elapsed time on work session keys is redrawn next.
    #[cfg(feature = "test-hooks")]
    WorkSessionFaceDue {
        reply: oneshot::Sender<Option<i64>>,
    },
    #[cfg(feature = "test-hooks")]
    WorkSessionRunEnded {
        purpose: work_session::RunPurpose,
        can_finish: Option<bool>,
        reply: oneshot::Sender<()>,
    },
}

/// Cloneable access to a running agent.
#[derive(Debug, Clone)]
pub struct AgentHandle {
    commands: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<AgentEvent>,
    /// Plugin progress and state (protocol 10, `subscribePluginEvents`).
    plugin_events: broadcast::Sender<deckpilot_ipc::plugins::PluginEvent>,
    pub(crate) desktop_clients: Arc<AtomicUsize>,
    work_session: watch::Receiver<deckpilot_core::work_session::WorkSessionSnapshot>,
}

impl AgentHandle {
    pub async fn request(&self, request: Request) -> Result<Response, IpcError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(Command::Request { request, reply })
            .map_err(|_| IpcError::new(ErrorCode::Failed, "agent is shutting down"))?;
        response
            .await
            .map_err(|_| IpcError::new(ErrorCode::Failed, "agent is shutting down"))?
    }

    pub fn tray(&self, command: TrayCommand) {
        let _ = self.commands.send(Command::Tray(command));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }

    pub fn subscribe_plugins(&self) -> broadcast::Receiver<deckpilot_ipc::plugins::PluginEvent> {
        self.plugin_events.subscribe()
    }

    /// The work session as last published; changes on every transition.
    pub fn work_session(
        &self,
    ) -> watch::Receiver<deckpilot_core::work_session::WorkSessionSnapshot> {
        self.work_session.clone()
    }

    /// Applies a work session operation as a key gesture would, pressed
    /// now in the current state (`press: None`) or as described. Tests
    /// only: no client can start, verify or finish a session (D11).
    #[cfg(feature = "test-hooks")]
    pub async fn work_session_operation(
        &self,
        operation: WorkSessionOperation,
        gesture: deckpilot_core::gesture::Gesture,
        press: Option<PressContext>,
    ) -> Option<WorkSessionOutcome> {
        let (reply, outcome) = oneshot::channel();
        self.commands
            .send(Command::WorkSession {
                operation,
                gesture,
                press,
                reply,
            })
            .ok()?;
        outcome.await.ok()
    }

    /// When the elapsed time on work session keys is redrawn next
    /// (wall-clock ms of the agent's clock); `None`: never, as no such key
    /// is shown or its timer stands. Tests only.
    #[cfg(feature = "test-hooks")]
    pub async fn work_session_face_due(&self) -> Option<i64> {
        let (reply, due) = oneshot::channel();
        self.commands
            .send(Command::WorkSessionFaceDue { reply })
            .ok()?;
        due.await.ok().flatten()
    }

    /// Hands the work session the end of a tagged run as the execution
    /// service would: succeeded with `canFinish`, or failed (`None`).
    /// Tests only, for results that arrive late.
    #[cfg(feature = "test-hooks")]
    pub async fn work_session_run_ended(
        &self,
        purpose: work_session::RunPurpose,
        can_finish: Option<bool>,
    ) {
        let (reply, done) = oneshot::channel();
        if self
            .commands
            .send(Command::WorkSessionRunEnded {
                purpose,
                can_finish,
                reply,
            })
            .is_ok()
        {
            let _ = done.await;
        }
    }
}

struct DeviceSession {
    id: u64,
    io: DeviceIo,
    rendered: BTreeMap<ControlId, KeyFace>,
    /// The model has adjustable brightness ([`Capability::Brightness`]);
    /// brightness is only sent to such devices.
    brightness: bool,
}

impl DeviceSession {
    fn set_brightness(&self, percent: u8) {
        if self.brightness {
            self.io.set_brightness(percent);
        }
    }
}

pub struct Agent {
    options: AgentOptions,
    _lock: InstanceLock,
    /// Open while unlocked.
    store: Option<SqliteStore>,
    /// Key of the open database, handed to the configuration app on request.
    key: Option<DatabaseKey>,
    security: SecurityState,
    /// How the key is protected, once a database exists.
    protection: Option<ProtectionMode>,
    lock_reason: Option<LockReason>,
    /// Why the database could not be opened automatically.
    security_problem: Option<String>,
    auto_lock: AutoLock,
    /// Seconds since the Unix epoch of the last device input or request
    /// that changed something; wall-clock time, so sleep counts as idle.
    last_activity: u64,
    /// Wall-clock time of the last auto-lock check, to notice sleep.
    last_check: SystemTime,
    /// Whether the screen was locked at the last check.
    screen_was_locked: bool,
    /// A read of the system key is running on a blocking thread.
    key_pending: bool,
    /// Requests waiting for that read.
    key_waiters: Vec<oneshot::Sender<Result<Response, IpcError>>>,
    /// Automatic retries after the credential store was unreachable.
    key_retries: u32,
    runtime: Runtime,
    /// Feature modules with their stored state; the runtime's engine
    /// executes the action types of the enabled ones.
    modules: ModuleRegistry,
    brightness: u8,
    started_at: u64,
    endpoint: String,
    sessions: BTreeMap<DeviceId, DeviceSession>,
    next_session: u64,
    /// Last open error per device, to avoid repeating the same warning.
    open_failures: BTreeMap<DeviceId, String>,
    persisted_project: Option<ProjectId>,
    /// The pages devices show per profile, as last saved (see `deck.rs`).
    saved_pages: Option<String>,
    last_input_at: Option<u64>,
    /// Report of the last workflow, for the summary window.
    last_workflow: Option<deckpilot_core::WorkflowReport>,
    last_unlock_prompt: Option<std::time::Instant>,
    /// Workflows running in the background, and unsaved history.
    executions: executions::Executions,
    /// Schedules and lifecycle events that start workflows.
    triggers: triggers::Triggers,
    /// Live key states from module providers.
    states: states::ButtonStates,
    /// Pending key gestures.
    gestures: gestures::Gestures,
    /// Press-time states and hints of stateful keys.
    stateful: stateful::StatefulKeys,
    /// Rendered key images by face and size.
    frames: render::FrameCache,
    /// Decoded icons of the active profile.
    icons: render::Icons,
    /// Shared variables, read by runs and previews.
    variables: Arc<variables::VariableStore>,
    /// The active project of each profile, for actions that ask.
    active_projects: Arc<variables::ActiveProjects>,
    /// The profile and project last announced to clients.
    context_watch: variables::ContextWatch,
    /// The work session (one per database).
    work: work_session::WorkSessions,
    /// Work session keys shown, and when their elapsed time changes.
    session_faces: work_session_faces::WorkSessionFaces,
    /// Plugins (experimental; answered only with the `plugins` feature
    /// and `--enable-plugins`).
    plugins: crate::plugins::PluginRuntime,
    /// Detached actions (plugin actions from keys and `runAction`) running
    /// on their own threads.
    detached: Arc<AtomicUsize>,
    handle: AgentHandle,
    commands: mpsc::UnboundedReceiver<Command>,
    signals_tx: mpsc::UnboundedSender<DeviceSignal>,
    signals: mpsc::UnboundedReceiver<DeviceSignal>,
    shutdown_tx: watch::Sender<bool>,
    server: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

enum Flow {
    Continue,
    Stop(ShutdownReason),
}

impl Agent {
    /// Opens storage, loads the active profile, writes a fresh client token
    /// and starts listening for IPC clients. Must be called from within a
    /// Tokio runtime.
    pub fn start(options: AgentOptions, lock: InstanceLock) -> Result<Self, AgentError> {
        let paths = &options.paths;
        // The agent starts locked. `run` then opens a database with a
        // system-managed key by itself; a password-protected one waits for
        // the key from the configuration app. Nothing is read before that.
        let security = security_state(&paths.database)?;
        let modules = deckpilot_modules::builtin(Os::current());
        // Built-in models, plus models contributed by registered drivers.
        let mut catalog = ModelCatalog::builtin();
        for model in options.backend.models() {
            let id = model.id.clone();
            if catalog.register_missing(model) {
                info!(model = %id, driver = options.backend.name(), "driver model registered");
            }
        }
        let mut runtime = Runtime::new(
            Engine::new(modules.action_registry()),
            catalog,
            MachineConfig::new(Os::current()),
        );
        runtime.set_placeholder(Some("Locked"));

        let endpoint = paths.endpoint();
        let token = token::create(&paths.token_file).map_err(AgentError::Token)?;
        let listener = endpoint.listen().map_err(AgentError::Endpoint)?;

        let (commands_tx, commands) = mpsc::unbounded_channel();
        let (signals_tx, signals) = mpsc::unbounded_channel();
        // Room for the node events of several running workflows.
        let (events, _) = broadcast::channel(512);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (work, work_session) = work_session::WorkSessions::new((options.clock)());
        let variables = Arc::new(variables::VariableStore::with_work_session(
            variables::WorkSessionSource::new(work_session.clone(), options.clock.clone()),
        ));
        let handle = AgentHandle {
            commands: commands_tx,
            events,
            plugin_events: broadcast::channel(256).0,
            desktop_clients: Arc::new(AtomicUsize::new(0)),
            work_session,
        };
        let server = crate::server::spawn(listener, handle.clone(), token, shutdown_rx);
        info!(endpoint = %endpoint.display(), backend = options.backend.name(), ?security, "agent started");

        let persisted_project = runtime.active_project_id();
        let protection = security::protection_mode(&paths.database);
        let last_check = (options.clock)();
        Ok(Self {
            endpoint: endpoint.display(),
            options,
            _lock: lock,
            store: None,
            key: None,
            lock_reason: (security == SecurityState::Locked).then_some(LockReason::Startup),
            protection,
            security,
            security_problem: None,
            auto_lock: AutoLock::Never,
            last_activity: 0,
            last_check,
            screen_was_locked: false,
            key_pending: false,
            key_waiters: Vec::new(),
            key_retries: 0,
            runtime,
            modules,
            brightness: DEFAULT_BRIGHTNESS,
            started_at: unix_now(),
            sessions: BTreeMap::new(),
            next_session: 0,
            open_failures: BTreeMap::new(),
            persisted_project,
            saved_pages: None,
            last_input_at: None,
            last_workflow: None,
            last_unlock_prompt: None,
            executions: executions::Executions::default(),
            triggers: triggers::Triggers::default(),
            states: states::ButtonStates::default(),
            gestures: gestures::Gestures::default(),
            stateful: stateful::StatefulKeys::default(),
            frames: render::FrameCache::new(FRAME_CACHE),
            icons: render::Icons::default(),
            variables,
            active_projects: Arc::default(),
            context_watch: variables::ContextWatch::default(),
            work,
            session_faces: work_session_faces::WorkSessionFaces::default(),
            plugins: crate::plugins::PluginRuntime::default(),
            detached: Arc::default(),
            handle,
            commands,
            signals_tx,
            signals,
            shutdown_tx,
            server,
        })
    }

    pub fn handle(&self) -> AgentHandle {
        self.handle.clone()
    }

    /// Sets up plugins (before [`Agent::run`]). Without the `plugins`
    /// feature, or with `settings.enabled` off, plugin requests are
    /// answered with "plugins not available".
    #[must_use]
    pub fn with_plugins(mut self, settings: crate::plugins::PluginSettings) -> Self {
        self.plugins.stop();
        let commands = self.handle.commands.clone();
        self.plugins = crate::plugins::PluginRuntime::new(
            settings,
            &self.options.paths.data_dir,
            &self.handle.events,
            &self.handle.plugin_events,
            Arc::new(crate::notifications::AgentNotifier {
                events: self.handle.events.clone(),
                desktop_clients: self.handle.desktop_clients.clone(),
                native: self.options.notifier.clone(),
            }),
            Arc::new(move || {
                let _ = commands.send(Command::PluginsChanged);
            }),
        );
        if let Some(key) = &self.key {
            self.plugins.opened(key);
        }
        self
    }

    /// Runs until `shutdown` completes or a client or the tray requests it.
    pub async fn run(mut self, shutdown: impl Future<Output = ()>) -> Result<(), AgentError> {
        tokio::pin!(shutdown);
        // A database with a system-managed key opens without the user; a new
        // one is set up the same way. The key is read on a blocking thread,
        // so a credential store prompt does not stop the agent. Failures
        // leave a clear locked state.
        self.open_automatically(None);
        // Initial discovery and state for subscribers (e.g. the tray menu).
        self.rescan();
        self.publish();
        let mut rescan = tokio::time::interval(self.options.rescan_interval);
        rescan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        rescan.reset();
        let mut auto_lock = tokio::time::interval(self.options.auto_lock_check);
        auto_lock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        auto_lock.reset();
        loop {
            // Sleeps until the next key state is due; nothing polls per
            // frame.
            let state_due = self.next_state_due().map_or_else(
                || tokio::time::Instant::now() + Duration::from_secs(3600),
                tokio::time::Instant::from_std,
            );
            // Sleeps until the next pending gesture is due, if any.
            let gesture_due = self.next_gesture_deadline().map_or_else(
                || tokio::time::Instant::now() + Duration::from_secs(3600),
                tokio::time::Instant::from_std,
            );
            // Sleeps until the elapsed time on a shown work session key
            // changes, if any.
            let face_due = self.next_work_session_face_due().map_or_else(
                || tokio::time::Instant::now() + Duration::from_secs(3600),
                tokio::time::Instant::from_std,
            );
            let flow = tokio::select! {
                () = &mut shutdown => Flow::Stop(ShutdownReason::Signal),
                Some(command) = self.commands.recv() => self.handle_command(command),
                Some(signal) = self.signals.recv() => {
                    self.handle_signal(signal);
                    Flow::Continue
                }
                _ = rescan.tick() => {
                    self.rescan();
                    Flow::Continue
                }
                _ = auto_lock.tick() => {
                    // Before auto-lock: a sleep is recorded while the
                    // database is still open.
                    self.check_work_session();
                    self.check_auto_lock();
                    self.expire_cancelled_executions();
                    self.tick_schedules();
                    Flow::Continue
                }
                () = tokio::time::sleep_until(state_due) => {
                    self.refresh_states();
                    Flow::Continue
                }
                () = tokio::time::sleep_until(gesture_due) => {
                    self.on_gesture_deadline();
                    Flow::Continue
                }
                () = tokio::time::sleep_until(face_due) => {
                    self.on_work_session_face_due();
                    Flow::Continue
                }
            };
            if let Flow::Stop(reason) = flow {
                self.stop(reason).await;
                return Ok(());
            }
            self.observe_lifecycle();
        }
    }

    async fn stop(mut self, reason: ShutdownReason) {
        info!(?reason, "agent shutting down");
        self.plugins.stop();
        self.work_session_shutdown();
        self.states.reset();
        self.interrupt_executions();
        let _ = self.handle.events.send(AgentEvent::ShuttingDown {
            reason: Some(reason),
        });
        let _ = self.shutdown_tx.send(true);
        // Every device thread blanks its keys at the same time; together
        // they may take at most CLOSE_TIMEOUT.
        let sessions = std::mem::take(&mut self.sessions);
        for session in sessions.values() {
            session
                .io
                .request_close(session.rendered.keys().copied().collect());
        }
        let deadline = std::time::Instant::now() + CLOSE_TIMEOUT;
        for session in sessions.into_values() {
            session.io.finish(deadline);
        }
        // Wait until the listener and every connection are closed, so that
        // an agent started right after (an update, a restart) can create
        // the endpoint again. The server closes them on the shutdown signal.
        if tokio::time::timeout(SERVER_STOP, &mut self.server)
            .await
            .is_err()
        {
            self.server.abort();
            let _ = (&mut self.server).await;
        }
        let _ = std::fs::remove_file(&self.options.paths.token_file);
    }

    #[allow(clippy::too_many_lines)] // One arm per command.
    fn handle_command(&mut self, command: Command) -> Flow {
        match command {
            Command::Request {
                request: Request::SystemUnlock,
                reply,
            } => {
                // Answered once the system key was read.
                self.open_automatically(Some(reply));
                Flow::Continue
            }
            Command::SystemKey(outcome) => {
                self.finish_system_key(outcome);
                Flow::Continue
            }
            Command::RetrySystemKey => {
                self.open_automatically(None);
                Flow::Continue
            }
            Command::Execution(update) => {
                self.handle_execution(update);
                Flow::Continue
            }
            Command::ButtonState(update) => {
                self.handle_button_state(update);
                Flow::Continue
            }
            Command::PluginsChanged => {
                self.plugins_changed();
                Flow::Continue
            }
            Command::DetachedFinished {
                action,
                name,
                result,
                reply,
            } => {
                self.detached_finished(action, &name, result, reply);
                Flow::Continue
            }
            Command::Request {
                request: Request::RunAction { action },
                reply,
            } if self.security == SecurityState::Unlocked && self.runtime.runs_detached(action) => {
                // Answered when the action finished on its own thread.
                self.last_activity = self.now_seconds();
                self.run_detached(
                    action,
                    None,
                    None,
                    deckpilot_core::RunOrigin::App,
                    Some(reply),
                );
                Flow::Continue
            }
            #[cfg(feature = "test-hooks")]
            Command::WorkSession {
                operation,
                gesture,
                press,
                reply,
            } => {
                let press = press.unwrap_or_else(|| {
                    PressContext::of(&self.work_session_snapshot(), std::time::Instant::now())
                });
                let outcome = self.work_session_operation(&WorkSessionRequest {
                    operation,
                    gesture,
                    press,
                });
                let _ = reply.send(outcome);
                Flow::Continue
            }
            #[cfg(feature = "test-hooks")]
            Command::WorkSessionFaceDue { reply } => {
                let _ = reply.send(self.work_session_face_due());
                Flow::Continue
            }
            #[cfg(feature = "test-hooks")]
            Command::WorkSessionRunEnded {
                purpose,
                can_finish,
                reply,
            } => {
                let end = work_session::RunEnd::with_can_finish(can_finish);
                self.work_session_run_ended(purpose, &end);
                let _ = reply.send(());
                Flow::Continue
            }
            Command::Request { request, reply } if crate::plugins::is_plugin_request(&request) => {
                self.plugins
                    .handle(request, reply, self.security == SecurityState::Unlocked);
                Flow::Continue
            }
            Command::Request {
                request: Request::RunWorkflow { workflow },
                reply,
            } => {
                // Answered when the run ends; the agent keeps working.
                self.last_activity = self.now_seconds();
                self.run_execution(workflow, reply);
                Flow::Continue
            }
            Command::Request { request, reply } => {
                let stop = match request {
                    Request::Shutdown => Some(ShutdownReason::Requested),
                    Request::PrepareForUpdate => Some(ShutdownReason::Update),
                    _ => None,
                };
                if matches!(request, Request::PrepareForUpdate) {
                    info!("stopping so that an update can be installed");
                }
                let _ = reply.send(self.handle_request(request));
                stop.map_or(Flow::Continue, Flow::Stop)
            }
            // "Quit DeckPilot": the agent and every open configuration app.
            Command::Tray(TrayCommand::Quit) => Flow::Stop(ShutdownReason::Quit),
            Command::Tray(TrayCommand::OpenWindow) => {
                let opened = if self.security == SecurityState::Locked {
                    self.request_window(UNLOCK_WINDOW)
                } else {
                    self.show_window()
                };
                if let Err(error) = opened {
                    warn!(%error, "cannot open the configuration app");
                }
                Flow::Continue
            }
            Command::Tray(TrayCommand::SelectProfile(profile)) => {
                if let Err(error) = self.set_active_profile(profile) {
                    warn!(%error, "cannot switch profile");
                }
                Flow::Continue
            }
            Command::Tray(TrayCommand::RefreshDevices) => {
                self.refresh_devices();
                Flow::Continue
            }
        }
    }

    fn handle_request(&mut self, request: Request) -> Result<Response, IpcError> {
        // Status queries are frequent; log them only at trace level.
        if matches!(
            request,
            Request::GetStatus
                | Request::ListDevices
                | Request::ListProfiles
                | Request::GetAutostart
        ) {
            tracing::trace!(?request, "request");
        } else {
            debug!(?request, "request");
            self.last_activity = self.now_seconds();
        }
        if self.security != SecurityState::Unlocked
            && matches!(
                request,
                Request::ListProfiles | Request::GetActiveProfile | Request::GetActiveProject
            )
        {
            return Err(locked());
        }
        if let Some(result) = self.handle_work_session_request(&request) {
            return result;
        }
        match request {
            Request::GetStatus => Ok(Response::Status {
                status: Box::new(self.status()),
            }),
            Request::ListDevices => Ok(Response::Devices {
                devices: self.device_infos(),
            }),
            Request::ListProfiles => Ok(Response::Profiles {
                profiles: self.profiles(),
                active: self.runtime.profile().map(|p| p.id),
            }),
            Request::GetActiveProfile => Ok(Response::ActiveProfile {
                profile: self.active_profile_info(),
            }),
            Request::GetActiveProject => Ok(self.active_project_response()),
            Request::ListVariables { context, filter } => self.list_variables(&context, &filter),
            Request::PreviewReferences { context, items } => {
                self.preview_references(&context, &items)
            }
            Request::GetAutostart => {
                let enabled = self.autostart()?.is_enabled().map_err(failed)?;
                Ok(Response::Autostart { enabled })
            }
            // Subscriptions are handled per connection by the server; the
            // run loop stops after replying to a shutdown request.
            Request::Subscribe
            | Request::SubscribePluginEvents { .. }
            | Request::Shutdown
            | Request::PrepareForUpdate => Ok(Response::Ok),
            other => self.handle_change(other),
        }
    }

    /// Requests that change runtime state.
    #[allow(clippy::too_many_lines)] // One arm per request.
    fn handle_change(&mut self, request: Request) -> Result<Response, IpcError> {
        if let Some(result) = self.handle_security(&request) {
            return result;
        }
        match request {
            Request::SetActiveProfile { profile } => {
                self.set_active_profile(profile)?;
                Ok(Response::ActiveProfile {
                    profile: self.active_profile_info(),
                })
            }
            Request::SelectProject { project } => {
                self.require_projects()?;
                self.runtime
                    .select_project(project)
                    .map_err(|e| engine_error(&e))?;
                self.after_change();
                Ok(self.active_project_response())
            }
            Request::CycleProject { direction } => {
                self.cycle_project(direction)?;
                Ok(self.active_project_response())
            }
            Request::RunAction { action } => {
                let mut result = self
                    .runtime
                    .run_action(action)
                    .map_err(|e| engine_error(&e))?;
                let name = self.action_name(action);
                self.apply_deck_request(&mut result);
                self.report_action_as(action, name, &result);
                self.after_change();
                Ok(Response::ActionResult { result })
            }
            Request::StartWorkflow { workflow } => Ok(Response::ExecutionStarted {
                execution: self.start_execution(workflow, ExecutionTrigger::App)?,
            }),
            Request::CancelExecution { execution } => {
                self.cancel_execution(execution)?;
                Ok(Response::Ok)
            }
            Request::ListExecutions { workflow, limit } => Ok(Response::Executions {
                executions: self.list_executions(workflow, limit)?,
            }),
            Request::GetExecution { execution } => Ok(Response::Execution {
                execution: Box::new(self.execution(execution)?),
            }),
            Request::ClearExecutionHistory { workflow } => {
                self.clear_execution_history(workflow)?;
                Ok(Response::Ok)
            }
            Request::RefreshDevices => {
                self.refresh_devices();
                Ok(Response::Ok)
            }
            Request::ReloadConfiguration => {
                self.reload_configuration().map_err(|e| storage_error(&e))?;
                Ok(Response::Ok)
            }
            Request::SetBrightness { percent } => {
                self.set_brightness(percent)?;
                Ok(Response::Ok)
            }
            Request::SetAutostart { enabled } => self.set_autostart(enabled),
            Request::ShowWindow => {
                self.show_window().map_err(failed)?;
                Ok(Response::Ok)
            }
            Request::RequestWindow { window } => {
                self.request_window(&window).map_err(failed)?;
                Ok(Response::Ok)
            }
            Request::GetLastWorkflowReport => Ok(Response::LastWorkflow {
                report: self.last_workflow.clone(),
            }),
            Request::SimulateInput {
                device,
                control,
                input,
            } => {
                self.simulate_input(&device, control, input)?;
                Ok(Response::Ok)
            }
            Request::GetStatus
            | Request::ListDevices
            | Request::ListProfiles
            | Request::GetActiveProfile
            | Request::GetActiveProject
            | Request::ListVariables { .. }
            | Request::PreviewReferences { .. }
            | Request::GetAutostart
            | Request::Subscribe
            | Request::SubscribePluginEvents { .. }
            | Request::Shutdown
            | Request::PrepareForUpdate
            | Request::GetWorkSession
            | Request::ListWorkSessions { .. }
            | Request::GetWorkSessionSettings { .. }
            | Request::SaveWorkSessionSettings { .. }
            | Request::ResolveWorkSessionReview { .. }
            | Request::ClearWorkSessionNotices { .. } => self.handle_request(request),
            Request::RunWorkflow { .. } => unreachable!("handled by handle_command"),
            Request::GetPluginRuntime
            | Request::ListPlugins
            | Request::GetPlugin { .. }
            | Request::ListPluginsPaged
            | Request::GetPluginPaged { .. }
            | Request::GetPluginActions { .. }
            | Request::GetPluginIcon { .. }
            | Request::ListAvailablePlugins { .. }
            | Request::InstallPlugin { .. }
            | Request::EnablePlugin { .. }
            | Request::DisablePlugin { .. }
            | Request::GrantPluginPermissions { .. }
            | Request::RevokePluginPermission { .. }
            | Request::UpdatePlugin { .. }
            | Request::ApprovePluginUpdate { .. }
            | Request::DeclinePluginUpdate { .. }
            | Request::RollbackPlugin { .. }
            | Request::UninstallPlugin { .. }
            | Request::InvokePluginAction { .. }
            | Request::ListKnownPlugins
            | Request::GetPluginSettings { .. }
            | Request::SetPluginSettings { .. }
            | Request::ListPluginCredentials { .. }
            | Request::SetPluginCredential { .. }
            | Request::DeletePluginCredential { .. }
            | Request::ListModuleMigrations
            | Request::DecideModuleMigration { .. }
            | Request::RetryModuleMigration { .. }
            | Request::GrantPluginUserOrigin { .. }
            | Request::RevokePluginUserOrigin { .. } => {
                unreachable!("plugin requests are handled by handle_command")
            }
            Request::Unlock { .. }
            | Request::Lock
            | Request::GetDatabaseKey
            | Request::SystemUnlock
            | Request::UseSystemKey { .. } => {
                unreachable!("handled above")
            }
        }
    }

    fn store(&self) -> Result<&SqliteStore, IpcError> {
        self.store.as_ref().ok_or_else(locked)
    }

    /// Reads machine bindings, brightness, modules and the active profile
    /// after unlocking.
    fn load_configuration(&mut self) -> Result<(), StorageError> {
        let store = self.store.as_ref().ok_or(StorageError::InvalidKey)?;
        let machine = if let Some(machine) = store.load_machine_config()? {
            machine
        } else {
            let machine = MachineConfig::new(Os::current());
            store.save_machine_config(&machine)?;
            machine
        };
        let brightness = store.device_brightness()?.unwrap_or(DEFAULT_BRIGHTNESS);
        self.runtime.set_machine(machine);
        self.recover_executions();
        if brightness != self.brightness {
            self.brightness = brightness;
            for session in self.sessions.values() {
                session.set_brightness(brightness);
            }
        }
        // Before the profile is loaded: devices go to the page they showed.
        self.restore_pages();
        self.reload_configuration()?;
        self.recover_work_session();
        Ok(())
    }

    fn active_project_response(&self) -> Response {
        Response::ActiveProject {
            project: self.runtime.active_project().cloned(),
        }
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), IpcError> {
        self.store()?
            .set_device_brightness(percent)
            .map_err(|e| storage_error(&e))?;
        self.brightness = percent;
        for session in self.sessions.values() {
            session.set_brightness(percent);
        }
        self.publish();
        Ok(())
    }

    fn set_autostart(&self, enabled: bool) -> Result<Response, IpcError> {
        let autostart = self.autostart()?;
        autostart.set_enabled(enabled).map_err(failed)?;
        info!(enabled, "autostart changed");
        Ok(Response::Autostart {
            enabled: autostart.is_enabled().map_err(failed)?,
        })
    }

    fn simulate_input(
        &self,
        device: &DeviceId,
        control: ControlId,
        input: deckpilot_device::ControlInput,
    ) -> Result<(), IpcError> {
        let mock = self.options.mock.as_ref().ok_or_else(|| {
            IpcError::new(
                ErrorCode::Unsupported,
                "the agent is not using mock devices",
            )
        })?;
        if !self.sessions.contains_key(device) {
            return Err(IpcError::new(
                ErrorCode::NotFound,
                "device is not connected",
            ));
        }
        mock.send_input(device, control, input);
        Ok(())
    }

    fn autostart(&self) -> Result<&Autostart, IpcError> {
        self.options.autostart.as_ref().ok_or_else(|| {
            IpcError::new(
                ErrorCode::Unsupported,
                "autostart is not available for this agent",
            )
        })
    }

    /// Has the configuration app open the window `id`: through a connected
    /// app, or by starting the app for that window only. The agent never
    /// draws windows itself and sends nothing but the identifier.
    fn request_window(&self, id: &str) -> Result<(), String> {
        if !deckpilot_ipc::is_window_id(id) {
            return Err(format!("invalid window identifier `{id}`"));
        }
        if self.handle.desktop_clients.load(Ordering::SeqCst) > 0 {
            let _ = self.handle.events.send(AgentEvent::OpenWindow {
                window: id.to_owned(),
            });
            return Ok(());
        }
        match &self.options.open_window {
            Some(open) => open(id),
            None => Err("opening windows is not supported".into()),
        }
    }

    /// A key was pressed while locked: ask to unlock, but not on
    /// every press.
    fn prompt_unlock(&mut self) {
        let due = self
            .last_unlock_prompt
            .is_none_or(|last| last.elapsed() >= UNLOCK_PROMPT_INTERVAL);
        if !due {
            return;
        }
        self.last_unlock_prompt = Some(std::time::Instant::now());
        if let Err(error) = self.request_window(UNLOCK_WINDOW) {
            warn!(%error, "cannot show the unlock prompt");
        }
    }

    fn show_window(&self) -> Result<(), String> {
        if self.handle.desktop_clients.load(Ordering::SeqCst) > 0 {
            let _ = self.handle.events.send(AgentEvent::ShowWindow);
            return Ok(());
        }
        match &self.options.open_desktop {
            Some(open) => open(),
            None => Err("opening the configuration app is not supported".into()),
        }
    }

    fn set_active_profile(&mut self, id: ProfileId) -> Result<(), IpcError> {
        let store = self.store()?;
        let profile = store
            .load_profile(id)
            .map_err(|e| storage_error(&e))?
            .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "profile does not exist"))?;
        store
            .set_active_profile(Some(id))
            .map_err(|e| storage_error(&e))?;
        let project = store.active_project(id).map_err(|e| storage_error(&e))?;
        info!(profile = %profile.name, "profile activated");
        self.states.reset();
        self.invalidate_gestures(deckpilot_core::gesture::GestureCancel::ProfileChanged);
        self.runtime.set_profile(Some(profile), project);
        self.reload_icons();
        self.persisted_project = self.runtime.active_project_id();
        self.after_change();
        Ok(())
    }

    /// Project switching belongs to the projects module and is refused while
    /// that module is disabled.
    fn require_projects(&self) -> Result<(), IpcError> {
        let id = deckpilot_modules::projects::module_id();
        if self.modules.is_enabled(&id) {
            Ok(())
        } else {
            Err(IpcError::new(
                ErrorCode::Failed,
                "the Projects module is disabled",
            ))
        }
    }

    fn cycle_project(&mut self, direction: CycleDirection) -> Result<(), IpcError> {
        self.require_projects()?;
        let profile = self
            .runtime
            .profile()
            .ok_or_else(|| engine_error(&EngineError::NoActiveProfile))?;
        // Same order and wrap-around setting as the projects actions.
        let wrap = deckpilot_modules::projects::switching::wrap_around(&self.modules);
        let target = deckpilot_modules::projects::switching::step(
            profile,
            self.runtime.active_project_id(),
            direction,
            wrap,
        )
        .map_err(|e| IpcError::new(ErrorCode::Failed, e.to_string()))?;
        self.runtime
            .select_project(target)
            .map_err(|e| engine_error(&e))?;
        self.after_change();
        Ok(())
    }

    fn reload_configuration(&mut self) -> Result<(), StorageError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let mut stages = ReloadStages::start();
        if let Some(machine) = store.load_machine_config()?
            && machine != *self.runtime.machine()
        {
            self.runtime.set_machine(machine);
            let _ = self.handle.events.send(AgentEvent::ResourceBindingsChanged);
        }
        stages.done("machine");
        // Module states may have changed in the configuration app.
        let before = self.modules.variables().clone();
        self.modules = load_modules(store)?;
        stages.done("modules");
        self.plugins.migrate(store, &self.modules);
        register_plugin_modules(&mut self.modules, &self.plugins);
        stages.done("plugins");
        self.update_variable_permissions();
        self.publish_module_variables(&before);
        self.reload_variables()?;
        stages.done("variables");
        self.auto_lock = deckpilot_storage::policy::security_policy(store)?.auto_lock;
        self.runtime
            .set_gesture_defaults(deckpilot_storage::gestures::gesture_defaults(store)?);
        let (profile, project) = load_active(store)?;
        // Credentials are loaded with the configuration, so the app asks
        // for a reload after changing one.
        let credentials = store.load_credentials()?;
        stages.done("profile");
        let notifier = crate::notifications::AgentNotifier {
            events: self.handle.events.clone(),
            desktop_clients: self.handle.desktop_clients.clone(),
            native: self.options.notifier.clone(),
        };
        self.runtime.set_engine(
            Engine::new(self.modules.action_registry())
                .with_credentials(std::sync::Arc::new(credentials))
                .with_notifier(std::sync::Arc::new(notifier))
                .with_variables(self.variables.clone())
                .with_live_state(self.variables.clone())
                .with_variable_registry(Arc::new(self.modules.variables().clone()))
                .with_projects(self.active_projects.clone()),
        );
        // Actions, their modules or bindings may have changed: running
        // state calls are cancelled and cached states dropped, and pending
        // gestures are dropped.
        self.states.reset();
        self.invalidate_gestures(deckpilot_core::gesture::GestureCancel::ProfileChanged);
        match profile {
            Some(profile) if self.runtime.profile().is_some_and(|p| p.id == profile.id) => {
                self.runtime.reload_profile(profile);
            }
            other => self.runtime.set_profile(other, project),
        }
        stages.done("engine");
        self.reload_icons();
        self.work_session_settings_reloaded();
        stages.done("faces");
        stages.report();
        info!("configuration reloaded");
        self.after_change();
        Ok(())
    }

    /// Persists the project selection if it changed, redraws devices and
    /// notifies subscribers.
    fn after_change(&mut self) {
        let project = self.runtime.active_project_id();
        if project != self.persisted_project
            && let Some(profile) = self.runtime.profile().map(|p| p.id)
        {
            let saved = match self.store.as_ref() {
                Some(store) => store.set_active_project(profile, project),
                None => Ok(()),
            };
            match saved {
                Ok(()) => self.persisted_project = project,
                Err(error) => warn!(%error, "cannot persist the active project"),
            }
        }
        self.persist_pages();
        // Something happened: live states may refresh once their
        // interval allows.
        self.states.invalidate();
        self.render_all();
        self.publish();
        self.publish_context();
    }

    /// Decodes the active profile's icons (with limits) after it changed.
    /// Rendered frames are dropped, as an icon may have changed.
    fn reload_icons(&mut self) {
        self.icons = self
            .runtime
            .profile()
            .map(render::Icons::from_profile)
            .unwrap_or_default();
        self.frames.clear();
    }

    /// Redraws the keys of every device whose face changed and starts the
    /// key state calls that are due.
    fn render_all(&mut self) {
        let devices: Vec<DeviceId> = self.sessions.keys().cloned().collect();
        for device in devices {
            self.render(&device, false);
        }
        self.refresh_states();
    }

    /// The installed plugins changed: the configuration is reloaded so the
    /// engine registers, drops or re-describes their action types.
    fn plugins_changed(&mut self) {
        if self.store.is_none() {
            return;
        }
        if let Err(error) = self.reload_configuration() {
            warn!(%error, "the configuration could not be reloaded after a plugin change");
        }
    }

    /// Runs a detached action (a plugin's) on a thread of its own and
    /// reports it when it finished: never on the service task, which keeps
    /// serving devices and IPC meanwhile.
    fn run_detached(
        &mut self,
        action: ActionInstanceId,
        overrides: Option<deckpilot_core::ActionParameters>,
        device: Option<DeviceId>,
        origin: deckpilot_core::RunOrigin,
        reply: Option<oneshot::Sender<Result<Response, IpcError>>>,
    ) {
        let run = match self
            .runtime
            .detached_action(action, overrides, device, origin)
        {
            Ok(run) => run,
            Err(error) => {
                warn!(%error, "the action could not start");
                if let Some(reply) = reply {
                    let _ = reply.send(Err(engine_error(&error)));
                }
                return;
            }
        };
        let name = run.name();
        let running = self.detached.clone();
        if running.fetch_add(1, Ordering::SeqCst) >= MAX_DETACHED {
            running.fetch_sub(1, Ordering::SeqCst);
            let result = Ok(ActionResult::error(
                "busy",
                format!("{MAX_DETACHED} actions of plugins are already running; try again later"),
            ));
            self.detached_finished(action, &name, result, reply);
            return;
        }
        let commands = self.handle.commands.clone();
        let spawned = std::thread::Builder::new()
            .name("detached action".into())
            .spawn({
                let running = running.clone();
                let name = name.clone();
                move || {
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.run()))
                            .unwrap_or_else(|_| {
                                Ok(ActionResult::error(
                                    "plugin-host-failed",
                                    "the action failed inside DeckPilot",
                                ))
                            });
                    running.fetch_sub(1, Ordering::SeqCst);
                    let _ = commands.send(Command::DetachedFinished {
                        action,
                        name,
                        result,
                        reply,
                    });
                }
            });
        if let Err(error) = spawned {
            running.fetch_sub(1, Ordering::SeqCst);
            warn!(%error, "no thread for the action");
        }
    }

    fn detached_finished(
        &mut self,
        action: ActionInstanceId,
        name: &str,
        result: Result<ActionResult, EngineError>,
        reply: Option<oneshot::Sender<Result<Response, IpcError>>>,
    ) {
        let answer = match result {
            Ok(result) => {
                info!(action = %name, status = ?result.status, "action executed");
                let _ = self.handle.events.send(AgentEvent::ActionCompleted {
                    action,
                    name: name.to_owned(),
                    status: result.status,
                });
                Ok(Response::ActionResult { result })
            }
            Err(error) => {
                warn!(%error, "the action could not run");
                Err(engine_error(&error))
            }
        };
        if let Some(reply) = reply {
            let _ = reply.send(answer);
        }
        self.after_change();
    }

    /// The name of `action` in the active profile.
    fn action_name(&self, action: ActionInstanceId) -> String {
        self.runtime
            .profile()
            .and_then(|p| p.action(action))
            .map_or_else(|| action.to_string(), |a| a.name.clone())
    }

    fn report_action(&self, action: ActionInstanceId, result: &ActionResult) {
        self.report_action_as(action, self.action_name(action), result);
    }

    /// [`Agent::report_action`] for an action whose `name` was read before
    /// it ran: one that switched the profile is no longer in the active
    /// one.
    fn report_action_as(&self, action: ActionInstanceId, name: String, result: &ActionResult) {
        // The result message may contain paths or other machine details, so
        // only the status is logged.
        info!(action = %name, status = ?result.status, "action executed");
        let _ = self.handle.events.send(AgentEvent::ActionCompleted {
            action,
            name,
            status: result.status,
        });
    }

    fn handle_signal(&mut self, signal: DeviceSignal) {
        match signal {
            DeviceSignal::Input {
                session,
                device,
                control,
                input,
                at,
            } => self.handle_input(session, &device, control, input, at),
            DeviceSignal::Closed {
                session,
                device,
                error,
            } => {
                if self.sessions.get(&device).map(|s| s.id) == Some(session) {
                    if let Some(error) = &error {
                        warn!(%device, %error, "device connection lost");
                    }
                    self.disconnect(&device);
                }
            }
        }
    }

    fn refresh_devices(&mut self) {
        self.rescan();
        for session in self.sessions.values_mut() {
            session.rendered.clear();
        }
        let devices: Vec<DeviceId> = self.sessions.keys().cloned().collect();
        for device in devices {
            self.render(&device, true);
        }
        self.refresh_states();
        self.publish();
    }

    fn rescan(&mut self) {
        let found = match self.options.backend.discover() {
            Ok(found) => found,
            Err(error) => {
                debug!(%error, "device discovery failed");
                return;
            }
        };
        let present: BTreeSet<DeviceId> = found.into_iter().map(|d| d.id).collect();
        let gone: Vec<DeviceId> = self
            .sessions
            .keys()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        for device in gone {
            self.disconnect(&device);
        }
        self.open_failures.retain(|id, _| present.contains(id));
        for device in present {
            if !self.sessions.contains_key(&device) {
                self.connect(device);
            }
        }
    }

    fn connect(&mut self, device: DeviceId) {
        let handle = match self.options.backend.open(&device) {
            Ok(handle) => handle,
            Err(error) => {
                let message = error.to_string();
                if self.open_failures.get(&device) != Some(&message) {
                    warn!(%device, error = %message, "cannot open device");
                    self.open_failures.insert(device, message);
                }
                return;
            }
        };
        self.next_session += 1;
        let io = match DeviceIo::spawn(self.next_session, handle, self.signals_tx.clone()) {
            Ok(io) => io,
            Err(error) => {
                warn!(%device, %error, "cannot start device thread");
                return;
            }
        };
        self.open_failures.remove(&device);
        let session = DeviceSession {
            id: self.next_session,
            io,
            rendered: BTreeMap::new(),
            brightness: self
                .runtime
                .catalog()
                .get(&device.model)
                .is_some_and(|model| model.capabilities.supports(Capability::Brightness)),
        };
        session.set_brightness(self.brightness);
        self.sessions.insert(device.clone(), session);
        let _ = self.runtime.handle_event(&DeviceEvent::Connected {
            device: device.clone(),
        });
        info!(%device, "device connected");
        self.render(&device, true);
        self.refresh_states();
        self.publish();
        self.publish_device_context(Some(device));
    }

    fn disconnect(&mut self, device: &DeviceId) {
        if let Some(session) = self.sessions.remove(device) {
            session.io.close(Vec::new(), CLOSE_TIMEOUT);
        }
        self.forget_gestures(device);
        self.states.forget_device(device);
        self.session_faces.forget_device(device);
        let _ = self.runtime.handle_event(&DeviceEvent::Disconnected {
            device: device.clone(),
        });
        info!(%device, "device disconnected");
        self.publish();
        self.publish_device_context(Some(device.clone()));
    }

    /// Sends images for every control whose face changed (or all, if
    /// `force`). Faces include live states; images come from the frame
    /// cache, so only new faces are drawn.
    fn render(&mut self, device: &DeviceId, force: bool) {
        let mut faces = self.runtime.faces(device);
        if self.sessions.contains_key(device) {
            self.decorate_faces(device, &mut faces, std::time::Instant::now());
        }
        let Some(model) = self.runtime.catalog().get(&device.model) else {
            return;
        };
        let capabilities = model.capabilities.clone();
        let Some(session) = self.sessions.get_mut(device) else {
            return;
        };
        let mut updates = Vec::new();
        for (control, face) in faces {
            if !force && session.rendered.get(&control) == Some(&face) {
                continue;
            }
            let size = match control {
                ControlId::Key { .. } => capabilities.keys.and_then(|k| k.image),
                ControlId::TouchZone { .. } => capabilities.touch.as_ref().and_then(|t| t.image),
                ControlId::Encoder { .. } => None,
            };
            let Some(spec) = size else { continue };
            let frame = (!face.is_empty()).then(|| {
                self.frames
                    .frame(&face, spec.width, spec.height, &self.icons)
            });
            updates.push((control, frame));
            session.rendered.insert(control, face);
        }
        session.io.set_images(updates);
    }

    fn profiles(&self) -> Vec<ProfileInfo> {
        let Some(store) = self.store.as_ref() else {
            return Vec::new();
        };
        store
            .list_profiles()
            .map(|list| {
                list.into_iter()
                    .map(|p| ProfileInfo {
                        id: p.id,
                        name: p.name,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn active_profile_info(&self) -> Option<ProfileInfo> {
        self.runtime.profile().map(|p| ProfileInfo {
            id: p.id,
            name: p.name.clone(),
        })
    }

    /// The vendor-neutral descriptor of a known device: model data from the
    /// catalog, driver from the backend that provides it.
    pub(crate) fn device_descriptor(&self, device: &DeviceId) -> Option<DeviceDescriptor> {
        self.runtime
            .catalog()
            .describe(device, Some(self.options.backend.driver(device)))
    }

    fn device_infos(&self) -> Vec<DeviceInfo> {
        self.runtime
            .devices()
            .statuses()
            .into_iter()
            .map(|status| {
                let model = self.runtime.catalog().get(&status.device.model);
                let page = self.runtime.current_page(&status.device).and_then(|page| {
                    self.runtime
                        .layout_for(&status.device.model)
                        .and_then(|layout| layout.page(page))
                        .map(|page| page.name.clone())
                });
                DeviceInfo {
                    descriptor: self.device_descriptor(&status.device),
                    model_name: model
                        .map_or_else(|| status.device.model.to_string(), |m| m.name.clone()),
                    layout: self.runtime.layout_status(&status.device.model),
                    state: status.state,
                    page,
                    source: self.options.backend.source(&status.device).into(),
                    id: status.device,
                }
            })
            .collect()
    }

    fn status(&self) -> AgentStatus {
        AgentStatus {
            version: VERSION.into(),
            protocol: PROTOCOL_VERSION,
            pid: std::process::id(),
            started_at: self.started_at,
            data_dir: self.options.paths.data_dir.display().to_string(),
            log_dir: self.options.paths.log_dir.display().to_string(),
            endpoint: self.endpoint.clone(),
            backend: self.options.backend.name().into(),
            profiles: self.profiles(),
            active_profile: self.active_profile_info(),
            active_project: self.runtime.active_project().cloned(),
            devices: self.device_infos(),
            brightness: self.brightness,
            last_input_at: self.last_input_at,
            security: self.security,
            protection: self.protection,
            lock_reason: self.lock_reason,
            security_problem: self.security_problem.clone(),
            waiting_for_key_store: self.key_pending,
            screen_lock_detection: self.options.screen_lock.is_some(),
            running_executions: Some(
                u32::try_from(self.executions.running_count()).unwrap_or(u32::MAX),
            ),
            distribution: Some(DistributionRecord::current()),
        }
    }

    fn publish(&self) {
        let _ = self.handle.events.send(AgentEvent::State {
            status: Box::new(self.status()),
        });
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn locked() -> IpcError {
    IpcError::new(ErrorCode::Locked, "the configuration is locked")
}

/// The agent's state for the database at `path`, without opening it.
fn security_state(path: &std::path::Path) -> Result<SecurityState, StorageError> {
    Ok(match Vault::new(path).state()? {
        DatabaseState::Missing => SecurityState::Uninitialized,
        DatabaseState::Plaintext => SecurityState::LegacyPlaintext,
        DatabaseState::Encrypted => SecurityState::Locked,
    })
}

/// A reload longer than this is reported with the time of each stage: the
/// app waits a few seconds for a reload after a save, and nothing else is
/// answered while one runs.
const SLOW_RELOAD: Duration = Duration::from_millis(500);

/// How long each stage of a configuration reload took, so a slow one says
/// where the time went (the database, the plugin worker, rendering).
struct ReloadStages {
    started: std::time::Instant,
    last: std::time::Instant,
    stages: Vec<(&'static str, Duration)>,
}

impl ReloadStages {
    fn start() -> Self {
        let now = std::time::Instant::now();
        Self {
            started: now,
            last: now,
            stages: Vec::new(),
        }
    }

    /// The stage `name` ended now.
    fn done(&mut self, name: &'static str) {
        let now = std::time::Instant::now();
        self.stages.push((name, now.duration_since(self.last)));
        self.last = now;
    }

    /// `machine 1 ms, modules 5210 ms, …`.
    fn describe(&self) -> String {
        self.stages
            .iter()
            .map(|(name, took)| format!("{name} {} ms", took.as_millis()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Logs the stages: a warning for a slow reload, otherwise at debug.
    fn report(&self) {
        let total = self.last.duration_since(self.started);
        let stages = self.describe();
        if total >= SLOW_RELOAD {
            warn!(total_ms = total.as_millis(), %stages, "the configuration reload was slow");
        } else {
            debug!(total_ms = total.as_millis(), %stages, "configuration reload stages");
        }
    }
}

fn load_modules(store: &SqliteStore) -> Result<ModuleRegistry, StorageError> {
    let (modules, problems) = crate::plugins::configured_modules(&store.module_states()?);
    // Schemas of all registered modules, enabled or not.
    store.apply_module_schemas(&modules.schemas())?;
    for problem in problems {
        warn!(%problem, "stored module state was not applied");
    }
    Ok(modules)
}

/// Registers one module per installed plugin, after the built-in ones (a
/// plugin id has a `.`, so it never collides with a built-in id). Their
/// state comes from the plugin store, never from the `modules` table. The
/// legacy action types of migrated modules are routed as the migration
/// engine decided (`plugins/migration.rs`).
fn register_plugin_modules(modules: &mut ModuleRegistry, plugins: &crate::plugins::PluginRuntime) {
    modules.set_legacy_routing(plugins.legacy_routing());
    for module in plugins.modules() {
        let id = module.manifest().id.clone();
        if let Err(error) = modules.register(module) {
            warn!(plugin = %id, %error, "a plugin's actions cannot be registered");
        }
    }
}

fn load_active(
    store: &SqliteStore,
) -> Result<(Option<deckpilot_core::Profile>, Option<ProjectId>), StorageError> {
    let Some(id) = store.active_profile()? else {
        return Ok((None, None));
    };
    let profile = store.load_profile(id)?;
    let project = store.active_project(id)?;
    Ok((profile, project))
}

fn engine_error(error: &EngineError) -> IpcError {
    let code = match error {
        EngineError::UnknownAction(_)
        | EngineError::UnknownWorkflow(_)
        | EngineError::UnknownProject(_) => ErrorCode::NotFound,
        EngineError::NoActiveProfile | EngineError::Context(_) => ErrorCode::Failed,
        EngineError::InvalidWorkflow(_) => ErrorCode::InvalidRequest,
    };
    IpcError::new(code, error.to_string())
}

fn storage_error(error: &StorageError) -> IpcError {
    warn!(%error, "storage error");
    IpcError::new(ErrorCode::Failed, error.to_string())
}

fn failed(message: String) -> IpcError {
    IpcError::new(ErrorCode::Failed, message)
}

/// Options used by `main`: USB devices through the HID driver (unless
/// `usb` is false, e.g. in tests that must not touch a plugged-in Stream
/// Deck), plus one simulated device per model in `mock_models`
/// (development without hardware).
pub fn default_options(paths: AgentPaths, mock_models: &[DeviceModel], usb: bool) -> AgentOptions {
    let mut backend = CombinedBackend::default();
    if usb {
        backend = backend.with(Box::new(deckpilot_hid::ElgatoBackend::new()));
    }
    let mock = (!mock_models.is_empty()).then(|| {
        let mock = MockBackend::new();
        for (index, model) in mock_models.iter().enumerate() {
            mock.plug(
                model,
                &format!(
                    "SIM-{}-{:02}",
                    model
                        .id
                        .as_str()
                        .rsplit('.')
                        .next()
                        .unwrap_or("dev")
                        .to_uppercase(),
                    index + 1
                ),
            );
        }
        mock
    });
    if let Some(mock) = &mock {
        backend = backend.with(Box::new(mock.clone()));
    }
    let autostart = match Autostart::for_current_exe(deckpilot_distribution::current()) {
        Ok(autostart) => {
            if let Err(error) = autostart.repair() {
                warn!(%error, "cannot update the autostart entry");
            }
            Some(autostart)
        }
        Err(error) => {
            warn!(%error, "autostart is unavailable");
            None
        }
    };
    AgentOptions {
        paths,
        backend: Box::new(backend),
        mock,
        rescan_interval: Duration::from_secs(2),
        autostart,
        open_desktop: Some(Box::new(crate::launcher::open_desktop)),
        open_window: Some(Box::new(crate::launcher::open_window)),
        key_store: security::system_key_store(),
        auto_lock_check: AUTO_LOCK_CHECK,
        cancel_grace: executions::CANCEL_GRACE,
        clock: options_clock(),
        screen_lock: deckpilot_presence::screen_lock_monitor().map(Arc::from),
        notifier: Some(crate::notifications::native()),
    }
}

/// The agent's clock: the system's. Test only (feature `test-hooks`, never
/// read by a release build): with `DECKPILOT_TEST_CLOCK_OFFSET` naming a
/// file, the clock runs as many seconds ahead as the file says (read at
/// every look at the clock), so an end-to-end test of a separate agent
/// process can make a schedule due without waiting for it.
fn options_clock() -> Clock {
    #[cfg(feature = "test-hooks")]
    if let Some(path) = std::env::var_os("DECKPILOT_TEST_CLOCK_OFFSET") {
        let path = std::path::PathBuf::from(path);
        return Arc::new(move || {
            let ahead = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.trim().parse::<u64>().ok())
                .unwrap_or(0);
            SystemTime::now() + Duration::from_secs(ahead)
        });
    }
    system_clock()
}

/// Completes on Ctrl+C, and on SIGTERM on Unix.
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod reload_stage_tests {
    use super::*;

    #[test]
    fn stages_are_named_in_order_with_their_times() {
        let mut stages = ReloadStages::start();
        stages.done("machine");
        std::thread::sleep(Duration::from_millis(20));
        stages.done("modules");
        let described = stages.describe();
        let (machine, modules) = described.split_once(", ").unwrap();
        assert!(machine.starts_with("machine ") && machine.ends_with(" ms"));
        let took: u64 = modules
            .strip_prefix("modules ")
            .and_then(|m| m.strip_suffix(" ms"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(took >= 20, "{described}");
        assert!(stages.last.duration_since(stages.started) >= Duration::from_millis(20));
    }
}
