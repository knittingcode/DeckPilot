//! A device driver that stops responding must not keep the agent from
//! stopping: its I/O thread is left behind after a bounded wait.

mod support;

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use deckpilot_agent::{Agent, AgentOptions, InstanceLock};
use deckpilot_core::Profile;
use deckpilot_device::{
    ConnectionState, ControlId, ControlInput, DeviceBackend, DeviceError, DeviceHandle, DeviceId,
    DiscoveredDevice, Frame, models,
};
use deckpilot_ipc::{Request, Response};
use support::{TIMEOUT, TempDir};
use tokio::sync::oneshot;

/// One MK.2 whose reads block until the test drops `release`.
#[derive(Clone)]
struct StuckBackend {
    id: DeviceId,
    release: Arc<Mutex<Option<mpsc::Sender<()>>>>,
}

struct StuckHandle {
    id: DeviceId,
    wait: Mutex<mpsc::Receiver<()>>,
}

impl DeviceBackend for StuckBackend {
    fn name(&self) -> &'static str {
        "stuck"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
        Ok(vec![DiscoveredDevice {
            id: self.id.clone(),
            model: self.id.model.clone(),
        }])
    }

    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
        let (release, wait) = mpsc::channel();
        *self.release.lock().unwrap() = Some(release);
        Ok(Box::new(StuckHandle {
            id: device.clone(),
            wait: Mutex::new(wait),
        }))
    }
}

impl DeviceHandle for StuckHandle {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn read_input(&mut self, _: Duration) -> Result<Vec<(ControlId, ControlInput)>, DeviceError> {
        // Ignores the timeout, like a hung HID read.
        let _ = self.wait.lock().unwrap().recv();
        Err(DeviceError::Disconnected)
    }

    fn set_image(&mut self, _: ControlId, _: &Frame) -> Result<(), DeviceError> {
        Ok(())
    }

    fn clear_image(&mut self, _: ControlId) -> Result<(), DeviceError> {
        Ok(())
    }

    fn set_brightness(&mut self, _: u8) -> Result<(), DeviceError> {
        Ok(())
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_stuck_in_a_read_does_not_block_stopping() {
    let dir = TempDir::new("deckpilot-stuck");
    let (paths, _key) = support::data_dir(&dir, &Profile::new("Work"), &[]);
    let backend = StuckBackend {
        id: DeviceId {
            model: models::stream_deck_mk2().id,
            serial: "STUCK-1".into(),
        },
        release: Arc::default(),
    };
    let options = AgentOptions {
        paths: paths.clone(),
        backend: Box::new(backend.clone()),
        mock: None,
        rescan_interval: Duration::from_millis(50),
        autostart: None,
        open_desktop: None,
        open_window: None,
        key_store: None,
        auto_lock_check: Duration::from_millis(50),
        cancel_grace: Duration::from_secs(10),
        clock: deckpilot_agent::system_clock(),
        notifier: None,
        screen_lock: None,
    };
    let agent = Agent::start(options, InstanceLock::acquire(&paths.lock_file).unwrap()).unwrap();
    let (_stop, stopped) = oneshot::channel::<()>();
    let mut task = tokio::spawn(agent.run(async move {
        let _ = stopped.await;
    }));

    let client = || async {
        deckpilot_ipc::AgentClient::connect(&paths, deckpilot_ipc::ClientKind::Other)
            .await
            .unwrap()
    };
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(Response::Status { status }) = client().await.request(Request::GetStatus).await
            && status
                .devices
                .iter()
                .any(|d| d.state == ConnectionState::Connected)
        {
            break;
        }
        assert!(Instant::now() < deadline, "the device did not connect");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let started = Instant::now();
    client().await.request(Request::Shutdown).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(15), &mut task).await;
    assert!(
        result.is_ok(),
        "the agent did not stop within 15 s while a device thread was stuck"
    );
    result.unwrap().unwrap().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));

    // Let the thread that was left behind finish.
    drop(backend.release.lock().unwrap().take());
}
