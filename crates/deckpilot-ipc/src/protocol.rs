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
    ProjectId,
};
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId};

pub const PROTOCOL_VERSION: u32 = 1;

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
    /// Injects input into a simulated device. Only accepted when the agent
    /// runs with mock devices.
    SimulateInput {
        device: DeviceId,
        control: ControlId,
        input: ControlInput,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_use_tagged_json() {
        let hello = ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
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
