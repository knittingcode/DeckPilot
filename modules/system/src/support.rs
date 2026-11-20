//! Validation shared by the system actions. Paths inside resources and
//! URLs to open are checked by the core (`deckpilot_core::paths`), which
//! the plugin host shares; they are re-exported here unchanged.

use std::path::{Path, PathBuf};

use deckpilot_core::ActionResult;
pub use deckpilot_core::paths::{existing, existing_inside, join_inside, resolve, web_url};

use crate::codes;

/// Most arguments a command or application may be given.
pub const MAX_ARGUMENTS: usize = 64;
/// Longest single argument, in characters.
pub const MAX_ARGUMENT_CHARS: usize = 4096;

/// A failure, as the action's result.
pub type Failure = ActionResult;

pub fn fail(code: &str, message: impl Into<String>) -> Failure {
    ActionResult::error(code, message)
}

/// Arguments from a multi-line parameter: one per line, empty lines
/// skipped. Nothing is split on spaces or interpreted.
pub fn arguments(text: Option<&str>) -> Result<Vec<String>, Failure> {
    let lines: Vec<String> = text
        .unwrap_or_default()
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    if lines.len() > MAX_ARGUMENTS {
        return Err(fail(
            codes::INVALID_ARGUMENTS,
            format!("at most {MAX_ARGUMENTS} arguments are allowed"),
        ));
    }
    if let Some(bad) = lines
        .iter()
        .find(|a| a.chars().count() > MAX_ARGUMENT_CHARS || a.contains('\0'))
    {
        let preview: String = bad.chars().take(20).collect();
        return Err(fail(
            codes::INVALID_ARGUMENTS,
            format!("argument “{preview}…” is too long or contains a null character"),
        ));
    }
    Ok(lines)
}

/// The program a resource value names: an absolute path to an existing
/// file, or a bare name found on `PATH`. Relative paths are refused because
/// their meaning depends on the working directory.
pub fn program(value: &str) -> Result<PathBuf, Failure> {
    let value = value.trim();
    let path = Path::new(value);
    if value.contains('/') || value.contains('\\') {
        if !path.is_absolute() {
            return Err(fail(
                codes::INVALID_PATH,
                format!("the program “{value}” must be an absolute path or a name on PATH"),
            ));
        }
        return if path.is_file() {
            Ok(path.to_path_buf())
        } else {
            Err(fail(
                codes::NOT_FOUND,
                format!("the program {value} does not exist"),
            ))
        };
    }
    if value.is_empty() || value.contains(char::is_whitespace) {
        return Err(fail(
            codes::INVALID_PATH,
            format!("“{value}” is not a program name"),
        ));
    }
    find_on_path(value).ok_or_else(|| {
        fail(
            codes::NOT_FOUND,
            format!("the program “{value}” was not found on PATH"),
        )
    })
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| {
            extensions
                .iter()
                .map(move |ext| dir.join(format!("{name}{ext}")))
        })
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_lines_and_never_split() {
        assert_eq!(
            arguments(Some("--name\nJohn Smith\r\n\n; rm -rf /")).unwrap(),
            ["--name", "John Smith", "; rm -rf /"]
        );
        assert_eq!(arguments(None).unwrap(), Vec::<String>::new());
        let many = "a\n".repeat(MAX_ARGUMENTS + 1);
        assert!(arguments(Some(&many)).is_err());
        assert!(arguments(Some("a\0b")).is_err());
    }

    #[test]
    fn programs_must_be_absolute_or_on_path() {
        assert_eq!(
            program("bin/tool").unwrap_err().code.as_deref(),
            Some(codes::INVALID_PATH)
        );
        assert_eq!(
            program("deckpilot-no-such-program")
                .unwrap_err()
                .code
                .as_deref(),
            Some(codes::NOT_FOUND)
        );
        assert!(program("ls -la").is_err());
        let found = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(program(found).unwrap().is_absolute());
    }
}
