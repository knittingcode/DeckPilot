//! Which controls and inputs an action can be bound to, and whether a
//! binding is compatible with the control it is on. See
//! `docs/device-controls.md`.
//!
//! An [`ActionType`] declares [`ActionControls`]: per control kind, the
//! inputs it can react to. A type without a declaration (a plugin's from
//! before declarations, a stored definition) gets [`ActionControls::legacy`]:
//! keys only. [`compatibility`] decides a binding from the declaration, the
//! control's [`ControlCapabilities`] and the binding's [`Trigger`].
//! Workflows, pages, Back and stateful bindings are not actions: they keep
//! their own rules.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::{ControlCapabilities, ControlKind, InputCapability};

use crate::{ActionType, Trigger};

/// Most inputs an action may declare for one control kind.
pub const MAX_INPUTS_PER_KIND: usize = 8;

/// The inputs of a key an action without a declaration can be bound to,
/// and the default of one-shot actions: press and release, click, double
/// click and long press.
pub const KEY_INPUTS: [InputCapability; 5] = [
    InputCapability::Press,
    InputCapability::Release,
    InputCapability::Click,
    InputCapability::DoubleClick,
    InputCapability::LongPress,
];

/// The encoder inputs of an action that does the same thing once per
/// event, with no amount (e.g. *Next page*): either rotation, a press or a
/// click of a pressable encoder. It runs once per rotation event the
/// device reports, whatever its number of detents: nothing passes the
/// amount on.
pub const STEP_ENCODER_INPUTS: [InputCapability; 4] = [
    InputCapability::RotateClockwise,
    InputCapability::RotateCounterClockwise,
    InputCapability::Press,
    InputCapability::Click,
];

/// Per control kind, the inputs an action can be bound to. A kind that is
/// absent is not supported. Serialized as the map itself, e.g.
/// `{"key": ["press", "click"], "encoder": ["rotate-clockwise"]}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionControls {
    pub bindings: BTreeMap<ControlKind, BTreeSet<InputCapability>>,
}

impl ActionControls {
    /// No control at all; add kinds with [`ActionControls::with`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or extends) `kind` with `inputs`.
    #[must_use]
    pub fn with(
        mut self,
        kind: ControlKind,
        inputs: impl IntoIterator<Item = InputCapability>,
    ) -> Self {
        self.bindings.entry(kind).or_default().extend(inputs);
        self
    }

    /// Keys only, with [`KEY_INPUTS`]: the declaration of one-shot
    /// actions.
    pub fn keys() -> Self {
        Self::new().with(ControlKind::Key, KEY_INPUTS)
    }

    /// Keys with [`KEY_INPUTS`] and encoders with
    /// [`STEP_ENCODER_INPUTS`]: actions that step once per event.
    pub fn keys_and_steps() -> Self {
        Self::keys().with(ControlKind::Encoder, STEP_ENCODER_INPUTS)
    }

    /// What a type without a declaration gets: keys only, never encoders
    /// or touch zones (the same as [`ActionControls::keys`]).
    pub fn legacy() -> Self {
        Self::keys()
    }

    /// The kinds the action supports, in a stable order.
    pub fn kinds(&self) -> Vec<ControlKind> {
        self.bindings.keys().copied().collect()
    }

    /// The inputs the action supports on `kind`; `None`: not the kind.
    pub fn inputs(&self, kind: ControlKind) -> Option<&BTreeSet<InputCapability>> {
        self.bindings.get(&kind)
    }

    pub fn supports(&self, kind: ControlKind, input: InputCapability) -> bool {
        self.inputs(kind)
            .is_some_and(|inputs| inputs.contains(&input))
    }

    /// Problems of a declaration: no kind, a kind without inputs, an input
    /// a control of the kind can never produce (a rotation on a key, a tap
    /// on an encoder), more than [`MAX_INPUTS_PER_KIND`] inputs.
    pub fn issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if self.bindings.is_empty() {
            issues.push("no control kind is declared".to_owned());
        }
        for (kind, inputs) in &self.bindings {
            if inputs.is_empty() {
                issues.push(format!("{kind}: no input is declared"));
            }
            if inputs.len() > MAX_INPUTS_PER_KIND {
                issues.push(format!(
                    "{kind}: more than {MAX_INPUTS_PER_KIND} inputs are declared"
                ));
            }
            for input in inputs.iter().filter(|input| !input.applies_to(*kind)) {
                issues.push(format!("{kind}: a {kind} has no {input}"));
            }
        }
        issues
    }
}

impl ActionType {
    /// The declared controls, or [`ActionControls::legacy`] without a
    /// declaration.
    pub fn effective_controls(&self) -> Cow<'_, ActionControls> {
        self.controls
            .as_ref()
            .map_or_else(|| Cow::Owned(ActionControls::legacy()), Cow::Borrowed)
    }

    /// Whether a binding of this type to a control with `capabilities`
    /// for `trigger` can run; see [`compatibility`].
    pub fn compatibility(
        &self,
        capabilities: &ControlCapabilities,
        trigger: Trigger,
    ) -> Compatibility {
        compatibility(self, capabilities, trigger)
    }
}

/// Whether a binding can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "camelCase")]
pub enum Compatibility {
    Compatible,
    Incompatible(IncompatibilityReason),
}

impl Compatibility {
    pub fn is_compatible(&self) -> bool {
        matches!(self, Self::Compatible)
    }

    /// The reason, when incompatible.
    pub fn reason(&self) -> Option<&IncompatibilityReason> {
        match self {
            Self::Compatible => None,
            Self::Incompatible(reason) => Some(reason),
        }
    }
}

/// Why an action cannot run from a binding.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum IncompatibilityReason {
    /// The action cannot be bound to a control of this kind.
    #[error("the action cannot be bound to {} {kind}; {}", article(*kind), kinds_supported(supported))]
    ControlKindUnsupported {
        kind: ControlKind,
        /// The kinds it supports, in a stable order.
        supported: Vec<ControlKind>,
    },
    /// The action supports the kind, but not this input on it.
    #[error("the action does not react to {input} on {} {kind}; it supports {}", article(*kind), list(supported))]
    InputUnsupportedByAction {
        kind: ControlKind,
        input: InputCapability,
        /// The inputs it supports on the kind.
        supported: Vec<InputCapability>,
    },
    /// The control cannot produce the input (e.g. a double click on a
    /// dial that cannot be pressed, a long touch on a surface without it).
    #[error("this {kind} has no {input}")]
    InputUnsupportedByControl {
        kind: ControlKind,
        input: InputCapability,
    },
}

fn article(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::Encoder => "an",
        ControlKind::Key | ControlKind::TouchZone => "a",
    }
}

fn kinds_supported(kinds: &[ControlKind]) -> String {
    let plural = |kind: &ControlKind| match kind {
        ControlKind::Key => "keys",
        ControlKind::Encoder => "encoders",
        ControlKind::TouchZone => "touch zones",
    };
    match kinds {
        [] => "it cannot be bound to any control".to_owned(),
        [only] => format!("it works on {} only", plural(only)),
        _ => format!(
            "it works on {}",
            kinds.iter().map(plural).collect::<Vec<_>>().join(" and ")
        ),
    }
}

fn list(inputs: &[InputCapability]) -> String {
    if inputs.is_empty() {
        return "nothing".to_owned();
    }
    inputs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for Compatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compatible => f.write_str("compatible"),
            Self::Incompatible(reason) => reason.fmt(f),
        }
    }
}

/// Whether `action` can run from a binding of a control with
/// `capabilities` (the control exists on the device) for `trigger`:
/// the action supports the control's kind, it supports the trigger's
/// input on that kind, and the control has that input natively or as a
/// derived gesture. Checked in that order; the first failure is the
/// reason.
pub fn compatibility(
    action: &ActionType,
    capabilities: &ControlCapabilities,
    trigger: Trigger,
) -> Compatibility {
    let controls = action.effective_controls();
    let kind = capabilities.kind;
    let input = trigger.input_capability();
    let Some(supported) = controls.inputs(kind) else {
        return Compatibility::Incompatible(IncompatibilityReason::ControlKindUnsupported {
            kind,
            supported: controls.kinds(),
        });
    };
    if !supported.contains(&input) {
        return Compatibility::Incompatible(IncompatibilityReason::InputUnsupportedByAction {
            kind,
            input,
            supported: supported.iter().copied().collect(),
        });
    }
    if !capabilities.supports(input) {
        return Compatibility::Incompatible(IncompatibilityReason::InputUnsupportedByControl {
            kind,
            input,
        });
    }
    Compatibility::Compatible
}

#[cfg(test)]
mod tests;
