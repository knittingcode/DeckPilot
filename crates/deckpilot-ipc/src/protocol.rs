//! Versioned message types.
//!
//! Messages are JSON objects tagged by `type`, one per line. A client sends
//! [`ClientMessage::Hello`] first; afterwards it sends
//! [`ClientMessage::Request`]s with a client-chosen `id` that the agent echoes
//! in its [`ServerMessage::Response`] or [`ServerMessage::Error`]. After a
//! [`Request::Subscribe`], the agent also pushes [`ServerMessage::Event`]s.
//!
//! Incompatible changes require a new [`PROTOCOL_VERSION`].

use serde::{Deserialize, Serialize};

use deckpilot_core::{
    ActionInstanceId, ActionResult, ActionStatus, CycleDirection, LayoutStatus, ProfileId, Project,
    ProjectId, WorkflowId, WorkflowReport,
};
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId};

/// Protocol spoken by this build.
///
/// Version 3: action results only report `selectProject` effects (project
/// switching is computed by the projects module), and action types use
/// module namespaces such as `projects.next`.
///
/// Version 4: the encrypted database. The agent starts locked and is
/// unlocked with the database key (`unlock`, `lock`, `getDatabaseKey`); the
/// status reports the [`SecurityState`].
pub const PROTOCOL_VERSION: u32 = 4;

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
    /// Runs a workflow of the active profile. Since protocol 2.
    RunWorkflow {
        workflow: WorkflowId,
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
}

impl IpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage {
    Welcome {
        protocol: u32,
        agent_version: String,
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
    /// Answer to `getDatabaseKey`. Since protocol 4.
    DatabaseKey {
        key: KeyMaterial,
    },
    Autostart {
        enabled: bool,
    },
    Ok,
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
    }
}
