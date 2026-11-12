//! Validation shared by the system actions.

use std::path::{Component, Path, PathBuf};

use deckpilot_core::{ActionResult, ExecutionContext, ResourceKind, ResourceRef};

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

/// Resolves a resource parameter to its value on this computer.
pub fn resolve(
    context: &ExecutionContext<'_>,
    reference: Option<&ResourceRef>,
    kind: Option<ResourceKind>,
    what: &str,
) -> Result<String, Failure> {
    let reference = reference.ok_or_else(|| fail("invalid-parameters", format!("no {what}")))?;
    context
        .resolve(reference, kind)
        .map(|binding| binding.value.clone())
        .map_err(|error| fail(error.code(), error.to_string()))
}

/// `relative` joined to `base`, refusing absolute paths and `..`, so the
/// result stays inside the resource.
pub fn join_inside(base: &str, relative: Option<&str>) -> Result<PathBuf, Failure> {
    let base = PathBuf::from(base);
    let Some(relative) = relative.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(base);
    };
    let path = Path::new(relative);
    let inside = path
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    // `C:foo` or `\\server` parse as prefixes on Windows only; reject the
    // characters everywhere so a workflow means the same on every machine.
    if !inside || relative.contains(':') || relative.starts_with(['/', '\\']) {
        return Err(fail(
            codes::INVALID_PATH,
            format!("“{relative}” must be a relative path inside the resource"),
        ));
    }
    Ok(base.join(path))
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

/// An existing file or folder.
pub fn existing(path: PathBuf, directory: bool) -> Result<PathBuf, Failure> {
    let found = if directory {
        path.is_dir()
    } else {
        path.exists()
    };
    if found {
        Ok(path)
    } else {
        Err(fail(
            codes::NOT_FOUND,
            format!(
                "{} {} does not exist",
                if directory { "the folder" } else { "the file" },
                path.display()
            ),
        ))
    }
}

/// An existing file or folder at `relative` inside the resource folder
/// `base`, checked by name ([`join_inside`]) and by where it really is:
/// links are followed, and the target must still be inside `base`. Both
/// sides are canonicalized the same way, so `/var` and `/private/var` on
/// macOS and `\\?\` prefixes on Windows compare alike, and a resource
/// folder that is itself a link is fine. Without `relative`, the folder
/// itself is returned (it only has to exist).
///
/// The returned path is the joined one, as the user wrote it; it has been
/// checked to resolve inside the resource.
pub fn existing_inside(
    base: &str,
    relative: Option<&str>,
    directory: bool,
) -> Result<PathBuf, Failure> {
    let path = join_inside(base, relative)?;
    let path = existing(path, directory)?;
    if relative.map(str::trim).is_none_or(str::is_empty) {
        return Ok(path);
    }
    let unresolved = |what: &Path| {
        fail(
            codes::NOT_FOUND,
            format!("{} could not be resolved", what.display()),
        )
    };
    let root = Path::new(base)
        .canonicalize()
        .map_err(|_| unresolved(Path::new(base)))?;
    let target = path.canonicalize().map_err(|_| unresolved(&path))?;
    if target.starts_with(&root) {
        Ok(path)
    } else {
        Err(fail(
            codes::INVALID_PATH,
            format!(
                "“{}” leads outside the resource through a link",
                relative.unwrap_or_default().trim()
            ),
        ))
    }
}

/// A validated `http`, `https` or `mailto` URL.
pub fn web_url(text: &str) -> Result<String, Failure> {
    let text = text.trim();
    let invalid = |reason: &str| fail(codes::INVALID_URL, format!("“{text}” {reason}"));
    if text.len() > 2048 {
        return Err(invalid("is too long"));
    }
    let url = url::Url::parse(text).map_err(|_| invalid("is not a valid URL"))?;
    match url.scheme() {
        "http" | "https" if url.host().is_some() => Ok(url.to_string()),
        "mailto" => Ok(url.to_string()),
        _ => Err(invalid(
            "must be an http, https or mailto address; other schemes can start programs",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_stay_inside_their_resource() {
        assert_eq!(
            join_inside("/src/shop", Some("docs/README.md")).unwrap(),
            PathBuf::from("/src/shop/docs/README.md")
        );
        assert_eq!(
            join_inside("/src/shop", Some("  ")).unwrap(),
            PathBuf::from("/src/shop")
        );
        for bad in [
            "../etc",
            "/etc/passwd",
            "a/../../b",
            r"C:\Windows",
            r"\\server\x",
        ] {
            assert_eq!(
                join_inside("/src/shop", Some(bad))
                    .unwrap_err()
                    .code
                    .as_deref(),
                Some(codes::INVALID_PATH),
                "{bad}"
            );
        }
    }

    /// A link at `link` to `target`; false where links cannot be made
    /// (Windows without the privilege).
    fn link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        made.is_ok()
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One tree of links and its cases.
    fn links_must_resolve_inside_their_resource() {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-links-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base = dir.join("repo");
        let outside = dir.join("outside");
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(base.join("sub/inside.txt"), "in").unwrap();
        std::fs::write(outside.join("secret.txt"), "out").unwrap();
        // A sibling whose name starts like the resource's.
        std::fs::create_dir_all(dir.join("repo-other")).unwrap();
        std::fs::write(dir.join("repo-other/x.txt"), "x").unwrap();
        let made = [
            link(&outside.join("secret.txt"), &base.join("out-file")),
            link(&outside, &base.join("out-dir")),
            link(&dir.join("repo-other"), &base.join("sibling")),
            link(&base.join("sub/inside.txt"), &base.join("in-file")),
            link(&base.join("sub"), &base.join("in-dir")),
            link(&base.join("missing.txt"), &base.join("dangling")),
            link(&base, &dir.join("linked-repo")),
        ];
        if made.contains(&false) {
            eprintln!("links cannot be made here; skipped");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let root = base.display().to_string();
        let code = |base: &str, relative: &str, directory: bool| {
            existing_inside(base, Some(relative), directory)
                .err()
                .and_then(|f| f.code)
                .unwrap_or_default()
        };

        // Plain paths and links that stay inside.
        assert_eq!(
            existing_inside(&root, Some("sub/inside.txt"), false).unwrap(),
            base.join("sub/inside.txt")
        );
        assert_eq!(code(&root, "in-file", false), "");
        assert_eq!(code(&root, "in-dir", true), "");
        assert_eq!(code(&root, "in-dir/inside.txt", false), "");
        assert_eq!(code(&root, "in-dir/../in-file", false), codes::INVALID_PATH);

        // Links that lead outside, to a file, a folder or through one.
        for (relative, directory) in [
            ("out-file", false),
            ("out-dir", true),
            ("out-dir", false),
            ("out-dir/secret.txt", false),
            ("sibling/x.txt", false),
        ] {
            assert_eq!(
                code(&root, relative, directory),
                codes::INVALID_PATH,
                "{relative}"
            );
        }
        let refused = existing_inside(&root, Some("out-file"), false).unwrap_err();
        assert!(refused.message.contains("outside"), "{}", refused.message);
        assert!(
            !refused.message.contains("secret"),
            "the target is not shown: {}",
            refused.message
        );

        // A link to nothing is not found.
        assert_eq!(code(&root, "dangling", false), codes::NOT_FOUND);

        // The resource folder may itself be a link: compared resolved
        // against resolved.
        let linked = dir.join("linked-repo").display().to_string();
        assert_eq!(code(&linked, "sub/inside.txt", false), "");
        assert_eq!(code(&linked, "in-file", false), "");
        assert_eq!(code(&linked, "out-file", false), codes::INVALID_PATH);
        assert_eq!(
            existing_inside(&linked, None, true).unwrap(),
            dir.join("linked-repo")
        );
        // The temporary folder is itself under a link on macOS
        // (/var -> /private/var); both spellings work.
        let resolved = base.canonicalize().unwrap().display().to_string();
        assert_eq!(code(&resolved, "in-file", false), "");
        assert_eq!(code(&resolved, "out-file", false), codes::INVALID_PATH);

        let _ = std::fs::remove_dir_all(&dir);
    }

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

    #[test]
    fn only_web_and_mail_urls_open() {
        assert_eq!(
            web_url(" https://example.com/a?b=c ").unwrap(),
            "https://example.com/a?b=c"
        );
        assert!(web_url("mailto:team@example.com").is_ok());
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vscode://open",
            "http://",
            "example.com",
        ] {
            assert_eq!(
                web_url(bad).unwrap_err().code.as_deref(),
                Some(codes::INVALID_URL),
                "{bad}"
            );
        }
    }
}
