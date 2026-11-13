//! Interfaces implemented by device drivers.
//!
//! A [`DeviceBackend`] discovers devices and opens them; a [`DeviceHandle`]
//! performs I/O on one opened device. Drivers translate their native protocol
//! into [`ControlId`]/[`ControlInput`] pairs and accept device-independent
//! [`Frame`]s, converting them to the device's image format themselves.

use std::fmt;
use std::time::Duration;

use thiserror::Error;

use crate::{ControlId, ControlInput, DeviceId, DeviceModelId, DriverInfo};

/// An RGB image (8 bits per channel, row-major, no padding).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Frame {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl Frame {
    /// A frame filled with one colour.
    pub fn filled(width: u32, height: u32, color: [u8; 3]) -> Self {
        let pixels = width as usize * height as usize;
        Self {
            width,
            height,
            rgb: color.repeat(pixels),
        }
    }

    /// Wraps raw RGB data, checking that its length matches the dimensions.
    pub fn from_rgb(width: u32, height: u32, rgb: Vec<u8>) -> Result<Self, DeviceError> {
        if rgb.len() == width as usize * height as usize * 3 {
            Ok(Self { width, height, rgb })
        } else {
            Err(DeviceError::InvalidFrame(format!(
                "{} bytes do not match {width} x {height} RGB",
                rgb.len()
            )))
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    pub fn into_rgb(self) -> Vec<u8> {
        self.rgb
    }

    /// Sets one pixel; coordinates outside the frame are ignored.
    pub fn put(&mut self, x: u32, y: u32, color: [u8; 3]) {
        if x < self.width && y < self.height {
            let offset = (y as usize * self.width as usize + x as usize) * 3;
            self.rgb[offset..offset + 3].copy_from_slice(&color);
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        (x < self.width && y < self.height).then(|| {
            let offset = (y as usize * self.width as usize + x as usize) * 3;
            [self.rgb[offset], self.rgb[offset + 1], self.rgb[offset + 2]]
        })
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Frame({} x {})", self.width, self.height)
    }
}

/// A device found during discovery but not necessarily opened yet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiscoveredDevice {
    pub id: DeviceId,
    pub model: DeviceModelId,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeviceError {
    #[error("device is disconnected")]
    Disconnected,
    #[error("device does not have {0}")]
    InvalidControl(ControlId),
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
    #[error("operation not supported: {0}")]
    Unsupported(String),
    #[error("device I/O failed: {0}")]
    Io(String),
}

/// Discovers and opens devices of one driver.
pub trait DeviceBackend: Send {
    /// Short driver name used in logs.
    fn name(&self) -> &'static str;

    /// Lists currently attached devices that this backend can drive.
    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError>;

    /// Opens a previously discovered device.
    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError>;

    /// Name of the backend that provides `device`; differs from
    /// [`DeviceBackend::name`] only for backends that combine others.
    fn source(&self, _device: &DeviceId) -> &'static str {
        self.name()
    }

    /// Version of the driver implementation, reported with each device.
    fn version(&self) -> &'static str {
        "unknown"
    }

    /// The driver that provides `device`: [`DeviceBackend::source`] and its
    /// [`DeviceBackend::version`].
    fn driver(&self, device: &DeviceId) -> DriverInfo {
        DriverInfo {
            id: self.source(device).to_owned(),
            version: self.version().to_owned(),
        }
    }
}

/// Several backends behind one interface, e.g. USB devices plus simulated
/// ones during development. Each device is opened by the backend that
/// discovered it.
#[derive(Default)]
pub struct CombinedBackend {
    backends: Vec<Box<dyn DeviceBackend>>,
    owners: std::collections::BTreeMap<DeviceId, usize>,
}

impl fmt::Debug for CombinedBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.backends.iter().map(|b| b.name()).collect();
        f.debug_struct("CombinedBackend")
            .field("backends", &names)
            .field("owners", &self.owners)
            .finish()
    }
}

impl CombinedBackend {
    #[must_use]
    pub fn with(mut self, backend: Box<dyn DeviceBackend>) -> Self {
        self.backends.push(backend);
        self
    }
}

impl DeviceBackend for CombinedBackend {
    fn name(&self) -> &'static str {
        match self.backends.as_slice() {
            [single] => single.name(),
            _ => "combined",
        }
    }

    /// Returns devices from every backend that succeeds. Fails only if all
    /// backends fail, so one broken backend does not hide the others.
    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
        let mut found = Vec::new();
        let mut first_error = None;
        let mut succeeded = false;
        self.owners.clear();
        for (index, backend) in self.backends.iter_mut().enumerate() {
            match backend.discover() {
                Ok(devices) => {
                    succeeded = true;
                    for device in devices {
                        self.owners.entry(device.id.clone()).or_insert(index);
                        found.push(device);
                    }
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) if !succeeded => Err(error),
            _ => Ok(found),
        }
    }

    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
        let index = *self.owners.get(device).ok_or(DeviceError::Disconnected)?;
        self.backends[index].open(device)
    }

    fn source(&self, device: &DeviceId) -> &'static str {
        self.owners
            .get(device)
            .map_or_else(|| self.name(), |index| self.backends[*index].source(device))
    }

    fn driver(&self, device: &DeviceId) -> DriverInfo {
        match self.owners.get(device) {
            Some(index) => self.backends[*index].driver(device),
            None => DriverInfo {
                id: self.name().to_owned(),
                version: self.version().to_owned(),
            },
        }
    }
}

/// An opened device. Implementations are used from one dedicated I/O thread.
pub trait DeviceHandle: Send {
    fn id(&self) -> &DeviceId;

    /// Waits up to `timeout` for input. Returns an empty list when nothing
    /// happened and [`DeviceError::Disconnected`] once the device is gone.
    fn read_input(
        &mut self,
        timeout: Duration,
    ) -> Result<Vec<(ControlId, ControlInput)>, DeviceError>;

    /// Queues an image for a control. Takes effect after [`DeviceHandle::flush`].
    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError>;

    /// Queues a blank image for a control. Takes effect after [`DeviceHandle::flush`].
    fn clear_image(&mut self, control: ControlId) -> Result<(), DeviceError>;

    /// Sets display brightness in percent (0-100).
    fn set_brightness(&mut self, percent: u8) -> Result<(), DeviceError>;

    /// Sends all queued images to the device.
    fn flush(&mut self) -> Result<(), DeviceError>;
}

/// Turns full key-state snapshots, as reported by many HID devices, into
/// press and release transitions. Repeated reports of an unchanged state
/// produce no events, which prevents duplicate action execution.
#[derive(Debug, Clone, Default)]
pub struct KeyStateTracker {
    pressed: Vec<bool>,
}

impl KeyStateTracker {
    pub fn update(&mut self, states: &[bool]) -> Vec<(ControlId, ControlInput)> {
        if self.pressed.len() < states.len() {
            self.pressed.resize(states.len(), false);
        }
        let mut events = Vec::new();
        for (index, (&now, before)) in states.iter().zip(self.pressed.iter_mut()).enumerate() {
            if now != *before {
                *before = now;
                let Ok(index) = u16::try_from(index) else {
                    break;
                };
                let input = if now {
                    ControlInput::Press
                } else {
                    ControlInput::Release
                };
                events.push((ControlId::Key { index }, input));
            }
        }
        events
    }

    /// Forgets all pressed keys, e.g. after a reconnect.
    pub fn reset(&mut self) {
        self.pressed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_validate_size_and_pixels() {
        let mut frame = Frame::filled(2, 2, [1, 2, 3]);
        assert_eq!(frame.rgb().len(), 12);
        frame.put(1, 1, [9, 9, 9]);
        frame.put(5, 5, [9, 9, 9]);
        assert_eq!(frame.pixel(1, 1), Some([9, 9, 9]));
        assert_eq!(frame.pixel(0, 0), Some([1, 2, 3]));
        assert_eq!(frame.pixel(2, 0), None);
        assert!(Frame::from_rgb(2, 2, vec![0; 11]).is_err());
        assert!(Frame::from_rgb(2, 2, vec![0; 12]).is_ok());
    }

    #[cfg(feature = "mock")]
    #[test]
    fn combined_backend_routes_devices_to_their_source() {
        use crate::{mock::MockBackend, models};

        struct Failing;
        impl DeviceBackend for Failing {
            fn name(&self) -> &'static str {
                "failing"
            }
            fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
                Err(DeviceError::Io("no access".into()))
            }
            fn open(&mut self, _: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
                Err(DeviceError::Disconnected)
            }
        }

        let mock = MockBackend::new();
        let id = mock.plug(&models::stream_deck_xl(), "SIM-1");
        let mut combined = CombinedBackend::default()
            .with(Box::new(Failing))
            .with(Box::new(mock));
        let found = combined.discover().unwrap();
        assert_eq!(found.len(), 1, "a failing backend does not hide the others");
        assert_eq!(combined.source(&id), "mock");
        let driver = combined.driver(&id);
        assert_eq!(driver.id, "mock");
        assert_eq!(driver.version, env!("CARGO_PKG_VERSION"));
        assert!(combined.open(&id).is_ok());

        let mut only_failing = CombinedBackend::default().with(Box::new(Failing));
        assert!(only_failing.discover().is_err());
    }

    #[test]
    fn key_tracker_emits_transitions_only() {
        let mut tracker = KeyStateTracker::default();
        let press = |index| (ControlId::Key { index }, ControlInput::Press);
        let release = |index| (ControlId::Key { index }, ControlInput::Release);

        assert_eq!(tracker.update(&[false, true, false]), vec![press(1)]);
        // A repeated report of the same state must not trigger again.
        assert_eq!(tracker.update(&[false, true, false]), vec![]);
        assert_eq!(
            tracker.update(&[true, false, false]),
            vec![press(0), release(1)]
        );
        tracker.reset();
        assert_eq!(tracker.update(&[true, false, false]), vec![press(0)]);
    }
}
