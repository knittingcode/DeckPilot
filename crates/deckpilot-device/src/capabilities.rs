use serde::{Deserialize, Serialize};

use crate::{ControlId, ControlInput, ControlKind, InputError};

/// Pixel format expected by a device for dynamic images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFormat {
    Jpeg,
    Bmp,
}

/// Transformation the device applies when displaying an image. A driver must
/// apply the inverse before uploading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageTransform {
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    /// Clockwise rotation in degrees; one of 0, 90, 180, 270.
    pub rotation: u16,
}

/// Description of a display area that accepts dynamic images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSpec {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub transform: ImageTransform,
}

/// A rectangular grid of keys. Keys are indexed row-major starting at 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyGrid {
    pub rows: u16,
    pub columns: u16,
    /// `None` for keys without a display.
    pub image: Option<ImageSpec>,
}

impl KeyGrid {
    pub fn key_count(&self) -> u16 {
        self.rows * self.columns
    }

    /// Row and column of a key index, or `None` when out of range.
    pub fn position(&self, index: u16) -> Option<(u16, u16)> {
        (index < self.key_count()).then(|| (index / self.columns, index % self.columns))
    }
}

/// Rotary encoder description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderSpec {
    pub count: u8,
    pub pressable: bool,
}

/// Touch gestures a touch surface can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TouchGesture {
    Tap,
    LongPress,
    Swipe,
}

/// A touch-sensitive display, optionally split into zones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchSurface {
    pub zones: u8,
    pub image: Option<ImageSpec>,
    pub gestures: Vec<TouchGesture>,
}

/// A family of capabilities that core code drives generically. Drivers
/// implement them through [`crate::DeviceHandle`]; models declare them in
/// [`DeviceCapabilities`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    /// Keys that report press and release.
    Keys,
    /// Keys that show images.
    KeyDisplays,
    /// Rotary encoders (dials).
    Dials,
    /// A touch-sensitive strip, split into zones.
    TouchStrip,
    /// Adjustable display brightness.
    Brightness,
}

/// Everything a device model can do. Each control family is optional so that
/// models with only keys, only encoders, or any combination can be described.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCapabilities {
    pub keys: Option<KeyGrid>,
    pub encoders: Option<EncoderSpec>,
    pub touch: Option<TouchSurface>,
    /// Whether the display brightness can be set. The agent only sends
    /// brightness to devices that have it.
    #[serde(default)]
    pub brightness: bool,
}

impl DeviceCapabilities {
    pub fn supports(&self, capability: Capability) -> bool {
        match capability {
            Capability::Keys => self.keys.is_some_and(|grid| grid.key_count() > 0),
            Capability::KeyDisplays => self.keys.is_some_and(|grid| grid.image.is_some()),
            Capability::Dials => self.encoders.is_some_and(|spec| spec.count > 0),
            Capability::TouchStrip => self.touch.as_ref().is_some_and(|touch| touch.zones > 0),
            Capability::Brightness => self.brightness,
        }
    }

    /// The capability families of this model, in a stable order.
    pub fn families(&self) -> Vec<Capability> {
        [
            Capability::Keys,
            Capability::KeyDisplays,
            Capability::Dials,
            Capability::TouchStrip,
            Capability::Brightness,
        ]
        .into_iter()
        .filter(|capability| self.supports(*capability))
        .collect()
    }

    /// All addressable controls of the device in a stable order.
    pub fn controls(&self) -> Vec<ControlId> {
        let keys = self.keys.map_or(0, |grid| grid.key_count());
        let encoders = self.encoders.map_or(0, |spec| spec.count);
        let zones = self.touch.as_ref().map_or(0, |touch| touch.zones);

        (0..keys)
            .map(|index| ControlId::Key { index })
            .chain((0..encoders).map(|index| ControlId::Encoder { index }))
            .chain((0..zones).map(|index| ControlId::TouchZone { index }))
            .collect()
    }

    pub fn has_control(&self, control: ControlId) -> bool {
        match control {
            ControlId::Key { index } => self.keys.is_some_and(|grid| index < grid.key_count()),
            ControlId::Encoder { index } => self.encoders.is_some_and(|spec| index < spec.count),
            ControlId::TouchZone { index } => {
                self.touch.as_ref().is_some_and(|touch| index < touch.zones)
            }
        }
    }

    /// Checks that `input` can physically occur on `control` for this model.
    pub fn validate_input(
        &self,
        control: ControlId,
        input: ControlInput,
    ) -> Result<(), InputError> {
        if !self.has_control(control) {
            return Err(InputError::UnknownControl(control));
        }
        let supported = match (control.kind(), input) {
            (ControlKind::Key, ControlInput::Press | ControlInput::Release)
            | (ControlKind::Encoder, ControlInput::Rotate { .. }) => true,
            (ControlKind::Encoder, ControlInput::Press | ControlInput::Release) => {
                self.encoders.is_some_and(|spec| spec.pressable)
            }
            (ControlKind::TouchZone, ControlInput::Touch { gesture }) => self
                .touch
                .as_ref()
                .is_some_and(|touch| touch.gestures.contains(&gesture)),
            _ => false,
        };
        if supported {
            Ok(())
        } else {
            Err(InputError::UnsupportedInput { control, input })
        }
    }

    /// Whether a control can display a dynamic image.
    pub fn has_display(&self, control: ControlId) -> bool {
        self.has_control(control)
            && match control {
                ControlId::Key { .. } => self.keys.is_some_and(|grid| grid.image.is_some()),
                ControlId::Encoder { .. } => false,
                ControlId::TouchZone { .. } => self
                    .touch
                    .as_ref()
                    .is_some_and(|touch| touch.image.is_some()),
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plus_like() -> DeviceCapabilities {
        DeviceCapabilities {
            keys: Some(KeyGrid {
                rows: 2,
                columns: 4,
                image: None,
            }),
            encoders: Some(EncoderSpec {
                count: 4,
                pressable: true,
            }),
            touch: Some(TouchSurface {
                zones: 4,
                image: None,
                gestures: vec![TouchGesture::Tap, TouchGesture::Swipe],
            }),
            brightness: false,
        }
    }

    #[test]
    fn capability_families() {
        assert_eq!(
            plus_like().families(),
            vec![Capability::Keys, Capability::Dials, Capability::TouchStrip]
        );
        let none = DeviceCapabilities::default();
        assert_eq!(none.families(), Vec::<Capability>::new());
        let json = serde_json::to_value(plus_like()).unwrap();
        assert_eq!(json["brightness"], false);
        // Data without the field (older versions) reads as no brightness.
        let mut old = json;
        old.as_object_mut().unwrap().remove("brightness");
        let back: DeviceCapabilities = serde_json::from_value(old).unwrap();
        assert_eq!(back, plus_like());
    }

    #[test]
    fn grid_positions_are_row_major() {
        let grid = KeyGrid {
            rows: 3,
            columns: 5,
            image: None,
        };
        assert_eq!(grid.key_count(), 15);
        assert_eq!(grid.position(0), Some((0, 0)));
        assert_eq!(grid.position(7), Some((1, 2)));
        assert_eq!(grid.position(14), Some((2, 4)));
        assert_eq!(grid.position(15), None);
    }

    #[test]
    fn controls_cover_every_family() {
        let controls = plus_like().controls();
        assert_eq!(controls.len(), 16);
        assert_eq!(controls[8], ControlId::Encoder { index: 0 });
        assert_eq!(controls[15], ControlId::TouchZone { index: 3 });
    }

    #[test]
    fn validates_inputs_against_capabilities() {
        let caps = plus_like();
        let encoder = ControlId::Encoder { index: 1 };
        assert!(
            caps.validate_input(encoder, ControlInput::Rotate { delta: -2 })
                .is_ok()
        );
        assert!(caps.validate_input(encoder, ControlInput::Press).is_ok());
        assert!(
            caps.validate_input(
                ControlId::Key { index: 0 },
                ControlInput::Rotate { delta: 1 }
            )
            .is_err()
        );
        assert_eq!(
            caps.validate_input(ControlId::Key { index: 8 }, ControlInput::Press),
            Err(InputError::UnknownControl(ControlId::Key { index: 8 }))
        );
        let zone = ControlId::TouchZone { index: 0 };
        assert!(
            caps.validate_input(
                zone,
                ControlInput::Touch {
                    gesture: TouchGesture::Tap
                }
            )
            .is_ok()
        );
        assert!(
            caps.validate_input(
                zone,
                ControlInput::Touch {
                    gesture: TouchGesture::LongPress
                }
            )
            .is_err()
        );
    }

    #[test]
    fn non_pressable_encoders_reject_presses() {
        let caps = DeviceCapabilities {
            encoders: Some(EncoderSpec {
                count: 1,
                pressable: false,
            }),
            ..DeviceCapabilities::default()
        };
        assert!(
            caps.validate_input(ControlId::Encoder { index: 0 }, ControlInput::Press)
                .is_err()
        );
    }
}
