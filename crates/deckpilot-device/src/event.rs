use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ControlId, ControlInput, DeviceModelId};

/// Identifies one physical device: its model plus a serial number.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceId {
    pub model: DeviceModelId,
    pub serial: String,
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.model, self.serial)
    }
}

/// Events produced by a device backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DeviceEvent {
    Connected {
        device: DeviceId,
    },
    Disconnected {
        device: DeviceId,
    },
    Input {
        device: DeviceId,
        control: ControlId,
        input: ControlInput,
    },
}

impl DeviceEvent {
    pub fn device(&self) -> &DeviceId {
        match self {
            Self::Connected { device }
            | Self::Disconnected { device }
            | Self::Input { device, .. } => device,
        }
    }
}
