//! The agent's core: owns the runtime, devices and runtime state, and
//! processes commands from IPC clients, the tray and device threads on a
//! single task, so no state is shared between threads.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use deckpilot_core::{
    ActionInstanceId, ActionRegistry, ActionResult, CycleDirection, Engine, EngineError, KeyFace,
    MachineConfig, Os, ProfileId, ProjectId, Runtime, RuntimeOutcome,
};
use deckpilot_device::{
    ControlId, DeviceBackend, DeviceEvent, DeviceId, ModelCatalog, mock::MockBackend,
};
use deckpilot_ipc::{
    AgentEvent, AgentPaths, AgentStatus, DeviceInfo, ErrorCode, IpcError, PROTOCOL_VERSION,
    ProfileInfo, Request, Response, token,
};
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore, StorageError};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use crate::autostart::Autostart;
use crate::devices::{DeviceIo, DeviceSignal};
use crate::instance::{InstanceLock, LockError};
use crate::render;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_BRIGHTNESS: u8 = 60;

/// Opens the configuration application.
pub type DesktopOpener = Box<dyn Fn() -> Result<(), String> + Send>;

pub struct AgentOptions {
    pub paths: AgentPaths,
    pub backend: Box<dyn DeviceBackend>,
    /// Present when running with simulated devices; enables `SimulateInput`.
    pub mock: Option<MockBackend>,
    /// How often to look for newly attached or removed devices.
    pub rescan_interval: Duration,
    pub autostart: Option<Autostart>,
    pub open_desktop: Option<DesktopOpener>,
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
}

/// Cloneable access to a running agent.
#[derive(Debug, Clone)]
pub struct AgentHandle {
    commands: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<AgentEvent>,
    pub(crate) desktop_clients: Arc<AtomicUsize>,
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
}

struct DeviceSession {
    id: u64,
    io: DeviceIo,
    rendered: BTreeMap<ControlId, KeyFace>,
}

pub struct Agent {
    options: AgentOptions,
    _lock: InstanceLock,
    store: SqliteStore,
    runtime: Runtime,
    brightness: u8,
    started_at: u64,
    endpoint: String,
    sessions: BTreeMap<DeviceId, DeviceSession>,
    next_session: u64,
    /// Last open error per device, to avoid repeating the same warning.
    open_failures: BTreeMap<DeviceId, String>,
    persisted_project: Option<ProjectId>,
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
    Stop,
}

impl Agent {
    /// Opens storage, loads the active profile, writes a fresh client token
    /// and starts listening for IPC clients. Must be called from within a
    /// Tokio runtime.
    pub fn start(options: AgentOptions, lock: InstanceLock) -> Result<Self, AgentError> {
        let paths = &options.paths;
        let store = SqliteStore::open(&paths.database, &Encryption::None)?;
        let machine = if let Some(machine) = store.load_machine_config()? {
            machine
        } else {
            let machine = MachineConfig::new(Os::current());
            store.save_machine_config(&machine)?;
            machine
        };
        let brightness = store.device_brightness()?.unwrap_or(DEFAULT_BRIGHTNESS);
        let mut runtime = Runtime::new(
            Engine::new(ActionRegistry::with_builtins()),
            ModelCatalog::builtin(),
            machine,
        );
        let (profile, project) = load_active(&store)?;
        runtime.set_profile(profile, project);

        let endpoint = paths.endpoint();
        let token = token::create(&paths.token_file)?;
        let listener = endpoint.listen()?;

        let (commands_tx, commands) = mpsc::unbounded_channel();
        let (signals_tx, signals) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(64);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = AgentHandle {
            commands: commands_tx,
            events,
            desktop_clients: Arc::new(AtomicUsize::new(0)),
        };
        let server = crate::server::spawn(listener, handle.clone(), token, shutdown_rx);
        info!(endpoint = %endpoint.display(), backend = options.backend.name(), "agent started");

        let persisted_project = runtime.active_project_id();
        Ok(Self {
            endpoint: endpoint.display(),
            options,
            _lock: lock,
            store,
            runtime,
            brightness,
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            sessions: BTreeMap::new(),
            next_session: 0,
            open_failures: BTreeMap::new(),
            persisted_project,
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

    /// Runs until `shutdown` completes or a client or the tray requests it.
    pub async fn run(mut self, shutdown: impl Future<Output = ()>) -> Result<(), AgentError> {
        tokio::pin!(shutdown);
        // Initial discovery and state for subscribers (e.g. the tray menu).
        self.rescan();
        self.publish();
        let mut rescan = tokio::time::interval(self.options.rescan_interval);
        rescan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        rescan.reset();
        loop {
            let flow = tokio::select! {
                () = &mut shutdown => Flow::Stop,
                Some(command) = self.commands.recv() => self.handle_command(command),
                Some(signal) = self.signals.recv() => {
                    self.handle_signal(signal);
                    Flow::Continue
                }
                _ = rescan.tick() => {
                    self.rescan();
                    Flow::Continue
                }
            };
            if matches!(flow, Flow::Stop) {
                break;
            }
        }
        self.stop();
        Ok(())
    }

    fn stop(mut self) {
        info!("agent shutting down");
        let _ = self.handle.events.send(AgentEvent::ShuttingDown);
        let _ = self.shutdown_tx.send(true);
        for (_, session) in std::mem::take(&mut self.sessions) {
            let controls = session.rendered.keys().copied().collect();
            session.io.close(controls);
        }
        self.server.abort();
        let _ = std::fs::remove_file(&self.options.paths.token_file);
    }

    fn handle_command(&mut self, command: Command) -> Flow {
        match command {
            Command::Request { request, reply } => {
                let stop = matches!(request, Request::Shutdown);
                let _ = reply.send(self.handle_request(request));
                if stop { Flow::Stop } else { Flow::Continue }
            }
            Command::Tray(TrayCommand::Quit) => Flow::Stop,
            Command::Tray(TrayCommand::OpenWindow) => {
                if let Err(error) = self.show_window() {
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
        debug!(?request, "request");
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
            Request::GetAutostart => {
                let enabled = self.autostart()?.is_enabled().map_err(failed)?;
                Ok(Response::Autostart { enabled })
            }
            // Subscriptions are handled per connection by the server; the
            // run loop stops after replying to a shutdown request.
            Request::Subscribe | Request::Shutdown => Ok(Response::Ok),
            other => self.handle_change(other),
        }
    }

    /// Requests that change runtime state.
    fn handle_change(&mut self, request: Request) -> Result<Response, IpcError> {
        match request {
            Request::SetActiveProfile { profile } => {
                self.set_active_profile(profile)?;
                Ok(Response::ActiveProfile {
                    profile: self.active_profile_info(),
                })
            }
            Request::SelectProject { project } => {
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
                let result = self
                    .runtime
                    .run_action(action)
                    .map_err(|e| engine_error(&e))?;
                self.report_action(action, &result);
                self.after_change();
                Ok(Response::ActionResult { result })
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
            | Request::GetAutostart
            | Request::Subscribe
            | Request::Shutdown => self.handle_request(request),
        }
    }

    fn active_project_response(&self) -> Response {
        Response::ActiveProject {
            project: self.runtime.active_project().cloned(),
        }
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), IpcError> {
        self.store
            .set_device_brightness(percent)
            .map_err(|e| storage_error(&e))?;
        self.brightness = percent;
        for session in self.sessions.values() {
            session.io.set_brightness(percent);
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
        let profile = self
            .store
            .load_profile(id)
            .map_err(|e| storage_error(&e))?
            .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "profile does not exist"))?;
        self.store
            .set_active_profile(Some(id))
            .map_err(|e| storage_error(&e))?;
        let project = self
            .store
            .active_project(id)
            .map_err(|e| storage_error(&e))?;
        info!(profile = %profile.name, "profile activated");
        self.runtime.set_profile(Some(profile), project);
        self.persisted_project = self.runtime.active_project_id();
        self.after_change();
        Ok(())
    }

    fn cycle_project(&mut self, direction: CycleDirection) -> Result<(), IpcError> {
        self.runtime
            .cycle_project(direction)
            .map_err(|e| engine_error(&e))?;
        self.after_change();
        Ok(())
    }

    fn reload_configuration(&mut self) -> Result<(), StorageError> {
        if let Some(machine) = self.store.load_machine_config()? {
            self.runtime.set_machine(machine);
        }
        let (profile, project) = load_active(&self.store)?;
        match profile {
            Some(profile) if self.runtime.profile().is_some_and(|p| p.id == profile.id) => {
                self.runtime.reload_profile(profile);
            }
            other => self.runtime.set_profile(other, project),
        }
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
            match self.store.set_active_project(profile, project) {
                Ok(()) => self.persisted_project = project,
                Err(error) => warn!(%error, "cannot persist the active project"),
            }
        }
        let devices: Vec<DeviceId> = self.sessions.keys().cloned().collect();
        for device in devices {
            self.render(&device, false);
        }
        self.publish();
    }

    fn report_action(&self, action: ActionInstanceId, result: &ActionResult) {
        let name = self
            .runtime
            .profile()
            .and_then(|p| p.action(action))
            .map_or_else(|| action.to_string(), |a| a.name.clone());
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
            } => {
                if self.sessions.get(&device).map(|s| s.id) != Some(session) {
                    return;
                }
                let event = DeviceEvent::Input {
                    device: device.clone(),
                    control,
                    input,
                };
                match self.runtime.handle_event(&event) {
                    Ok(RuntimeOutcome::Action { action, result }) => {
                        self.report_action(action, &result);
                    }
                    Ok(RuntimeOutcome::Workflow { report }) => {
                        for step in &report.steps {
                            self.report_action(step.action, &step.result);
                        }
                    }
                    Ok(RuntimeOutcome::Ignored { reason }) => {
                        debug!(%device, %control, %reason, "input ignored");
                        return;
                    }
                    Ok(_) => {}
                    Err(error) => warn!(%error, "input could not be handled"),
                }
                self.after_change();
            }
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
        io.set_brightness(self.brightness);
        self.sessions.insert(
            device.clone(),
            DeviceSession {
                id: self.next_session,
                io,
                rendered: BTreeMap::new(),
            },
        );
        let _ = self.runtime.handle_event(&DeviceEvent::Connected {
            device: device.clone(),
        });
        info!(%device, "device connected");
        self.render(&device, true);
        self.publish();
    }

    fn disconnect(&mut self, device: &DeviceId) {
        if let Some(session) = self.sessions.remove(device) {
            session.io.close(Vec::new());
        }
        let _ = self.runtime.handle_event(&DeviceEvent::Disconnected {
            device: device.clone(),
        });
        info!(%device, "device disconnected");
        self.publish();
    }

    /// Sends images for every control whose face changed (or all, if `force`).
    fn render(&mut self, device: &DeviceId, force: bool) {
        let faces = self.runtime.faces(device);
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
            let frame = (!face.is_empty()).then(|| render::render(&face, spec.width, spec.height));
            updates.push((control, frame));
            session.rendered.insert(control, face);
        }
        session.io.set_images(updates);
    }

    fn profiles(&self) -> Vec<ProfileInfo> {
        self.store
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
                    model_name: model
                        .map_or_else(|| status.device.model.to_string(), |m| m.name.clone()),
                    layout: self.runtime.layout_status(&status.device.model),
                    state: status.state,
                    page,
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
        }
    }

    fn publish(&self) {
        let _ = self.handle.events.send(AgentEvent::State {
            status: Box::new(self.status()),
        });
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

/// Used by `main` to build options with the real HID backend or simulated
/// devices. A simulated MK.2 is attached when `mock` is set.
pub fn default_options(paths: AgentPaths, mock: bool) -> AgentOptions {
    let (backend, mock_backend): (Box<dyn DeviceBackend>, Option<MockBackend>) = if mock {
        let backend = MockBackend::new();
        backend.plug(
            &deckpilot_device::models::stream_deck_mk2(),
            "MOCK-MK2-0001",
        );
        (Box::new(backend.clone()), Some(backend))
    } else {
        (Box::new(deckpilot_hid::ElgatoBackend::new()), None)
    };
    let autostart = match Autostart::for_current_exe() {
        Ok(autostart) => Some(autostart),
        Err(error) => {
            warn!(%error, "autostart is unavailable");
            None
        }
    };
    AgentOptions {
        paths,
        backend,
        mock: mock_backend,
        rescan_interval: Duration::from_secs(2),
        autostart,
        open_desktop: Some(Box::new(crate::launcher::open_desktop)),
    }
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
