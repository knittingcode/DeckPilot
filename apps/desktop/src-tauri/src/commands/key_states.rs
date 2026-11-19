//! Live key states for the layout editor's preview, from the agent.
//!
//! The agent is the only place that reads them: a key's state (e.g. a CPU
//! monitor's value) is what its module's provider answered for the device,
//! and the preview asks the agent for the same states (`watchKeyStates`,
//! protocol 11) instead of reading anything itself. One watch at a time:
//! the editor watches the actions of the page it shows, and a new watch
//! (another page, profile or project) replaces the old one. The watch is a
//! connection of its own, so it ends when the editor stops it, when this
//! app ends, or when the agent goes away; it is set up again when the agent
//! comes back.
//!
//! The frontend gets [`KEY_STATES_EVENT`]s tagged with the watch's number,
//! so a late event of a replaced watch is ignored.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

use deckpilot_core::{ActionInstanceId, ProfileId};
use deckpilot_ipc::{AgentClient, AgentPaths, Capability, ClientError, ClientKind, KeyStateEntry};

use crate::state::AppState;

/// Event with the live key states of the watched page ([`KeyStatesEvent`]).
pub const KEY_STATES_EVENT: &str = "key-states";

/// How long to wait before connecting again after the agent went away.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// What the preview learns about the watched keys.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum KeyStatesUpdate {
    /// Every watched key with a live state, as it is now. `live`: the
    /// profile is the agent's active one (only its keys have live states).
    Snapshot {
        live: bool,
        states: Vec<KeyStateEntry>,
    },
    /// Keys whose state changed.
    Changed { states: Vec<KeyStateEntry> },
    /// The agent is not running or went away; the last values are stale.
    Disconnected,
    /// The agent predates live key states (protocol 11).
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatesEvent {
    /// The watch it belongs to, from [`watch_key_states`].
    pub watch: u64,
    #[serde(flatten)]
    pub update: KeyStatesUpdate,
}

/// The running watch, if any.
#[derive(Debug, Default)]
pub struct KeyStateWatch {
    inner: Mutex<WatchState>,
}

#[derive(Debug, Default)]
struct WatchState {
    number: u64,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl KeyStateWatch {
    /// Ends the running watch and returns the number of the next one.
    fn replace(&self) -> u64 {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(task) = state.task.take() {
            task.abort();
        }
        state.number += 1;
        state.number
    }

    fn start(&self, number: u64, task: tauri::async_runtime::JoinHandle<()>) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.number == number {
            state.task = Some(task);
        } else {
            task.abort();
        }
    }
}

/// Watches the live states of `actions` of `profile` (the actions on the
/// page the editor shows), replacing the previous watch; none stops
/// watching. Returns the watch's number, which its events carry.
#[tauri::command]
pub fn watch_key_states<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    profile_id: ProfileId,
    actions: Vec<ActionInstanceId>,
) -> u64 {
    let number = state.key_watch.replace();
    if actions.is_empty() {
        return number;
    }
    let paths = state.paths.clone();
    let task = tauri::async_runtime::spawn(async move {
        let emit = |update: KeyStatesUpdate| {
            let _ = app.emit(
                KEY_STATES_EVENT,
                KeyStatesEvent {
                    watch: number,
                    update,
                },
            );
        };
        loop {
            match forward(&paths, profile_id, &actions, &emit).await {
                Ok(()) => return,
                Err(_) => emit(KeyStatesUpdate::Disconnected),
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    });
    state.key_watch.start(number, task);
    number
}

/// Watches on a connection of its own until it fails; `Ok` when the agent
/// cannot watch at all.
async fn forward(
    paths: &AgentPaths,
    profile: ProfileId,
    actions: &[ActionInstanceId],
    emit: &impl Fn(KeyStatesUpdate),
) -> Result<(), ClientError> {
    let mut client = AgentClient::connect(paths, ClientKind::Desktop).await?;
    if !client.supports(Capability::KeyStates) {
        emit(KeyStatesUpdate::Unsupported);
        return Ok(());
    }
    let (live, states) = client.watch_key_states(profile, actions.to_vec()).await?;
    emit(KeyStatesUpdate::Snapshot { live, states });
    if !live {
        // Nothing will change until another profile is active; the editor
        // watches again when it is.
        return Ok(());
    }
    loop {
        let states = client.next_key_states().await?;
        emit(KeyStatesUpdate::Changed { states });
    }
}
