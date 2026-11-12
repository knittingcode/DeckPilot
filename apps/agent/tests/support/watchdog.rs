//! Keeps a hanging test from hanging the whole test run.
//!
//! `tokio::time::timeout` only helps while the test's future is polled and
//! the runtime's timers run. A call that blocks a thread (a stuck pipe or
//! socket call, a lock that is never released) is not interrupted by it,
//! and a test that panicked can still hang while its runtime shuts down
//! and waits for a blocked worker thread. A [`Watchdog`] runs on its own
//! thread: if the test is still running after its limit, it writes the
//! test and the step it was at straight to stderr (not through the test
//! harness's output capture, which would never be shown) and ends the test
//! binary with a failure, so CI reports the hang within minutes instead of
//! at the job timeout.

use std::io::Write as _;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Watchdog {
    done: Option<mpsc::Sender<()>>,
    step: Arc<Mutex<String>>,
}

thread_local! {
    /// The last panic on this thread, as the panic hook saw it.
    static LAST_PANIC: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Writes every panic (message and location) straight to stderr, so it is
/// seen even if the test then hangs and its captured output is never
/// printed, and remembers it for the watchdog's message.
fn install_panic_hook() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let text = format!("{info}");
            let _ = writeln!(
                std::io::stderr(),
                "\n[panic in {}] {text}",
                thread.name().unwrap_or("a thread")
            );
            let _ = LAST_PANIC.try_with(|last| *last.borrow_mut() = Some(text));
            previous(info);
        }));
    });
}

impl Watchdog {
    /// Watches the test `name` for at most `limit`.
    pub fn new(name: impl Into<String>, limit: Duration) -> Self {
        install_panic_hook();
        let name = name.into();
        let (done, finished) = mpsc::channel::<()>();
        let step = Arc::new(Mutex::new("starting".to_owned()));
        let at = step.clone();
        std::thread::spawn(move || {
            // The sender is dropped when the test ends.
            if finished.recv_timeout(limit) == Err(RecvTimeoutError::Timeout) {
                let at = at.lock().map(|s| s.clone()).unwrap_or_default();
                let _ = writeln!(
                    std::io::stderr(),
                    "\n{name} hung: still running after {limit:?}, at step `{at}`; \
                     ending the test binary"
                );
                std::process::exit(101);
            }
        });
        Self {
            done: Some(done),
            step,
        }
    }

    /// Records what the test does next, for the message on a hang.
    pub fn step(&self, what: &str) {
        if let Ok(mut step) = self.step.lock() {
            what.clone_into(&mut step);
        }
    }
}

thread_local! {
    /// Watchdogs of a test that panicked, disarmed when its thread ends.
    static PANICKED: std::cell::RefCell<Vec<mpsc::Sender<()>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // Stay armed until the test's thread ends: the runtime of a
            // test that panicked may still hang while it shuts down.
            if let Ok(mut step) = self.step.lock() {
                step.insert_str(0, "panicked, then did not finish, after: ");
                if let Ok(Some(panic)) = LAST_PANIC.try_with(|last| last.borrow_mut().take()) {
                    step.push_str("; the panic: ");
                    step.push_str(&panic);
                }
            }
            if let Some(done) = self.done.take() {
                let _ = PANICKED.try_with(|kept| kept.borrow_mut().push(done));
            }
        }
    }
}

/// Awaits `future` for at most `limit`, failing the test with `what`.
pub async fn within<T>(what: &str, limit: Duration, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(limit, future)
        .await
        .unwrap_or_else(|_| panic!("{what} did not finish within {limit:?}"))
}
