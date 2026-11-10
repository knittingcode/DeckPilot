//! IPC server: authenticates clients and forwards requests to the agent.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use deckpilot_ipc::framing::{FrameError, read_message, write_message};
use deckpilot_ipc::{
    AgentEvent, ClientKind, ClientMessage, ErrorCode, IpcError, PROTOCOL_VERSION, Request,
    Response, ServerMessage, SharedStream, token,
};
use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use tokio::io::BufReader;
use tokio::sync::{broadcast, watch};
use tracing::{debug, warn};

use crate::service::{AgentHandle, VERSION};

/// A client must authenticate within this time.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn spawn(
    listener: Listener,
    handle: AgentHandle,
    token: String,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let token: Arc<str> = token.into();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                connection = listener.accept() => match connection {
                    Ok(stream) => {
                        tokio::spawn(serve(stream, handle.clone(), token.clone(), shutdown.clone()));
                    }
                    Err(error) => {
                        warn!(%error, "accepting an IPC connection failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
                _ = shutdown.changed() => break,
            }
        }
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

async fn serve(
    stream: Stream,
    handle: AgentHandle,
    token: Arc<str>,
    mut shutdown: watch::Receiver<bool>,
) {
    let stream = SharedStream::new(stream);
    let mut reader = BufReader::new(stream.clone());
    let mut writer = stream;

    let hello = tokio::time::timeout(HELLO_TIMEOUT, read_message::<_, ClientMessage>(&mut reader));
    let client = match hello.await {
        Ok(Ok(Some(ClientMessage::Hello {
            protocol,
            token: provided,
            client,
        }))) => {
            if protocol != PROTOCOL_VERSION {
                let message =
                    format!("protocol {protocol} is not supported; expected {PROTOCOL_VERSION}");
                send(
                    &mut writer,
                    &error(None, ErrorCode::UnsupportedProtocol, message),
                )
                .await;
                return;
            }
            if !token::matches(&token, &provided) {
                warn!("rejected an IPC client with an invalid token");
                send(
                    &mut writer,
                    &error(None, ErrorCode::Unauthorized, "invalid token"),
                )
                .await;
                return;
            }
            client
        }
        Ok(Ok(Some(ClientMessage::Request { id, .. }))) => {
            send(
                &mut writer,
                &error(Some(id), ErrorCode::Unauthorized, "hello expected"),
            )
            .await;
            return;
        }
        Ok(Err(FrameError::Malformed(_))) => {
            send(
                &mut writer,
                &error(None, ErrorCode::InvalidRequest, "malformed hello"),
            )
            .await;
            return;
        }
        _ => return,
    };
    let welcome = ServerMessage::Welcome {
        protocol: PROTOCOL_VERSION,
        agent_version: VERSION.into(),
    };
    if !send(&mut writer, &welcome).await {
        return;
    }

    let mut events: Option<broadcast::Receiver<AgentEvent>> = None;
    let mut desktop: Option<DesktopGuard> = None;
    loop {
        tokio::select! {
            message = read_message::<_, serde_json::Value>(&mut reader) => {
                let reply = match message {
                    Ok(None) | Err(FrameError::Io(_)) => break,
                    Ok(Some(value)) => {
                        handle_message(value, &handle, client, &mut events, &mut desktop).await
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
    client: ClientKind,
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
        Ok(response) => ServerMessage::Response { id, response },
        Err(error) => ServerMessage::Error {
            id: Some(id),
            error,
        },
    }
}
