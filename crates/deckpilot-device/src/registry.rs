use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{DeviceEvent, DeviceId};

/// Connection state of a known device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub device: DeviceId,
    pub state: ConnectionState,
}

/// Tracks every device seen during a session, including disconnected ones, so
/// that hotplug and multiple simultaneous devices are handled uniformly.
#[derive(Debug, Clone, Default)]
pub struct DeviceRegistry {
    devices: BTreeMap<DeviceId, ConnectionState>,
}

impl DeviceRegistry {
    /// Applies a device event. Returns `true` when the connection state changed.
    pub fn apply(&mut self, event: &DeviceEvent) -> bool {
        let new_state = match event {
            DeviceEvent::Connected { .. } => ConnectionState::Connected,
            DeviceEvent::Disconnected { .. } => ConnectionState::Disconnected,
            DeviceEvent::Input { .. } => return false,
        };
        let previous = self.devices.insert(event.device().clone(), new_state);
        previous != Some(new_state)
    }

    pub fn state(&self, device: &DeviceId) -> Option<ConnectionState> {
        self.devices.get(device).copied()
    }

    pub fn is_connected(&self, device: &DeviceId) -> bool {
        self.state(device) == Some(ConnectionState::Connected)
    }

    pub fn statuses(&self) -> Vec<DeviceStatus> {
        self.devices
            .iter()
            .map(|(device, state)| DeviceStatus {
                device: device.clone(),
                state: *state,
            })
            .collect()
    }

    pub fn connected(&self) -> impl Iterator<Item = &DeviceId> {
        self.devices
            .iter()
            .filter(|(_, state)| **state == ConnectionState::Connected)
            .map(|(device, _)| device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlId, ControlInput, DeviceModelId};

    fn device(serial: &str) -> DeviceId {
        DeviceId {
            model: DeviceModelId::new("test.model"),
            serial: serial.into(),
        }
    }

    #[test]
    fn tracks_hotplug_for_multiple_devices() {
        let mut registry = DeviceRegistry::default();
        assert!(registry.apply(&DeviceEvent::Connected {
            device: device("a")
        }));
        assert!(registry.apply(&DeviceEvent::Connected {
            device: device("b")
        }));
        assert!(!registry.apply(&DeviceEvent::Connected {
            device: device("a")
        }));
        assert_eq!(registry.connected().count(), 2);

        assert!(registry.apply(&DeviceEvent::Disconnected {
            device: device("a")
        }));
        assert_eq!(
            registry.state(&device("a")),
            Some(ConnectionState::Disconnected)
        );
        assert!(registry.is_connected(&device("b")));
        assert_eq!(registry.statuses().len(), 2);
    }

    #[test]
    fn input_events_do_not_change_state() {
        let mut registry = DeviceRegistry::default();
        let changed = registry.apply(&DeviceEvent::Input {
            device: device("a"),
            control: ControlId::Key { index: 0 },
            input: ControlInput::Press,
        });
        assert!(!changed);
        assert_eq!(registry.state(&device("a")), None);
    }
}
