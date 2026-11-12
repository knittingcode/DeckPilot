//! Async client for the agent's IPC endpoint.

use std::collections::VecDeque;
use std::io;

use thiserror::Error;
use tokio::io::BufReader;

use crate::framing::{FrameError, read_message, write_message};
use crate::{
    AgentEvent, AgentPaths, Capability, ClientKind, ClientMessage, ErrorCode, IpcError,
    MIN_COMPATIBLE_AGENT_PROTOCOL, PROTOCOL_VERSION, Request, Response, ServerMessage,
    SharedStream, negotiate, token,
};

/// How long the agent has to answer a new connection's `hello`.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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
    /// The request is newer than the protocol the agent speaks; it was not
    /// sent. The agent is from an older release and needs to be updated
    /// (restarted with the agent of this release) for it.
    #[error(
        "the running agent ({agent}) is too old for `{request}` (it needs protocol {needs}, the agent speaks {protocol}); restart the agent"
    )]
    Unsupported {
        request: String,
        needs: u32,
        protocol: u32,
        agent: String,
    },
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
    /// The newest protocol the agent speaks.
    agent_protocol: u32,
    /// The protocol of this connection: the lower of the client's and the
    /// agent's.
    protocol: u32,
    capabilities: Vec<Capability>,
    events: VecDeque<AgentEvent>,
}

impl AgentClient {
    /// Connects and authenticates with the newest protocol the agent
    /// accepts, from [`PROTOCOL_VERSION`] down to
    /// [`MIN_COMPATIBLE_AGENT_PROTOCOL`]. Agents before protocol 6 reject a
    /// newer `hello`, so the client steps down until one is accepted; the
    /// connection then only offers what that protocol has
    /// ([`AgentClient::capabilities`]). Returns [`ClientError::NotRunning`]
    /// when no agent listens for this data directory, and the agent's
    /// [`ErrorCode::UnsupportedProtocol`] error for an agent too old for
    /// this app.
    pub async fn connect(paths: &AgentPaths, client: ClientKind) -> Result<Self, ClientError> {
        let mut protocol = PROTOCOL_VERSION;
        loop {
            match Self::connect_with_protocol(paths, client, protocol).await {
                Err(ClientError::Agent(error))
                    if error.code == ErrorCode::UnsupportedProtocol
                        && protocol > MIN_COMPATIBLE_AGENT_PROTOCOL =>
                {
                    protocol -= 1;
                }
                other => return other,
            }
        }
    }

    /// Connects speaking an older protocol version. Used to talk to an agent
    /// from a previous release, for example to stop it during an update.
    pub async fn connect_with_protocol(
        paths: &AgentPaths,
        client: ClientKind,
        protocol: u32,
    ) -> Result<Self, ClientError> {
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
            agent_protocol: protocol,
            protocol,
            capabilities: Vec::new(),
            events: VecDeque::new(),
        };
        let hello = ClientMessage::Hello {
            protocol,
            token,
            client,
        };
        // An endpoint that accepts but never answers (an agent that is
        // stopping or stuck) must not keep the caller waiting forever.
        let welcome = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            write_message(&mut connection.writer, &hello).await?;
            connection.receive().await
        })
        .await
        .map_err(|_| {
            ClientError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "the agent did not answer the handshake",
            ))
        })?
        // Closed before it answered: an agent that is stopping (on Windows
        // the pipe instance of a stopped agent can still be opened for a
        // moment). Nothing runs that could serve this client.
        .map_err(|error| match error {
            ClientError::Closed => ClientError::NotRunning,
            other => other,
        })?;
        match welcome {
            ServerMessage::Welcome {
                agent_version,
                protocol: agent_protocol,
                capabilities,
            } => {
                connection.agent_version = agent_version;
                connection.agent_protocol = agent_protocol;
                connection.protocol = negotiate(protocol, agent_protocol);
                // Agents before protocol 6 do not list capabilities; theirs
                // follow from the protocol. An agent never gets credit for
                // more than the negotiated protocol has.
                let derived = Capability::of_protocol(connection.protocol);
                connection.capabilities = if capabilities.is_empty() {
                    derived
                } else {
                    capabilities
                        .into_iter()
                        .filter(|c| derived.contains(c))
                        .collect()
                };
                Ok(connection)
            }
            ServerMessage::Error { error, .. } => Err(ClientError::Agent(error)),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }

    pub fn agent_version(&self) -> &str {
        &self.agent_version
    }

    /// The protocol of this connection.
    pub fn protocol(&self) -> u32 {
        self.protocol
    }

    /// The newest protocol the agent speaks.
    pub fn agent_protocol(&self) -> u32 {
        self.agent_protocol
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }

    async fn receive(&mut self) -> Result<ServerMessage, ClientError> {
        read_message(&mut self.reader)
            .await?
            .ok_or(ClientError::Closed)
    }

    /// Sends a request and waits for its response. Events that arrive in the
    /// meantime are kept for [`AgentClient::next_event`].
    pub async fn request(&mut self, request: Request) -> Result<Response, ClientError> {
        let needs = request.since_protocol();
        if needs > self.protocol {
            return Err(ClientError::Unsupported {
                request: request.name(),
                needs,
                protocol: self.protocol,
                agent: self.agent_version.clone(),
            });
        }
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
