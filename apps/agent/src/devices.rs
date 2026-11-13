//! One I/O thread per opened device.
//!
//! HID handles are not shareable between threads, so a dedicated thread owns
//! each handle. It blocks in `read_input` with a short timeout (no busy
//! polling) and, between reads, applies queued image and brightness updates.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use deckpilot_device::{ControlId, ControlInput, DeviceError, DeviceHandle, DeviceId, Frame};
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

/// Upper bound for how long queued image updates wait while the thread is
/// blocked reading input. Input itself is delivered immediately.
const READ_TIMEOUT: Duration = Duration::from_millis(100);

/// How long stopping waits for device threads to blank their keys and end.
/// They normally take one read timeout; a driver stuck beyond that must
/// not keep the agent from stopping.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum DeviceSignal {
    /// `session` identifies the connection, so signals from a previous
    /// connection of the same device can be told apart.
    Input {
        session: u64,
        device: DeviceId,
        control: ControlId,
        input: ControlInput,
        /// When the input was read: gesture timing uses this, not the time
        /// the service task gets to it.
        at: Instant,
    },
    /// The I/O thread ended. `error` is `None` when it was asked to close.
    Closed {
        session: u64,
        device: DeviceId,
        error: Option<DeviceError>,
    },
}

#[derive(Debug)]
enum IoCommand {
    /// `None` clears the control.
    Images(Vec<(ControlId, Option<Frame>)>),
    Brightness(u8),
    Close {
        blank: Vec<ControlId>,
    },
}

#[derive(Debug)]
pub struct DeviceIo {
    device: DeviceId,
    commands: mpsc::Sender<IoCommand>,
    thread: Option<JoinHandle<()>>,
    /// Disconnects when the thread ends (its sender is dropped then, also
    /// on a panic), so waiting for it can be bounded.
    finished: mpsc::Receiver<()>,
}

impl DeviceIo {
    pub fn spawn(
        session: u64,
        mut handle: Box<dyn DeviceHandle>,
        signals: UnboundedSender<DeviceSignal>,
    ) -> std::io::Result<Self> {
        let (commands, inbox) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel::<()>();
        let device = handle.id().clone();
        let thread_device = device.clone();
        let thread = std::thread::Builder::new()
            .name(format!("device-{}", device.serial))
            .spawn(move || {
                let _finished = finished_tx;
                let error = io_loop(session, handle.as_mut(), &inbox, &signals);
                let _ = signals.send(DeviceSignal::Closed {
                    session,
                    device: thread_device,
                    error,
                });
            })?;
        Ok(Self {
            device,
            commands,
            thread: Some(thread),
            finished,
        })
    }

    pub fn set_images(&self, images: Vec<(ControlId, Option<Frame>)>) {
        if !images.is_empty() {
            let _ = self.commands.send(IoCommand::Images(images));
        }
    }

    pub fn set_brightness(&self, percent: u8) {
        let _ = self.commands.send(IoCommand::Brightness(percent));
    }

    /// Asks the thread to blank `blank` and stop, without waiting; see
    /// [`DeviceIo::finish`].
    pub fn request_close(&self, blank: Vec<ControlId>) {
        let _ = self.commands.send(IoCommand::Close { blank });
    }

    /// Waits until the thread has stopped, but not past `deadline`. A
    /// thread that is still busy then (a driver blocked in a read or write
    /// beyond its timeout) is left behind instead of blocking the agent;
    /// it ends with the process. Returns whether the thread stopped.
    pub fn finish(mut self, deadline: Instant) -> bool {
        let wait = deadline.saturating_duration_since(Instant::now());
        match self.finished.recv_timeout(wait) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(thread) = self.thread.take() {
                    let _ = thread.join();
                }
                true
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                warn!(device = %self.device, "the device thread did not stop in time; leaving it behind");
                // Dropping the join handle detaches the thread.
                self.thread = None;
                false
            }
        }
    }

    /// Blanks `blank`, stops the thread and waits for it, at most `timeout`.
    pub fn close(self, blank: Vec<ControlId>, timeout: Duration) -> bool {
        self.request_close(blank);
        self.finish(Instant::now() + timeout)
    }
}

fn io_loop(
    session: u64,
    handle: &mut dyn DeviceHandle,
    inbox: &mpsc::Receiver<IoCommand>,
    signals: &UnboundedSender<DeviceSignal>,
) -> Option<DeviceError> {
    loop {
        match handle.read_input(READ_TIMEOUT) {
            Ok(events) => {
                let at = Instant::now();
                for (control, input) in events {
                    let device = handle.id().clone();
                    let signal = DeviceSignal::Input {
                        session,
                        device,
                        control,
                        input,
                        at,
                    };
                    if signals.send(signal).is_err() {
                        return None;
                    }
                }
            }
            Err(error) => return Some(error),
        }
        loop {
            let result = match inbox.try_recv() {
                Ok(IoCommand::Images(images)) => apply_images(handle, images),
                Ok(IoCommand::Brightness(percent)) => handle.set_brightness(percent),
                Ok(IoCommand::Close { blank }) => {
                    let images = blank.into_iter().map(|control| (control, None)).collect();
                    // Best effort: the device may already be gone.
                    let _ = apply_images(handle, images);
                    return None;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return None,
            };
            if let Err(error) = result {
                return Some(error);
            }
        }
    }
}

fn apply_images(
    handle: &mut dyn DeviceHandle,
    images: Vec<(ControlId, Option<Frame>)>,
) -> Result<(), DeviceError> {
    for (control, frame) in images {
        match frame {
            Some(frame) => handle.set_image(control, &frame)?,
            None => handle.clear_image(control)?,
        }
    }
    handle.flush()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use deckpilot_device::mock::MockBackend;
    use deckpilot_device::{DeviceBackend, DeviceModelId, models};

    use super::*;

    /// A driver whose read does not return until `release` is dropped, as
    /// a HID read that ignores its timeout would.
    struct StuckHandle {
        id: DeviceId,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl DeviceHandle for StuckHandle {
        fn id(&self) -> &DeviceId {
            &self.id
        }

        fn read_input(
            &mut self,
            _: Duration,
        ) -> Result<Vec<(ControlId, ControlInput)>, DeviceError> {
            let _ = self.release.lock().unwrap().recv();
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

    #[test]
    fn closing_blanks_the_keys_and_waits_for_the_thread() {
        let mut mock = MockBackend::new();
        let id = mock.plug(&models::stream_deck_mk2(), "A1");
        let (signals, mut received) = tokio::sync::mpsc::unbounded_channel();
        let io = DeviceIo::spawn(1, mock.open(&id).unwrap(), signals).unwrap();
        let key = ControlId::Key { index: 0 };
        io.set_images(vec![(key, Some(Frame::filled(72, 72, [9, 9, 9])))]);
        assert!(mock.wait_for(&id, Duration::from_secs(10), |frames, _| {
            frames.contains_key(&key)
        }));

        assert!(io.close(vec![key], CLOSE_TIMEOUT));
        assert!(mock.frames(&id).is_empty(), "blanked before stopping");
        assert!(matches!(
            received.try_recv(),
            Ok(DeviceSignal::Closed { error: None, .. })
        ));
    }

    #[test]
    fn a_stuck_driver_does_not_block_closing() {
        let (release, stuck) = mpsc::channel();
        let handle = StuckHandle {
            id: DeviceId {
                model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
                serial: "STUCK".into(),
            },
            release: Mutex::new(stuck),
        };
        let (signals, mut received) = tokio::sync::mpsc::unbounded_channel();
        let io = DeviceIo::spawn(1, Box::new(handle), signals).unwrap();

        let started = Instant::now();
        assert!(!io.close(Vec::new(), Duration::from_millis(200)));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "close returned after {:?}",
            started.elapsed()
        );

        // The thread left behind still ends once its read returns.
        drop(release);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(DeviceSignal::Closed { error, .. }) = received.try_recv() {
                assert_eq!(error, Some(DeviceError::Disconnected));
                break;
            }
            assert!(Instant::now() < deadline, "the thread did not end");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
