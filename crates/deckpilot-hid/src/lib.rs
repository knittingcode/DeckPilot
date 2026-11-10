//! USB HID drivers.
//!
//! Currently one driver: Elgato Stream Deck MK.2 (USB `0fd9:0080`), built on
//! the `elgato-streamdeck` protocol implementation and `hidapi`. Other Stream
//! Deck models are recognised by the library but deliberately not exposed
//! until they are described and tested.
//!
//! Platform notes: on Linux, non-root access requires a udev rule granting
//! the user access to the device's `hidraw` node (see the documentation).

use std::fmt;
use std::time::Duration;

use deckpilot_device::{
    ControlId, ControlInput, DeviceBackend, DeviceError, DeviceHandle, DeviceId, DeviceModel,
    DiscoveredDevice, Frame, KeyStateTracker, models,
};
use elgato_streamdeck::info::Kind;
use elgato_streamdeck::{StreamDeck, StreamDeckError, StreamDeckInput};
use hidapi::HidApi;
use image::{DynamicImage, RgbImage};

/// Maps a library device kind to the DeckPilot model descriptor, for the
/// kinds DeckPilot supports.
fn model_for(kind: Kind) -> Option<DeviceModel> {
    match kind {
        Kind::Mk2 => Some(models::stream_deck_mk2()),
        _ => None,
    }
}

/// Discovers and opens supported Elgato devices.
#[derive(Default)]
pub struct ElgatoBackend {
    api: Option<HidApi>,
}

impl fmt::Debug for ElgatoBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ElgatoBackend")
            .field("initialised", &self.api.is_some())
            .finish()
    }
}

impl ElgatoBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn api(&mut self) -> Result<&mut HidApi, DeviceError> {
        if self.api.is_none() {
            let api =
                elgato_streamdeck::new_hidapi().map_err(|e| DeviceError::Io(e.to_string()))?;
            self.api = Some(api);
        }
        self.api
            .as_mut()
            .ok_or_else(|| DeviceError::Io("HID API unavailable".into()))
    }
}

impl DeviceBackend for ElgatoBackend {
    fn name(&self) -> &'static str {
        "elgato-hid"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
        let api = self.api()?;
        elgato_streamdeck::refresh_device_list(api).map_err(|e| DeviceError::Io(e.to_string()))?;
        Ok(elgato_streamdeck::list_devices(api)
            .into_iter()
            .filter_map(|(kind, serial)| {
                let model = model_for(kind)?;
                Some(DiscoveredDevice {
                    id: DeviceId {
                        model: model.id.clone(),
                        serial,
                    },
                    model: model.id,
                })
            })
            .collect())
    }

    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
        let (kind, model) = [Kind::Mk2]
            .into_iter()
            .find_map(|kind| {
                model_for(kind)
                    .filter(|m| m.id == device.model)
                    .map(|m| (kind, m))
            })
            .ok_or_else(|| DeviceError::Unsupported(format!("model {}", device.model)))?;
        let api = self.api()?;
        let deck = StreamDeck::connect(api, kind, &device.serial).map_err(map_error)?;
        Ok(Box::new(ElgatoHandle {
            deck,
            id: device.clone(),
            model,
            keys: KeyStateTracker::default(),
        }))
    }
}

struct ElgatoHandle {
    deck: StreamDeck,
    id: DeviceId,
    model: DeviceModel,
    keys: KeyStateTracker,
}

impl fmt::Debug for ElgatoHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ElgatoHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// HID errors on an open device almost always mean it was unplugged; the
/// agent then closes the handle and rediscovers the device.
fn map_error(error: StreamDeckError) -> DeviceError {
    match error {
        StreamDeckError::HidError(_) => DeviceError::Disconnected,
        StreamDeckError::InvalidKeyIndex => DeviceError::Io("invalid key index".into()),
        other => DeviceError::Io(other.to_string()),
    }
}

impl ElgatoHandle {
    fn key_index(&self, control: ControlId) -> Result<u8, DeviceError> {
        match control {
            ControlId::Key { index } if self.model.capabilities.has_display(control) => {
                u8::try_from(index).map_err(|_| DeviceError::InvalidControl(control))
            }
            _ => Err(DeviceError::InvalidControl(control)),
        }
    }
}

impl DeviceHandle for ElgatoHandle {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn read_input(
        &mut self,
        timeout: Duration,
    ) -> Result<Vec<(ControlId, ControlInput)>, DeviceError> {
        match self.deck.read_input(Some(timeout)).map_err(map_error)? {
            StreamDeckInput::ButtonStateChange(states) => Ok(self.keys.update(&states)),
            // The MK.2 has keys only; other report types do not occur.
            _ => Ok(Vec::new()),
        }
    }

    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError> {
        let key = self.key_index(control)?;
        let image = RgbImage::from_raw(frame.width(), frame.height(), frame.rgb().to_vec())
            .ok_or_else(|| DeviceError::InvalidFrame("frame size mismatch".into()))?;
        // The library resizes, mirrors and JPEG-encodes for the device.
        self.deck
            .set_button_image(key, DynamicImage::ImageRgb8(image))
            .map_err(map_error)
    }

    fn clear_image(&mut self, control: ControlId) -> Result<(), DeviceError> {
        let key = self.key_index(control)?;
        self.deck.clear_button_image(key).map_err(map_error)
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), DeviceError> {
        self.deck
            .set_brightness(percent.min(100))
            .map_err(map_error)
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        self.deck.flush().map_err(map_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_device::{ImageFormat, KeyGrid};
    use elgato_streamdeck::info::{ImageMirroring, ImageMode, ImageRotation};

    /// The descriptor must agree with the protocol implementation used to
    /// drive the device; this guards against silent mismatches.
    #[test]
    fn mk2_descriptor_matches_protocol_implementation() {
        let model = model_for(Kind::Mk2).unwrap();
        let usb = model.usb.unwrap();
        assert_eq!(usb.vendor_id, Kind::Mk2.vendor_id());
        assert_eq!(usb.product_id, Kind::Mk2.product_id());

        let KeyGrid {
            rows,
            columns,
            image,
        } = model.capabilities.keys.unwrap();
        assert_eq!(rows, u16::from(Kind::Mk2.row_count()));
        assert_eq!(columns, u16::from(Kind::Mk2.column_count()));
        assert_eq!(rows * columns, u16::from(Kind::Mk2.key_count()));
        assert!(model.capabilities.encoders.is_none());
        assert!(model.capabilities.touch.is_none());

        let image = image.unwrap();
        let format = Kind::Mk2.key_image_format();
        assert_eq!((image.width as usize, image.height as usize), format.size);
        assert_eq!(image.format, ImageFormat::Jpeg);
        assert!(matches!(format.mode, ImageMode::JPEG));
        assert!(matches!(format.rotation, ImageRotation::Rot0));
        assert_eq!(image.transform.rotation, 0);
        assert!(matches!(format.mirror, ImageMirroring::Both));
        assert!(image.transform.flip_horizontal && image.transform.flip_vertical);
    }

    #[test]
    fn only_supported_kinds_are_exposed() {
        assert!(model_for(Kind::Mk2).is_some());
        for kind in [
            Kind::Original,
            Kind::Mini,
            Kind::Xl,
            Kind::Plus,
            Kind::Mk2Scissor,
        ] {
            assert!(
                model_for(kind).is_none(),
                "{kind:?} must not be exposed yet"
            );
        }
    }
}
