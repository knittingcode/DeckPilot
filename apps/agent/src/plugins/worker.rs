//! The plugin worker thread. It owns the [`PluginService`] while the
//! configuration is unlocked and answers plugin requests one at a time, so
//! installs, verifications and invocations (which block on the plugin
//! host) never run on the agent's service task.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use deckpilot_ipc::plugins::PluginEvent;
use deckpilot_ipc::{AgentEvent, IpcError, Request, Response};
use deckpilot_plugin_store::StoreKey;
use tokio::sync::{broadcast, oneshot};
use tracing::{info, warn};

use super::PluginSettings;
use super::service::{PluginService, closed_status, open_problem, store_unavailable};

/// How long stopping waits for the worker (hosts get a grace period each).
const STOP_WAIT: Duration = Duration::from_secs(10);

/// Where the plugin runtime reports: agent events (notifications) and
/// plugin events (progress, state).
#[derive(Debug, Clone)]
pub(crate) struct Events {
    pub agent: broadcast::Sender<AgentEvent>,
    pub plugins: broadcast::Sender<PluginEvent>,
}

enum Message {
    Open(StoreKey),
    Close,
    Rekey(StoreKey, StoreKey),
    Request(Request, oneshot::Sender<Result<Response, IpcError>>),
    Stop,
}

#[derive(Debug)]
pub(super) struct PluginWorker {
    tx: mpsc::Sender<Message>,
    thread: Option<JoinHandle<()>>,
}

struct State {
    settings: PluginSettings,
    data_dir: PathBuf,
    events: Events,
    service: Option<PluginService>,
    /// Why the store did not open (wrong key, newer schema…).
    problem: Option<String>,
    unlocked: bool,
}

impl State {
    fn open(&mut self, key: &StoreKey) {
        self.close();
        self.unlocked = true;
        match PluginService::open(&self.settings, &self.data_dir, key, self.events.clone()) {
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

    fn request(&mut self, request: Request) -> Result<Response, IpcError> {
        match (&mut self.service, request) {
            (Some(service), request) => service.handle(request),
            (None, Request::GetPluginRuntime) => Ok(Response::PluginRuntime {
                status: Box::new(closed_status(self.problem.as_deref(), self.unlocked)),
            }),
            (None, _) => Err(match &self.problem {
                Some(problem) => store_unavailable(problem),
                None => super::locked(),
            }),
        }
    }
}

impl PluginWorker {
    pub fn start(settings: PluginSettings, data_dir: &Path, events: Events) -> Self {
        let (tx, rx) = mpsc::channel::<Message>();
        let mut state = State {
            settings,
            data_dir: data_dir.to_path_buf(),
            events,
            service: None,
            problem: None,
            unlocked: false,
        };
        let thread = std::thread::Builder::new()
            .name("plugins".into())
            .spawn(move || {
                while let Ok(message) = rx.recv() {
                    match message {
                        Message::Open(key) => state.open(&key),
                        Message::Close => state.close(),
                        Message::Rekey(old, new) => state.rekey(&old, &new),
                        Message::Request(request, reply) => {
                            let _ = reply.send(state.request(request));
                        }
                        Message::Stop => break,
                    }
                }
                state.close();
            })
            .map_err(|error| warn!(%error, "the plugin worker could not start"))
            .ok();
        Self { tx, thread }
    }

    pub fn request(&self, request: Request, reply: oneshot::Sender<Result<Response, IpcError>>) {
        if let Err(mpsc::SendError(Message::Request(_, reply))) =
            self.tx.send(Message::Request(request, reply))
        {
            let _ = reply.send(Err(super::unavailable(&PluginSettings::default())));
        }
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
