//! IPC server: authenticates clients and forwards requests to the agent.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use deckpilot_core::ActionInstanceId;
use deckpilot_ipc::framing::{FrameError, read_message, write_message};
use deckpilot_ipc::plugin_answers;
use deckpilot_ipc::{
    AgentEvent, Capability, ClientKind, ClientMessage, ErrorCode, IpcError, KeyStateEntry,
    MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, Request, Response, ServerMessage, SharedStream, token,
};
use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use tokio::io::BufReader;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::service::{AgentHandle, VERSION};
use deckpilot_ipc::plugins::PluginEvent;

/// A client must authenticate within this time.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long connections may take to close when the agent stops.
const CLOSE_GRACE: Duration = Duration::from_millis(500);

pub(crate) fn spawn(
    listener: Listener,
    handle: AgentHandle,
    token: String,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let token: Arc<str> = token.into();
    tokio::spawn(async move {
        // Connections are tracked so that stopping closes all of them: on
        // Windows the pipe name stays taken while any instance is open, and
        // an agent started right after must be able to listen again.
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                connection = listener.accept() => match connection {
                    Ok(stream) => {
                        connections.spawn(serve(stream, handle.clone(), token.clone(), shutdown.clone()));
                    }
                    Err(error) => {
                        warn!(%error, "accepting an IPC connection failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
                _ = shutdown.changed() => break,
            }
        }
        drop(listener);
        // Connections deliver their last events and close; any still busy
        // (e.g. waiting for an answer the stopped agent will not give) are
        // closed after a short grace period.
        let drained = async { while connections.join_next().await.is_some() {} };
        let _ = tokio::time::timeout(CLOSE_GRACE, drained).await;
        connections.shutdown().await;
    })
}

/// Identifies connections for `watchKeyStates`.
static NEXT_VIEWER: AtomicU64 = AtomicU64::new(1);

/// What a connection watches with `watchKeyStates` (protocol 11).
struct KeyWatch {
    viewer: u64,
    actions: BTreeSet<ActionInstanceId>,
    changes: Option<broadcast::Receiver<Vec<KeyStateEntry>>>,
}

/// The live preview messages of a connection that began a session
/// (protocol 16): for every connection, only its own.
type LiveEvents = Option<broadcast::Receiver<(u64, deckpilot_ipc::live_preview::LivePreviewEvent)>>;

/// Counts subscribed desktop clients while alive.
struct DesktopGuard(Arc<AtomicUsize>);

impl DesktopGuard {
    fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter.clone())
    }
}

impl Drop for DesktopGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What a connection asked for with `subscribePluginEvents` beyond the
/// plugin events themselves.
#[derive(Default)]
struct PluginSubscription {
    /// Module migration events.
    migrations: bool,
    /// Plugin dialogs (announced, protocol 12).
    dialogs: Option<DialogApp>,
}

/// A connection that announced plugin dialogs (protocol 12): it receives
/// them and may answer. Dropped with the connection; the last one ends
/// every open dialog `appClosed`.
struct DialogApp {
    events: Option<broadcast::Receiver<PluginEvent>>,
    _guard: crate::plugin_dialogs::AppGuard,
}

/// Writes `message`; `false` when the connection is gone. A message larger
/// than one frame is not written at all, so the connection is still
/// aligned: an answer is shortened or refused in its place
/// ([`plugin_answers::fit`]), an event is left out.
async fn send(writer: &mut SharedStream, message: &ServerMessage) -> bool {
    match write_message(writer, message).await {
        Ok(()) => true,
        Err(FrameError::TooLarge) => {
            if let Some(fitted) = plugin_answers::fit(message.clone()) {
                warn!("an IPC answer was too large; it is sent shortened or refused");
                written(write_message(writer, &fitted).await)
            } else {
                warn!("an IPC message was too large to send; it is left out");
                true
            }
        }
        result => written(result),
    }
}

fn written(result: Result<(), FrameError>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            debug!(%error, "writing to an IPC client failed");
            false
        }
    }
}

fn error(id: Option<u64>, code: ErrorCode, message: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        id,
        error: IpcError::new(code, message),
    }
}

/// The next plugin event of a connection, and whether it is a dialog event
/// (only for a connection that announced plugin dialogs).
async fn next_plugin_event(
    plugin_events: &mut Option<broadcast::Receiver<PluginEvent>>,
    dialogs: &mut Option<DialogApp>,
) -> (Result<PluginEvent, broadcast::error::RecvError>, bool) {
    // Borrowed, not taken: the caller's `select!` may drop this future, and
    // `recv` is cancel safe.
    let dialog_events = dialogs.as_mut().and_then(|app| app.events.as_mut());
    tokio::select! {
        event = next_event(plugin_events) => (event, false),
        event = async move {
            match dialog_events {
                Some(events) => events.recv().await,
                None => std::future::pending().await,
            }
        } => (event, true),
    }
}

async fn next_event<T: Clone>(
    events: &mut Option<broadcast::Receiver<T>>,
) -> Result<T, broadcast::error::RecvError> {
    match events {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

/// Reads and checks the `hello` message and answers with `welcome`.
/// Returns `None` (after sending an error where useful) if the client may not
/// continue.
async fn authenticate(
    reader: &mut BufReader<SharedStream>,
    writer: &mut SharedStream,
    token: &str,
) -> Option<(ClientKind, u32)> {
    let hello = tokio::time::timeout(HELLO_TIMEOUT, read_message::<_, ClientMessage>(reader));
    let rejection = match hello.await {
        Ok(Ok(Some(ClientMessage::Hello {
            protocol,
            token: provided,
            client,
        }))) => {
            if !(MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&protocol) {
                let message = format!(
                    "protocol {protocol} is not supported; expected {MIN_PROTOCOL_VERSION} to {PROTOCOL_VERSION}"
                );
                error(None, ErrorCode::UnsupportedProtocol, message)
            } else if !token::matches(token, &provided) {
                warn!("rejected an IPC client with an invalid token");
                error(None, ErrorCode::Unauthorized, "invalid token")
            } else {
                let welcome = ServerMessage::Welcome {
                    protocol: PROTOCOL_VERSION,
                    agent_version: VERSION.into(),
                    capabilities: Capability::of_protocol(protocol),
                };
                return send(writer, &welcome).await.then_some((client, protocol));
            }
        }
        Ok(Ok(Some(ClientMessage::Request { id, .. }))) => {
            error(Some(id), ErrorCode::Unauthorized, "hello expected")
        }
        Ok(Err(FrameError::Malformed(_))) => {
            error(None, ErrorCode::InvalidRequest, "malformed hello")
        }
        _ => return None,
    };
    send(writer, &rejection).await;
    None
}

#[allow(clippy::too_many_lines)] // One arm per kind of message a connection gets.
async fn serve(
    stream: Stream,
    handle: AgentHandle,
    token: Arc<str>,
    mut shutdown: watch::Receiver<bool>,
) {
    let stream = SharedStream::new(stream);
    let mut reader = BufReader::new(stream.clone());
    let mut writer = stream;

    let Some((client, protocol)) = authenticate(&mut reader, &mut writer, &token).await else {
        return;
    };

    let mut events: Option<broadcast::Receiver<AgentEvent>> = None;
    let mut plugin_events: Option<broadcast::Receiver<PluginEvent>> = None;
    let mut subscription = PluginSubscription::default();
    let mut desktop: Option<DesktopGuard> = None;
    let mut watch = KeyWatch {
        viewer: NEXT_VIEWER.fetch_add(1, Ordering::Relaxed),
        actions: BTreeSet::new(),
        changes: None,
    };
    let mut live: LiveEvents = None;
    loop {
        tokio::select! {
            message = read_message::<_, serde_json::Value>(&mut reader) => {
                let reply = match message {
                    Ok(None) | Err(FrameError::Io(_)) => break,
                    Ok(Some(value)) => {
                        handle_message(
                            value,
                            &handle,
                            (client, protocol),
                            (&mut events, &mut plugin_events, &mut subscription),
                            &mut desktop,
                            (&mut watch, &mut live),
                        )
                        .await
                    }
                    Err(FrameError::Malformed(e)) => {
                        error(None, ErrorCode::InvalidRequest, format!("malformed message: {e}"))
                    }
                    Err(other @ (FrameError::TooLarge | FrameError::Encoding)) => {
                        send(&mut writer, &error(None, ErrorCode::InvalidRequest, other.to_string())).await;
                        break;
                    }
                };
                if !send(&mut writer, &reply).await {
                    break;
                }
            }
            event = next_event(&mut events) => match event {
                // Clients of an older protocol do not know newer events
                // (nor, before protocol 10, plugin variables).
                Ok(event) => {
                    if let Some(event) = event.for_protocol(protocol)
                        && !send(&mut writer, &ServerMessage::Event { event }).await
                    {
                        break;
                    }
                }
                // Missed state snapshots are superseded by the next one.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            // Only to connections that asked (protocol 10); migrations and
            // dialogs (their own channel; a missed one times out) only to
            // those that asked for them.
            (event, dialog) = next_plugin_event(&mut plugin_events, &mut subscription.dialogs) => match event {
                Ok(event) => {
                    if event.delivered(subscription.migrations && !dialog, dialog)
                        && !send(&mut writer, &ServerMessage::PluginEvent { event }).await
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) if dialog => subscription.dialogs = None,
                Err(broadcast::error::RecvError::Closed) => plugin_events = None,
            },
            // Only the watched keys, only to the connection that watches.
            changes = next_event(&mut watch.changes) => match changes {
                Ok(changes) => {
                    let states: Vec<KeyStateEntry> = changes
                        .into_iter()
                        .filter(|entry| watch.actions.contains(&entry.action))
                        .collect();
                    if !states.is_empty()
                        && !send(&mut writer, &ServerMessage::KeyStates { states }).await
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => watch.changes = None,
            },
            // Only this connection's sessions.
            message = next_event(&mut live) => match message {
                Ok((viewer, event)) => {
                    if viewer == watch.viewer
                        && !send(&mut writer, &ServerMessage::LivePreview { event }).await
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => live = None,
            },
            _ = shutdown.changed() => {
                // Deliver the final notifications before closing.
                if let Some(events) = events.as_mut() {
                    while let Ok(event) = events.try_recv() {
                        if let Some(event) = event.for_protocol(protocol) {
                            send(&mut writer, &ServerMessage::Event { event }).await;
                        }
                    }
                }
                break;
            }
        }
    }
    drop(desktop);
    if watch.changes.is_some() {
        handle.unwatch_key_states(watch.viewer);
    }
    // Its live previews end with it: the devices show the saved
    // configuration again.
    if live.is_some() {
        handle.live_preview_closed(watch.viewer);
    }
}

/// Plugin dialogs (protocol 12): only a connection of that protocol that
/// announces them receives them. Subscribed before it counts as an app, so
/// no dialog opened for it is missed.
fn announce_dialogs(
    handle: &AgentHandle,
    protocol: u32,
    features: &[String],
    dialogs: &mut Option<DialogApp>,
) {
    if protocol >= Capability::PluginDialogs.since_protocol()
        && dialogs.is_none()
        && features
            .iter()
            .any(|f| f == deckpilot_ipc::plugin_dialogs::FEATURE)
    {
        let events = handle.plugin_dialogs.subscribe();
        *dialogs = Some(DialogApp {
            events: Some(events),
            _guard: handle.plugin_dialogs.app_connected(),
        });
    }
}

fn reply(id: u64, result: Result<Response, IpcError>) -> ServerMessage {
    match result {
        Ok(response) => ServerMessage::Response { id, response },
        Err(error) => ServerMessage::Error {
            id: Some(id),
            error,
        },
    }
}

#[allow(clippy::too_many_lines)] // One block per request handled per connection.
async fn handle_message(
    value: serde_json::Value,
    handle: &AgentHandle,
    (client, protocol): (ClientKind, u32),
    (events, plugin_events, subscription): (
        &mut Option<broadcast::Receiver<AgentEvent>>,
        &mut Option<broadcast::Receiver<PluginEvent>>,
        &mut PluginSubscription,
    ),
    desktop: &mut Option<DesktopGuard>,
    (watch, live): (&mut KeyWatch, &mut LiveEvents),
) -> ServerMessage {
    let id = value.get("id").and_then(serde_json::Value::as_u64);
    let message = match serde_json::from_value::<ClientMessage>(value) {
        Ok(message) => message,
        Err(e) => {
            return error(
                id,
                ErrorCode::InvalidRequest,
                format!("invalid request: {e}"),
            );
        }
    };
    let (id, request) = match message {
        ClientMessage::Request { id, request } => (id, request),
        ClientMessage::Hello { .. } => {
            return error(None, ErrorCode::InvalidRequest, "already authenticated");
        }
    };
    if let Err(message) = request.validate() {
        return error(Some(id), ErrorCode::InvalidRequest, message);
    }
    // A request the client's protocol does not have is refused rather than
    // guessed at: the client and the agent would disagree on its meaning.
    let needs = request.since_protocol();
    if needs > protocol {
        return error(
            Some(id),
            ErrorCode::Unsupported,
            format!(
                "`{}` needs protocol {needs}; this connection uses protocol {protocol}",
                request.name()
            ),
        );
    }
    if let Request::SubscribePluginEvents {
        migrations,
        features,
    } = &request
    {
        if plugin_events.is_none() {
            *plugin_events = Some(handle.subscribe_plugins());
        }
        // Asking once is enough; a later plain subscription keeps them.
        subscription.migrations |= *migrations;
        announce_dialogs(handle, protocol, features, &mut subscription.dialogs);
        return ServerMessage::Response {
            id,
            response: Response::Ok,
        };
    }
    if let Request::AnswerPluginDialog {
        dialog,
        plugin,
        answer,
    } = request
    {
        let answered = (handle.plugin_dialogs).answer(
            subscription.dialogs.is_some(),
            &dialog,
            &plugin,
            answer,
        );
        return reply(id, answered.map(|()| Response::Ok));
    }
    if let Request::WatchKeyStates { profile, actions } = request {
        // Subscribed before asking, so no change after the answer is missed.
        if watch.changes.is_none() {
            watch.changes = Some(handle.subscribe_key_states());
        }
        watch.actions = actions.iter().copied().collect();
        return match handle
            .watch_key_states(watch.viewer, profile, actions)
            .await
        {
            Ok(response) => ServerMessage::Response { id, response },
            Err(error) => ServerMessage::Error {
                id: Some(id),
                error,
            },
        };
    }
    if matches!(
        request,
        Request::BeginLivePreview { .. }
            | Request::ApplyLivePreview { .. }
            | Request::NavigateLivePreview { .. }
            | Request::EndLivePreview { .. }
    ) {
        // Subscribed before the session exists, so nothing of it is missed.
        if live.is_none() {
            *live = Some(handle.subscribe_live_preview());
        }
        return reply(id, handle.live_preview(watch.viewer, request).await);
    }
    if matches!(request, Request::Subscribe) {
        if events.is_none() {
            *events = Some(handle.subscribe());
            if client == ClientKind::Desktop {
                *desktop = Some(DesktopGuard::new(&handle.desktop_clients));
            }
        }
        return ServerMessage::Response {
            id,
            response: Response::Ok,
        };
    }
    match handle.request(request).await {
        Ok(response) => ServerMessage::Response {
            id,
            response: response.for_protocol(protocol),
        },
        Err(error) => ServerMessage::Error {
            id: Some(id),
            error,
        },
    }
}
