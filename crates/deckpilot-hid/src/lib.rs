//! USB HID drivers.
//!
//! Currently one driver: Elgato Stream Deck MK.2 (USB `0fd9:0080`). The wire
//! protocol is implemented in [`mk2`] on top of `hidapi`, as pure functions
//! that are tested byte by byte; other Stream Deck models are deliberately
//! not exposed until they are described and tested.
//!
//! Platform notes: on Linux, non-root access requires a udev rule granting
//! the user access to the device's `hidraw` node (see the documentation).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use deckpilot_device::{
    ControlId, ControlInput, DeviceBackend, DeviceError, DeviceHandle, DeviceId, DeviceModel,
    DiscoveredDevice, Frame, ImageSpec, KeyStateTracker, models,
};
use hidapi::{HidApi, HidDevice, HidError};

pub mod mk2;

/// Models that have a driver.
fn supported_models() -> Vec<DeviceModel> {
    vec![models::stream_deck_mk2()]
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
            let api = HidApi::new().map_err(|e| DeviceError::Io(e.to_string()))?;
            self.api = Some(api);
        }
        self.api
            .as_mut()
            .ok_or_else(|| DeviceError::Io("HID API unavailable".into()))
    }
}

/// The supported device behind a USB identity and serial number, if any.
/// Devices without a serial number cannot be told apart and are skipped.
fn identify(
    models: &[DeviceModel],
    vendor: u16,
    product: u16,
    serial: Option<&str>,
) -> Option<DeviceId> {
    let model = models.iter().find(|m| {
        m.usb
            .is_some_and(|usb| (usb.vendor_id, usb.product_id) == (vendor, product))
    })?;
    let serial = serial.filter(|s| !s.is_empty())?;
    Some(DeviceId {
        model: model.id.clone(),
        serial: serial.to_owned(),
    })
}

impl DeviceBackend for ElgatoBackend {
    fn name(&self) -> &'static str {
        "elgato-hid"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
        let api = self.api()?;
        api.refresh_devices()
            .map_err(|e| DeviceError::Io(e.to_string()))?;
        let models = supported_models();
        // A device can be listed once per HID interface or usage; the set
        // reports each physical device once.
        let found: BTreeSet<DeviceId> = api
            .device_list()
            .filter_map(|info| {
                identify(
                    &models,
                    info.vendor_id(),
                    info.product_id(),
                    info.serial_number(),
                )
            })
            .collect();
        Ok(found
            .into_iter()
            .map(|id| DiscoveredDevice {
                model: id.model.clone(),
                id,
            })
            .collect())
    }

    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
        let model = supported_models()
            .into_iter()
            .find(|m| m.id == device.model)
            .ok_or_else(|| DeviceError::Unsupported(format!("model {}", device.model)))?;
        let usb = model
            .usb
            .ok_or_else(|| DeviceError::Unsupported(format!("model {}", device.model)))?;
        let api = self.api()?;
        let hid = api
            .open_serial(usb.vendor_id, usb.product_id, &device.serial)
            .map_err(map_error)?;
        Ok(Box::new(Mk2Handle::new(hid, device.clone(), model)?))
    }
}

/// HID errors on an open device almost always mean it was unplugged; the
/// agent then closes the handle and rediscovers the device.
fn map_error(_error: HidError) -> DeviceError {
    DeviceError::Disconnected
}

/// The raw report I/O a driver needs; implemented by [`HidDevice`] and by
/// recording fakes in tests.
trait Transport: Send {
    /// Sends an output report; the first byte is the report ID.
    fn write(&mut self, report: &[u8]) -> Result<(), DeviceError>;
    /// Sends a feature report; the first byte is the report ID.
    fn send_feature_report(&mut self, report: &[u8]) -> Result<(), DeviceError>;
    /// Reads one input report into `buffer` and returns its length, or 0
    /// when nothing arrived within `timeout`.
    fn read(&mut self, buffer: &mut [u8], timeout: Duration) -> Result<usize, DeviceError>;
}

impl Transport for HidDevice {
    fn write(&mut self, report: &[u8]) -> Result<(), DeviceError> {
        HidDevice::write(self, report).map(drop).map_err(map_error)
    }

    fn send_feature_report(&mut self, report: &[u8]) -> Result<(), DeviceError> {
        HidDevice::send_feature_report(self, report).map_err(map_error)
    }

    fn read(&mut self, buffer: &mut [u8], timeout: Duration) -> Result<usize, DeviceError> {
        let millis = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        self.read_timeout(buffer, millis).map_err(map_error)
    }
}

/// An open Stream Deck MK.2.
///
/// Images are encoded when queued and uploaded on [`DeviceHandle::flush`],
/// at most one per key: when a key is set or cleared several times before a
/// flush, only the last request is sent.
struct Mk2Handle<T> {
    transport: T,
    id: DeviceId,
    model: DeviceModel,
    spec: ImageSpec,
    keys: KeyStateTracker,
    /// Encoded images waiting for the next flush, by key index.
    pending: BTreeMap<u8, Vec<u8>>,
    /// Encoded black image used to clear keys.
    blank: Vec<u8>,
}

impl<T> fmt::Debug for Mk2Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mk2Handle")
            .field("id", &self.id)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl<T: Transport> Mk2Handle<T> {
    fn new(transport: T, id: DeviceId, model: DeviceModel) -> Result<Self, DeviceError> {
        let spec = model
            .capabilities
            .keys
            .and_then(|keys| keys.image)
            .ok_or_else(|| DeviceError::Unsupported("keys without images".into()))?;
        let black_frame = Frame::filled(spec.width, spec.height, [0; 3]);
        let blank = mk2::encode_key_image(&black_frame, &spec)?;
        Ok(Self {
            transport,
            id,
            model,
            spec,
            keys: KeyStateTracker::default(),
            pending: BTreeMap::new(),
            blank,
        })
    }

    fn key_index(&self, control: ControlId) -> Result<u8, DeviceError> {
        match control {
            ControlId::Key { index }
                if self.model.capabilities.has_display(control)
                    && usize::from(index) < mk2::KEY_COUNT =>
            {
                u8::try_from(index).map_err(|_| DeviceError::InvalidControl(control))
            }
            _ => Err(DeviceError::InvalidControl(control)),
        }
    }
}

impl<T: Transport> DeviceHandle for Mk2Handle<T> {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn read_input(
        &mut self,
        timeout: Duration,
    ) -> Result<Vec<(ControlId, ControlInput)>, DeviceError> {
        let mut buffer = [0u8; 64];
        let length = self.transport.read(&mut buffer, timeout)?;
        Ok(buffer
            .get(..length)
            .and_then(mk2::parse_input)
            .map(|states| self.keys.update(&states))
            .unwrap_or_default())
    }

    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError> {
        let key = self.key_index(control)?;
        let image = mk2::encode_key_image(frame, &self.spec)?;
        self.pending.insert(key, image);
        Ok(())
    }

    fn clear_image(&mut self, control: ControlId) -> Result<(), DeviceError> {
        let key = self.key_index(control)?;
        self.pending.insert(key, self.blank.clone());
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), DeviceError> {
        self.transport
            .send_feature_report(&mk2::brightness_report(percent))
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        for (key, image) in std::mem::take(&mut self.pending) {
            for report in mk2::image_reports(key, &image) {
                self.transport.write(&report)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use deckpilot_device::{ImageFormat, KeyGrid};
    use elgato_streamdeck::info::{ImageMirroring, ImageMode, ImageRotation, Kind};

    /// Records every report sent and replays queued input reports.
    #[derive(Debug, Clone, Default)]
    struct Recorder {
        inner: Arc<Mutex<RecorderState>>,
    }

    #[derive(Debug, Default)]
    struct RecorderState {
        written: Vec<Vec<u8>>,
        features: Vec<Vec<u8>>,
        input: VecDeque<Vec<u8>>,
        unplugged: bool,
    }

    impl Recorder {
        fn state(&self) -> std::sync::MutexGuard<'_, RecorderState> {
            self.inner.lock().unwrap()
        }

        fn take_written(&self) -> Vec<Vec<u8>> {
            std::mem::take(&mut self.state().written)
        }
    }

    impl Transport for Recorder {
        fn write(&mut self, report: &[u8]) -> Result<(), DeviceError> {
            let mut state = self.state();
            if state.unplugged {
                return Err(DeviceError::Disconnected);
            }
            state.written.push(report.to_vec());
            Ok(())
        }

        fn send_feature_report(&mut self, report: &[u8]) -> Result<(), DeviceError> {
            let mut state = self.state();
            if state.unplugged {
                return Err(DeviceError::Disconnected);
            }
            state.features.push(report.to_vec());
            Ok(())
        }

        fn read(&mut self, buffer: &mut [u8], _: Duration) -> Result<usize, DeviceError> {
            let mut state = self.state();
            if state.unplugged {
                return Err(DeviceError::Disconnected);
            }
            let Some(report) = state.input.pop_front() else {
                return Ok(0);
            };
            let length = report.len().min(buffer.len());
            buffer[..length].copy_from_slice(&report[..length]);
            Ok(length)
        }
    }

    fn handle() -> (Mk2Handle<Recorder>, Recorder) {
        let recorder = Recorder::default();
        let model = models::stream_deck_mk2();
        let id = DeviceId {
            model: model.id.clone(),
            serial: "TEST".into(),
        };
        (
            Mk2Handle::new(recorder.clone(), id, model).unwrap(),
            recorder,
        )
    }

    fn key(index: u16) -> ControlId {
        ControlId::Key { index }
    }

    fn key_report(pressed: &[usize]) -> Vec<u8> {
        let mut report = vec![0x01, 0x00, 0x0F, 0x00];
        report.extend((0..15).map(|i| u8::from(pressed.contains(&i))));
        report
    }

    /// Reassembles the image uploads in `reports`, by key.
    fn uploads(reports: &[Vec<u8>]) -> Vec<(u8, Vec<u8>)> {
        let mut done = Vec::new();
        let mut current: Option<(u8, Vec<u8>, u16)> = None;
        for report in reports {
            assert_eq!(report.len(), mk2::IMAGE_REPORT_LEN);
            assert_eq!(report[..2], [0x02, 0x07]);
            let key = report[2];
            let last = report[3] == 1;
            let length = usize::from(u16::from_le_bytes([report[4], report[5]]));
            let page = u16::from_le_bytes([report[6], report[7]]);
            let (k, mut data, expected) = current.take().unwrap_or((key, Vec::new(), 0));
            assert_eq!(k, key, "pages of one image are not interleaved");
            assert_eq!(page, expected, "pages are consecutive");
            data.extend_from_slice(&report[8..8 + length]);
            if last {
                done.push((key, data));
            } else {
                current = Some((key, data, page + 1));
            }
        }
        assert!(current.is_none(), "every upload ends with a last page");
        done
    }

    fn decode(jpeg: &[u8]) -> image::RgbImage {
        image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .unwrap()
            .into_rgb8()
    }

    #[test]
    fn images_are_queued_until_flush_and_sent_as_reports() {
        let (mut handle, recorder) = handle();
        handle
            .set_image(key(7), &Frame::filled(72, 72, [255, 0, 0]))
            .unwrap();
        assert!(recorder.take_written().is_empty(), "nothing before flush");
        handle.flush().unwrap();
        let reports = recorder.take_written();
        let images = uploads(&reports);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].0, 7);
        let image = decode(&images[0].1);
        assert_eq!(image.dimensions(), (72, 72));
        assert!(image.get_pixel(36, 36)[0] > 200);

        handle.flush().unwrap();
        assert!(recorder.take_written().is_empty(), "the queue is emptied");
    }

    #[test]
    fn the_last_request_per_key_wins_within_a_flush() {
        let (mut handle, recorder) = handle();
        // Set then clear: the key must end blank (the previous library
        // sent clears immediately and images only on flush, so the image
        // would have won).
        handle
            .set_image(key(3), &Frame::filled(72, 72, [255, 255, 255]))
            .unwrap();
        handle.clear_image(key(3)).unwrap();
        // Clear then set: the key must end with the image.
        handle.clear_image(key(9)).unwrap();
        handle
            .set_image(key(9), &Frame::filled(72, 72, [0, 255, 0]))
            .unwrap();
        handle.flush().unwrap();

        let images = uploads(&recorder.take_written());
        let keys: Vec<u8> = images.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![3, 9], "one upload per key, in key order");
        assert_eq!(images[0].1, handle.blank);
        assert!(
            decode(&images[0].1)
                .pixels()
                .all(|p| p.0.iter().all(|c| *c < 16))
        );
        assert!(decode(&images[1].1).get_pixel(10, 10)[1] > 200);
    }

    #[test]
    fn every_key_can_be_addressed_and_others_are_rejected() {
        let (mut handle, recorder) = handle();
        for index in 0..15 {
            handle.clear_image(key(index)).unwrap();
        }
        handle.flush().unwrap();
        let keys: Vec<u8> = uploads(&recorder.take_written())
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(keys, (0..15).collect::<Vec<u8>>());

        let frame = Frame::filled(72, 72, [0; 3]);
        for control in [
            key(15),
            key(300),
            ControlId::Encoder { index: 0 },
            ControlId::TouchZone { index: 0 },
        ] {
            assert_eq!(
                handle.set_image(control, &frame),
                Err(DeviceError::InvalidControl(control))
            );
            assert!(handle.clear_image(control).is_err());
        }
    }

    #[test]
    fn brightness_sends_the_feature_report() {
        let (mut handle, recorder) = handle();
        handle.set_brightness(55).unwrap();
        handle.set_brightness(200).unwrap();
        let features = std::mem::take(&mut recorder.state().features);
        assert_eq!(features.len(), 2);
        assert_eq!(features[0].len(), 32);
        assert_eq!(features[0][..3], [0x03, 0x08, 55]);
        assert_eq!(features[1][..3], [0x03, 0x08, 100]);
        assert_eq!(recorder.take_written(), Vec::<Vec<u8>>::new());
    }

    #[test]
    fn input_reports_become_press_and_release_events() {
        let (mut handle, recorder) = handle();
        let timeout = Duration::from_millis(1);
        assert_eq!(handle.read_input(timeout).unwrap(), vec![], "timeout");

        recorder.state().input.extend([
            key_report(&[0]),
            key_report(&[0]), // repeated state: no new event
            key_report(&[14]),
            key_report(&[]),
        ]);
        let mut events = Vec::new();
        for _ in 0..4 {
            events.extend(handle.read_input(timeout).unwrap());
        }
        assert_eq!(
            events,
            vec![
                (key(0), ControlInput::Press),
                (key(0), ControlInput::Release),
                (key(14), ControlInput::Press),
                (key(14), ControlInput::Release),
            ]
        );

        let mut foreign = key_report(&[5]);
        foreign[0] = 0x02;
        recorder.state().input.push_back(foreign);
        assert_eq!(handle.read_input(timeout).unwrap(), vec![]);
    }

    #[test]
    fn unplugging_reports_disconnected() {
        let (mut handle, recorder) = handle();
        handle.clear_image(key(0)).unwrap();
        recorder.state().unplugged = true;
        let timeout = Duration::from_millis(1);
        assert_eq!(handle.read_input(timeout), Err(DeviceError::Disconnected));
        assert_eq!(handle.flush(), Err(DeviceError::Disconnected));
        assert_eq!(handle.set_brightness(10), Err(DeviceError::Disconnected));
    }

    #[test]
    fn discovery_identifies_supported_devices_by_usb_id_and_serial() {
        let models = supported_models();
        let mk2 = identify(&models, 0x0fd9, 0x0080, Some("AL12345")).unwrap();
        assert_eq!(mk2.model, models::stream_deck_mk2().id);
        assert_eq!(mk2.serial, "AL12345");
        assert_eq!(identify(&models, 0x0fd9, 0x0080, None), None);
        assert_eq!(identify(&models, 0x0fd9, 0x0080, Some("")), None);
        // XL, +, MK.2 Scissor and other vendors have no driver.
        for product in [0x006c, 0x0084, 0x00a5, 0x0060] {
            assert_eq!(identify(&models, 0x0fd9, product, Some("X")), None);
        }
        assert_eq!(identify(&models, 0x1234, 0x0080, Some("X")), None);
    }

    /// The descriptor must agree with an independent protocol
    /// implementation; this guards against silent mismatches.
    #[test]
    fn mk2_descriptor_matches_reference_implementation() {
        let model = models::stream_deck_mk2();
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
        assert_eq!(usize::from(rows * columns), mk2::KEY_COUNT);
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

    /// The blank image decodes to the same all-black key as the reference
    /// implementation's built-in blank image.
    #[test]
    fn blank_matches_reference_blank() {
        let (handle, _) = handle();
        let ours = decode(&handle.blank);
        let reference = decode(&Kind::Mk2.blank_image());
        assert_eq!(ours.dimensions(), reference.dimensions());
        assert!(ours.pixels().all(|p| p.0.iter().all(|c| *c < 8)));
        assert!(reference.pixels().all(|p| p.0.iter().all(|c| *c < 8)));
    }

    /// XL and + have no driver, but their layout-design descriptors must
    /// still match the reference implementation.
    #[test]
    fn descriptor_only_models_match_reference_implementation() {
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

    #[test]
    fn only_the_mk2_has_a_driver() {
        let ids: Vec<_> = supported_models().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![models::stream_deck_mk2().id]);
    }
}
