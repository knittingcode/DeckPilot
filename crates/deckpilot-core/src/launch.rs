//! Opening applications, files, folders and web pages the way the
//! operating system would, without a shell.
//!
//! [`plan`] turns what to open into the program and arguments that open it
//! on each operating system (`open` on macOS, `xdg-open` or the
//! application itself on Linux, `explorer`/`rundll32` or the application
//! on Windows). It is pure, so every platform's plan is tested everywhere.
//! A [`Launcher`] runs a plan; modules receive one so tests can record
//! plans instead of opening windows.
//!
//! [`editor_plan`] opens a file or folder in an editor at a line, with the
//! arguments each known [`Editor`] takes, and [`url_plan`] a page in the
//! default or a given browser. The `workspace` module and the plugin
//! host's `resource.open` (plugin API 1.9) share them; the program is
//! always an application the user bound, never one a plugin names.
//!
//! Opened programs are independent of DeckPilot: they are not stopped when
//! a workflow is cancelled and are never ended with it.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::Os;

/// Something to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    /// A web address; validated by the caller.
    Url(String),
    /// A file or folder, opened with its default application.
    Path(PathBuf),
    /// An application: a path to it (an `.app` bundle, an executable), a
    /// macOS bundle identifier, a Linux `.desktop` id, or a program name.
    Application {
        application: String,
        arguments: Vec<String>,
    },
    /// A file or folder opened with a specific application.
    PathWith { path: PathBuf, application: String },
}

/// The program that opens a target and how to treat it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    /// The program is a launcher (`open`, `xdg-open`) that exits once the
    /// target is open, so its exit code says whether that worked. Otherwise
    /// the program is the application itself and keeps running.
    pub wait: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("{0}")]
    Unsupported(String),
    #[error("could not start `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("`{program}` could not open it (exit code {code})")]
    Failed { program: String, code: i32 },
}

impl LaunchError {
    /// Stable error code for [`crate::ActionResult::code`].
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "unsupported-target",
            Self::Spawn { source, .. } if source.kind() == io::ErrorKind::NotFound => {
                "program-not-found"
            }
            Self::Spawn { .. } => "spawn-failed",
            Self::Failed { .. } => "open-failed",
        }
    }
}

fn is_path(value: &str) -> bool {
    value.contains('/') || value.contains('\\')
}

/// How `target` is opened on `os`.
pub fn plan(os: Os, target: &OpenTarget) -> Result<LaunchPlan, LaunchError> {
    let path = |p: &PathBuf| p.to_string_lossy().into_owned();
    Ok(match (os, target) {
        (Os::MacOs, OpenTarget::Url(url)) => launcher("open", vec![url.clone()]),
        (Os::MacOs, OpenTarget::Path(p)) => launcher("open", vec![path(p)]),
        (
            Os::MacOs,
            OpenTarget::Application {
                application,
                arguments,
            },
        ) => {
            // A bundle identifier such as `com.apple.Safari` has dots but
            // no slash; a path or a name (`Safari`) is opened with `-a`.
            let by_id = !is_path(application)
                && application.contains('.')
                && !application.to_ascii_lowercase().ends_with(".app");
            let mut args = vec![
                if by_id { "-b" } else { "-a" }.to_owned(),
                application.clone(),
            ];
            if !arguments.is_empty() {
                args.push("--args".into());
                args.extend(arguments.iter().cloned());
            }
            launcher("open", args)
        }
        (
            Os::MacOs,
            OpenTarget::PathWith {
                path: p,
                application,
            },
        ) => launcher("open", vec!["-a".into(), application.clone(), path(p)]),
        (Os::Linux, OpenTarget::Url(url)) => launcher("xdg-open", vec![url.clone()]),
        (Os::Linux, OpenTarget::Path(p)) => launcher("xdg-open", vec![path(p)]),
        (
            Os::Linux,
            OpenTarget::Application {
                application,
                arguments,
            },
        ) => match application.strip_suffix(".desktop") {
            Some(id) if !is_path(application) => {
                let mut args = vec![id.to_owned()];
                args.extend(arguments.iter().cloned());
                launcher("gtk-launch", args)
            }
            _ => direct(application, arguments.clone()),
        },
        (
            Os::Linux | Os::Windows,
            OpenTarget::PathWith {
                path: p,
                application,
            },
        ) => {
            if application.ends_with(".desktop") {
                return Err(LaunchError::Unsupported(
                    "a .desktop application cannot be given a file to open; bind the program instead"
                        .into(),
                ));
            }
            direct(application, vec![path(p)])
        }
        // `rundll32 url.dll,FileProtocolHandler` opens a URL or file with
        // its associated program, like double-clicking it.
        (Os::Windows, OpenTarget::Url(url)) => LaunchPlan {
            program: "rundll32.exe".into(),
            args: vec!["url.dll,FileProtocolHandler".into(), url.clone()],
            wait: false,
        },
        (Os::Windows, OpenTarget::Path(p)) => {
            if p.is_dir() {
                LaunchPlan {
                    program: "explorer.exe".into(),
                    args: vec![path(p)],
                    // explorer.exe exits with 1 even when it worked.
                    wait: false,
                }
            } else {
                LaunchPlan {
                    program: "rundll32.exe".into(),
                    args: vec!["url.dll,FileProtocolHandler".into(), path(p)],
                    wait: false,
                }
            }
        }
        (
            Os::Windows,
            OpenTarget::Application {
                application,
                arguments,
            },
        ) => direct(application, arguments.clone()),
        (Os::Other, _) => {
            return Err(LaunchError::Unsupported(
                "opening is not supported on this operating system".into(),
            ));
        }
    })
}

fn launcher(program: &str, args: Vec<String>) -> LaunchPlan {
    LaunchPlan {
        program: program.into(),
        args,
        wait: true,
    }
}

fn direct(application: &str, args: Vec<String>) -> LaunchPlan {
    LaunchPlan {
        program: application.into(),
        args,
        wait: false,
    }
}

/// Highest line or column an editor is opened at.
pub const MAX_POSITION: i64 = 10_000_000;

/// Highest line or column a plugin's `resource.open` takes (the security
/// review of `docs/marketplace/module-migration.md` §7 X4: 1 to
/// 1,000,000), below the built-in's [`MAX_POSITION`]. The `workspace`
/// aliases cap `line` and `column` there (`alias::ParamRule::AtMost`).
pub const PLUGIN_MAX_POSITION: i64 = 1_000_000;

/// Error codes of the editor plans, as [`crate::ActionResult::code`].
pub mod codes {
    /// A line or column that is out of range, or a column without a line.
    pub const INVALID_POSITION: &str = "invalid-position";
}

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

/// The names of [`Editor`]s, as stored in parameters.
pub const EDITORS: [&str; 6] = ["vscode", "cursor", "jetbrains", "zed", "sublime", "other"];

impl Editor {
    /// The editor named `value`; [`Editor::Other`] for any other name.
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

    /// The editor's name, one of [`EDITORS`].
    pub fn name(self) -> &'static str {
        match self {
            Self::VsCode => "vscode",
            Self::Cursor => "cursor",
            Self::JetBrains => "jetbrains",
            Self::Zed => "zed",
            Self::Sublime => "sublime",
            Self::Other => "other",
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
    pub fn new(
        line: Option<i64>,
        column: Option<i64>,
    ) -> Result<Option<Self>, crate::ActionResult> {
        let invalid =
            |message: String| crate::ActionResult::error(codes::INVALID_POSITION, message);
        let check = |value: i64, what: &str| {
            u32::try_from(value)
                .ok()
                .filter(|v| (1..=MAX_POSITION).contains(&i64::from(*v)))
                .ok_or_else(|| invalid(format!("the {what} must be between 1 and {MAX_POSITION}")))
        };
        match (line, column) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(invalid("a column needs a line".into())),
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
        let plan = plan(
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
    let plan = plan(
        os,
        &OpenTarget::Application {
            application: application.to_owned(),
            arguments,
        },
    )?;
    Ok((plan, true))
}

/// How `url` (validated by the caller) is opened: in the default browser,
/// or in `browser` (an application resource value).
pub fn url_plan(os: Os, url: &str, browser: Option<&str>) -> Result<LaunchPlan, LaunchError> {
    match browser {
        None => plan(os, &OpenTarget::Url(url.to_owned())),
        // `open -a Browser <url>` on macOS, `<browser> <url>` elsewhere:
        // the page reaches a browser that is already running.
        Some(browser) => plan(
            os,
            &OpenTarget::PathWith {
                path: url.into(),
                application: browser.to_owned(),
            },
        ),
    }
}

/// Runs launch plans.
pub trait Launcher: Send + Sync + fmt::Debug {
    fn launch(&self, plan: &LaunchPlan) -> Result<(), LaunchError>;
}

/// How long a launcher such as `open` may take to report back. One that is
/// still running after this has opened something that keeps it busy, which
/// counts as success.
const LAUNCHER_WAIT: Duration = Duration::from_secs(10);

/// Starts programs for real.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLauncher;

impl Launcher for SystemLauncher {
    fn launch(&self, plan: &LaunchPlan) -> Result<(), LaunchError> {
        let mut command = Command::new(&plan.program);
        command
            .args(&plan.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            // Its own process group: the opened program is not part of the
            // agent's and is never signalled with it.
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child =
            crate::process::spawn(&mut command).map_err(|source| LaunchError::Spawn {
                program: plan.program.clone(),
                source,
            })?;
        if plan.wait {
            let until = Instant::now() + LAUNCHER_WAIT;
            while Instant::now() < until {
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => return Ok(()),
                    Ok(Some(status)) => {
                        return Err(LaunchError::Failed {
                            program: plan.program.clone(),
                            code: status.code().unwrap_or(-1),
                        });
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                    Err(_) => break,
                }
            }
        }
        // Reap the program whenever it exits, without waiting for it.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

/// Records plans instead of running them, for tests.
#[derive(Debug, Default)]
pub struct RecordingLauncher {
    plans: std::sync::Mutex<Vec<LaunchPlan>>,
    fail: bool,
}

impl RecordingLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    /// A launcher whose launches fail, like a missing `xdg-open`.
    pub fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }

    pub fn plans(&self) -> Vec<LaunchPlan> {
        self.plans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Launcher for RecordingLauncher {
    fn launch(&self, plan: &LaunchPlan) -> Result<(), LaunchError> {
        if self.fail {
            return Err(LaunchError::Spawn {
                program: plan.program.clone(),
                source: io::Error::from(io::ErrorKind::NotFound),
            });
        }
        self.plans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(plan.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(application: &str, arguments: &[&str]) -> OpenTarget {
        OpenTarget::Application {
            application: application.into(),
            arguments: arguments.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    fn args(plan: &LaunchPlan) -> Vec<&str> {
        plan.args.iter().map(String::as_str).collect()
    }

    #[test]
    fn macos_uses_open_for_everything() {
        let url = plan(Os::MacOs, &OpenTarget::Url("https://example.com".into())).unwrap();
        assert_eq!((url.program.as_str(), url.wait), ("open", true));
        assert_eq!(args(&url), ["https://example.com"]);

        let bundle = plan(Os::MacOs, &app("/Applications/Safari.app", &[])).unwrap();
        assert_eq!(args(&bundle), ["-a", "/Applications/Safari.app"]);
        let by_id = plan(Os::MacOs, &app("com.apple.Safari", &["--new"])).unwrap();
        assert_eq!(args(&by_id), ["-b", "com.apple.Safari", "--args", "--new"]);
        let by_name = plan(Os::MacOs, &app("Safari", &[])).unwrap();
        assert_eq!(args(&by_name), ["-a", "Safari"]);

        let with = plan(
            Os::MacOs,
            &OpenTarget::PathWith {
                path: "/tmp/a.txt".into(),
                application: "TextEdit".into(),
            },
        )
        .unwrap();
        assert_eq!(args(&with), ["-a", "TextEdit", "/tmp/a.txt"]);
    }

    #[test]
    fn linux_uses_xdg_open_or_the_application() {
        let path = plan(Os::Linux, &OpenTarget::Path("/home/u/src".into())).unwrap();
        assert_eq!((path.program.as_str(), path.wait), ("xdg-open", true));
        let desktop = plan(Os::Linux, &app("org.gnome.Terminal.desktop", &["x"])).unwrap();
        assert_eq!(desktop.program, "gtk-launch");
        assert_eq!(args(&desktop), ["org.gnome.Terminal", "x"]);
        let exe = plan(Os::Linux, &app("/usr/bin/code", &["--new-window"])).unwrap();
        assert_eq!((exe.program.as_str(), exe.wait), ("/usr/bin/code", false));
        assert_eq!(args(&exe), ["--new-window"]);
        assert!(matches!(
            plan(
                Os::Linux,
                &OpenTarget::PathWith {
                    path: "/a".into(),
                    application: "code.desktop".into()
                }
            ),
            Err(LaunchError::Unsupported(_))
        ));
    }

    #[test]
    fn windows_never_goes_through_cmd() {
        let url = plan(Os::Windows, &OpenTarget::Url("https://example.com".into())).unwrap();
        assert_eq!(url.program, "rundll32.exe");
        assert_eq!(
            args(&url),
            ["url.dll,FileProtocolHandler", "https://example.com"]
        );
        let exe = plan(Os::Windows, &app(r"C:\Tools\app.exe", &["a b"])).unwrap();
        assert_eq!(exe.program, r"C:\Tools\app.exe");
        assert_eq!(args(&exe), ["a b"]);
        for target in [url, exe] {
            assert!(!target.program.to_lowercase().contains("cmd"));
        }
    }

    #[test]
    fn editor_names_round_trip_and_plans_never_start_a_shell() {
        for name in EDITORS {
            assert_eq!(Editor::parse(name).name(), name);
        }
        assert_eq!(Editor::parse("notepad"), Editor::Other);
        let file = Path::new("/src/shop/main.rs");
        let at = Some(Position {
            line: 3,
            column: Some(4),
        });
        for os in [Os::MacOs, Os::Linux, Os::Windows] {
            for name in EDITORS {
                let (plan, _) =
                    editor_plan(os, Editor::parse(name), "/usr/bin/code", file, at).unwrap();
                assert!(
                    !plan.program.to_lowercase().contains("cmd"),
                    "{os:?} {name}"
                );
                assert!(!plan.program.ends_with("sh"), "{os:?} {name}");
            }
        }
        assert_eq!(
            Position::new(None, Some(1)).unwrap_err().code.as_deref(),
            Some(codes::INVALID_POSITION)
        );
    }

    #[test]
    fn recording_launchers_record_or_fail() {
        let launcher = RecordingLauncher::new();
        let plan = plan(Os::Linux, &OpenTarget::Url("https://example.com".into())).unwrap();
        launcher.launch(&plan).unwrap();
        assert_eq!(launcher.plans(), std::slice::from_ref(&plan));
        let error = RecordingLauncher::failing().launch(&plan).unwrap_err();
        assert_eq!(error.code(), "program-not-found");
    }
}
