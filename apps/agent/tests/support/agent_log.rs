//! The agents' log, kept in memory so that a failing test can show it.
//!
//! Test agents run in-process without the binary's file logging, so a
//! condition that is never reached (an unlock that failed, a step that
//! took too long) would otherwise fail with no hint why. Every event at
//! `debug` and above is kept with the time since the binary started and
//! the thread it came from; [`since`] returns the lines written after a
//! test started. Tests run in parallel, so lines of other tests may be
//! interleaved: the thread names and times tell them apart.
//!
//! [`super::watchdog::Watchdog::new`] calls [`test_started`], so every
//! watched test can show its log with [`test_log`].

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::Write;
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

/// Lines kept at most (the oldest are dropped first).
const KEPT: usize = 4000;
/// Lines returned at most by [`since`] (the latest ones).
const SHOWN: usize = 400;

struct Line {
    at: Duration,
    text: String,
}

fn start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn lines() -> &'static Mutex<VecDeque<Line>> {
    static LINES: OnceLock<Mutex<VecDeque<Line>>> = OnceLock::new();
    LINES.get_or_init(|| Mutex::new(VecDeque::with_capacity(KEPT)))
}

/// Collects one formatted event and stores it as a line.
struct Capture(Vec<u8>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0).trim_end().to_owned();
        if text.is_empty() {
            return;
        }
        let at = start().elapsed();
        if let Ok(mut lines) = lines().lock() {
            if lines.len() == KEPT {
                lines.pop_front();
            }
            lines.push_back(Line { at, text });
        }
    }
}

/// Installs the capture once per test binary (as the global subscriber).
pub fn install() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let _ = start();
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .without_time()
            .with_thread_names(true)
            .with_writer(|| Capture(Vec::new()))
            .try_init();
    });
}

thread_local! {
    /// When the test on this thread started.
    static TEST_START: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
}

/// Installs the capture and records that the test on this thread starts.
pub fn test_started() {
    install();
    let now = start().elapsed();
    TEST_START.with(|start| start.set(Some(now)));
}

/// The log since the test on this thread started.
pub fn test_log() -> String {
    since(TEST_START.with(std::cell::Cell::get).unwrap_or_default())
}

/// The lines logged since `from` (at most the latest [`SHOWN`]), one per
/// line, each with its time in seconds since the binary started.
pub fn since(from: Duration) -> String {
    let Ok(lines) = lines().lock() else {
        return "(the log is not available)".into();
    };
    let selected: Vec<&Line> = lines.iter().filter(|line| line.at >= from).collect();
    let skipped = selected.len().saturating_sub(SHOWN);
    let mut out = String::new();
    if skipped > 0 {
        let _ = writeln!(out, "({skipped} earlier lines left out)");
    }
    for line in selected.into_iter().skip(skipped) {
        let _ = writeln!(out, "{:>9.3}s {}", line.at.as_secs_f64(), line.text);
    }
    if out.is_empty() {
        out.push_str("(nothing was logged)\n");
    }
    out
}
