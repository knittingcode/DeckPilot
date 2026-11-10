//! Timestamped log files for session processes.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `YYYY-MM-DD HH:MM:SS.mmm` in UTC.
pub fn timestamp(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        since.subsec_millis()
    )
}

/// Removes terminal colour and cursor sequences so log files stay readable.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            // Parameters and intermediates, then one final byte (@ to ~).
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn log_path(logs: &Path, role: &str) -> PathBuf {
    logs.join(format!("{role}.log"))
}

/// Copies lines from `source` into `file`, prefixed with a timestamp and the
/// role, and optionally echoes them to the console.
pub fn pipe(
    source: impl Read + Send + 'static,
    role: &'static str,
    file: Arc<Mutex<File>>,
    echo: bool,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(source);
        for line in reader.split(b'\n') {
            let Ok(line) = line else { break };
            let text = strip_ansi(&String::from_utf8_lossy(&line));
            let entry = format!(
                "{} [{role}] {}",
                timestamp(SystemTime::now()),
                text.trim_end()
            );
            if let Ok(mut file) = file.lock() {
                let _ = writeln!(file, "{entry}");
            }
            if echo {
                println!("{entry}");
            }
        }
    })
}

pub fn open_append(path: &Path) -> std::io::Result<Arc<Mutex<File>>> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(Arc::new(Mutex::new(file)))
}

/// Prints the last `lines` lines of each log, then optionally keeps
/// printing new lines until interrupted.
pub fn show(paths: &[PathBuf], lines: usize, follow: bool, stop: &std::sync::atomic::AtomicBool) {
    let mut offsets = Vec::new();
    for path in paths {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let all: Vec<&str> = content.lines().collect();
        if !all.is_empty() {
            println!("==> {} <==", path.display());
            for line in &all[all.len().saturating_sub(lines)..] {
                println!("{line}");
            }
        }
        offsets.push(content.len() as u64);
    }
    while follow && !stop.load(std::sync::atomic::Ordering::SeqCst) {
        for (path, offset) in paths.iter().zip(offsets.iter_mut()) {
            let Ok(mut file) = File::open(path) else {
                continue;
            };
            let length = file.metadata().map_or(0, |m| m.len());
            if length < *offset {
                *offset = 0; // the file was truncated or recreated
            }
            if length > *offset && file.seek(SeekFrom::Start(*offset)).is_ok() {
                let mut appended = String::new();
                if file.read_to_string(&mut appended).is_ok() {
                    print!("{appended}");
                    *offset = length;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(
            strip_ansi("\u{1b}[1m\u{1b}[92mFinished\u{1b}[0m build"),
            "Finished build"
        );
        assert_eq!(strip_ansi("plain [text]"), "plain [text]");
    }

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01 00:00:00.000");
        let time = UNIX_EPOCH + Duration::from_millis(1_794_310_402_123);
        assert_eq!(timestamp(time), "2026-11-10 11:33:22.123");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29
        assert!(timestamp(leap).starts_with("2000-02-29"));
    }
}
