//! IPC server: authenticates clients and forwards requests to the agent.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use deckpilot_ipc::framing::{FrameError, read_message, write_message};
use deckpilot_ipc::{
    AgentEvent, Capability, ClientKind, ClientMessage, ErrorCode, IpcError, MIN_PROTOCOL_VERSION,
    PROTOCOL_VERSION, Request, Response, ServerMessage, SharedStream, token,
};
use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use tokio::io::BufReader;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::service::{AgentHandle, VERSION};

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

async fn send(writer: &mut SharedStream, message: &ServerMessage) -> bool {
    match write_message(writer, message).await {
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

async fn next_event(
    events: &mut Option<broadcast::Receiver<AgentEvent>>,
) -> Result<AgentEvent, broadcast::error::RecvError> {
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
    let mut desktop: Option<DesktopGuard> = None;
    loop {
        tokio::select! {
            message = read_message::<_, serde_json::Value>(&mut reader) => {
                let reply = match message {
                    Ok(None) | Err(FrameError::Io(_)) => break,
                    Ok(Some(value)) => {
                        handle_message(value, &handle, (client, protocol), &mut events, &mut desktop).await
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
                // Clients of an older protocol do not know newer events.
                Ok(event) if event.since_protocol() > protocol => {}
                Ok(event) => {
                    if !send(&mut writer, &ServerMessage::Event { event }).await {
                        break;
                    }
                }
                // Missed state snapshots are superseded by the next one.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = shutdown.changed() => {
                // Deliver the final notifications before closing.
                if let Some(events) = events.as_mut() {
                    while let Ok(event) = events.try_recv() {
                        if event.since_protocol() > protocol {
                            continue;
                        }
                        send(&mut writer, &ServerMessage::Event { event }).await;
                    }
                }
                break;
            }
        }
    }
    drop(desktop);
}

async fn handle_message(
    value: serde_json::Value,
    handle: &AgentHandle,
    (client, protocol): (ClientKind, u32),
    events: &mut Option<broadcast::Receiver<AgentEvent>>,
    desktop: &mut Option<DesktopGuard>,
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
