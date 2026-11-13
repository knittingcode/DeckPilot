//! A device driver for a vendor DeckPilot has never heard of, defined only in
//! this test, is registered with the agent like any other driver. Without a
//! change to core or to the built-in catalog, its devices are discovered,
//! described generically, drive a layout through generic events, and receive
//! rendered frames. The simulated (mock) backend keeps working next to it.

mod support;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_core::{BindingTarget, ControlBinding, DeviceLayout, LayoutPage, Profile, Trigger};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{
    CombinedBackend, ConnectionState, ControlId, ControlInput, DeviceBackend, DeviceCapabilities,
    DeviceError, DeviceHandle, DeviceId, DeviceModel, DeviceModelId, DiscoveredDevice, EncoderSpec,
    Frame, HardwareSupport, ImageFormat, ImageSpec, ImageTransform, KeyGrid, KeyStateTracker,
    ModelVerification, Platform, PlatformVerification, VerificationLevel, models,
};
use deckpilot_ipc::{AgentStatus, Request};
use support::{TIMEOUT, TempDir, TestAgent};

const MODEL: &str = "acme.pad.p4";

/// Four 32 x 32 px keys (2 x 2) and one dial; no adjustable brightness.
fn acme_model() -> DeviceModel {
    DeviceModel {
        id: DeviceModelId::new(MODEL),
        vendor: "Acme".into(),
        name: "Acme Pad P4".into(),
        usb: None,
        capabilities: DeviceCapabilities {
            keys: Some(KeyGrid {
                rows: 2,
                columns: 2,
                image: Some(ImageSpec {
                    width: 32,
                    height: 32,
                    format: ImageFormat::Bmp,
                    transform: ImageTransform::default(),
                }),
            }),
            encoders: Some(EncoderSpec {
                count: 1,
                pressable: false,
            }),
            touch: None,
            brightness: false,
        },
        support: HardwareSupport::Unverified,
        // A driver may claim anything; physical verification is capped.
        verification: ModelVerification::new([PlatformVerification::new(
            Platform::Macos,
            VerificationLevel::PhysicallyVerified,
            Some("2026-11-13"),
            "claimed by the test driver",
        )]),
    }
}

/// The vendor's native reports: `[0x01, key bitmask]` or `[0x02, dial, delta]`.
type Report = Vec<u8>;

#[derive(Default)]
struct Bus {
    attached: Vec<String>,
    reports: BTreeMap<String, VecDeque<Report>>,
    /// Size of the last frame written per key.
    frames: BTreeMap<String, BTreeMap<ControlId, (u32, u32)>>,
    brightness_calls: usize,
}

/// The test vendor's driver (adapter). Clones share the simulated bus.
#[derive(Clone, Default)]
struct AcmeDriver {
    bus: Arc<Mutex<Bus>>,
}

impl AcmeDriver {
    fn attach(&self, serial: &str) -> DeviceId {
        self.bus.lock().unwrap().attached.push(serial.into());
        DeviceId {
            model: DeviceModelId::new(MODEL),
            serial: serial.into(),
        }
    }

    fn send(&self, serial: &str, report: Report) {
        self.bus
            .lock()
            .unwrap()
            .reports
            .entry(serial.into())
            .or_default()
            .push_back(report);
    }

    fn frames(&self, serial: &str) -> BTreeMap<ControlId, (u32, u32)> {
        self.bus
            .lock()
            .unwrap()
            .frames
            .get(serial)
            .cloned()
            .unwrap_or_default()
    }
}

impl DeviceBackend for AcmeDriver {
    fn name(&self) -> &'static str {
        "acme-usb"
    }

    fn version(&self) -> &'static str {
        "9.9.9"
    }

    fn models(&self) -> Vec<DeviceModel> {
        vec![acme_model()]
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
        Ok(self
            .bus
            .lock()
            .unwrap()
            .attached
            .iter()
            .map(|serial| DiscoveredDevice {
                id: DeviceId {
                    model: DeviceModelId::new(MODEL),
                    serial: serial.clone(),
                },
                model: DeviceModelId::new(MODEL),
            })
            .collect())
    }

    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
        Ok(Box::new(AcmeHandle {
            bus: self.bus.clone(),
            id: device.clone(),
            keys: KeyStateTracker::default(),
            pending: Vec::new(),
        }))
    }
}

struct AcmeHandle {
    bus: Arc<Mutex<Bus>>,
    id: DeviceId,
    keys: KeyStateTracker,
    pending: Vec<(ControlId, (u32, u32))>,
}

impl AcmeHandle {
    /// Translates one native report into generic DeckPilot inputs.
    fn translate(&mut self, report: &[u8]) -> Vec<(ControlId, ControlInput)> {
        match report {
            [0x01, mask] => {
                let states: Vec<bool> = (0..4).map(|bit| mask & (1 << bit) != 0).collect();
                self.keys.update(&states)
            }
            [0x02, dial, delta] => vec![(
                ControlId::Encoder { index: *dial },
                ControlInput::Rotate {
                    delta: i8::from_ne_bytes([*delta]),
                },
            )],
            _ => Vec::new(),
        }
    }
}

impl DeviceHandle for AcmeHandle {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn read_input(
        &mut self,
        timeout: Duration,
    ) -> Result<Vec<(ControlId, ControlInput)>, DeviceError> {
        let deadline = Instant::now() + timeout;
        loop {
            let report = self
                .bus
                .lock()
                .unwrap()
                .reports
                .get_mut(&self.id.serial)
                .and_then(VecDeque::pop_front);
            if let Some(report) = report {
                return Ok(self.translate(&report));
            }
            if Instant::now() >= deadline {
                return Ok(Vec::new());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError> {
        self.pending
            .push((control, (frame.width(), frame.height())));
        Ok(())
    }

    fn clear_image(&mut self, _: ControlId) -> Result<(), DeviceError> {
        Ok(())
    }

    /// The model has no brightness; the agent must never ask.
    fn set_brightness(&mut self, _: u8) -> Result<(), DeviceError> {
        self.bus.lock().unwrap().brightness_calls += 1;
        Err(DeviceError::Unsupported("brightness".into()))
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        let mut bus = self.bus.lock().unwrap();
        let frames = bus.frames.entry(self.id.serial.clone()).or_default();
        for (control, size) in self.pending.drain(..) {
            frames.insert(control, size);
        }
        Ok(())
    }
}

/// A layout for the Acme model: the dial and key 3 open "Folder"; key 1 of
/// "Folder" returns home.
fn profile() -> Profile {
    let mut profile = Profile::new("Work");
    let mut layout = DeviceLayout::new("Acme", DeviceModelId::new(MODEL));
    let home = layout.home_page;
    let folder = LayoutPage::new("Folder");
    let folder_id = folder.id;
    layout.pages.push(folder);
    let open = || BindingTarget::OpenPage { page: folder_id };
    let bindings = &mut layout.pages[0].bindings;
    bindings.push(ControlBinding::new(
        ControlId::Encoder { index: 0 },
        Trigger::RotateClockwise,
        open(),
    ));
    bindings.push(ControlBinding::new(
        ControlId::Key { index: 3 },
        Trigger::Press,
        open(),
    ));
    layout.pages[1].bindings.push(ControlBinding::new(
        ControlId::Key { index: 1 },
        Trigger::Press,
        BindingTarget::OpenPage { page: home },
    ));
    profile.layouts.push(layout);
    profile
}

async fn wait_status(agent: &TestAgent, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if predicate(&status) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not reached in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn page<'a>(status: &'a AgentStatus, device: &DeviceId) -> Option<&'a str> {
    status
        .devices
        .iter()
        .find(|d| d.id == *device)
        .and_then(|d| d.page.as_deref())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_driver_registered_only_in_a_test_drives_a_layout() {
    let dir = TempDir::new("deckpilot-vendor");
    let (paths, key) = support::data_dir(&dir, &profile(), &[]);
    let acme = AcmeDriver::default();
    let device = acme.attach("ACME-1");
    let mock = MockBackend::new();
    let mk2 = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let backend = CombinedBackend::default()
        .with(Box::new(acme.clone()))
        .with(Box::new(mock.clone()));
    let agent = TestAgent::start_with_backend(&paths, &key, &mock, Box::new(backend), None, None);
    agent.unlock().await;

    // Discovered and described generically.
    let status = wait_status(&agent, |s| {
        page(s, &device) == Some("Home")
            && s.devices
                .iter()
                .any(|d| d.id == mk2 && d.state == ConnectionState::Connected)
    })
    .await;
    let info = status.devices.iter().find(|d| d.id == device).unwrap();
    assert_eq!(info.source, "acme-usb");
    assert_eq!(info.model_name, "Acme Pad P4");
    assert!(matches!(
        info.layout,
        deckpilot_core::LayoutStatus::Compatible { .. }
    ));
    let descriptor = info.descriptor.clone().expect("a descriptor");
    assert_eq!(descriptor.vendor_id, "acme");
    assert_eq!(descriptor.serial, "ACME-1");
    let driver = descriptor.driver.unwrap();
    assert_eq!(
        (driver.id.as_str(), driver.version.as_str()),
        ("acme-usb", "9.9.9")
    );
    assert_eq!(
        descriptor.verification.level(Platform::Macos),
        VerificationLevel::AutomatedTests,
        "a driver cannot claim physical verification"
    );
    assert!(!descriptor.verification.is_supported());

    // Frames are rendered at the model's image size.
    let deadline = Instant::now() + TIMEOUT;
    while !acme
        .frames("ACME-1")
        .contains_key(&ControlId::Key { index: 3 })
    {
        assert!(Instant::now() < deadline, "no frame for key 3");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        acme.frames("ACME-1")[&ControlId::Key { index: 3 }],
        (32, 32)
    );

    // Native dial and key reports become generic events that drive the layout.
    acme.send("ACME-1", vec![0x02, 0, 1]);
    wait_status(&agent, |s| page(s, &device) == Some("Folder")).await;
    acme.send("ACME-1", vec![0x01, 0b0010]);
    acme.send("ACME-1", vec![0x01, 0]);
    wait_status(&agent, |s| page(s, &device) == Some("Home")).await;
    acme.send("ACME-1", vec![0x01, 0b1000]);
    acme.send("ACME-1", vec![0x01, 0]);
    wait_status(&agent, |s| page(s, &device) == Some("Folder")).await;

    // Brightness goes only to devices that have it.
    agent
        .request(Request::SetBrightness { percent: 40 })
        .await
        .unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while mock.brightness(&mk2) != Some(40) {
        assert!(
            Instant::now() < deadline,
            "the MK.2 did not get the brightness"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(acme.bus.lock().unwrap().brightness_calls, 0);
    let status = agent.status().await;
    assert!(
        status
            .devices
            .iter()
            .all(|d| d.state == ConnectionState::Connected)
    );
    agent.shutdown().await;
}
