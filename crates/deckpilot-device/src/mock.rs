//! In-memory device backend for automated tests and hardware-free development.
//!
//! [`MockBackend`] is cheaply cloneable; clones share state, so a test can keep
//! one clone to plug devices, inject input and inspect rendered frames while
//! the code under test owns another.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::{
    ControlId, ControlInput, DeviceBackend, DeviceError, DeviceHandle, DeviceId, DeviceModel,
    DiscoveredDevice, Frame,
};

#[derive(Debug, Default)]
struct MockDevice {
    model: Option<DeviceModel>,
    connected: bool,
    /// Incremented on every plug so handles from a previous connection fail.
    generation: u64,
    input: VecDeque<(ControlId, ControlInput)>,
    pending: BTreeMap<ControlId, Option<Frame>>,
    frames: BTreeMap<ControlId, Frame>,
    image_writes: usize,
    brightness: Option<u8>,
}

#[derive(Debug, Default)]
struct Shared {
    devices: Mutex<BTreeMap<DeviceId, MockDevice>>,
    changed: Condvar,
}

/// Simulated devices with shared, inspectable state.
#[derive(Debug, Clone, Default)]
pub struct MockBackend {
    shared: Arc<Shared>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<DeviceId, MockDevice>> {
        self.shared
            .devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Attaches a device of `model` with the given serial number. Like a
    /// real device after a power cycle, it starts blank and with its own
    /// default brightness (reported as `None`).
    pub fn plug(&self, model: &DeviceModel, serial: &str) -> DeviceId {
        let id = DeviceId {
            model: model.id.clone(),
            serial: serial.into(),
        };
        let mut devices = self.lock();
        let device = devices.entry(id.clone()).or_default();
        device.model = Some(model.clone());
        device.connected = true;
        device.generation += 1;
        device.input.clear();
        device.frames.clear();
        device.pending.clear();
        device.brightness = None;
        drop(devices);
        self.shared.changed.notify_all();
        id
    }

    /// Detaches a device. Open handles report [`DeviceError::Disconnected`].
    pub fn unplug(&self, id: &DeviceId) {
        if let Some(device) = self.lock().get_mut(id) {
            device.connected = false;
        }
        self.shared.changed.notify_all();
    }

    /// Queues an input as if it was produced by the hardware.
    pub fn send_input(&self, id: &DeviceId, control: ControlId, input: ControlInput) {
        if let Some(device) = self.lock().get_mut(id) {
            device.input.push_back((control, input));
        }
        self.shared.changed.notify_all();
    }

    /// Queues several inputs at once: the device's reader takes them in
    /// one read, so they all carry the same read time. Tests use this for
    /// sequences whose gesture must not depend on scheduling (a click held
    /// for no time at all, a double click with no gap).
    pub fn send_inputs(&self, id: &DeviceId, inputs: &[(ControlId, ControlInput)]) {
        if let Some(device) = self.lock().get_mut(id) {
            device.input.extend(inputs.iter().copied());
        }
        self.shared.changed.notify_all();
    }

    /// Queues a press followed by a release.
    pub fn tap(&self, id: &DeviceId, control: ControlId) {
        self.send_input(id, control, ControlInput::Press);
        self.send_input(id, control, ControlInput::Release);
    }

    /// Frames currently shown on the device.
    pub fn frames(&self, id: &DeviceId) -> BTreeMap<ControlId, Frame> {
        self.lock()
            .get(id)
            .map(|device| device.frames.clone())
            .unwrap_or_default()
    }

    /// Total number of images written since the device was created.
    pub fn image_writes(&self, id: &DeviceId) -> usize {
        self.lock().get(id).map_or(0, |device| device.image_writes)
    }

    pub fn brightness(&self, id: &DeviceId) -> Option<u8> {
        self.lock().get(id).and_then(|device| device.brightness)
    }

    /// Blocks until `predicate` holds for the device state or `timeout` passes.
    /// Returns whether the predicate was satisfied.
    pub fn wait_for(
        &self,
        id: &DeviceId,
        timeout: Duration,
        predicate: impl Fn(&BTreeMap<ControlId, Frame>, usize) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        let mut devices = self.lock();
        loop {
            if let Some(device) = devices.get(id)
                && predicate(&device.frames, device.image_writes)
            {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            devices = self
                .shared
                .changed
                .wait_timeout(devices, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl DeviceBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
        Ok(self
            .lock()
            .iter()
            .filter(|(_, device)| device.connected)
            .map(|(id, _)| DiscoveredDevice {
                id: id.clone(),
                model: id.model.clone(),
            })
            .collect())
    }

    fn open(&mut self, id: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
        let devices = self.lock();
        let device = devices
            .get(id)
            .filter(|d| d.connected)
            .ok_or(DeviceError::Disconnected)?;
        let model = device.model.clone().ok_or(DeviceError::Disconnected)?;
        let generation = device.generation;
        drop(devices);
        Ok(Box::new(MockHandle {
            backend: self.clone(),
            id: id.clone(),
            model,
            generation,
        }))
    }
}

#[derive(Debug)]
struct MockHandle {
    backend: MockBackend,
    id: DeviceId,
    model: DeviceModel,
    generation: u64,
}

impl MockHandle {
    fn with_device<T>(
        &self,
        f: impl FnOnce(&mut MockDevice) -> Result<T, DeviceError>,
    ) -> Result<T, DeviceError> {
        let mut devices = self.backend.lock();
        match devices.get_mut(&self.id) {
            Some(device) if device.connected && device.generation == self.generation => f(device),
            _ => Err(DeviceError::Disconnected),
        }
    }

    fn check_display(&self, control: ControlId) -> Result<(), DeviceError> {
        if self.model.capabilities.has_display(control) {
            Ok(())
        } else {
            Err(DeviceError::InvalidControl(control))
        }
    }
}

impl DeviceHandle for MockHandle {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn read_input(
        &mut self,
        timeout: Duration,
    ) -> Result<Vec<(ControlId, ControlInput)>, DeviceError> {
        let deadline = Instant::now() + timeout;
        let mut devices = self.backend.lock();
        loop {
            match devices.get_mut(&self.id) {
                Some(device) if device.connected && device.generation == self.generation => {
                    if !device.input.is_empty() {
                        return Ok(device.input.drain(..).collect());
                    }
                }
                _ => return Err(DeviceError::Disconnected),
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(Vec::new());
            }
            devices = self
                .backend
                .shared
                .changed
                .wait_timeout(devices, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError> {
        self.check_display(control)?;
        self.with_device(|device| {
            device.pending.insert(control, Some(frame.clone()));
            Ok(())
        })
    }

    fn clear_image(&mut self, control: ControlId) -> Result<(), DeviceError> {
        self.check_display(control)?;
        self.with_device(|device| {
            device.pending.insert(control, None);
            Ok(())
        })
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), DeviceError> {
        self.with_device(|device| {
            device.brightness = Some(percent.min(100));
            Ok(())
        })?;
        self.backend.shared.changed.notify_all();
        Ok(())
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        self.with_device(|device| {
            for (control, frame) in std::mem::take(&mut device.pending) {
                device.image_writes += 1;
                match frame {
                    Some(frame) => device.frames.insert(control, frame),
                    None => device.frames.remove(&control),
                };
            }
            Ok(())
        })?;
        self.backend.shared.changed.notify_all();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models;

    #[test]
    fn discovery_follows_plug_state() {
        let mut backend = MockBackend::new();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        assert_eq!(backend.discover().unwrap().len(), 1);
        backend.unplug(&id);
        assert_eq!(backend.discover().unwrap(), vec![]);
        assert!(backend.open(&id).is_err());
    }

    #[test]
    fn handle_reads_input_and_writes_frames() {
        let mut backend = MockBackend::new();
        let probe = backend.clone();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        let mut handle = backend.open(&id).unwrap();

        assert_eq!(handle.read_input(Duration::from_millis(1)).unwrap(), vec![]);
        probe.tap(&id, ControlId::Key { index: 3 });
        let events = handle.read_input(Duration::from_millis(100)).unwrap();
        assert_eq!(events.len(), 2);

        let key = ControlId::Key { index: 3 };
        handle
            .set_image(key, &Frame::filled(72, 72, [255, 0, 0]))
            .unwrap();
        assert_eq!(probe.frames(&id).len(), 0, "images apply on flush");
        handle.flush().unwrap();
        assert_eq!(probe.frames(&id)[&key].pixel(0, 0), Some([255, 0, 0]));
        assert!(
            handle
                .set_image(ControlId::Key { index: 15 }, &Frame::filled(1, 1, [0; 3]))
                .is_err()
        );

        handle.set_brightness(150).unwrap();
        assert_eq!(probe.brightness(&id), Some(100));
    }

    #[test]
    fn stale_handles_fail_after_replug() {
        let mut backend = MockBackend::new();
        let probe = backend.clone();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        let mut old = backend.open(&id).unwrap();
        probe.unplug(&id);
        assert_eq!(
            old.read_input(Duration::from_millis(1)),
            Err(DeviceError::Disconnected)
        );
        probe.plug(&models::stream_deck_mk2(), "A1");
        assert_eq!(
            old.read_input(Duration::from_millis(1)),
            Err(DeviceError::Disconnected)
        );
        assert!(
            backend
                .open(&id)
                .unwrap()
                .read_input(Duration::from_millis(1))
                .is_ok()
        );
    }
}
