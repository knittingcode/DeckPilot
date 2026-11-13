//! The Elgato adapter: USB HID drivers for Elgato Stream Decks.
//!
//! Currently one model: Elgato Stream Deck MK.2 (USB `0fd9:0080`), built on
//! the `elgato-streamdeck` protocol implementation and `hidapi`. Other Stream
//! Deck models are recognised by the library but deliberately not exposed
//! until they are described and tested.
//!
//! This crate is the only place that knows Elgato protocol types. It
//! translates the library's reports into generic [`ControlId`] and
//! [`ControlInput`] values and accepts generic [`Frame`]s; everything else in
//! DeckPilot works with those.
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

/// The models this adapter drives: the library's device kind and the
/// DeckPilot descriptor, whose capabilities (key grid, image size and
/// orientation) select the model-specific behaviour.
const SUPPORTED: [(Kind, fn() -> DeviceModel); 1] = [(Kind::Mk2, models::stream_deck_mk2)];

/// Maps a library device kind to the DeckPilot model descriptor, for the
/// kinds DeckPilot supports.
fn model_for(kind: Kind) -> Option<DeviceModel> {
    SUPPORTED
        .iter()
        .find(|(supported, _)| *supported == kind)
        .map(|(_, model)| model())
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

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn models(&self) -> Vec<DeviceModel> {
        SUPPORTED.iter().map(|(_, model)| model()).collect()
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
        let (kind, model) = SUPPORTED
            .iter()
            .map(|(kind, model)| (*kind, model()))
            .find(|(_, model)| model.id == device.model)
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

/// The protocol operations the adapter uses. Implemented by the library's
/// [`StreamDeck`]; tests use a recording fake.
///
/// Every image operation is queued and sent on [`Deck::flush`], in order.
trait Deck: Send {
    fn read_input(&self, timeout: Duration) -> Result<StreamDeckInput, StreamDeckError>;
    fn queue_image(&self, key: u8, image: DynamicImage) -> Result<(), StreamDeckError>;
    fn queue_blank(&self, key: u8) -> Result<(), StreamDeckError>;
    fn set_brightness(&self, percent: u8) -> Result<(), StreamDeckError>;
    fn flush(&self) -> Result<(), StreamDeckError>;
}

impl Deck for StreamDeck {
    fn read_input(&self, timeout: Duration) -> Result<StreamDeckInput, StreamDeckError> {
        StreamDeck::read_input(self, Some(timeout))
    }

    /// The library resizes, mirrors and JPEG-encodes for the device, and
    /// queues the result until `flush`.
    fn queue_image(&self, key: u8, image: DynamicImage) -> Result<(), StreamDeckError> {
        self.set_button_image(key, image)
    }

    /// `clear_button_image` of `elgato-streamdeck` 0.14 writes at once,
    /// while images are queued until `flush`: an image set and then cleared
    /// before a flush would stay visible. The blank image is queued like any
    /// other, so a clear takes effect on flush, in order.
    fn queue_blank(&self, key: u8) -> Result<(), StreamDeckError> {
        self.write_image(key, &self.kind().blank_image())
    }

    fn set_brightness(&self, percent: u8) -> Result<(), StreamDeckError> {
        StreamDeck::set_brightness(self, percent)
    }

    fn flush(&self) -> Result<(), StreamDeckError> {
        StreamDeck::flush(self)
    }
}

struct ElgatoHandle<D: Deck = StreamDeck> {
    deck: D,
    id: DeviceId,
    model: DeviceModel,
    keys: KeyStateTracker,
}

impl<D: Deck> fmt::Debug for ElgatoHandle<D> {
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

impl<D: Deck> ElgatoHandle<D> {
    fn key_index(&self, control: ControlId) -> Result<u8, DeviceError> {
        match control {
            ControlId::Key { index } if self.model.capabilities.has_display(control) => {
                u8::try_from(index).map_err(|_| DeviceError::InvalidControl(control))
            }
            _ => Err(DeviceError::InvalidControl(control)),
        }
    }
}

impl<D: Deck> DeviceHandle for ElgatoHandle<D> {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn read_input(
        &mut self,
        timeout: Duration,
    ) -> Result<Vec<(ControlId, ControlInput)>, DeviceError> {
        match self.deck.read_input(timeout).map_err(map_error)? {
            StreamDeckInput::ButtonStateChange(states) => Ok(self.keys.update(&states)),
            // The MK.2 has keys only; other report types do not occur.
            _ => Ok(Vec::new()),
        }
    }

    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError> {
        let key = self.key_index(control)?;
        let image = RgbImage::from_raw(frame.width(), frame.height(), frame.rgb().to_vec())
            .ok_or_else(|| DeviceError::InvalidFrame("frame size mismatch".into()))?;
        self.deck
            .queue_image(key, DynamicImage::ImageRgb8(image))
            .map_err(map_error)
    }

    fn clear_image(&mut self, control: ControlId) -> Result<(), DeviceError> {
        let key = self.key_index(control)?;
        self.deck.queue_blank(key).map_err(map_error)
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
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::{Arc, Mutex};

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

    /// XL and + have no driver, but their layout-design descriptors must
    /// still match the protocol implementation.
    #[test]
    fn descriptor_only_models_match_protocol_implementation() {
        for (kind, model) in [
            (Kind::Xl, models::stream_deck_xl()),
            (Kind::Plus, models::stream_deck_plus()),
        ] {
            let usb = model.usb.unwrap();
            assert_eq!(
                (usb.vendor_id, usb.product_id),
                (kind.vendor_id(), kind.product_id())
            );
            let keys = model.capabilities.keys.unwrap();
            assert_eq!(keys.rows, u16::from(kind.row_count()));
            assert_eq!(keys.columns, u16::from(kind.column_count()));
            let image = keys.image.unwrap();
            let format = kind.key_image_format();
            assert_eq!((image.width as usize, image.height as usize), format.size);
            let mirrored = matches!(format.mirror, ImageMirroring::Both);
            assert_eq!(image.transform.flip_horizontal, mirrored);
            assert_eq!(
                model.capabilities.encoders.map_or(0, |e| e.count),
                kind.encoder_count()
            );
        }
        let plus = models::stream_deck_plus();
        let touch = plus.capabilities.touch.unwrap();
        let strip = Kind::Plus.lcd_strip_size().unwrap();
        let zone = touch.image.unwrap();
        assert_eq!(zone.width as usize * usize::from(touch.zones), strip.0);
        assert_eq!(zone.height as usize, strip.1);
    }

    /// What the fake deck was asked to do, in order.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Op {
        Image(u8),
        Blank(u8),
        Brightness(u8),
        Flush,
    }

    /// Records operations and, like the library, sends queued images only on
    /// flush. `shown` is what each key displays afterwards.
    #[derive(Default)]
    struct FakeDeck {
        ops: Mutex<Vec<Op>>,
        queue: Mutex<Vec<(u8, bool)>>,
        shown: Mutex<BTreeMap<u8, bool>>,
        reports: Mutex<VecDeque<Vec<bool>>>,
    }

    impl FakeDeck {
        fn ops(&self) -> Vec<Op> {
            self.ops.lock().unwrap().clone()
        }

        /// `Some(true)`: an image, `Some(false)`: blank, `None`: never written.
        fn shown(&self, key: u8) -> Option<bool> {
            self.shown.lock().unwrap().get(&key).copied()
        }
    }

    impl Deck for Arc<FakeDeck> {
        fn read_input(&self, _: Duration) -> Result<StreamDeckInput, StreamDeckError> {
            Ok(self
                .reports
                .lock()
                .unwrap()
                .pop_front()
                .map_or(StreamDeckInput::NoData, StreamDeckInput::ButtonStateChange))
        }

        fn queue_image(&self, key: u8, _: DynamicImage) -> Result<(), StreamDeckError> {
            self.ops.lock().unwrap().push(Op::Image(key));
            self.queue.lock().unwrap().push((key, true));
            Ok(())
        }

        fn queue_blank(&self, key: u8) -> Result<(), StreamDeckError> {
            self.ops.lock().unwrap().push(Op::Blank(key));
            self.queue.lock().unwrap().push((key, false));
            Ok(())
        }

        fn set_brightness(&self, percent: u8) -> Result<(), StreamDeckError> {
            self.ops.lock().unwrap().push(Op::Brightness(percent));
            Ok(())
        }

        fn flush(&self) -> Result<(), StreamDeckError> {
            self.ops.lock().unwrap().push(Op::Flush);
            let mut shown = self.shown.lock().unwrap();
            for (key, image) in self.queue.lock().unwrap().drain(..) {
                shown.insert(key, image);
            }
            Ok(())
        }
    }

    fn fake_handle() -> (Arc<FakeDeck>, ElgatoHandle<Arc<FakeDeck>>) {
        let deck = Arc::new(FakeDeck::default());
        let model = models::stream_deck_mk2();
        let handle = ElgatoHandle {
            deck: deck.clone(),
            id: DeviceId {
                model: model.id.clone(),
                serial: "FAKE".into(),
            },
            model,
            keys: KeyStateTracker::default(),
        };
        (deck, handle)
    }

    /// A clear is queued like an image: setting and then clearing a key
    /// before a flush leaves it blank, and nothing is written before flush.
    #[test]
    fn clearing_takes_effect_on_flush_in_order() {
        let (deck, mut handle) = fake_handle();
        let key = ControlId::Key { index: 3 };
        handle
            .set_image(key, &Frame::filled(72, 72, [255, 0, 0]))
            .unwrap();
        handle.clear_image(key).unwrap();
        assert_eq!(deck.shown(3), None, "nothing is written before flush");
        handle.flush().unwrap();
        assert_eq!(deck.ops(), vec![Op::Image(3), Op::Blank(3), Op::Flush]);
        assert_eq!(deck.shown(3), Some(false), "the clear wins");

        // The other way round, the image wins.
        handle.clear_image(key).unwrap();
        handle
            .set_image(key, &Frame::filled(72, 72, [0, 0, 255]))
            .unwrap();
        handle.flush().unwrap();
        assert_eq!(deck.shown(3), Some(true));

        handle.set_brightness(150).unwrap();
        assert_eq!(deck.ops().last(), Some(&Op::Brightness(100)));
        assert!(handle.clear_image(ControlId::Key { index: 15 }).is_err());
        assert!(handle.clear_image(ControlId::Encoder { index: 0 }).is_err());
    }

    /// Native key-state reports become generic press and release events.
    #[test]
    fn key_reports_become_generic_inputs() {
        let (deck, mut handle) = fake_handle();
        let mut pressed = vec![false; 15];
        pressed[1] = true;
        deck.reports.lock().unwrap().extend([
            vec![false; 15],
            pressed.clone(),
            pressed,
            vec![false; 15],
        ]);
        let timeout = Duration::from_millis(1);
        assert_eq!(handle.read_input(timeout).unwrap(), vec![]);
        assert_eq!(
            handle.read_input(timeout).unwrap(),
            vec![(ControlId::Key { index: 1 }, ControlInput::Press)]
        );
        assert_eq!(handle.read_input(timeout).unwrap(), vec![], "no repeat");
        assert_eq!(
            handle.read_input(timeout).unwrap(),
            vec![(ControlId::Key { index: 1 }, ControlInput::Release)]
        );
        assert_eq!(handle.read_input(timeout).unwrap(), vec![]);
    }

    #[test]
    fn the_adapter_contributes_its_models() {
        let backend = ElgatoBackend::new();
        let ids: Vec<_> = backend.models().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![models::stream_deck_mk2().id]);
        assert_eq!(backend.version(), env!("CARGO_PKG_VERSION"));
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
