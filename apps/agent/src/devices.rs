//! One I/O thread per opened device.
//!
//! HID handles are not shareable between threads, so a dedicated thread owns
//! each handle. It blocks in `read_input` with a short timeout (no busy
//! polling) and, between reads, applies queued image and brightness updates.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use deckpilot_device::{ControlId, ControlInput, DeviceError, DeviceHandle, DeviceId, Frame};
use tokio::sync::mpsc::UnboundedSender;

/// Upper bound for how long queued image updates wait while the thread is
/// blocked reading input. Input itself is delivered immediately.
const READ_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub enum DeviceSignal {
    /// `session` identifies the connection, so signals from a previous
    /// connection of the same device can be told apart.
    Input {
        session: u64,
        device: DeviceId,
        control: ControlId,
        input: ControlInput,
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
    commands: mpsc::Sender<IoCommand>,
    thread: Option<JoinHandle<()>>,
}

impl DeviceIo {
    pub fn spawn(
        session: u64,
        mut handle: Box<dyn DeviceHandle>,
        signals: UnboundedSender<DeviceSignal>,
    ) -> std::io::Result<Self> {
        let (commands, inbox) = mpsc::channel();
        let device = handle.id().clone();
        let thread = std::thread::Builder::new()
            .name(format!("device-{}", device.serial))
            .spawn(move || {
                let error = io_loop(session, handle.as_mut(), &inbox, &signals);
                let _ = signals.send(DeviceSignal::Closed {
                    session,
                    device,
                    error,
                });
            })?;
        Ok(Self {
            commands,
            thread: Some(thread),
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

    /// Blanks `controls`, stops the thread and waits for it to finish.
    pub fn close(mut self, blank: Vec<ControlId>) {
        let _ = self.commands.send(IoCommand::Close { blank });
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
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
                for (control, input) in events {
                    let device = handle.id().clone();
                    let signal = DeviceSignal::Input {
                        session,
                        device,
                        control,
                        input,
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
