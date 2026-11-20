//! Simulated presses (protocol 14, capability `simulateInput`,
//! `docs/editor-navigation-and-simulation.md`).
//!
//! The app asks the agent to press a control of a connected device as the
//! user would: a single press, a double press or a long press
//! ([`PressKind`]). The agent checks that the device shows the profile,
//! layout and page the app shows, that the control exists and takes a
//! press, and that nothing is locked; then it feeds press and release edges
//! with their timing into its own gesture engine on a lane of its own (the
//! device's physical keys are untouched) and runs what the engine
//! recognizes through the usual dispatch, as
//! [`deckpilot_core::RunOrigin::Simulated`]. Nothing is written to the
//! device but the key images the action itself changes.
//!
//! A press whose bindings may change something outside DeckPilot (an
//! action whose effect is `modifies`, a workflow with such a step) is not
//! run until the request carries `confirmed`: the agent answers
//! [`SimulationOutcome::ConfirmationRequired`] with the actions' names.
//! Confirmation never bypasses the settings that gate those actions on this
//! computer (running commands, Docker changes, plugin permissions); they
//! apply to a simulated press as to a real one.

use serde::{Deserialize, Serialize};

/// Most action names a [`SimulationOutcome::ConfirmationRequired`] lists.
pub const MAX_CONFIRMATION_NAMES: usize = 32;

/// How the control is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PressKind {
    /// Press and release.
    Single,
    /// Two presses and releases, the second inside the key's double-click
    /// window.
    Double,
    /// Press, hold for the key's long-press time, release.
    Long,
}

/// What ran for a simulated press: an edge of a key without gestures, or
/// the gesture the engine recognized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SimulatedTrigger {
    Press,
    Release,
    Click,
    DoubleClick,
    LongPress,
    /// A trigger this version does not know.
    #[serde(other)]
    Other,
}

/// Why a simulated press was not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SimulationRefusal {
    /// DeckPilot is locked.
    Locked,
    /// No profile is active.
    NoActiveProfile,
    /// The device is not connected.
    DeviceNotConnected,
    /// The active profile is another one.
    WrongProfile,
    /// The device shows another layout (or none).
    WrongLayout,
    /// The device shows another page.
    WrongPage,
    /// The device model has no such control.
    UnknownControl,
    /// The control cannot be pressed (a touch strip, a dial's rotation).
    IncompatibleControl,
    /// The binding could not run.
    Failed,
    /// A reason this version does not know.
    #[serde(other)]
    Other,
}

impl SimulationRefusal {
    /// The refusal's name on the wire, e.g. `wrongPage`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::NoActiveProfile => "noActiveProfile",
            Self::DeviceNotConnected => "deviceNotConnected",
            Self::WrongProfile => "wrongProfile",
            Self::WrongLayout => "wrongLayout",
            Self::WrongPage => "wrongPage",
            Self::UnknownControl => "unknownControl",
            Self::IncompatibleControl => "incompatibleControl",
            Self::Failed => "failed",
            Self::Other => "other",
        }
    }
}

/// What a simulated press did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SimulationOutcome {
    /// The press went through the gesture engine; `dispatched` are the
    /// triggers whose bindings ran, in order (`press` and `release` for a
    /// key without gestures, else the gesture recognized). Empty when the
    /// control has no binding for what was pressed. Actions and workflows
    /// report as usual (executions, notifications); a plugin action runs
    /// in the background.
    Ran { dispatched: Vec<SimulatedTrigger> },
    /// Nothing ran: the press may change something outside DeckPilot.
    /// Send it again with `confirmed` to run it.
    #[serde(rename_all = "camelCase")]
    ConfirmationRequired {
        action_names: Vec<String>,
        reason: String,
    },
    /// Nothing ran.
    Refused {
        code: SimulationRefusal,
        message: String,
    },
}

impl SimulationOutcome {
    pub fn refused(code: SimulationRefusal, message: impl Into<String>) -> Self {
        Self::Refused {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_are_tagged_by_status() {
        assert_eq!(
            serde_json::to_value(SimulationOutcome::Ran {
                dispatched: vec![SimulatedTrigger::DoubleClick]
            })
            .unwrap(),
            serde_json::json!({"status": "ran", "dispatched": ["doubleClick"]})
        );
        assert_eq!(
            serde_json::to_value(SimulationOutcome::ConfirmationRequired {
                action_names: vec!["Deploy".into()],
                reason: "runs a command".into(),
            })
            .unwrap(),
            serde_json::json!({"status": "confirmationRequired",
                "actionNames": ["Deploy"], "reason": "runs a command"})
        );
        let refused = SimulationOutcome::refused(SimulationRefusal::WrongPage, "another page");
        assert_eq!(
            serde_json::to_value(&refused).unwrap(),
            serde_json::json!({"status": "refused", "code": "wrongPage",
                "message": "another page"})
        );
        for code in [
            SimulationRefusal::Locked,
            SimulationRefusal::NoActiveProfile,
            SimulationRefusal::DeviceNotConnected,
            SimulationRefusal::WrongProfile,
            SimulationRefusal::WrongLayout,
            SimulationRefusal::WrongPage,
            SimulationRefusal::UnknownControl,
            SimulationRefusal::IncompatibleControl,
            SimulationRefusal::Failed,
            SimulationRefusal::Other,
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
        }
        assert_eq!(
            serde_json::from_value::<SimulationRefusal>("somethingNew".into()).unwrap(),
            SimulationRefusal::Other
        );
        for (kind, name) in [
            (PressKind::Single, "single"),
            (PressKind::Double, "double"),
            (PressKind::Long, "long"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), name);
        }
    }
}
