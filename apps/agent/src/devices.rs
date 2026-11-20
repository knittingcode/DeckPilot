//! One I/O thread per opened device.
//!
//! HID handles are not shareable between threads, so a dedicated thread owns
//! each handle. It blocks in `read_input` with a short timeout (no busy
//! polling) and, between reads, applies queued image and brightness updates.
//! While keys scroll (`marquee.rs`) the read timeout is shorter, so their
//! frames are written on time.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use deckpilot_device::{ControlId, ControlInput, DeviceError, DeviceHandle, DeviceId, Frame};
use tokio::sync::mpsc::UnboundedSender;
use tracing::warn;

/// Upper bound for how long queued image updates wait while the thread is
/// blocked reading input. Input itself is delivered immediately.
const READ_TIMEOUT: Duration = Duration::from_millis(100);

/// The read timeout while keys scroll: half a marquee frame, so frames
/// wait at most that long.
const ANIMATION_READ_TIMEOUT: Duration = Duration::from_millis(30);

/// How long after the last scrolling frame the shorter timeout is kept.
const ANIMATION_LINGER: Duration = Duration::from_millis(500);

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
    /// Frames of scrolling keys, made in `epoch`: dropped unless it is
    /// still the current one.
    Animation {
        epoch: u64,
        images: Vec<(ControlId, Frame)>,
    },
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
    /// The current epoch of scrolling frames ([`DeviceIo::next_epoch`]).
    epoch: Arc<AtomicU64>,
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
        let epoch = Arc::new(AtomicU64::new(0));
        let current = epoch.clone();
        let thread = std::thread::Builder::new()
            .name(format!("device-{}", device.serial))
            .spawn(move || {
                let _finished = finished_tx;
                let error = io_loop(session, handle.as_mut(), &inbox, &signals, &current);
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
            epoch,
        })
    }

    /// Starts a new epoch of scrolling frames and returns it: frames of
    /// earlier epochs still on their way are dropped. Called before the
    /// images that replace them are queued.
    pub fn next_epoch(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Where an animator sends scrolling frames for this device.
    pub fn animation_sink(&self) -> crate::marquee::FrameSink {
        let commands = self.commands.clone();
        Box::new(move |epoch, images| {
            if !images.is_empty() {
                let _ = commands.send(IoCommand::Animation { epoch, images });
            }
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
    epoch: &AtomicU64,
) -> Option<DeviceError> {
    let mut animating_until: Option<Instant> = None;
    loop {
        let timeout = if animating_until.is_some_and(|until| Instant::now() < until) {
            ANIMATION_READ_TIMEOUT
        } else {
            READ_TIMEOUT
        };
        match handle.read_input(timeout) {
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
        let mut next = None;
        loop {
            let command = match next.take() {
                Some(command) => Ok(command),
                None => inbox.try_recv(),
            };
            let result = match command {
                Ok(IoCommand::Images(images)) => {
                    // Images queued meanwhile (the agent redraws faster
                    // than the device takes them, e.g. while the editor's
                    // preview changes): one write per key, the latest.
                    let (images, after) = coalesce(images, inbox);
                    next = after;
                    apply_images(handle, images)
                }
                Ok(IoCommand::Animation {
                    epoch: made,
                    images,
                }) => {
                    if made == epoch.load(Ordering::SeqCst) {
                        animating_until = Some(Instant::now() + ANIMATION_LINGER);
                        apply_images(
                            handle,
                            images
                                .into_iter()
                                .map(|(control, frame)| (control, Some(frame)))
                                .collect(),
                        )
                    } else {
                        Ok(())
                    }
                }
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

/// `images` and the image commands queued right after it, as one batch
/// with the latest image of each control (in the order controls first
/// appeared); the first other command is handed back.
fn coalesce(
    images: Vec<(ControlId, Option<Frame>)>,
    inbox: &mpsc::Receiver<IoCommand>,
) -> (Vec<(ControlId, Option<Frame>)>, Option<IoCommand>) {
    let mut batch = images;
    loop {
        match inbox.try_recv() {
            Ok(IoCommand::Images(more)) => {
                for (control, frame) in more {
                    match batch.iter_mut().find(|(c, _)| *c == control) {
                        Some(slot) => slot.1 = frame,
                        None => batch.push((control, frame)),
                    }
                }
            }
            Ok(other) => return (batch, Some(other)),
            Err(_) => return (batch, None),
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
    fn queued_images_are_written_once_per_key_with_the_latest() {
        let (tx, rx) = mpsc::channel();
        let key = |index| ControlId::Key { index };
        let frame = |shade| Some(Frame::filled(2, 2, [shade, 0, 0]));
        tx.send(IoCommand::Images(vec![
            (key(1), frame(2)),
            (key(2), frame(3)),
        ]))
        .unwrap();
        tx.send(IoCommand::Images(vec![(key(1), frame(4))]))
            .unwrap();
        tx.send(IoCommand::Brightness(50)).unwrap();
        tx.send(IoCommand::Images(vec![(key(1), frame(9))]))
            .unwrap();
        let (batch, next) = coalesce(vec![(key(1), frame(1)), (key(3), None)], &rx);
        assert_eq!(
            batch,
            vec![(key(1), frame(4)), (key(3), None), (key(2), frame(3))]
        );
        // What follows another command is written after it.
        assert!(matches!(next, Some(IoCommand::Brightness(50))));
        assert!(matches!(rx.try_recv(), Ok(IoCommand::Images(_))));
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
    fn scrolling_frames_of_an_old_epoch_are_dropped() {
        let mut mock = MockBackend::new();
        let id = mock.plug(&models::stream_deck_mk2(), "A1");
        let (signals, _received) = tokio::sync::mpsc::unbounded_channel();
        let io = DeviceIo::spawn(1, mock.open(&id).unwrap(), signals).unwrap();
        let key = ControlId::Key { index: 0 };
        let sink = io.animation_sink();
        let first = io.next_epoch();
        sink(first, vec![(key, Frame::filled(72, 72, [1, 1, 1]))]);
        assert!(mock.wait_for(&id, Duration::from_secs(10), |frames, _| {
            frames.get(&key) == Some(&Frame::filled(72, 72, [1, 1, 1]))
        }));
        // A new list: the still image comes first, a late frame of the old
        // list does not overwrite it.
        let second = io.next_epoch();
        io.set_images(vec![(key, Some(Frame::filled(72, 72, [2, 2, 2])))]);
        sink(first, vec![(key, Frame::filled(72, 72, [9, 9, 9]))]);
        sink(
            second,
            vec![(
                ControlId::Key { index: 1 },
                Frame::filled(72, 72, [3, 3, 3]),
            )],
        );
        assert!(mock.wait_for(&id, Duration::from_secs(10), |frames, _| {
            frames.contains_key(&ControlId::Key { index: 1 })
        }));
        assert_eq!(
            mock.frames(&id).get(&key),
            Some(&Frame::filled(72, 72, [2, 2, 2]))
        );
        assert!(io.close(Vec::new(), CLOSE_TIMEOUT));
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
