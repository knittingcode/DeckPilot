//! What one physical control can do, derived from a model's
//! [`DeviceCapabilities`] (never from its name). See
//! `docs/device-controls.md`.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ControlId, ControlKind, DeviceCapabilities, ImageSpec, TouchGesture};

/// An input a control can produce, as bindings and actions name it.
///
/// `Press`, `Release`, the rotations and the touch gestures are reported by
/// the hardware. `Click`, `DoubleClick` and `LongPress` are recognized by
/// DeckPilot's gesture engine from presses and releases: a control has them
/// as *derived* inputs only, never as hardware inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputCapability {
    Press,
    Release,
    RotateClockwise,
    RotateCounterClockwise,
    /// A tap on a touch zone.
    Tap,
    /// A long touch on a touch zone (the hardware's long press).
    LongTouch,
    /// A swipe on a touch zone.
    Swipe,
    Click,
    DoubleClick,
    LongPress,
}

impl InputCapability {
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

    /// The gestures DeckPilot derives from a control's presses and
    /// releases.
    pub const DERIVED: [Self; 3] = [Self::Click, Self::DoubleClick, Self::LongPress];

    /// Whether DeckPilot derives this input (a gesture) rather than the
    /// hardware reporting it.
    pub fn is_derived(self) -> bool {
        Self::DERIVED.contains(&self)
    }

    /// The input of a touch gesture.
    pub fn from_touch(gesture: TouchGesture) -> Self {
        match gesture {
            TouchGesture::Tap => Self::Tap,
            TouchGesture::LongPress => Self::LongTouch,
            TouchGesture::Swipe => Self::Swipe,
        }
    }

    /// Whether a control of `kind` can produce this input on *some* model:
    /// no rotation on a key, no tap on an encoder, no press on a touch
    /// zone. Used to check declarations, independent of a device.
    pub fn applies_to(self, kind: ControlKind) -> bool {
        match kind {
            ControlKind::Key => matches!(
                self,
                Self::Press | Self::Release | Self::Click | Self::DoubleClick | Self::LongPress
            ),
            ControlKind::Encoder => matches!(
                self,
                Self::RotateClockwise
                    | Self::RotateCounterClockwise
                    | Self::Press
                    | Self::Release
                    | Self::Click
                    | Self::DoubleClick
                    | Self::LongPress
            ),
            ControlKind::TouchZone => matches!(self, Self::Tap | Self::LongTouch | Self::Swipe),
        }
    }

    /// The stable (serialized) name, e.g. `rotate-clockwise`.
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
}

impl fmt::Display for InputCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Output on the control itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayCapability {
    /// The image the control shows.
    pub image: ImageSpec,
    /// Whether a title is drawn on the control (into its image).
    pub title: bool,
}

/// What one physical control can do. Derived from [`DeviceCapabilities`]
/// by [`DeviceCapabilities::control_capabilities`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlCapabilities {
    pub kind: ControlKind,
    /// Hardware events the device reports for this control (native only).
    pub inputs: BTreeSet<InputCapability>,
    /// Gestures DeckPilot derives from the native inputs.
    pub derived: BTreeSet<InputCapability>,
    /// Output on the control itself; `None` without a display.
    pub display: Option<DisplayCapability>,
}

impl ControlCapabilities {
    /// Whether the control produces `input`, natively or as a derived
    /// gesture.
    pub fn supports(&self, input: InputCapability) -> bool {
        self.inputs.contains(&input) || self.derived.contains(&input)
    }

    /// Native and derived inputs together.
    pub fn all_inputs(&self) -> BTreeSet<InputCapability> {
        self.inputs.union(&self.derived).copied().collect()
    }
}

impl DeviceCapabilities {
    /// What `control` can do on this model; `None` when the model has no
    /// such control.
    ///
    /// - A key: press and release; click, double click and long press
    ///   derived; a display (image and title) when the grid has images.
    /// - An encoder: both rotations, press and release only when it is
    ///   pressable, and then the derived gestures; no display.
    /// - A touch zone: the gestures of its surface (tap, long touch,
    ///   swipe); a display when the surface has images; nothing derived.
    pub fn control_capabilities(&self, control: ControlId) -> Option<ControlCapabilities> {
        if !self.has_control(control) {
            return None;
        }
        let press_family = |inputs: &mut BTreeSet<InputCapability>,
                            derived: &mut BTreeSet<InputCapability>| {
            inputs.extend([InputCapability::Press, InputCapability::Release]);
            derived.extend(InputCapability::DERIVED);
        };
        let display =
            |image: Option<ImageSpec>| image.map(|image| DisplayCapability { image, title: true });
        let mut inputs = BTreeSet::new();
        let mut derived = BTreeSet::new();
        let shows = match control.kind() {
            ControlKind::Key => {
                press_family(&mut inputs, &mut derived);
                display(self.keys.and_then(|grid| grid.image))
            }
            ControlKind::Encoder => {
                inputs.extend([
                    InputCapability::RotateClockwise,
                    InputCapability::RotateCounterClockwise,
                ]);
                if self.encoders.is_some_and(|spec| spec.pressable) {
                    press_family(&mut inputs, &mut derived);
                }
                None
            }
            ControlKind::TouchZone => {
                let touch = self.touch.as_ref()?;
                inputs.extend(
                    touch
                        .gestures
                        .iter()
                        .copied()
                        .map(InputCapability::from_touch),
                );
                display(touch.image)
            }
        };
        Some(ControlCapabilities {
            kind: control.kind(),
            inputs,
            derived,
            display: shows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncoderSpec, TouchSurface, models};
    use InputCapability as I;

    fn set<const N: usize>(inputs: [InputCapability; N]) -> BTreeSet<InputCapability> {
        inputs.into_iter().collect()
    }

    #[test]
    fn mk2_keys_press_release_and_derived_gestures_with_a_display() {
        let caps = models::stream_deck_mk2().capabilities;
        for index in 0..15 {
            let key = caps.control_capabilities(ControlId::Key { index }).unwrap();
            assert_eq!(key.kind, ControlKind::Key);
            assert_eq!(key.inputs, set([I::Press, I::Release]));
            assert_eq!(key.derived, set([I::Click, I::DoubleClick, I::LongPress]));
            let display = key.display.unwrap();
            assert_eq!((display.image.width, display.image.height), (72, 72));
            assert!(display.title);
            assert!(!key.supports(I::RotateClockwise));
            assert!(!key.supports(I::Tap));
        }
        assert_eq!(
            caps.control_capabilities(ControlId::Key { index: 15 }),
            None
        );
        assert_eq!(
            caps.control_capabilities(ControlId::Encoder { index: 0 }),
            None
        );
        assert_eq!(
            caps.control_capabilities(ControlId::TouchZone { index: 0 }),
            None
        );
    }

    #[test]
    fn xl_has_thirty_two_display_keys_and_nothing_else() {
        let caps = models::stream_deck_xl().capabilities;
        let key = caps
            .control_capabilities(ControlId::Key { index: 31 })
            .unwrap();
        assert_eq!(key.inputs, set([I::Press, I::Release]));
        assert_eq!(key.display.unwrap().image.width, 96);
        assert!(
            caps.control_capabilities(ControlId::Key { index: 32 })
                .is_none()
        );
        assert!(
            caps.control_capabilities(ControlId::Encoder { index: 0 })
                .is_none()
        );
    }

    #[test]
    fn plus_keys_pressable_encoders_and_touch_zones() {
        let caps = models::stream_deck_plus().capabilities;
        let key = caps
            .control_capabilities(ControlId::Key { index: 7 })
            .unwrap();
        assert_eq!(key.display.unwrap().image.width, 120);

        let encoder = caps
            .control_capabilities(ControlId::Encoder { index: 3 })
            .unwrap();
        assert_eq!(encoder.kind, ControlKind::Encoder);
        assert_eq!(
            encoder.inputs,
            set([
                I::Press,
                I::Release,
                I::RotateClockwise,
                I::RotateCounterClockwise
            ])
        );
        assert_eq!(
            encoder.derived,
            set([I::Click, I::DoubleClick, I::LongPress])
        );
        assert_eq!(encoder.display, None);

        let zone = caps
            .control_capabilities(ControlId::TouchZone { index: 0 })
            .unwrap();
        assert_eq!(zone.kind, ControlKind::TouchZone);
        assert_eq!(zone.inputs, set([I::Tap, I::LongTouch, I::Swipe]));
        assert!(zone.derived.is_empty());
        let display = zone.display.unwrap();
        assert_eq!((display.image.width, display.image.height), (200, 100));
    }

    #[test]
    fn a_non_pressable_encoder_only_rotates() {
        let caps = DeviceCapabilities {
            encoders: Some(EncoderSpec {
                count: 1,
                pressable: false,
            }),
            ..DeviceCapabilities::default()
        };
        let encoder = caps
            .control_capabilities(ControlId::Encoder { index: 0 })
            .unwrap();
        assert_eq!(
            encoder.inputs,
            set([I::RotateClockwise, I::RotateCounterClockwise])
        );
        assert!(encoder.derived.is_empty());
        for input in [I::Press, I::Release, I::Click, I::DoubleClick, I::LongPress] {
            assert!(!encoder.supports(input), "{input}");
        }
    }

    #[test]
    fn touch_inputs_follow_the_surface_gestures() {
        let caps = DeviceCapabilities {
            touch: Some(TouchSurface {
                zones: 2,
                image: None,
                gestures: vec![TouchGesture::Tap],
            }),
            ..DeviceCapabilities::default()
        };
        let zone = caps
            .control_capabilities(ControlId::TouchZone { index: 1 })
            .unwrap();
        assert_eq!(zone.inputs, set([I::Tap]));
        assert!(!zone.supports(I::LongTouch));
        assert!(!zone.supports(I::Swipe));
        assert_eq!(zone.display, None);
    }

    #[test]
    fn derived_gestures_are_never_hardware_inputs() {
        for model in [
            models::stream_deck_mk2(),
            models::stream_deck_xl(),
            models::stream_deck_plus(),
        ] {
            for control in model.capabilities.controls() {
                let caps = model.capabilities.control_capabilities(control).unwrap();
                assert!(caps.inputs.iter().all(|input| !input.is_derived()));
                assert!(caps.derived.iter().all(|input| input.is_derived()));
                assert!(caps.all_inputs().iter().all(|i| i.applies_to(caps.kind)));
            }
        }
    }

    #[test]
    fn inputs_serialize_in_kebab_case() {
        let json = serde_json::to_string(&[
            I::Press,
            I::RotateCounterClockwise,
            I::LongTouch,
            I::DoubleClick,
        ])
        .unwrap();
        assert_eq!(
            json,
            r#"["press","rotate-counter-clockwise","long-touch","double-click"]"#
        );
        for input in I::ALL {
            assert_eq!(
                serde_json::to_string(&input).unwrap(),
                format!("\"{}\"", input.as_str())
            );
        }
        let caps = models::stream_deck_plus()
            .capabilities
            .control_capabilities(ControlId::Encoder { index: 0 })
            .unwrap();
        let back: ControlCapabilities =
            serde_json::from_value(serde_json::to_value(&caps).unwrap()).unwrap();
        assert_eq!(back, caps);
    }

    #[test]
    fn kinds_limit_the_inputs_they_can_ever_have() {
        assert!(!I::RotateClockwise.applies_to(ControlKind::Key));
        assert!(!I::Tap.applies_to(ControlKind::Encoder));
        assert!(!I::Press.applies_to(ControlKind::TouchZone));
        assert!(I::LongPress.applies_to(ControlKind::Encoder));
    }
}
