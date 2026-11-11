//! How editors and browsers are opened: the arguments known editors take
//! for a line, and the launch plan on each operating system. Pure, so
//! every platform's plan is tested everywhere.

use std::path::Path;

use deckpilot_core::Os;
use deckpilot_core::launch::{self, LaunchError, LaunchPlan, OpenTarget};
use deckpilot_module_system::support::{self, Failure};

/// Highest line or column accepted.
pub const MAX_POSITION: i64 = 10_000_000;

/// Editors whose way of opening a file at a line is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    /// Visual Studio Code: `--goto file:line:column`.
    VsCode,
    /// Cursor: like Visual Studio Code.
    Cursor,
    /// IDEs by `JetBrains` (IDEA and the others): `--line N --column M file`.
    JetBrains,
    /// Zed: `file:line:column`.
    Zed,
    /// Sublime Text: `file:line:column`.
    Sublime,
    /// Anything else: the file is opened with the application, without a
    /// line.
    Other,
}

pub const EDITORS: [&str; 6] = ["vscode", "cursor", "jetbrains", "zed", "sublime", "other"];

impl Editor {
    pub fn parse(value: &str) -> Self {
        match value {
            "vscode" => Self::VsCode,
            "cursor" => Self::Cursor,
            "jetbrains" => Self::JetBrains,
            "zed" => Self::Zed,
            "sublime" => Self::Sublime,
            _ => Self::Other,
        }
    }

    /// The arguments that open `path` at `position`, or `None` when the
    /// editor has no known convention.
    pub fn arguments(self, path: &Path, position: Position) -> Option<Vec<String>> {
        let file = path.to_string_lossy();
        let located = match position.column {
            Some(column) => format!("{file}:{}:{column}", position.line),
            None => format!("{file}:{}", position.line),
        };
        match self {
            Self::VsCode | Self::Cursor => Some(vec!["--goto".into(), located]),
            Self::Zed | Self::Sublime => Some(vec![located]),
            Self::JetBrains => {
                let mut args = vec!["--line".into(), position.line.to_string()];
                if let Some(column) = position.column {
                    args.extend(["--column".into(), column.to_string()]);
                }
                args.push(file.into_owned());
                Some(args)
            }
            Self::Other => None,
        }
    }
}

/// A line and optional column, both counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: Option<u32>,
}

impl Position {
    /// Checks a line and column: within 1..=[`MAX_POSITION`], and a column
    /// only with a line.
    pub fn new(line: Option<i64>, column: Option<i64>) -> Result<Option<Self>, Failure> {
        let check = |value: i64, what: &str| {
            u32::try_from(value)
                .ok()
                .filter(|v| (1..=MAX_POSITION).contains(&i64::from(*v)))
                .ok_or_else(|| {
                    support::fail(
                        crate::codes::INVALID_POSITION,
                        format!("the {what} must be between 1 and {MAX_POSITION}"),
                    )
                })
        };
        match (line, column) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(support::fail(
                crate::codes::INVALID_POSITION,
                "a column needs a line",
            )),
            (Some(line), column) => Ok(Some(Self {
                line: check(line, "line")?,
                column: column.map(|c| check(c, "column")).transpose()?,
            })),
        }
    }
}

/// An application value that names a command-line program by path (such
/// as `/usr/local/bin/code`) rather than an application bundle, a bundle
/// identifier or a `.desktop` id.
fn is_command_line_tool(application: &str) -> bool {
    let extension = Path::new(application)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    application.contains(['/', '\\']) && !matches!(extension.as_deref(), Some("app" | "desktop"))
}

/// How `application` opens `path`, at `position` when given and the
/// editor knows how. Returns the plan and whether the position is passed.
///
/// A command-line tool is started directly with the arguments on every
/// system, so the line reaches an editor that is already running (on
/// macOS, `open -a … --args` only passes arguments to an editor that is
/// not running yet).
pub fn editor_plan(
    os: Os,
    editor: Editor,
    application: &str,
    path: &Path,
    position: Option<Position>,
) -> Result<(LaunchPlan, bool), LaunchError> {
    let arguments = position.and_then(|p| editor.arguments(path, p));
    let Some(arguments) = arguments else {
        let plan = launch::plan(
            os,
            &OpenTarget::PathWith {
                path: path.to_path_buf(),
                application: application.to_owned(),
            },
        )?;
        return Ok((plan, false));
    };
    if os == Os::Other {
        return Err(LaunchError::Unsupported(
            "opening is not supported on this operating system".into(),
        ));
    }
    if is_command_line_tool(application) {
        return Ok((
            LaunchPlan {
                program: application.to_owned(),
                args: arguments,
                wait: false,
            },
            true,
        ));
    }
    let plan = launch::plan(
        os,
        &OpenTarget::Application {
            application: application.to_owned(),
            arguments,
        },
    )?;
    Ok((plan, true))
}

/// A page to open in a browser: the System module's URL rules, and only
/// `http` or `https`.
pub fn browser_url(text: &str) -> Result<String, Failure> {
    let url = support::web_url(text)?;
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url)
    } else {
        Err(support::fail(
            deckpilot_module_system::codes::INVALID_URL,
            format!("“{}” must be an http or https address", text.trim()),
        ))
    }
}

/// How `url` is opened: in the default browser, or in `browser` (an
/// application resource value).
pub fn url_plan(os: Os, url: &str, browser: Option<&str>) -> Result<LaunchPlan, LaunchError> {
    match browser {
        None => launch::plan(os, &OpenTarget::Url(url.to_owned())),
        // `open -a Browser <url>` on macOS, `<browser> <url>` elsewhere:
        // the page reaches a browser that is already running.
        Some(browser) => launch::plan(
            os,
            &OpenTarget::PathWith {
                path: url.into(),
                application: browser.to_owned(),
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn at(line: u32, column: Option<u32>) -> Option<Position> {
        Position::new(Some(line.into()), column.map(Into::into)).unwrap()
    }

    #[test]
    fn known_editors_get_their_line_arguments() {
        let file = PathBuf::from("/src/shop/main.rs");
        let args = |editor: Editor, column| editor.arguments(&file, Position { line: 12, column });
        assert_eq!(
            args(Editor::VsCode, Some(3)).unwrap(),
            ["--goto", "/src/shop/main.rs:12:3"]
        );
        assert_eq!(
            args(Editor::Cursor, None).unwrap(),
            ["--goto", "/src/shop/main.rs:12"]
        );
        assert_eq!(
            args(Editor::JetBrains, Some(3)).unwrap(),
            ["--line", "12", "--column", "3", "/src/shop/main.rs"]
        );
        assert_eq!(
            args(Editor::JetBrains, None).unwrap(),
            ["--line", "12", "/src/shop/main.rs"]
        );
        assert_eq!(args(Editor::Zed, None).unwrap(), ["/src/shop/main.rs:12"]);
        assert_eq!(
            args(Editor::Sublime, Some(1)).unwrap(),
            ["/src/shop/main.rs:12:1"]
        );
        assert_eq!(args(Editor::Other, None), None);
        assert_eq!(Editor::parse("zed"), Editor::Zed);
        assert_eq!(Editor::parse("notepad"), Editor::Other);
    }

    #[test]
    fn positions_are_checked() {
        assert_eq!(Position::new(None, None).unwrap(), None);
        assert_eq!(Position::new(Some(5), Some(2)).unwrap(), at(5, Some(2)));
        for (line, column) in [
            (Some(0), None),
            (Some(-1), None),
            (Some(MAX_POSITION + 1), None),
            (Some(1), Some(0)),
            (None, Some(3)),
            (Some(i64::MAX), None),
        ] {
            assert_eq!(
                Position::new(line, column).unwrap_err().code.as_deref(),
                Some(crate::codes::INVALID_POSITION),
                "{line:?} {column:?}"
            );
        }
    }

    #[test]
    fn editor_plans_on_every_system() {
        let file = PathBuf::from("/src/shop/main.rs");
        // No line: open with the application.
        let (mac, located) =
            editor_plan(Os::MacOs, Editor::VsCode, "Visual Studio Code", &file, None).unwrap();
        assert!(!located);
        assert_eq!(mac.program, "open");
        assert_eq!(mac.args, ["-a", "Visual Studio Code", "/src/shop/main.rs"]);
        let (linux, _) = editor_plan(Os::Linux, Editor::Zed, "/usr/bin/zed", &file, None).unwrap();
        assert_eq!(linux.program, "/usr/bin/zed");
        assert_eq!(linux.args, ["/src/shop/main.rs"]);

        // A line through an application bundle: `open -a … --args`.
        let (mac, located) = editor_plan(
            Os::MacOs,
            Editor::VsCode,
            "Visual Studio Code",
            &file,
            at(7, None),
        )
        .unwrap();
        assert!(located);
        assert_eq!(
            mac.args,
            [
                "-a",
                "Visual Studio Code",
                "--args",
                "--goto",
                "/src/shop/main.rs:7"
            ]
        );
        // A command-line tool is started directly, on macOS too.
        for os in [Os::MacOs, Os::Linux, Os::Windows] {
            let (plan, located) = editor_plan(
                os,
                Editor::JetBrains,
                "/usr/local/bin/idea",
                &file,
                at(7, Some(2)),
            )
            .unwrap();
            assert!(located, "{os:?}");
            assert_eq!(plan.program, "/usr/local/bin/idea");
            assert_eq!(
                plan.args,
                ["--line", "7", "--column", "2", "/src/shop/main.rs"]
            );
            assert!(!plan.wait);
        }
        let (windows, _) = editor_plan(
            Os::Windows,
            Editor::Sublime,
            r"C:\Program Files\Sublime Text\subl.exe",
            &file,
            at(3, None),
        )
        .unwrap();
        assert_eq!(windows.program, r"C:\Program Files\Sublime Text\subl.exe");
        assert_eq!(windows.args, ["/src/shop/main.rs:3"]);
        // A Linux .desktop application gets its arguments through gtk-launch.
        let (desktop, _) = editor_plan(
            Os::Linux,
            Editor::VsCode,
            "code.desktop",
            &file,
            at(3, None),
        )
        .unwrap();
        assert_eq!(desktop.program, "gtk-launch");
        assert_eq!(desktop.args, ["code", "--goto", "/src/shop/main.rs:3"]);

        // Unknown editors open the file without a line.
        let (plan, located) =
            editor_plan(Os::MacOs, Editor::Other, "TextEdit", &file, at(3, None)).unwrap();
        assert!(!located);
        assert_eq!(plan.args, ["-a", "TextEdit", "/src/shop/main.rs"]);
        assert!(editor_plan(Os::Other, Editor::Zed, "zed", &file, at(1, None)).is_err());
    }

    #[test]
    fn only_web_pages_open_in_a_browser() {
        assert_eq!(
            browser_url(" https://example.com/a ").unwrap(),
            "https://example.com/a"
        );
        assert!(browser_url("http://localhost:3000/").is_ok());
        for bad in [
            "mailto:team@example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vscode://open",
            "example.com",
        ] {
            assert_eq!(
                browser_url(bad).unwrap_err().code.as_deref(),
                Some(deckpilot_module_system::codes::INVALID_URL),
                "{bad}"
            );
        }
    }

    #[test]
    fn url_plans_on_every_system() {
        let url = "https://example.com/";
        assert_eq!(
            url_plan(Os::MacOs, url, None).unwrap().args,
            ["https://example.com/"]
        );
        assert_eq!(url_plan(Os::Linux, url, None).unwrap().program, "xdg-open");
        assert_eq!(
            url_plan(Os::Windows, url, None).unwrap().args,
            ["url.dll,FileProtocolHandler", "https://example.com/"]
        );
        let firefox = url_plan(Os::MacOs, url, Some("Firefox")).unwrap();
        assert_eq!(firefox.args, ["-a", "Firefox", "https://example.com/"]);
        let linux = url_plan(Os::Linux, url, Some("/usr/bin/firefox")).unwrap();
        assert_eq!(linux.program, "/usr/bin/firefox");
        assert_eq!(linux.args, ["https://example.com/"]);
        let windows = url_plan(Os::Windows, url, Some(r"C:\Firefox\firefox.exe")).unwrap();
        assert_eq!(windows.program, r"C:\Firefox\firefox.exe");
        assert_eq!(windows.args, ["https://example.com/"]);
    }
}
