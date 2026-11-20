//! Async client for the agent's IPC endpoint.

use std::collections::VecDeque;
use std::io;

use thiserror::Error;
use tokio::io::BufReader;

use crate::framing::{FrameError, read_message, write_message};
use crate::live_preview::LivePreviewEvent;
use crate::{
    AgentEvent, AgentPaths, Capability, ClientKind, ClientMessage, ErrorCode, IpcError,
    KeyStateEntry, MIN_COMPATIBLE_AGENT_PROTOCOL, PROTOCOL_VERSION, Request, Response,
    ServerMessage, SharedStream, negotiate, plugins::PluginEvent, token,
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
        // Windows: `ERROR_PIPE_NOT_CONNECTED`, the server end of the pipe
        // was closed.
        #[cfg(windows)]
        if error.raw_os_error() == Some(233) {
            return Self::Closed;
        }
        match error.kind() {
            io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof => Self::Closed,
            _ => Self::Io(error),
        }
    }
}

/// An error while sending the `hello` or waiting for its answer. A
/// connection torn down at that point, however the platform reports it
/// (end of stream, reset, broken pipe, or `ENOTCONN` on macOS when the
/// peer closed before the `hello` was written), belongs to an agent that is
/// stopping (on Windows the pipe of a stopped agent can still be opened for
/// a moment): nothing runs that could serve this client.
fn handshake_error(error: ClientError) -> ClientError {
    match error {
        ClientError::Closed => ClientError::NotRunning,
        other => other,
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
    plugin_events: VecDeque<PluginEvent>,
    /// `keyStates` messages after `watchKeyStates`.
    key_states: VecDeque<Vec<KeyStateEntry>>,
    /// `livePreview` messages of this connection's session (protocol 16).
    live_preview: VecDeque<LivePreviewEvent>,
}

impl Drop for AgentClient {
    fn drop(&mut self) {
        // Nothing is left to deliver: close the connection at once (see
        // `SharedStream::skip_flush_on_close`).
        self.writer.skip_flush_on_close();
    }
}

/// Whether an agent holds the instance lock of `paths`. A lock that can be
/// taken (and is released again at once) means no agent runs; one that
/// cannot be checked counts as held.
fn agent_holds_lock(paths: &AgentPaths) -> bool {
    match std::fs::OpenOptions::new()
        .read(true)
        .open(&paths.lock_file)
    {
        Ok(file) => !matches!(file.try_lock(), Ok(())),
        // No lock file: no agent ever ran here.
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
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
                // An endpoint that stayed busy (on Windows, the pipe of an
                // agent that is gone, still held open by someone) while no
                // agent holds the instance lock: nothing runs.
                io::ErrorKind::TimedOut if !agent_holds_lock(paths) => ClientError::NotRunning,
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
            plugin_events: VecDeque::new(),
            key_states: VecDeque::new(),
            live_preview: VecDeque::new(),
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
        .map_err(handshake_error)?;
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
                ServerMessage::PluginEvent { event } => self.plugin_events.push_back(event),
                ServerMessage::KeyStates { states } => self.key_states.push_back(states),
                ServerMessage::LivePreview { event } => self.live_preview.push_back(event),
                other => return Err(ClientError::Protocol(format!("{other:?}"))),
            }
        }
    }

    /// Watches the live key states of `actions` of `profile` on this
    /// connection (protocol 11), replacing what it watched; none stops.
    /// Returns whether the profile has live states (it is the active one)
    /// and what the keys show now; changes follow with
    /// [`AgentClient::next_key_states`].
    pub async fn watch_key_states(
        &mut self,
        profile: deckpilot_core::ProfileId,
        actions: Vec<deckpilot_core::ActionInstanceId>,
    ) -> Result<(bool, Vec<KeyStateEntry>), ClientError> {
        match self
            .request(Request::WatchKeyStates { profile, actions })
            .await?
        {
            Response::KeyStates { live, states } => Ok((live, states)),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }

    /// Waits for the next change of watched key states. Events that arrive
    /// meanwhile are kept for [`AgentClient::next_event`] and
    /// [`AgentClient::next_plugin_event`].
    pub async fn next_key_states(&mut self) -> Result<Vec<KeyStateEntry>, ClientError> {
        loop {
            if let Some(states) = self.key_states.pop_front() {
                return Ok(states);
            }
            match self.receive().await? {
                ServerMessage::KeyStates { states } => return Ok(states),
                ServerMessage::Event { event } => self.events.push_back(event),
                ServerMessage::PluginEvent { event } => self.plugin_events.push_back(event),
                ServerMessage::LivePreview { event } => self.live_preview.push_back(event),
                ServerMessage::Error { error, .. } => return Err(ClientError::Agent(error)),
                other => return Err(ClientError::Protocol(format!("{other:?}"))),
            }
        }
    }

    /// Waits for the next `livePreview` message of this connection's
    /// session (protocol 16). Other messages that arrive meanwhile are
    /// kept for their own `next_…`.
    pub async fn next_live_preview(&mut self) -> Result<LivePreviewEvent, ClientError> {
        loop {
            if let Some(event) = self.live_preview.pop_front() {
                return Ok(event);
            }
            match self.receive().await? {
                ServerMessage::LivePreview { event } => return Ok(event),
                ServerMessage::Event { event } => self.events.push_back(event),
                ServerMessage::PluginEvent { event } => self.plugin_events.push_back(event),
                ServerMessage::KeyStates { states } => self.key_states.push_back(states),
                ServerMessage::Error { error, .. } => return Err(ClientError::Agent(error)),
                other => return Err(ClientError::Protocol(format!("{other:?}"))),
            }
        }
    }

    /// Subscribes to plugin events (protocol 10).
    pub async fn subscribe_plugin_events(&mut self) -> Result<(), ClientError> {
        self.subscribe_plugin_events_with(false).await
    }

    /// Subscribes to plugin events, with `migrations` also to module
    /// migration events ([`PluginEvent::MigrationChanged`]).
    pub async fn subscribe_plugin_events_with(
        &mut self,
        migrations: bool,
    ) -> Result<(), ClientError> {
        self.subscribe_plugin_events_announcing(migrations, Vec::new())
            .await
    }

    /// Tells the agent the app's effective language (`setLocale`, protocol
    /// 13). Returns `false` without sending anything when the connection's
    /// protocol is older: such an agent keeps English, and nothing else
    /// changes.
    pub async fn set_locale(&mut self, locale: &str) -> Result<bool, ClientError> {
        if !self.capabilities().contains(&Capability::Locale) {
            return Ok(false);
        }
        match self
            .request(Request::SetLocale {
                locale: locale.to_owned(),
            })
            .await?
        {
            Response::Ok => Ok(true),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }

    /// Subscribes to plugin events, announcing `features` this client
    /// renders: [`crate::plugin_dialogs::FEATURE`] also delivers plugin
    /// dialogs (protocol 12). Features are sent only on a connection whose
    /// protocol has [`Capability::PluginDialogs`]; on an older one they are
    /// left out, as an older agent would not know them.
    pub async fn subscribe_plugin_events_announcing(
        &mut self,
        migrations: bool,
        features: Vec<String>,
    ) -> Result<(), ClientError> {
        let features = if self.capabilities().contains(&Capability::PluginDialogs) {
            features
        } else {
            Vec::new()
        };
        match self
            .request(Request::SubscribePluginEvents {
                migrations,
                features,
            })
            .await?
        {
            Response::Ok => Ok(()),
            other => Err(ClientError::Protocol(format!("{other:?}"))),
        }
    }

    /// Waits for the next plugin event after
    /// [`AgentClient::subscribe_plugin_events`]. Runtime events that arrive
    /// meanwhile are kept for [`AgentClient::next_event`].
    pub async fn next_plugin_event(&mut self) -> Result<PluginEvent, ClientError> {
        loop {
            if let Some(event) = self.plugin_events.pop_front() {
                return Ok(event);
            }
            match self.receive().await? {
                ServerMessage::PluginEvent { event } => return Ok(event),
                ServerMessage::Event { event } => self.events.push_back(event),
                ServerMessage::KeyStates { states } => self.key_states.push_back(states),
                ServerMessage::LivePreview { event } => self.live_preview.push_back(event),
                ServerMessage::Error { error, .. } => return Err(ClientError::Agent(error)),
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
        loop {
            match self.receive().await? {
                ServerMessage::Event { event } => return Ok(event),
                // Kept for `next_plugin_event`.
                ServerMessage::PluginEvent { event } => self.plugin_events.push_back(event),
                ServerMessage::KeyStates { states } => self.key_states.push_back(states),
                ServerMessage::LivePreview { event } => self.live_preview.push_back(event),
                ServerMessage::Error { error, .. } => return Err(ClientError::Agent(error)),
                other => return Err(ClientError::Protocol(format!("{other:?}"))),
            }
        }
    }
}

impl AgentClient {
    /// Splits the connection into a writer of requests and a channel of
    /// everything the agent sends (answers, errors, events, `livePreview`
    /// messages), read by a task of its own: waiting on the channel can be
    /// cancelled at any time, so a client can wait for the agent and for
    /// its own work at once (protocol 16's live preview). Messages already
    /// received come first. Dropping the writer stops the reader and
    /// closes the connection.
    pub fn split(mut self) -> (RequestWriter, MessageReceiver) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let queued = self
            .events
            .drain(..)
            .map(|event| ServerMessage::Event { event })
            .chain(
                self.plugin_events
                    .drain(..)
                    .map(|event| ServerMessage::PluginEvent { event }),
            )
            .chain(
                self.key_states
                    .drain(..)
                    .map(|states| ServerMessage::KeyStates { states }),
            )
            .chain(
                self.live_preview
                    .drain(..)
                    .map(|event| ServerMessage::LivePreview { event }),
            );
        for message in queued {
            let _ = sender.send(Ok(message));
        }
        let writer = self.writer.clone();
        let mut reader = std::mem::replace(&mut self.reader, BufReader::new(writer.clone()));
        let task = tokio::spawn(async move {
            loop {
                let message = match read_message::<_, ServerMessage>(&mut reader).await {
                    Ok(Some(message)) => Ok(message),
                    Ok(None) => Err(ClientError::Closed),
                    Err(error) => Err(error.into()),
                };
                let end = message.is_err();
                if sender.send(message).is_err() || end {
                    return;
                }
            }
        });
        let writer = RequestWriter {
            writer,
            next_id: self.next_id,
            protocol: self.protocol,
            agent_version: self.agent_version.clone(),
            reader: task,
        };
        (writer, receiver)
    }
}

/// What the agent sends on a split connection ([`AgentClient::split`]);
/// ends with an error (`Closed` when the agent closed it).
pub type MessageReceiver = tokio::sync::mpsc::UnboundedReceiver<Result<ServerMessage, ClientError>>;

/// The writing half of a split connection.
#[derive(Debug)]
pub struct RequestWriter {
    writer: SharedStream,
    next_id: u64,
    protocol: u32,
    agent_version: String,
    reader: tokio::task::JoinHandle<()>,
}

impl RequestWriter {
    /// Sends `request` and returns its id, which its answer carries.
    /// Refused locally, as by [`AgentClient::request`], when the
    /// connection's protocol does not have it.
    pub async fn send(&mut self, request: Request) -> Result<u64, ClientError> {
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
        Ok(id)
    }

    /// The protocol of the connection.
    pub fn protocol(&self) -> u32 {
        self.protocol
    }
}

impl Drop for RequestWriter {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.skip_flush_on_close();
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

    const TEARDOWN: [io::ErrorKind; 5] = [
        io::ErrorKind::ConnectionReset,
        io::ErrorKind::ConnectionAborted,
        io::ErrorKind::BrokenPipe,
        io::ErrorKind::NotConnected,
        io::ErrorKind::UnexpectedEof,
    ];

    #[test]
    fn peer_teardown_is_reported_as_closed() {
        for kind in TEARDOWN {
            let error = ClientError::from(FrameError::Io(io::Error::from(kind)));
            assert!(matches!(error, ClientError::Closed), "{kind:?}");
        }
        // As the OS reports it: `ENOTCONN` (57 on macOS) for a write after
        // the peer closed.
        #[cfg(target_os = "macos")]
        assert!(matches!(
            ClientError::from(FrameError::Io(io::Error::from_raw_os_error(57))),
            ClientError::Closed
        ));
        let other = ClientError::from(FrameError::Io(io::Error::from(
            io::ErrorKind::PermissionDenied,
        )));
        assert!(matches!(other, ClientError::Io(_)));
    }

    #[test]
    fn a_connection_torn_down_during_the_hello_means_no_agent_runs() {
        for kind in TEARDOWN {
            let error = handshake_error(ClientError::from(FrameError::Io(io::Error::from(kind))));
            assert!(matches!(error, ClientError::NotRunning), "{kind:?}");
        }
        // Other failures are reported as they are.
        let other = handshake_error(ClientError::from(FrameError::Io(io::Error::from(
            io::ErrorKind::PermissionDenied,
        ))));
        assert!(matches!(other, ClientError::Io(_)));
    }

    #[test]
    fn the_instance_lock_tells_whether_an_agent_runs() {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-ipc-lock-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = AgentPaths::in_dir(&dir);
        // Never ran here.
        assert!(!agent_holds_lock(&paths));
        // Ran and stopped (the file stays).
        std::fs::write(&paths.lock_file, b"").unwrap();
        assert!(!agent_holds_lock(&paths));
        // Running: the lock is held.
        let held = std::fs::OpenOptions::new()
            .write(true)
            .open(&paths.lock_file)
            .unwrap();
        held.try_lock().unwrap();
        assert!(agent_holds_lock(&paths));
        // The check itself does not keep the lock.
        drop(held);
        let again = std::fs::File::open(&paths.lock_file).unwrap();
        assert!(again.try_lock().is_ok());
        drop(again);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
