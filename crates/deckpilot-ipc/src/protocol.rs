//! Versioned message types.
//!
//! Messages are JSON objects tagged by `type`, one per line. A client sends
//! [`ClientMessage::Hello`] first; afterwards it sends
//! [`ClientMessage::Request`]s with a client-chosen `id` that the agent echoes
//! in its [`ServerMessage::Response`] or [`ServerMessage::Error`]. After a
//! [`Request::Subscribe`], the agent also pushes [`ServerMessage::Event`]s.
//!
//! Incompatible changes require a new [`PROTOCOL_VERSION`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use deckpilot_core::graph::ExecutionEvent;
use deckpilot_core::variables::{
    CatalogEntry, CatalogFilter, PreviewItem, ProjectSource, ReferencePreview, ReferenceStatus,
    ResolutionContext,
};
use deckpilot_core::work_session::WorkSessionSnapshot;
use deckpilot_core::{
    ActionInstanceId, ActionResult, ActionStatus, ContextEffect, CycleDirection, ExecutionId,
    ExecutionRecord, ExecutionSummary, LayoutStatus, ProfileId, Project, ProjectId, WorkSessionId,
    WorkflowId, WorkflowReport,
};
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceDescriptor, DeviceId};

use crate::plugins;
use crate::work_session::{MAX_WORK_SESSION_LIST, WorkSessionSettings, WorkSessionSummary};

/// Protocol spoken by this build.
///
/// Version 3: action results only report `selectProject` effects (project
/// switching is computed by the projects module), and action types use
/// module namespaces such as `projects.next`.
///
/// Version 4: the encrypted database. The agent starts locked and is
/// unlocked with the database key (`unlock`, `lock`, `getDatabaseKey`); the
/// status reports the [`SecurityState`].
///
/// Version 5: workflows run in the background. `startWorkflow` returns an
/// execution that reports progress as events; runs can be cancelled, listed
/// and inspected afterwards (execution history).
///
/// Version 6: actions show notifications; the agent sends them to the app
/// as `notification` events for its status bar.
///
/// Version 7: shared variables and references. The agent lists the
/// references available in a context (`listVariables`, metadata only) and
/// previews them (`previewReferences`); it announces context changes
/// (`projectChanged`, `profileChanged`, `sharedVariablesChanged`,
/// `resourceBindingsChanged`, `deviceContextChanged`,
/// `executionContextChanged`). Action results may carry `cycleProject`
/// effects, which are removed for older clients.
///
/// Version 8: the work session. The agent reports the session
/// (`getWorkSession`, `workSessionChanged`), its history
/// (`listWorkSessions`) and settings (`getWorkSessionSettings`,
/// `saveWorkSessionSettings`), and lets the user clear notices and decide
/// flagged gaps. No request starts, verifies or finishes a session (D11).
///
/// Version 9: the unified variable registry. Catalog entries name their
/// owner (core, module or user), whether they are read-only and, for
/// legacy ids, the canonical id that replaces them; modules list their own
/// variables (`projects.current.id`, group `module`); a variable whose
/// module is disabled or missing previews as `unavailable`. Older clients
/// receive the catalog they knew: no module roots, `notInContext` and
/// `permissionDenied` in place of the new availability and status. No
/// request changes.
///
/// Version 10: plugins (experimental, capability `plugins`). Requests to
/// list, inspect, install, enable, disable, grant, revoke, update, roll
/// back, uninstall and invoke plugins ([`crate::plugins`]); their progress
/// and state changes arrive as `pluginEvent` messages after
/// `subscribePluginEvents` (a message of its own, so clients that do not
/// ask never see them, whatever their protocol). Errors may carry a
/// machine-readable `reason`, which older clients ignore. The agent answers
/// plugin requests only when built with plugins and with plugins enabled.
/// Nothing older clients receive changes.
pub const PROTOCOL_VERSION: u32 = 10;

/// Most references one `previewReferences` may ask about.
pub const MAX_PREVIEW_ITEMS: usize = 200;

/// Most executions returned by one `listExecutions`.
pub const MAX_EXECUTION_LIST: u32 = 200;

/// A window identifier: `<namespace>.<name>`, lowercase ASCII letters,
/// digits, `-` and `.`, at most 64 characters.
pub fn is_window_id(id: &str) -> bool {
    id.len() <= 64
        && id
            .split_once('.')
            .is_some_and(|(namespace, name)| !namespace.is_empty() && !name.is_empty())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

/// Whether the agent can read the configuration database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecurityState {
    /// The database is open. Agents before protocol 4 report nothing and
    /// are always unlocked.
    #[default]
    Unlocked,
    /// The database is encrypted and the agent has no key.
    Locked,
    /// There is no database yet; a password must be set up.
    Uninitialized,
    /// An unencrypted database from an earlier version must be migrated.
    LegacyPlaintext,
}

/// How the database key is protected; see `docs/security.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProtectionMode {
    /// A random key in the operating system's credential store.
    SystemManaged,
    /// A key derived from the user's password.
    Password,
}

/// Why a locked agent is locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LockReason {
    /// Started with password protection; the key arrives from the app.
    Startup,
    /// Locked on request (the app's "Lock now").
    Requested,
    /// Locked automatically after a period without activity.
    Idle,
    /// Locked automatically because the computer slept.
    Sleep,
    /// Locked automatically because the screen was locked.
    ScreenLocked,
    /// The system key could not be read from the credential store.
    KeyUnavailable,
}

/// A database key in transit (64 hex digits). Redacted from `Debug` output
/// and zeroized when dropped. Only sent over the authenticated local socket;
/// see `docs/security.md` for the trust boundary.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyMaterial(zeroize::Zeroizing<String>);

impl KeyMaterial {
    pub fn new(hex: zeroize::Zeroizing<String>) -> Self {
        Self(hex)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    fn is_well_formed(&self) -> bool {
        self.0.len() == 64 && self.0.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

impl std::fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyMaterial(<redacted>)")
    }
}
/// Oldest protocol the agent still accepts. Version 1 clients (DeckPilot
/// 0.2) can still connect, e.g. so that a newer app can shut down an older
/// agent during an update.
pub const MIN_PROTOCOL_VERSION: u32 = 1;

/// Oldest agent protocol the configuration app works with. Agents before
/// protocol 4 (DeckPilot 0.3 and earlier) predate the encrypted database;
/// the app can only stop them and start its own agent.
pub const MIN_COMPATIBLE_AGENT_PROTOCOL: u32 = 4;

/// What a connection can do, derived from its negotiated protocol. Clients
/// check these before using a feature, so an older agent is never asked for
/// something it does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    /// `runWorkflow`. Protocol 2.
    Workflows,
    /// `prepareForUpdate`. Protocol 2.
    PrepareForUpdate,
    /// Module namespaced action types and `selectProject` effects. Protocol 3.
    ModuleNamespaces,
    /// The encrypted database: lock state, `unlock`, `lock`, system keys,
    /// native windows, the last workflow report. Protocol 4.
    EncryptedDatabase,
    /// Background executions with progress, cancellation and history.
    /// Protocol 5.
    BackgroundExecutions,
    /// Notifications for the status bar. Protocol 6.
    Notifications,
    /// Shared variables: `listVariables`, `previewReferences` and context
    /// events. Without it the app shows no live values and does not save
    /// references, which the agent could not resolve. Protocol 7.
    Variables,
    /// The work session: its state, history, settings and events. Without
    /// it the app explains that the agent is too old. Protocol 8.
    WorkSession,
    /// The unified variable registry: owners, module variables, canonical
    /// ids and the `unavailable` status. Without it the app inserts the
    /// ids the agent lists (legacy ones included). Protocol 9.
    VariableRegistry,
    /// Plugin requests and events ([`crate::plugins`]). The protocol has
    /// them; whether this agent can use plugins is `getPluginRuntime`.
    /// Protocol 10.
    Plugins,
}

impl Capability {
    pub const ALL: [Self; 10] = [
        Self::Workflows,
        Self::PrepareForUpdate,
        Self::ModuleNamespaces,
        Self::EncryptedDatabase,
        Self::BackgroundExecutions,
        Self::Notifications,
        Self::Variables,
        Self::WorkSession,
        Self::VariableRegistry,
        Self::Plugins,
    ];

    /// The protocol that introduced the capability.
    pub fn since_protocol(self) -> u32 {
        match self {
            Self::Workflows | Self::PrepareForUpdate => 2,
            Self::ModuleNamespaces => 3,
            Self::EncryptedDatabase => 4,
            Self::BackgroundExecutions => 5,
            Self::Notifications => 6,
            Self::Variables => 7,
            Self::WorkSession => 8,
            Self::VariableRegistry => 9,
            Self::Plugins => 10,
        }
    }

    /// Every capability of `protocol`.
    pub fn of_protocol(protocol: u32) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|c| c.since_protocol() <= protocol)
            .collect()
    }
}

/// The protocol both sides speak: the lower of the two.
pub fn negotiate(client: u32, agent: u32) -> u32 {
    client.min(agent)
}

/// What kind of program is connecting. Used for diagnostics and to decide
/// whether the configuration window is already open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientKind {
    Desktop,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMessage {
    Hello {
        protocol: u32,
        token: String,
        client: ClientKind,
    },
    Request {
        id: u64,
        request: Request,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Request {
    GetStatus,
    ListDevices,
    ListProfiles,
    GetActiveProfile,
    SetActiveProfile {
        profile: ProfileId,
    },
    GetActiveProject,
    SelectProject {
        project: ProjectId,
    },
    CycleProject {
        direction: CycleDirection,
    },
    /// Runs an action of the active profile.
    RunAction {
        action: ActionInstanceId,
    },
    /// Runs a workflow of the active profile and answers when it finished.
    /// Since protocol 2. The run is an execution like one started with
    /// `startWorkflow`.
    RunWorkflow {
        workflow: WorkflowId,
    },
    /// Starts a workflow of the active profile in the background and
    /// answers right away. Progress arrives as execution events. Since
    /// protocol 5.
    StartWorkflow {
        workflow: WorkflowId,
    },
    /// Asks a running execution to stop. Running actions finish; nothing
    /// new starts. Since protocol 5.
    CancelExecution {
        execution: ExecutionId,
    },
    /// Running and recorded executions of the active profile, newest first.
    /// Since protocol 5.
    ListExecutions {
        #[serde(default)]
        workflow: Option<WorkflowId>,
        #[serde(default)]
        limit: Option<u32>,
    },
    /// One execution of the active profile with its node reports. Since
    /// protocol 5.
    GetExecution {
        execution: ExecutionId,
    },
    /// Deletes the recorded (finished) executions of the active profile, or
    /// of one of its workflows. Since protocol 5.
    ClearExecutionHistory {
        #[serde(default)]
        workflow: Option<WorkflowId>,
    },
    /// Rescans for devices and redraws every connected device.
    RefreshDevices,
    /// Re-reads configuration from the database after the configuration app
    /// changed it.
    ReloadConfiguration,
    SetBrightness {
        percent: u8,
    },
    GetAutostart,
    SetAutostart {
        enabled: bool,
    },
    /// Starts pushing [`AgentEvent`]s on this connection.
    Subscribe,
    /// Asks the agent to show the configuration window via a connected
    /// desktop client.
    ShowWindow,
    Shutdown,
    /// Releases devices and stops so that an update can replace the agent's
    /// executable. The agent finishes the request it is processing first.
    /// Since protocol 2.
    PrepareForUpdate,
    /// Injects input into a simulated device. Only accepted when the agent
    /// runs with mock devices.
    SimulateInput {
        device: DeviceId,
        control: ControlId,
        input: ControlInput,
    },
    /// Opens the encrypted database with `key`, derived from the user's
    /// password by the configuration app. Since protocol 4.
    Unlock {
        key: KeyMaterial,
    },
    /// Closes the database and forgets the key. Since protocol 4.
    Lock,
    /// Opens a database with system-managed protection with the key from
    /// the credential store: after a lock, or to retry when the store was
    /// unavailable. Also sets up a new database, or encrypts a legacy one,
    /// with a system key. Since protocol 4.
    SystemUnlock,
    /// Switches a password-protected database to system-managed
    /// protection. `key` is the current key, as proof of access. The agent
    /// re-encrypts the database and stays unlocked. Since protocol 4.
    UseSystemKey {
        key: KeyMaterial,
    },
    /// The key of the open database, so that the configuration app can open
    /// it without asking for the password again. Since protocol 4.
    GetDatabaseKey,
    /// Asks the agent to have a native window opened: through a connected
    /// configuration app, or by starting the app for that window only.
    /// Since protocol 4.
    RequestWindow {
        window: String,
    },
    /// The report of the last workflow the agent ran. Since protocol 4.
    GetLastWorkflowReport,
    /// The references available in `context` that match `filter`:
    /// metadata only, never values. Since protocol 7.
    ListVariables {
        context: Box<ResolutionContext>,
        #[serde(default)]
        filter: Box<CatalogFilter>,
    },
    /// What each reference resolves to in `context`, as a preview: values
    /// of sensitive variables are redacted and run-time values are
    /// unavailable. Since protocol 7.
    PreviewReferences {
        context: Box<ResolutionContext>,
        items: Vec<PreviewItem>,
    },
    /// The work session as it is now. Since protocol 8.
    GetWorkSession,
    /// Sessions of every profile, newest first: at most `limit`, only
    /// those requested before `before` (ms) when given. Since protocol 8.
    ListWorkSessions {
        limit: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<i64>,
    },
    /// The work session settings of `profile` (defaults if never saved).
    /// Since protocol 8.
    GetWorkSessionSettings {
        profile: ProfileId,
    },
    /// Saves the work session settings of `settings.profile` after
    /// checking its workflows: the start workflow needs no inputs a
    /// session cannot give, the verification workflow must fulfil the
    /// verification contract. Since protocol 8.
    SaveWorkSessionSettings {
        settings: Box<WorkSessionSettings>,
    },
    /// Decides a flagged gap of `session`: `count` keeps it counted,
    /// otherwise it is excluded. Either can be changed again. Since
    /// protocol 8.
    ResolveWorkSessionReview {
        session: WorkSessionId,
        review: i64,
        count: bool,
    },
    /// Clears the notice of `session`, or, without one, the notice an idle
    /// session shows. Since protocol 8.
    ClearWorkSessionNotices {
        #[serde(default)]
        session: Option<WorkSessionId>,
    },
    /// Starts pushing [`ServerMessage::PluginEvent`]s on this connection.
    /// Since protocol 10.
    SubscribePluginEvents,
    /// Whether plugins can be used here, and why not. Answered whatever
    /// the plugin runtime's state. Since protocol 10.
    GetPluginRuntime,
    /// Installed plugins. Since protocol 10.
    ListPlugins,
    /// One installed plugin in full. Since protocol 10.
    GetPlugin {
        plugin: String,
    },
    /// The plugins the verified registry offers; `refresh` re-reads the
    /// registry first. Since protocol 10.
    ListAvailablePlugins {
        #[serde(default)]
        refresh: bool,
    },
    /// Installs a plugin in state `disabled`, granting the permissions in
    /// `consent` (grant keys). Since protocol 10.
    InstallPlugin {
        source: plugins::PluginInstallSource,
        #[serde(default)]
        consent: Vec<String>,
    },
    /// Grants `consent`, then enables the plugin. Every declared
    /// permission must be granted. Since protocol 10.
    EnablePlugin {
        plugin: String,
        #[serde(default)]
        consent: Vec<String>,
    },
    /// Stops the plugin's host and disables it. Since protocol 10.
    DisablePlugin {
        plugin: String,
    },
    /// Grants declared permissions. Since protocol 10.
    GrantPluginPermissions {
        plugin: String,
        permissions: Vec<String>,
    },
    /// Revokes one permission; the next capability call is refused. Since
    /// protocol 10.
    RevokePluginPermission {
        plugin: String,
        permission: String,
    },
    /// Installs a newer version (`version`, or the latest stable one). An
    /// escalation is applied only if `consent` lists every new or broadened
    /// permission; otherwise the version is staged. Since protocol 10.
    UpdatePlugin {
        plugin: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(default)]
        consent: Vec<String>,
    },
    /// Activates the staged update with consent for its escalations. Since
    /// protocol 10.
    ApprovePluginUpdate {
        plugin: String,
        consent: Vec<String>,
    },
    /// Drops the staged update. Since protocol 10.
    DeclinePluginUpdate {
        plugin: String,
    },
    /// Makes the previous version active again. Since protocol 10.
    RollbackPlugin {
        plugin: String,
    },
    /// Removes the plugin, its files and its records, its storage and, unless
    /// `keep` says otherwise, its settings and credentials. Since protocol
    /// 10 (`keep` is optional and omitted when nothing is kept).
    UninstallPlugin {
        plugin: String,
        #[serde(default, skip_serializing_if = "plugins::PluginRetention::is_default")]
        keep: plugins::PluginRetention,
    },
    /// Runs one action of an enabled plugin in its sandboxed host. Since
    /// protocol 10.
    InvokePluginAction {
        plugin: String,
        action: String,
        #[serde(default)]
        params: BTreeMap<String, serde_json::Value>,
    },
    /// Every plugin installed here at some point, installed now or not.
    /// Since protocol 10.
    ListKnownPlugins,
    /// A plugin's settings: fields, saved and effective values. Since
    /// protocol 10.
    GetPluginSettings {
        plugin: String,
    },
    /// Saves settings values (merged into the saved ones; `null` resets a
    /// setting to its default) after validating them against the
    /// manifest. Since protocol 10.
    SetPluginSettings {
        plugin: String,
        values: BTreeMap<String, serde_json::Value>,
    },
    /// A plugin's credential slots and whether each has a secret (never
    /// the secrets). Since protocol 10.
    ListPluginCredentials {
        plugin: String,
    },
    /// Stores the secret of one credential slot. The answer never carries
    /// it. Since protocol 10.
    SetPluginCredential {
        plugin: String,
        key: String,
        secret: plugins::PluginSecret,
    },
    /// Deletes the secret of one credential slot. Since protocol 10.
    DeletePluginCredential {
        plugin: String,
        key: String,
    },
}

impl Request {
    /// Checks constraints that the type system does not express.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::SetBrightness { percent } if *percent > 100 => {
                Err(format!("brightness {percent} is outside 0-100"))
            }
            Self::SimulateInput { device, .. } if device.serial.is_empty() => {
                Err("device serial must not be empty".into())
            }
            Self::Unlock { key } | Self::UseSystemKey { key } if !key.is_well_formed() => {
                Err("malformed key".into())
            }
            Self::RequestWindow { window } if !is_window_id(window) => {
                Err(format!("invalid window identifier `{window}`"))
            }
            Self::ListExecutions {
                limit: Some(limit), ..
            } if *limit == 0 || *limit > MAX_EXECUTION_LIST => {
                Err(format!("limit must be between 1 and {MAX_EXECUTION_LIST}"))
            }
            Self::PreviewReferences { items, .. } if items.len() > MAX_PREVIEW_ITEMS => Err(
                format!("at most {MAX_PREVIEW_ITEMS} references can be previewed at once"),
            ),
            Self::ListWorkSessions { limit, .. }
                if *limit == 0 || *limit > MAX_WORK_SESSION_LIST =>
            {
                Err(format!(
                    "limit must be between 1 and {MAX_WORK_SESSION_LIST}"
                ))
            }
            Self::GetPlugin { plugin }
            | Self::DisablePlugin { plugin }
            | Self::DeclinePluginUpdate { plugin }
            | Self::RollbackPlugin { plugin }
            | Self::UninstallPlugin { plugin, .. }
            | Self::GetPluginSettings { plugin }
            | Self::ListPluginCredentials { plugin } => plugins::check_text("plugin", plugin),
            Self::SetPluginSettings { plugin, values } => {
                plugins::check_text("plugin", plugin)?;
                plugins::check_settings(values)
            }
            Self::SetPluginCredential {
                plugin,
                key,
                secret,
            } => {
                plugins::check_text("plugin", plugin)?;
                plugins::check_text("credential", key)?;
                plugins::check_secret(secret)
            }
            Self::DeletePluginCredential { plugin, key } => {
                plugins::check_text("plugin", plugin)?;
                plugins::check_text("credential", key)
            }
            Self::InstallPlugin { source, consent } => {
                source.check()?;
                plugins::check_consent(consent)
            }
            Self::EnablePlugin { plugin, consent }
            | Self::ApprovePluginUpdate { plugin, consent }
            | Self::GrantPluginPermissions {
                plugin,
                permissions: consent,
            } => {
                plugins::check_text("plugin", plugin)?;
                plugins::check_consent(consent)
            }
            Self::RevokePluginPermission { plugin, permission } => {
                plugins::check_text("plugin", plugin)?;
                plugins::check_text("permission", permission)
            }
            Self::UpdatePlugin {
                plugin,
                version,
                consent,
            } => {
                plugins::check_text("plugin", plugin)?;
                if let Some(version) = version {
                    plugins::check_text("version", version)?;
                }
                plugins::check_consent(consent)
            }
            Self::InvokePluginAction {
                plugin,
                action,
                params,
            } => {
                plugins::check_text("plugin", plugin)?;
                plugins::check_text("action", action)?;
                if params.len() > plugins::MAX_ACTION_PARAMS {
                    return Err(format!(
                        "an action takes at most {} parameters",
                        plugins::MAX_ACTION_PARAMS
                    ));
                }
                params
                    .keys()
                    .try_for_each(|key| plugins::check_text("parameter", key))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    InvalidRequest,
    Unauthorized,
    UnsupportedProtocol,
    NotFound,
    Unsupported,
    Failed,
    /// The database is locked; unlock it first. Since protocol 4.
    Locked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
    /// A stable, machine-readable reason (plugin errors use
    /// [`plugins::reason`]). Since protocol 10; absent from older agents
    /// and ignored by older clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl IpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            reason: None,
        }
    }

    /// The error with a machine-readable `reason`.
    #[must_use]
    pub fn with_reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.to_owned());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage {
    Welcome {
        /// The newest protocol the agent speaks. The connection uses the
        /// lower of this and the client's (see [`negotiate`]).
        protocol: u32,
        agent_version: String,
        /// What the connection can do. Since protocol 6; derived from the
        /// protocol for older agents.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<Capability>,
    },
    Response {
        id: u64,
        response: Response,
    },
    Error {
        id: Option<u64>,
        error: IpcError,
    },
    Event {
        event: AgentEvent,
    },
    /// A plugin operation's progress or a plugin's new state, after
    /// `subscribePluginEvents`. Since protocol 10.
    PluginEvent {
        event: plugins::PluginEvent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Response {
    Status {
        status: Box<AgentStatus>,
    },
    Devices {
        devices: Vec<DeviceInfo>,
    },
    Profiles {
        profiles: Vec<ProfileInfo>,
        active: Option<ProfileId>,
    },
    ActiveProfile {
        profile: Option<ProfileInfo>,
    },
    ActiveProject {
        project: Option<Project>,
    },
    ActionResult {
        result: ActionResult,
    },
    WorkflowReport {
        report: WorkflowReport,
    },
    /// Answer to `getLastWorkflowReport`. Since protocol 4.
    LastWorkflow {
        report: Option<WorkflowReport>,
    },
    /// Answer to `startWorkflow`. Since protocol 5.
    ExecutionStarted {
        execution: ExecutionSummary,
    },
    /// Answer to `listExecutions`. Since protocol 5.
    Executions {
        executions: Vec<ExecutionSummary>,
    },
    /// Answer to `getExecution`. Since protocol 5.
    Execution {
        execution: Box<ExecutionRecord>,
    },
    /// Answer to `getDatabaseKey`. Since protocol 4.
    DatabaseKey {
        key: KeyMaterial,
    },
    Autostart {
        enabled: bool,
    },
    /// Answer to `listVariables`. Since protocol 7.
    Variables {
        entries: Vec<CatalogEntry>,
    },
    /// Answer to `previewReferences`, in the order asked. Since protocol 7.
    ReferencePreviews {
        previews: Vec<ReferencePreview>,
    },
    /// Answer to `getWorkSession` and `clearWorkSessionNotices`. Since
    /// protocol 8.
    WorkSession {
        snapshot: Box<WorkSessionSnapshot>,
    },
    /// Answer to `listWorkSessions`. Since protocol 8.
    WorkSessions {
        entries: Vec<WorkSessionSummary>,
    },
    /// Answer to `resolveWorkSessionReview`: the session afterwards. Since
    /// protocol 8.
    WorkSessionEntry {
        entry: Box<WorkSessionSummary>,
    },
    /// Answer to `getWorkSessionSettings` and `saveWorkSessionSettings`.
    /// Since protocol 8.
    WorkSessionSettings {
        settings: Box<WorkSessionSettings>,
    },
    /// Answer to `getPluginRuntime`. Since protocol 10.
    PluginRuntime {
        status: Box<plugins::PluginRuntimeStatus>,
    },
    /// Answer to `listPlugins`. Since protocol 10.
    Plugins {
        plugins: Vec<plugins::PluginSummary>,
    },
    /// One installed plugin after `getPlugin` and every plugin change.
    /// Since protocol 10.
    Plugin {
        plugin: Box<plugins::PluginDetails>,
    },
    /// Answer to `listAvailablePlugins`. Since protocol 10.
    AvailablePlugins {
        registry: plugins::RegistryStatus,
        plugins: Vec<plugins::AvailablePlugin>,
    },
    /// Answer to `updatePlugin` and `approvePluginUpdate`. Since protocol
    /// 10.
    PluginUpdate {
        plugin: String,
        outcome: plugins::PluginUpdateOutcome,
    },
    /// Answer to `invokePluginAction`. Since protocol 10.
    PluginActionResult {
        outcome: plugins::PluginActionOutcome,
    },
    /// Answer to `listKnownPlugins`. Since protocol 10.
    KnownPlugins {
        plugins: Vec<plugins::KnownPlugin>,
    },
    /// Answer to `getPluginSettings` and `setPluginSettings`. Since
    /// protocol 10.
    PluginSettings {
        settings: Box<plugins::PluginSettings>,
    },
    /// Answer to `listPluginCredentials`, `setPluginCredential` and
    /// `deletePluginCredential`: the slots, never a secret. Since protocol
    /// 10.
    PluginCredentials {
        plugin: String,
        credentials: Vec<plugins::PluginCredentialSlot>,
    },
    Ok,
}

impl Response {
    /// The response as a client speaking `protocol` understands it: effects
    /// it does not know are removed from action results (`cycleProject`
    /// before protocol 7), and catalogs and previews lose what came with the
    /// variable registry (before protocol 9).
    #[must_use]
    pub fn for_protocol(mut self, protocol: u32) -> Self {
        if protocol < 10 {
            self = self.without_plugins();
        }
        if protocol < 9 {
            self = self.without_registry();
        }
        if protocol >= 7 {
            return self;
        }
        let strip = |result: &mut ActionResult| {
            result
                .effects
                .retain(|effect| matches!(effect, ContextEffect::SelectProject { .. }));
        };
        match &mut self {
            Self::ActionResult { result } => strip(result),
            Self::WorkflowReport { report }
            | Self::LastWorkflow {
                report: Some(report),
            } => report
                .steps
                .iter_mut()
                .for_each(|step| strip(&mut step.result)),
            _ => {}
        }
        self
    }

    /// Nothing a client before protocol 10 cannot explain: catalog entries
    /// owned by a plugin module (`<publisher>.<name>`) are removed, so an
    /// older app never offers what belongs to a plugin (design §2.3 item
    /// 7). Plugin answers never reach such a client: their requests have
    /// `since_protocol` 10. Instances already in profiles are untouched and
    /// show as unavailable there.
    fn without_plugins(self) -> Self {
        let plugin_owned = |entry: &CatalogEntry| {
            entry
                .module
                .as_ref()
                .is_some_and(deckpilot_core::ModuleId::is_plugin)
                || entry
                    .owner
                    .as_ref()
                    .and_then(deckpilot_core::variables::VariableOwner::module)
                    .is_some_and(deckpilot_core::ModuleId::is_plugin)
        };
        match self {
            Self::Variables { entries } => Self::Variables {
                entries: entries.into_iter().filter(|e| !plugin_owned(e)).collect(),
            },
            // A plugin variable (`plugins.<publisher>.<name>.…`) is not a
            // reference for such a client, exactly as before protocol 10.
            Self::ReferencePreviews { previews } => Self::ReferencePreviews {
                previews: previews
                    .into_iter()
                    .map(|preview| {
                        if is_plugin_reference(&preview.reference) {
                            ReferencePreview {
                                status: ReferenceStatus::Invalid {
                                    message: deckpilot_core::variables::ExpressionError::Invalid(
                                        preview.reference.clone(),
                                    )
                                    .to_string(),
                                },
                                ..preview
                            }
                        } else {
                            preview
                        }
                    })
                    .collect(),
            },
            other => other,
        }
    }

    /// Catalogs without module roots and registry metadata, previews with
    /// `permissionDenied` for `unavailable`, as protocol 8 had them.
    fn without_registry(self) -> Self {
        match self {
            Self::Variables { entries } => Self::Variables {
                entries: entries
                    .into_iter()
                    .filter_map(CatalogEntry::for_protocol_8)
                    .collect(),
            },
            Self::ReferencePreviews { previews } => Self::ReferencePreviews {
                previews: previews
                    .into_iter()
                    .map(|preview| match preview.status {
                        ReferenceStatus::Unavailable { .. } => ReferencePreview {
                            status: ReferenceStatus::PermissionDenied,
                            ..preview
                        },
                        _ => preview,
                    })
                    .collect(),
            },
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    /// Sent after every change of the runtime state.
    State {
        status: Box<AgentStatus>,
    },
    /// An action finished. The result message is not included because it may
    /// contain machine-specific details.
    ActionCompleted {
        action: ActionInstanceId,
        name: String,
        status: ActionStatus,
    },
    ShowWindow,
    ShuttingDown,
    /// Asks the configuration app to open a native window by its catalog
    /// identifier (e.g. `security.unlock`). Only the identifier is sent; the
    /// app decides whether the window exists and may open. Since protocol 4.
    OpenWindow {
        window: String,
    },
    /// A workflow started. Since protocol 5.
    ExecutionStarted {
        execution: ExecutionSummary,
    },
    /// A node of a running workflow started or finished, or is retried.
    /// `completed` of `total` nodes are resolved. Since protocol 5.
    ExecutionProgress {
        execution: ExecutionId,
        event: ExecutionEvent,
        completed: u32,
        total: u32,
    },
    /// A workflow finished, failed, was cancelled or interrupted. Since
    /// protocol 5.
    ExecutionFinished {
        execution: ExecutionSummary,
    },
    /// An action asked to show a notification; the app shows it in its
    /// status bar. Since protocol 6.
    Notification {
        notification: deckpilot_core::notify::Notification,
    },
    /// The application's active project of `profile` changed (a key, a
    /// request, a workflow with application scope, a reload). Since
    /// protocol 7.
    ProjectChanged {
        profile: ProfileId,
        project: Option<ProjectId>,
        source: ProjectSource,
    },
    /// Another profile became active. Since protocol 7.
    ProfileChanged {
        profile: ProfileId,
    },
    /// Definitions or values of these shared variables changed. Keys only,
    /// never values. Also names the `system.work.session.…` paths whose
    /// value changed with a work session transition. Since protocol 7.
    SharedVariablesChanged {
        keys: Vec<String>,
    },
    /// This computer's resource bindings changed. Since protocol 7.
    ResourceBindingsChanged,
    /// A device connected or disconnected. Since protocol 7.
    DeviceContextChanged {
        device: Option<DeviceId>,
    },
    /// A run started or finished with this project in its own context.
    /// Since protocol 7.
    ExecutionContextChanged {
        execution: ExecutionId,
        project: Option<ProjectId>,
    },
    /// The work session changed: its state, verification, approval,
    /// notices, or its counted time after a decided gap. Elapsed time is
    /// drawn from `elapsedMs` at `asOfMs`, so the agent does not send one
    /// per tick. Since protocol 8.
    WorkSessionChanged {
        snapshot: Box<WorkSessionSnapshot>,
    },
}

impl Request {
    /// The first protocol that has this request. Clients do not send it on
    /// a connection with an older protocol, and the agent refuses it there.
    pub fn since_protocol(&self) -> u32 {
        match self {
            Self::StartWorkflow { .. }
            | Self::CancelExecution { .. }
            | Self::ListExecutions { .. }
            | Self::GetExecution { .. }
            | Self::ClearExecutionHistory { .. } => 5,
            Self::Unlock { .. }
            | Self::Lock
            | Self::SystemUnlock
            | Self::UseSystemKey { .. }
            | Self::GetDatabaseKey
            | Self::RequestWindow { .. }
            | Self::GetLastWorkflowReport => 4,
            Self::ListVariables { .. } | Self::PreviewReferences { .. } => 7,
            Self::GetWorkSession
            | Self::ListWorkSessions { .. }
            | Self::GetWorkSessionSettings { .. }
            | Self::SaveWorkSessionSettings { .. }
            | Self::ResolveWorkSessionReview { .. }
            | Self::ClearWorkSessionNotices { .. } => 8,
            Self::SubscribePluginEvents
            | Self::GetPluginRuntime
            | Self::ListPlugins
            | Self::GetPlugin { .. }
            | Self::ListAvailablePlugins { .. }
            | Self::InstallPlugin { .. }
            | Self::EnablePlugin { .. }
            | Self::DisablePlugin { .. }
            | Self::GrantPluginPermissions { .. }
            | Self::RevokePluginPermission { .. }
            | Self::UpdatePlugin { .. }
            | Self::ApprovePluginUpdate { .. }
            | Self::DeclinePluginUpdate { .. }
            | Self::RollbackPlugin { .. }
            | Self::UninstallPlugin { .. }
            | Self::InvokePluginAction { .. }
            | Self::ListKnownPlugins
            | Self::GetPluginSettings { .. }
            | Self::SetPluginSettings { .. }
            | Self::ListPluginCredentials { .. }
            | Self::SetPluginCredential { .. }
            | Self::DeletePluginCredential { .. } => 10,
            Self::RunWorkflow { .. } | Self::PrepareForUpdate => 2,
            Self::GetStatus
            | Self::ListDevices
            | Self::ListProfiles
            | Self::GetActiveProfile
            | Self::SetActiveProfile { .. }
            | Self::GetActiveProject
            | Self::SelectProject { .. }
            | Self::CycleProject { .. }
            | Self::RunAction { .. }
            | Self::RefreshDevices
            | Self::ReloadConfiguration
            | Self::SetBrightness { .. }
            | Self::GetAutostart
            | Self::SetAutostart { .. }
            | Self::Subscribe
            | Self::ShowWindow
            | Self::Shutdown
            | Self::SimulateInput { .. } => 1,
        }
    }

    /// The request's name on the wire, e.g. `startWorkflow`.
    pub fn name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
            .unwrap_or_else(|| "request".to_owned())
    }
}

impl AgentEvent {
    /// The first protocol that has this event; clients that speak an older
    /// one do not receive it.
    pub fn since_protocol(&self) -> u32 {
        match self {
            Self::ExecutionStarted { .. }
            | Self::ExecutionProgress { .. }
            | Self::ExecutionFinished { .. } => 5,
            Self::Notification { .. } => 6,
            Self::ProjectChanged { .. }
            | Self::ProfileChanged { .. }
            | Self::SharedVariablesChanged { .. }
            | Self::ResourceBindingsChanged
            | Self::DeviceContextChanged { .. }
            | Self::ExecutionContextChanged { .. } => 7,
            Self::WorkSessionChanged { .. } => 8,
            Self::OpenWindow { .. } => 4,
            Self::State { .. }
            | Self::ActionCompleted { .. }
            | Self::ShowWindow
            | Self::ShuttingDown => 1,
        }
    }

    /// The event as a client speaking `protocol` receives it, or `None`
    /// when it must not receive it at all: events newer than its protocol,
    /// and before protocol 10 the plugin variables
    /// (`plugins.<publisher>.<name>.…`) among changed keys.
    #[must_use]
    pub fn for_protocol(self, protocol: u32) -> Option<Self> {
        if self.since_protocol() > protocol {
            return None;
        }
        match self {
            Self::SharedVariablesChanged { keys } if protocol < 10 => {
                let keys: Vec<String> = keys
                    .into_iter()
                    .filter(|key| !is_plugin_reference(key))
                    .collect();
                (!keys.is_empty()).then_some(Self::SharedVariablesChanged { keys })
            }
            other => Some(other),
        }
    }
}

/// Whether `reference` names a plugin's variable (protocol 10).
fn is_plugin_reference(reference: &str) -> bool {
    deckpilot_core::variables::VariableId::new(reference.trim())
        .is_ok_and(|id| id.plugin_owner().is_some())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    pub id: ProfileId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub model_name: String,
    pub state: ConnectionState,
    pub layout: LayoutStatus,
    /// Name of the page currently shown.
    pub page: Option<String>,
    /// Backend providing the device: `elgato-hid` for USB, `mock` for
    /// simulated devices.
    pub source: String,
    /// Vendor-neutral description of the device: vendor, model, serial,
    /// capabilities, driver and verification status. Absent from older
    /// agents and for models the agent does not know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<DeviceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub version: String,
    pub protocol: u32,
    pub pid: u32,
    /// Seconds since the Unix epoch.
    pub started_at: u64,
    pub data_dir: String,
    pub log_dir: String,
    pub endpoint: String,
    /// Device backend in use, e.g. `elgato-hid` or `mock`.
    pub backend: String,
    pub profiles: Vec<ProfileInfo>,
    pub active_profile: Option<ProfileInfo>,
    pub active_project: Option<Project>,
    pub devices: Vec<DeviceInfo>,
    pub brightness: u8,
    /// Seconds since the Unix epoch of the last device input, if any. Used
    /// to install updates only while the device is idle.
    pub last_input_at: Option<u64>,
    /// Since protocol 4; older agents are always unlocked.
    #[serde(default)]
    pub security: SecurityState,
    /// How the key is protected, once the database exists.
    #[serde(default)]
    pub protection: Option<ProtectionMode>,
    /// Set while locked.
    #[serde(default)]
    pub lock_reason: Option<LockReason>,
    /// Why the configuration could not be opened automatically, e.g. an
    /// unavailable credential store. Never contains key material.
    #[serde(default)]
    pub security_problem: Option<String>,
    /// The agent is reading or creating the system key; the credential
    /// store may be showing a prompt.
    #[serde(default)]
    pub waiting_for_key_store: bool,
    /// Whether this computer's screen lock can be detected (for automatic
    /// locking).
    #[serde(default)]
    pub screen_lock_detection: bool,
    /// Workflows running in the background. Since protocol 6; `None` from
    /// older agents, which cannot say. Updates and restarts wait for them
    /// or ask before cancelling them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running_executions: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_identifiers_are_restricted() {
        assert!(is_window_id("security.unlock"));
        assert!(is_window_id("workflows.execution-summary"));
        for bad in [
            "",
            "security",
            ".unlock",
            "security.",
            "Security.Unlock",
            "a.b/../c",
            "x.<script>",
        ] {
            assert!(!is_window_id(bad), "{bad}");
        }
        assert!(!is_window_id(&format!("a.{}", "b".repeat(64))));
        let request = Request::RequestWindow {
            window: "../../etc".into(),
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn messages_use_tagged_json() {
        let hello = ClientMessage::Hello {
            protocol: 1,
            token: "t".into(),
            client: ClientKind::Desktop,
        };
        assert_eq!(
            serde_json::to_string(&hello).unwrap(),
            r#"{"type":"hello","protocol":1,"token":"t","client":"desktop"}"#
        );
        let request: ClientMessage = serde_json::from_str(
            r#"{"type":"request","id":7,"request":{"type":"cycleProject","direction":"next"}}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            ClientMessage::Request {
                id: 7,
                request: Request::CycleProject {
                    direction: CycleDirection::Next
                }
            }
        );
    }

    #[test]
    fn rejects_unknown_and_invalid_requests() {
        assert!(serde_json::from_str::<Request>(r#"{"type":"formatDisk"}"#).is_err());
        assert!(
            serde_json::from_str::<Request>(r#"{"type":"setBrightness","percent":300}"#).is_err()
        );
        assert!(Request::SetBrightness { percent: 101 }.validate().is_err());
        assert!(Request::SetBrightness { percent: 100 }.validate().is_ok());
        assert!(Request::GetStatus.validate().is_ok());
        let list = |limit| Request::ListExecutions {
            workflow: None,
            limit,
        };
        assert!(list(None).validate().is_ok());
        assert!(list(Some(MAX_EXECUTION_LIST)).validate().is_ok());
        assert!(list(Some(0)).validate().is_err());
        assert!(list(Some(MAX_EXECUTION_LIST + 1)).validate().is_err());
    }

    #[test]
    fn variables_need_protocol_7() {
        let profile = ProfileId::new();
        let request: Request = serde_json::from_value(serde_json::json!({
            "type": "previewReferences",
            "context": {"profile": profile},
            "items": [{"reference": "shared.projects.favorite", "expected": {"type": "project"}}],
        }))
        .unwrap();
        assert_eq!(request.since_protocol(), 7);
        assert_eq!(request.name(), "previewReferences");
        let list: Request = serde_json::from_value(serde_json::json!({
            "type": "listVariables",
            "context": {"profile": profile, "projectSource": "execution"},
        }))
        .unwrap();
        let Request::ListVariables { context, filter } = &list else {
            panic!("listVariables");
        };
        assert_eq!(context.project_source, ProjectSource::Execution);
        assert_eq!(**filter, CatalogFilter::default());
        assert_eq!(list.since_protocol(), 7);
        let too_many = Request::PreviewReferences {
            context: Box::new(ResolutionContext::preview(profile)),
            items: vec![
                PreviewItem {
                    reference: "system.platform".into(),
                    expected: None,
                };
                MAX_PREVIEW_ITEMS + 1
            ],
        };
        assert!(too_many.validate().is_err());

        for (event, name) in [
            (
                AgentEvent::ProjectChanged {
                    profile,
                    project: None,
                    source: ProjectSource::Application,
                },
                "projectChanged",
            ),
            (AgentEvent::ProfileChanged { profile }, "profileChanged"),
            (
                AgentEvent::SharedVariablesChanged {
                    keys: vec!["projects.favorite".into()],
                },
                "sharedVariablesChanged",
            ),
            (
                AgentEvent::ResourceBindingsChanged,
                "resourceBindingsChanged",
            ),
            (
                AgentEvent::DeviceContextChanged { device: None },
                "deviceContextChanged",
            ),
            (
                AgentEvent::ExecutionContextChanged {
                    execution: ExecutionId::new(),
                    project: None,
                },
                "executionContextChanged",
            ),
        ] {
            assert_eq!(event.since_protocol(), 7);
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["type"], name);
            assert_eq!(serde_json::from_value::<AgentEvent>(json).unwrap(), event);
        }
        assert!(
            Capability::of_protocol(6)
                .iter()
                .all(|c| *c != Capability::Variables)
        );
        assert!(Capability::of_protocol(7).contains(&Capability::Variables));
        assert_eq!(
            serde_json::to_value(Capability::Variables).unwrap(),
            "variables"
        );
    }

    #[allow(clippy::too_many_lines)] // Every request and the event.
    #[test]
    fn the_work_session_needs_protocol_8() {
        use crate::work_session::{ElapsedFormat, WorkSessionSettings};
        use deckpilot_core::work_session::WorkSessionSnapshot;
        let profile = ProfileId::new();
        let session = WorkSessionId::new();
        for (json, name) in [
            (
                serde_json::json!({"type": "getWorkSession"}),
                "getWorkSession",
            ),
            (
                serde_json::json!({"type": "listWorkSessions", "limit": 20, "before": 5}),
                "listWorkSessions",
            ),
            (
                serde_json::json!({"type": "getWorkSessionSettings", "profile": profile}),
                "getWorkSessionSettings",
            ),
            (
                serde_json::json!({"type": "saveWorkSessionSettings", "settings": {
                    "profile": profile, "startWorkflow": null, "verificationWorkflow": null,
                    "approvalTtlSeconds": 600, "reviewGapSeconds": 900, "elapsedFormat": "seconds",
                }}),
                "saveWorkSessionSettings",
            ),
            (
                serde_json::json!({"type": "resolveWorkSessionReview",
                                   "session": session, "review": 4, "count": false}),
                "resolveWorkSessionReview",
            ),
            (
                serde_json::json!({"type": "clearWorkSessionNotices", "session": null}),
                "clearWorkSessionNotices",
            ),
            (
                serde_json::json!({"type": "clearWorkSessionNotices"}),
                "clearWorkSessionNotices",
            ),
        ] {
            let request: Request = serde_json::from_value(json.clone()).expect(name);
            assert_eq!(request.since_protocol(), 8, "{name}");
            assert_eq!(request.name(), name);
            assert!(request.validate().is_ok(), "{name}");
        }
        let save: Request = serde_json::from_value(serde_json::json!({
            "type": "saveWorkSessionSettings", "settings": {
                "profile": profile, "approvalTtlSeconds": 600, "reviewGapSeconds": 900,
                "elapsedFormat": "seconds",
            }
        }))
        .unwrap();
        assert_eq!(
            save,
            Request::SaveWorkSessionSettings {
                settings: Box::new(WorkSessionSettings {
                    profile,
                    start_workflow: None,
                    verification_workflow: None,
                    approval_ttl_seconds: 600,
                    review_gap_seconds: 900,
                    elapsed_format: ElapsedFormat::Seconds,
                    updated_at_ms: None,
                })
            }
        );
        // There is no way to operate a session (D11).
        for operation in [
            "startWorkSession",
            "verifyWorkSession",
            "finishWorkSession",
            "stopWorkSession",
        ] {
            assert!(
                serde_json::from_value::<Request>(serde_json::json!({"type": operation})).is_err(),
                "{operation}"
            );
        }
        let list = |limit| Request::ListWorkSessions {
            limit,
            before: None,
        };
        assert!(list(1).validate().is_ok());
        assert!(
            list(crate::work_session::MAX_WORK_SESSION_LIST)
                .validate()
                .is_ok()
        );
        assert!(list(0).validate().is_err());
        assert!(
            list(crate::work_session::MAX_WORK_SESSION_LIST + 1)
                .validate()
                .is_err()
        );

        let event = AgentEvent::WorkSessionChanged {
            snapshot: Box::new(WorkSessionSnapshot::idle(42)),
        };
        assert_eq!(event.since_protocol(), 8);
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "workSessionChanged",
                               "snapshot": {"state": "idle", "elapsedMs": 0, "asOfMs": 42}})
        );
        assert_eq!(serde_json::from_value::<AgentEvent>(json).unwrap(), event);
        assert!(!Capability::of_protocol(7).contains(&Capability::WorkSession));
        assert!(Capability::of_protocol(8).contains(&Capability::WorkSession));
        assert_eq!(
            serde_json::to_value(Capability::WorkSession).unwrap(),
            "workSession"
        );
    }

    #[test]
    fn clients_before_protocol_10_see_nothing_of_plugins() {
        use deckpilot_core::variables::{
            CatalogEntry, ReferenceAvailability, VariableGroup, VariableOwner, VariableRef,
            VariableType,
        };
        let entry = |module: &str| {
            let module = deckpilot_core::ModuleId::new(module).unwrap();
            CatalogEntry {
                reference: VariableRef::parse("projects.current.key").unwrap(),
                label: "A value".into(),
                description: String::new(),
                group: VariableGroup::Module,
                value_type: VariableType::String,
                scope: None,
                module: Some(module.clone()),
                sensitive: false,
                dynamic: false,
                availability: ReferenceAvailability::Available,
                owner: Some(VariableOwner::Module { module }),
                read_only: true,
                deprecated: None,
                declared_scope: None,
            }
        };
        let builtin = entry("projects");
        let plugin = entry("acme.hello-weather");
        let owned_only = CatalogEntry {
            module: None,
            ..plugin.clone()
        };
        let response = Response::Variables {
            entries: vec![builtin.clone(), plugin, owned_only],
        };
        assert_eq!(response.clone().for_protocol(10), response);
        assert_eq!(
            response.for_protocol(9),
            Response::Variables {
                entries: vec![builtin]
            }
        );
        // Answers without plugin data pass unchanged.
        assert_eq!(Response::Ok.for_protocol(9), Response::Ok);
    }

    #[test]
    fn clients_before_protocol_10_see_no_plugin_variables() {
        use deckpilot_core::variables::{ReferencePreview, ReferenceStatus};
        let preview = |reference: &str, status: ReferenceStatus| ReferencePreview {
            reference: reference.into(),
            status,
        };
        let unavailable = ReferenceStatus::Unavailable {
            owner: "acme.hello-weather".into(),
        };
        let previews = Response::ReferencePreviews {
            previews: vec![
                preview("plugins.acme.helloWeather.temperature", unavailable.clone()),
                preview("projects.current.key", unavailable.clone()),
            ],
        };
        assert_eq!(previews.clone().for_protocol(10), previews);
        let Response::ReferencePreviews { previews: old } = previews.for_protocol(9) else {
            panic!("previews")
        };
        // A plugin variable is not a reference, as before protocol 10;
        // a built-in one keeps its status.
        assert!(
            matches!(&old[0].status, ReferenceStatus::Invalid { message }
                if message.contains("is not a reference")),
            "{old:?}"
        );
        assert_eq!(old[1].status, unavailable);

        let changed = AgentEvent::SharedVariablesChanged {
            keys: vec![
                "plugins.acme.helloWeather.temperature".into(),
                "shared.team.name".into(),
            ],
        };
        assert_eq!(changed.clone().for_protocol(10), Some(changed.clone()));
        assert_eq!(
            changed.for_protocol(9),
            Some(AgentEvent::SharedVariablesChanged {
                keys: vec!["shared.team.name".into()]
            })
        );
        let only_plugins = AgentEvent::SharedVariablesChanged {
            keys: vec!["plugins.acme.helloWeather.temperature".into()],
        };
        assert_eq!(only_plugins.for_protocol(9), None);
        assert_eq!(
            AgentEvent::ShowWindow.for_protocol(9),
            Some(AgentEvent::ShowWindow)
        );
        assert_eq!(
            AgentEvent::SharedVariablesChanged { keys: vec![] }.for_protocol(6),
            None
        );
    }

    #[test]
    fn the_variable_registry_needs_protocol_9() {
        use deckpilot_core::variables::{
            CatalogEntry, Deprecation, ReferenceAvailability, ReferencePreview, ReferenceStatus,
            SystemVariable, VariableGroup, VariableId, VariableOwner, VariableRef, VariableType,
        };
        assert!(!Capability::of_protocol(8).contains(&Capability::VariableRegistry));
        assert!(Capability::of_protocol(9).contains(&Capability::VariableRegistry));
        assert_eq!(
            serde_json::to_value(Capability::VariableRegistry).unwrap(),
            "variableRegistry"
        );
        assert_eq!(PROTOCOL_VERSION, 10);
        let projects = deckpilot_core::ModuleId::new("projects").unwrap();
        let legacy = CatalogEntry {
            reference: VariableRef::System(SystemVariable::ProjectCurrentKey),
            label: "Project key".into(),
            description: String::new(),
            group: VariableGroup::System,
            value_type: VariableType::String,
            scope: None,
            module: Some(projects.clone()),
            sensitive: false,
            dynamic: true,
            availability: ReferenceAvailability::Unavailable,
            owner: Some(VariableOwner::Module {
                module: projects.clone(),
            }),
            read_only: true,
            deprecated: Some(Deprecation {
                canonical: VariableId::new("projects.current.key").unwrap(),
                since: "0.6.0".into(),
            }),
            declared_scope: Some(deckpilot_core::variables::DeclaredScope::Project),
        };
        let canonical = CatalogEntry {
            reference: VariableRef::parse("projects.current.key").unwrap(),
            group: VariableGroup::Module,
            deprecated: None,
            ..legacy.clone()
        };
        let response = Response::Variables {
            entries: vec![legacy.clone(), canonical.clone()],
        };
        // Protocol 9 clients get everything.
        assert_eq!(response.clone().for_protocol(9), response);
        // Older clients get the catalog they knew: no module root they could
        // not parse, no registry metadata, `notInContext` for `unavailable`.
        let Response::Variables { entries } = response.for_protocol(8) else {
            panic!("variables");
        };
        assert_eq!(entries.len(), 1);
        let old = serde_json::to_value(&entries[0]).unwrap();
        assert_eq!(old["reference"], "system.project.current.key");
        assert_eq!(old["availability"], "notInContext");
        for field in ["owner", "readOnly", "deprecated", "declaredScope"] {
            assert!(old.get(field).is_none(), "{field}");
        }
        let previews = Response::ReferencePreviews {
            previews: vec![ReferencePreview {
                reference: "system.project.current.key".into(),
                status: ReferenceStatus::Unavailable {
                    owner: "projects".into(),
                },
            }],
        };
        let Response::ReferencePreviews { previews: old } = previews.clone().for_protocol(7) else {
            panic!("previews");
        };
        assert_eq!(old[0].status, ReferenceStatus::PermissionDenied);
        assert_eq!(previews.clone().for_protocol(9), previews);
    }

    #[test]
    fn older_clients_never_see_new_effects() {
        let project = ProjectId::new();
        let result = ActionResult::success("switched")
            .with_effect(ContextEffect::SelectProject { project })
            .with_effect(ContextEffect::CycleProject {
                direction: CycleDirection::Next,
                wrap_around: true,
            });
        let response = Response::ActionResult {
            result: result.clone(),
        };
        let Response::ActionResult { result: old } = response.clone().for_protocol(6) else {
            panic!("an action result");
        };
        assert_eq!(old.effects, vec![ContextEffect::SelectProject { project }]);
        assert_eq!(
            response.clone().for_protocol(7),
            Response::ActionResult { result }
        );
        assert_eq!(Response::Ok.for_protocol(1), Response::Ok);
    }

    /// The descriptor is an additive field: older agents send none, and
    /// older clients ignore it.
    #[test]
    fn device_info_descriptor_is_optional_both_ways() {
        let old = serde_json::json!({
            "id": {"model": "elgato.streamdeck.mk2", "serial": "A1"},
            "modelName": "Stream Deck MK.2",
            "state": "connected",
            "layout": {"type": "missing"},
            "page": null,
            "source": "elgato-hid",
        });
        let info: DeviceInfo = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(info.descriptor, None);
        assert_eq!(serde_json::to_value(&info).unwrap(), old);

        let catalog = deckpilot_device::ModelCatalog::builtin();
        let descriptor = catalog
            .describe(
                &info.id,
                Some(deckpilot_device::DriverInfo {
                    id: "elgato-hid".into(),
                    version: "0.6.0".into(),
                }),
            )
            .unwrap();
        let new = DeviceInfo {
            descriptor: Some(descriptor),
            ..info
        };
        let json = serde_json::to_value(&new).unwrap();
        assert_eq!(json["descriptor"]["vendorId"], "elgato");
        assert_eq!(json["descriptor"]["driver"]["id"], "elgato-hid");
        assert_eq!(serde_json::from_value::<DeviceInfo>(json).unwrap(), new);
    }

    #[test]
    fn execution_events_need_protocol_5() {
        let event = AgentEvent::ExecutionProgress {
            execution: ExecutionId::new(),
            event: ExecutionEvent::NodeStarted {
                node: deckpilot_core::graph::NodeId::new("build").unwrap(),
            },
            completed: 0,
            total: 3,
        };
        assert_eq!(event.since_protocol(), 5);
        assert_eq!(AgentEvent::ShowWindow.since_protocol(), 1);
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "executionProgress");
        assert_eq!(json["event"]["type"], "nodeStarted");
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one case per request
    fn plugins_need_protocol_10() {
        use crate::plugins::{PluginInstallSource, PluginStage, PluginState};
        assert!(!Capability::of_protocol(9).contains(&Capability::Plugins));
        assert!(Capability::of_protocol(10).contains(&Capability::Plugins));
        assert_eq!(
            serde_json::to_value(Capability::Plugins).unwrap(),
            "plugins"
        );
        for (json, name) in [
            (
                serde_json::json!({"type": "getPluginRuntime"}),
                "getPluginRuntime",
            ),
            (serde_json::json!({"type": "listPlugins"}), "listPlugins"),
            (
                serde_json::json!({"type": "getPlugin", "plugin": "acme.hello-weather"}),
                "getPlugin",
            ),
            (
                serde_json::json!({"type": "listAvailablePlugins"}),
                "listAvailablePlugins",
            ),
            (
                serde_json::json!({"type": "installPlugin",
                    "source": {"type": "registry", "plugin": "acme.hello-weather"},
                    "consent": ["network:api.open-meteo.com"]}),
                "installPlugin",
            ),
            (
                serde_json::json!({"type": "installPlugin",
                    "source": {"type": "file", "path": "/tmp/a.dpplugin"}}),
                "installPlugin",
            ),
            (
                serde_json::json!({"type": "enablePlugin", "plugin": "acme.x", "consent": []}),
                "enablePlugin",
            ),
            (
                serde_json::json!({"type": "disablePlugin", "plugin": "acme.x"}),
                "disablePlugin",
            ),
            (
                serde_json::json!({"type": "grantPluginPermissions", "plugin": "acme.x",
                    "permissions": ["notifications"]}),
                "grantPluginPermissions",
            ),
            (
                serde_json::json!({"type": "revokePluginPermission", "plugin": "acme.x",
                    "permission": "notifications"}),
                "revokePluginPermission",
            ),
            (
                serde_json::json!({"type": "updatePlugin", "plugin": "acme.x"}),
                "updatePlugin",
            ),
            (
                serde_json::json!({"type": "approvePluginUpdate", "plugin": "acme.x",
                    "consent": ["notifications"]}),
                "approvePluginUpdate",
            ),
            (
                serde_json::json!({"type": "declinePluginUpdate", "plugin": "acme.x"}),
                "declinePluginUpdate",
            ),
            (
                serde_json::json!({"type": "rollbackPlugin", "plugin": "acme.x"}),
                "rollbackPlugin",
            ),
            (
                serde_json::json!({"type": "uninstallPlugin", "plugin": "acme.x"}),
                "uninstallPlugin",
            ),
            (
                serde_json::json!({"type": "invokePluginAction", "plugin": "acme.x",
                    "action": "greet", "params": {"name": "Ada", "count": 2}}),
                "invokePluginAction",
            ),
        ] {
            let request: Request = serde_json::from_value(json).unwrap();
            assert_eq!(request.since_protocol(), 10, "{name}");
            assert_eq!(request.name(), name);
            request.validate().unwrap();
        }
        assert_eq!(Request::SubscribePluginEvents.since_protocol(), 10);
        let message = ServerMessage::PluginEvent {
            event: crate::plugins::PluginEvent::Progress {
                plugin: "acme.x".into(),
                operation: crate::plugins::PluginOperation::Install,
                stage: PluginStage::Verifying,
                message: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            serde_json::json!({"type": "pluginEvent", "event": {"type": "progress",
                "plugin": "acme.x", "operation": "install", "stage": "verifying"}})
        );
        let changed: crate::plugins::PluginEvent =
            serde_json::from_value(serde_json::json!({"type": "changed", "plugin": "acme.x",
                "state": "enabled"}))
            .unwrap();
        assert_eq!(
            changed,
            crate::plugins::PluginEvent::Changed {
                plugin: "acme.x".into(),
                state: Some(PluginState::Enabled)
            }
        );
        // Bounds are checked before the agent sees a request.
        let long = "a".repeat(crate::plugins::MAX_PLUGIN_TEXT + 1);
        for bad in [
            Request::GetPlugin {
                plugin: long.clone(),
            },
            Request::GetPlugin {
                plugin: "acme x".into(),
            },
            Request::EnablePlugin {
                plugin: "acme.x".into(),
                consent: vec![String::new()],
            },
            Request::EnablePlugin {
                plugin: "acme.x".into(),
                consent: vec!["notifications".into(); crate::plugins::MAX_CONSENT_KEYS + 1],
            },
            Request::InstallPlugin {
                source: PluginInstallSource::File {
                    path: String::new(),
                },
                consent: vec![],
            },
            Request::InvokePluginAction {
                plugin: "acme.x".into(),
                action: "greet".into(),
                params: (0..=crate::plugins::MAX_ACTION_PARAMS)
                    .map(|i| (format!("p{i}"), serde_json::Value::Null))
                    .collect(),
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn error_reasons_are_optional_both_ways() {
        // A client of protocol 9 has no `reason` field and ignores it.
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Protocol9Error {
            code: ErrorCode,
            message: String,
        }
        // An older agent's error parses; one without a reason serializes as
        // before, so older clients read it unchanged.
        let old: IpcError = serde_json::from_str(r#"{"code":"failed","message":"m"}"#).unwrap();
        assert_eq!(old, IpcError::new(ErrorCode::Failed, "m"));
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"code":"failed","message":"m"}"#
        );
        let new = IpcError::new(ErrorCode::Unsupported, "no plugins")
            .with_reason(crate::plugins::reason::PLUGINS_UNAVAILABLE);
        let json = serde_json::to_value(&new).unwrap();
        assert_eq!(json["reason"], "pluginsUnavailable");
        serde_json::from_value::<Protocol9Error>(json).unwrap();
    }

    #[test]
    fn plugin_data_requests_are_additive_and_never_show_a_secret() {
        use crate::plugins::{PluginRetention, PluginSecret};
        // `uninstallPlugin` without `keep` keeps its original form.
        let old = r#"{"type":"uninstallPlugin","plugin":"acme.x"}"#;
        let parsed: Request = serde_json::from_str(old).unwrap();
        assert_eq!(
            parsed,
            Request::UninstallPlugin {
                plugin: "acme.x".into(),
                keep: PluginRetention::default()
            }
        );
        assert_eq!(serde_json::to_string(&parsed).unwrap(), old);
        let keeping: Request = serde_json::from_str(
            r#"{"type":"uninstallPlugin","plugin":"acme.x","keep":{"credentials":true}}"#,
        )
        .unwrap();
        assert!(matches!(
            keeping,
            Request::UninstallPlugin {
                keep: PluginRetention {
                    credentials: true,
                    settings: false
                },
                ..
            }
        ));
        let set = Request::SetPluginCredential {
            plugin: "acme.x".into(),
            key: "token".into(),
            secret: PluginSecret::new("s3cret-value"),
        };
        assert_eq!(set.since_protocol(), 10);
        set.validate().unwrap();
        assert!(!format!("{set:?}").contains("s3cret-value"));
        let empty = Request::SetPluginCredential {
            plugin: "acme.x".into(),
            key: "token".into(),
            secret: PluginSecret::new(""),
        };
        assert!(empty.validate().is_err());
        let too_many = Request::SetPluginSettings {
            plugin: "acme.x".into(),
            values: (0..=crate::plugins::MAX_SETTINGS_VALUES)
                .map(|i| (format!("s{i}"), serde_json::Value::Null))
                .collect(),
        };
        assert!(too_many.validate().is_err());
        for request in [
            Request::ListKnownPlugins,
            Request::GetPluginSettings {
                plugin: "acme.x".into(),
            },
            Request::ListPluginCredentials {
                plugin: "acme.x".into(),
            },
        ] {
            assert_eq!(request.since_protocol(), 10);
        }
    }
}
