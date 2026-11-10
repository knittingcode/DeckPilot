use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::TouchGesture;

/// Family of a physical control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ControlKind {
    Key,
    Encoder,
    TouchZone,
}

/// Identifies one control on a device, independent of any particular model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ControlId {
    Key { index: u16 },
    Encoder { index: u8 },
    TouchZone { index: u8 },
}

impl ControlId {
    pub fn kind(self) -> ControlKind {
        match self {
            Self::Key { .. } => ControlKind::Key,
            Self::Encoder { .. } => ControlKind::Encoder,
            Self::TouchZone { .. } => ControlKind::TouchZone,
        }
    }
}

impl fmt::Display for ControlId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key { index } => write!(f, "key {index}"),
            Self::Encoder { index } => write!(f, "encoder {index}"),
            Self::TouchZone { index } => write!(f, "touch zone {index}"),
        }
    }
}

/// A single input that occurred on a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ControlInput {
    /// Key press or encoder press.
    Press,
    /// Key release or encoder release.
    Release,
    /// Encoder rotation in detents; positive is clockwise.
    Rotate { delta: i8 },
    /// Gesture on a touch zone.
    Touch { gesture: TouchGesture },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InputError {
    #[error("device has no {0}")]
    UnknownControl(ControlId),
    #[error("{control} does not support input {input:?}")]
    UnsupportedInput {
        control: ControlId,
        input: ControlInput,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_ids_serialize_with_kind_tag() {
        let json = serde_json::to_string(&ControlId::Encoder { index: 2 }).unwrap();
        assert_eq!(json, r#"{"kind":"encoder","index":2}"#);
        let back: ControlId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ControlId::Encoder { index: 2 });
    }
}
