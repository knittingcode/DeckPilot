//! The agent's log, for the logs window.
//!
//! Logs never contain configuration values, keys or passwords (see
//! `docs/security.md`). Reading is bounded so that a large log cannot
//! exhaust memory.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// At most this much of the end of the log is read.
const MAX_BYTES: u64 = 512 * 1024;
const MAX_LINES: usize = 2000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogView {
    directory: String,
    /// The newest log file, if there is one.
    file: Option<String>,
    lines: Vec<String>,
    /// Earlier lines were left out.
    truncated: bool,
}

/// The newest `agent.*.log` in `dir` (daily files sort by date).
fn newest_log(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("agent")
                        && Path::new(name)
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
                })
        })
        .max()
}

/// The last `lines` lines (at most 2000) of the newest log.
pub fn tail(dir: &Path, lines: usize) -> std::io::Result<LogView> {
    let mut view = LogView {
        directory: dir.display().to_string(),
        file: None,
        lines: Vec::new(),
        truncated: false,
    };
    let Some(path) = newest_log(dir) else {
        return Ok(view);
    };
    let mut file = std::fs::File::open(&path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(MAX_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut all: Vec<&str> = text.lines().collect();
    if start > 0 && !all.is_empty() {
        // The first line was probably cut.
        all.remove(0);
    }
    let wanted = lines.clamp(1, MAX_LINES);
    view.truncated = start > 0 || all.len() > wanted;
    let skip = all.len().saturating_sub(wanted);
    view.lines = all[skip..].iter().map(|line| (*line).to_owned()).collect();
    view.file = Some(path.display().to_string());
    Ok(view)
}

#[tauri::command]
pub fn read_agent_log(state: State<'_, AppState>, lines: usize) -> CommandResult<LogView> {
    tail(&state.paths.log_dir, lines).map_err(|e| CommandError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_end_of_the_newest_log() {
        let dir = std::env::temp_dir().join(format!("deckpilot-logs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(tail(&dir, 10).unwrap().file, None);
        std::fs::write(dir.join("agent.2026-01-01.log"), "old\n").unwrap();
        let text = (0..50).fold(String::new(), |mut text, i| {
            use std::fmt::Write as _;
            let _ = writeln!(text, "line {i}");
            text
        });
        std::fs::write(dir.join("agent.2026-01-02.log"), text).unwrap();
        std::fs::write(dir.join("other.txt"), "ignored").unwrap();
        let view = tail(&dir, 3).unwrap();
        assert!(view.file.unwrap().ends_with("agent.2026-01-02.log"));
        assert_eq!(view.lines, ["line 47", "line 48", "line 49"]);
        assert!(view.truncated);
        // Large logs are read only from their end.
        let big = "x".repeat(1024) + "\n";
        std::fs::write(dir.join("agent.2026-01-03.log"), big.repeat(2048)).unwrap();
        let view = tail(&dir, 5000).unwrap();
        assert!(view.lines.len() <= MAX_LINES);
        assert!(view.truncated);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
