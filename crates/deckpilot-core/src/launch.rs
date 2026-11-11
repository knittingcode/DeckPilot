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
//! Opened programs are independent of DeckPilot: they are not stopped when
//! a workflow is cancelled and are never ended with it.

use std::fmt;
use std::io;
use std::path::PathBuf;
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
        let mut child = command.spawn().map_err(|source| LaunchError::Spawn {
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
    fn recording_launchers_record_or_fail() {
        let launcher = RecordingLauncher::new();
        let plan = plan(Os::Linux, &OpenTarget::Url("https://example.com".into())).unwrap();
        launcher.launch(&plan).unwrap();
        assert_eq!(launcher.plans(), std::slice::from_ref(&plan));
        let error = RecordingLauncher::failing().launch(&plan).unwrap_err();
        assert_eq!(error.code(), "program-not-found");
    }
}
