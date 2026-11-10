//! Async client for the agent's IPC endpoint.

use std::collections::VecDeque;
use std::io;

use thiserror::Error;
use tokio::io::BufReader;

use crate::framing::{FrameError, read_message, write_message};
use crate::{
    AgentEvent, AgentPaths, ClientKind, ClientMessage, IpcError, PROTOCOL_VERSION, Request,
    Response, ServerMessage, SharedStream, token,
};

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("the DeckPilot agent is not running")]
    NotRunning,
    #[error("connection to the agent was closed")]
    Closed,
    #[error("communication with the agent failed: {0}")]
    Io(#[from] io::Error),
    #[error("unexpected message from the agent: {0}")]
    Protocol(String),
    #[error(transparent)]
    Agent(#[from] IpcError),
}

impl From<FrameError> for ClientError {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Io(error) => Self::from_io(error),
            other => Self::Protocol(other.to_string()),
        }
    }
}

impl ClientError {
    /// A connection torn down by the agent is reported as [`ClientError::Closed`]
    /// regardless of how the platform signals it: a clean end of stream on
    /// macOS and Windows, but often a reset on Linux when the agent exits while
    /// a request is in flight.
    fn from_io(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::UnexpectedEof => Self::Closed,
            _ => Self::Io(error),
        }
    }
}

/// A connection to the agent.
#[derive(Debug)]
pub struct AgentClient {
    reader: BufReader<SharedStream>,
    writer: SharedStream,
    next_id: u64,
    agent_version: String,
    events: VecDeque<AgentEvent>,
}

impl AgentClient {
    /// Connects and authenticates. Returns [`ClientError::NotRunning`] when no
    /// agent listens for this data directory.
    pub async fn connect(paths: &AgentPaths, client: ClientKind) -> Result<Self, ClientError> {
        let stream = paths
            .endpoint()
            .connect()
            .await
            .map_err(|error| match error.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                    ClientError::NotRunning
                }
                _ => ClientError::Io(error),
            })?;
        let token = token::read(&paths.token_file).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => ClientError::NotRunning,
            _ => ClientError::Io(error),
        })?;
        let stream = SharedStream::new(stream);
        let mut connection = Self {
            reader: BufReader::new(stream.clone()),
            writer: stream,
            next_id: 1,
            agent_version: String::new(),
            events: VecDeque::new(),
        };
        let hello = ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            token,
            client,
        };
        write_message(&mut connection.writer, &hello).await?;
        match connection.receive().await? {
            ServerMessage::Welcome { agent_version, .. } => {
                connection.agent_version = agent_version;
                Ok(connection)
            }
            ServerMessage::Error { error, .. } => Err(ClientError::Agent(error)),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }

    pub fn agent_version(&self) -> &str {
        &self.agent_version
    }

    async fn receive(&mut self) -> Result<ServerMessage, ClientError> {
        read_message(&mut self.reader)
            .await?
            .ok_or(ClientError::Closed)
    }

    /// Sends a request and waits for its response. Events that arrive in the
    /// meantime are kept for [`AgentClient::next_event`].
    pub async fn request(&mut self, request: Request) -> Result<Response, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        write_message(&mut self.writer, &ClientMessage::Request { id, request }).await?;
        loop {
            match self.receive().await? {
                ServerMessage::Response {
                    id: reply,
                    response,
                } if reply == id => {
                    return Ok(response);
                }
                ServerMessage::Error { id: reply, error }
                    if reply == Some(id) || reply.is_none() =>
                {
                    return Err(ClientError::Agent(error));
                }
                ServerMessage::Event { event } => self.events.push_back(event),
                other => return Err(ClientError::Protocol(format!("{other:?}"))),
            }
        }
    }

    /// Subscribes to runtime events.
    pub async fn subscribe(&mut self) -> Result<(), ClientError> {
        match self.request(Request::Subscribe).await? {
            Response::Ok => Ok(()),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }

    /// Waits for the next event after [`AgentClient::subscribe`].
    pub async fn next_event(&mut self) -> Result<AgentEvent, ClientError> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        match self.receive().await? {
            ServerMessage::Event { event } => Ok(event),
            ServerMessage::Error { error, .. } => Err(ClientError::Agent(error)),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }
}

/// Connects, sends one request and disconnects.
pub async fn request_once(
    paths: &AgentPaths,
    client: ClientKind,
    request: Request,
) -> Result<Response, ClientError> {
    AgentClient::connect(paths, client)
        .await?
        .request(request)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_teardown_is_reported_as_closed() {
        for kind in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::UnexpectedEof,
        ] {
            let error = ClientError::from(FrameError::Io(io::Error::from(kind)));
            assert!(matches!(error, ClientError::Closed), "{kind:?}");
        }
        let other = ClientError::from(FrameError::Io(io::Error::from(
            io::ErrorKind::PermissionDenied,
        )));
        assert!(matches!(other, ClientError::Io(_)));
    }
}
