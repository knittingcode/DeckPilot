//! Which controls of a device an action can be bound to (plugin API 1.3):
//! the manifest's `contributes.actions[].controls`.
//!
//! An action without a declaration is offered on keys only (press,
//! release, click, double click, long press), never on encoders or touch
//! zones; `device.requires` in the manifest grants nothing on its own.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Most inputs an action may declare for one control kind.
pub const MAX_INPUTS_PER_CONTROL: usize = 8;

/// A family of physical controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Control {
    /// A key (`key`).
    Key,
    /// A dial (`encoder`).
    Encoder,
    /// A zone of a touch strip (`touchZone`).
    TouchZone,
}

impl Control {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 3] = [Self::Key, Self::Encoder, Self::TouchZone];

    /// The manifest's name of the kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Encoder => "encoder",
            Self::TouchZone => "touchZone",
        }
    }

    /// The kind with the manifest name `name`.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == name)
    }

    /// The inputs a control of this kind can produce.
    pub fn inputs(self) -> &'static [Input] {
        match self {
            Self::Key => &[
                Input::Press,
                Input::Release,
                Input::Click,
                Input::DoubleClick,
                Input::LongPress,
            ],
            Self::Encoder => &[
                Input::RotateClockwise,
                Input::RotateCounterClockwise,
                Input::Press,
                Input::Release,
                Input::Click,
                Input::DoubleClick,
                Input::LongPress,
            ],
            Self::TouchZone => &[Input::Tap, Input::LongTouch, Input::Swipe],
        }
    }
}

impl fmt::Display for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An input an action can react to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Input {
    Press,
    Release,
    RotateClockwise,
    RotateCounterClockwise,
    /// A tap on a touch zone.
    Tap,
    /// A long touch on a touch zone.
    LongTouch,
    /// A swipe on a touch zone.
    Swipe,
    Click,
    DoubleClick,
    LongPress,
}

impl Input {
    /// Every input, in a stable order.
    pub const ALL: [Self; 10] = [
        Self::Press,
        Self::Release,
        Self::RotateClockwise,
        Self::RotateCounterClockwise,
        Self::Tap,
        Self::LongTouch,
        Self::Swipe,
        Self::Click,
        Self::DoubleClick,
        Self::LongPress,
    ];

    /// The manifest's name of the input.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::Release => "release",
            Self::RotateClockwise => "rotate-clockwise",
            Self::RotateCounterClockwise => "rotate-counter-clockwise",
            Self::Tap => "tap",
            Self::LongTouch => "long-touch",
            Self::Swipe => "swipe",
            Self::Click => "click",
            Self::DoubleClick => "double-click",
            Self::LongPress => "long-press",
        }
    }

    /// The input with the manifest name `name`.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.as_str() == name)
    }
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Per control kind, the inputs an action can be bound to; a kind that is
/// absent is not supported. The manifest entry declares the same in
/// `controls` ([`Controls::to_json`]).
///
/// ```
/// use deckpilot_plugin_sdk::{Controls, Input};
///
/// let controls = Controls::new()
///     .key([Input::Press, Input::Click])
///     .encoder([Input::RotateClockwise, Input::RotateCounterClockwise]);
/// assert_eq!(controls.check(), Ok(()));
/// assert_eq!(
///     controls.to_json(),
///     serde_json::json!({"key": ["press", "click"],
///                        "encoder": ["rotate-clockwise", "rotate-counter-clockwise"]})
/// );
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Controls {
    bindings: BTreeMap<Control, BTreeSet<Input>>,
}

impl Controls {
    /// No control yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or extends) `control` with `inputs`.
    #[must_use]
    pub fn on(mut self, control: Control, inputs: impl IntoIterator<Item = Input>) -> Self {
        self.bindings.entry(control).or_default().extend(inputs);
        self
    }

    /// [`Controls::on`] for keys.
    #[must_use]
    pub fn key(self, inputs: impl IntoIterator<Item = Input>) -> Self {
        self.on(Control::Key, inputs)
    }

    /// [`Controls::on`] for encoders (dials).
    #[must_use]
    pub fn encoder(self, inputs: impl IntoIterator<Item = Input>) -> Self {
        self.on(Control::Encoder, inputs)
    }

    /// [`Controls::on`] for touch zones.
    #[must_use]
    pub fn touch_zone(self, inputs: impl IntoIterator<Item = Input>) -> Self {
        self.on(Control::TouchZone, inputs)
    }

    /// The declared kinds and their inputs.
    pub fn bindings(&self) -> &BTreeMap<Control, BTreeSet<Input>> {
        &self.bindings
    }

    /// The inputs declared for `control`; `None`: not supported.
    pub fn inputs(&self, control: Control) -> Option<&BTreeSet<Input>> {
        self.bindings.get(&control)
    }

    /// The manifest's rules: at least one kind, no kind without inputs,
    /// only inputs a control of the kind can produce, at most
    /// [`MAX_INPUTS_PER_CONTROL`] per kind. The error says what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.bindings.is_empty() {
            return Err("no control kind is declared".into());
        }
        for (control, inputs) in &self.bindings {
            if inputs.is_empty() {
                return Err(format!("{control}: no input is declared"));
            }
            if inputs.len() > MAX_INPUTS_PER_CONTROL {
                return Err(format!(
                    "{control}: more than {MAX_INPUTS_PER_CONTROL} inputs are declared"
                ));
            }
            if let Some(input) = inputs.iter().find(|i| !control.inputs().contains(i)) {
                return Err(format!("{control}: a {control} has no {input}"));
            }
        }
        Ok(())
    }

    /// The manifest form: `{"key": ["press", "click"], ...}`, kinds and
    /// inputs in a stable order.
    pub fn to_json(&self) -> serde_json::Value {
        self.bindings
            .iter()
            .map(|(control, inputs)| {
                (
                    control.as_str().to_owned(),
                    inputs.iter().map(|i| i.as_str()).collect::<Vec<_>>().into(),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into()
    }

    /// Reads the manifest form; `None` when it is not an object of known
    /// kinds to arrays of known inputs.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let mut controls = Self::new();
        for (kind, inputs) in value.as_object()? {
            let control = Control::parse(kind)?;
            let inputs = inputs
                .as_array()?
                .iter()
                .map(|i| i.as_str().and_then(Input::parse))
                .collect::<Option<Vec<_>>>()?;
            controls.bindings.entry(control).or_default().extend(inputs);
        }
        Some(controls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for c in Control::ALL {
            assert_eq!(Control::parse(c.as_str()), Some(c));
        }
        for i in Input::ALL {
            assert_eq!(Input::parse(i.as_str()), Some(i));
        }
        assert_eq!(Control::parse("touch-zone"), None);
        assert_eq!(Input::parse("rotate"), None);
    }

    #[test]
    fn rules() {
        assert_eq!(
            Controls::new().check(),
            Err("no control kind is declared".into())
        );
        assert_eq!(
            Controls::new().encoder([]).check(),
            Err("encoder: no input is declared".into())
        );
        assert_eq!(
            Controls::new().key([Input::RotateClockwise]).check(),
            Err("key: a key has no rotate-clockwise".into())
        );
        assert_eq!(
            Controls::new().encoder([Input::Tap]).check(),
            Err("encoder: a encoder has no tap".into())
        );
        assert!(Controls::new().touch_zone([Input::Press]).check().is_err());
        assert_eq!(
            Controls::new()
                .key([Input::Press, Input::Click, Input::LongPress])
                .encoder(Control::Encoder.inputs().iter().copied())
                .touch_zone([Input::Tap, Input::Swipe, Input::LongTouch])
                .check(),
            Ok(())
        );
    }

    #[test]
    fn json_round_trip() {
        let controls = Controls::new()
            .touch_zone([Input::Swipe, Input::Tap])
            .key([Input::Click, Input::Press]);
        let json = controls.to_json();
        assert_eq!(
            json,
            serde_json::json!({"key": ["press", "click"], "touchZone": ["tap", "swipe"]})
        );
        assert_eq!(Controls::from_json(&json), Some(controls));
        assert_eq!(
            Controls::from_json(&serde_json::json!({"dial": ["press"]})),
            None
        );
        assert_eq!(
            Controls::from_json(&serde_json::json!({"key": ["rotate"]})),
            None
        );
    }
}
