//! Start the agent after login, using per-user mechanisms only:
//! a `LaunchAgent` on macOS, an XDG autostart entry on Linux and the
//! `HKEY_CURRENT_USER` Run key on Windows. No system service is installed.

use std::path::{Path, PathBuf};

use auto_launch::{
    AutoLaunch, AutoLaunchBuilder, LinuxLaunchMode, MacOSLaunchMode, WindowsEnableMode,
};
use deckpilot_ipc::paths::{RUN_AGENT_ARG, appimage};

const APP_NAME: &str = "DeckPilot Agent";

#[derive(Debug)]
pub struct Autostart {
    inner: AutoLaunch,
}

/// What the autostart entry runs: the agent's own executable, or, inside an
/// AppImage, the AppImage file asked to run the agent. The executable's
/// path inside a mounted AppImage changes on every run and is gone after
/// it, so an entry pointing there would fail at the next login.
pub(crate) fn command(exe: &Path, appimage: Option<PathBuf>) -> (PathBuf, Vec<String>) {
    match appimage {
        Some(file) => (file, vec![RUN_AGENT_ARG.to_owned()]),
        None => (exe.to_path_buf(), Vec::new()),
    }
}

/// The path as the entry records it. XDG `Exec` lines split at spaces, so a
/// path containing one is quoted there.
fn entry_path(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or("executable path is not valid UTF-8")?;
    Ok(if cfg!(target_os = "linux") && path.contains(' ') {
        format!("\"{path}\"")
    } else {
        path.to_owned()
    })
}

impl Autostart {
    /// Autostart entry for the currently running agent.
    pub fn for_current_exe() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let (program, args) = command(&exe, appimage());
        let inner = AutoLaunchBuilder::new()
            .set_app_name(APP_NAME)
            .set_app_path(&entry_path(&program)?)
            .set_args(&args)
            .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
            .set_linux_launch_mode(LinuxLaunchMode::XdgAutostart)
            .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { inner })
    }

    /// Inside an AppImage: rewrites an enabled entry, so that one recorded
    /// by an earlier version with a path inside the AppImage's temporary
    /// mount (which no longer exists) starts the agent again.
    pub fn repair(&self) -> Result<(), String> {
        if appimage().is_some() && self.is_enabled()? {
            self.set_enabled(true)?;
        }
        Ok(())
    }

    pub fn is_enabled(&self) -> Result<bool, String> {
        self.inner.is_enabled().map_err(|e| e.to_string())
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let result = if enabled {
            self.inner.enable()
        } else {
            self.inner.disable()
        };
        result.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_agent_starts_itself() {
        // .deb and .rpm install to /usr/bin, a stable path.
        let (program, args) = command(Path::new("/usr/bin/deckpilot-agent"), None);
        assert_eq!(program, Path::new("/usr/bin/deckpilot-agent"));
        assert_eq!(args, Vec::<String>::new());
    }

    #[test]
    fn an_agent_in_an_appimage_is_started_through_the_appimage_file() {
        let (program, args) = command(
            Path::new("/tmp/.mount_DeckPiAbC123/usr/bin/deckpilot-agent"),
            Some(PathBuf::from("/home/u/Apps/DeckPilot.AppImage")),
        );
        assert_eq!(program, Path::new("/home/u/Apps/DeckPilot.AppImage"));
        assert_eq!(args, [RUN_AGENT_ARG]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn paths_with_spaces_are_quoted_in_xdg_entries() {
        assert_eq!(
            entry_path(Path::new("/home/u/My Apps/DeckPilot.AppImage")).unwrap(),
            "\"/home/u/My Apps/DeckPilot.AppImage\""
        );
        assert_eq!(
            entry_path(Path::new("/usr/bin/deckpilot-agent")).unwrap(),
            "/usr/bin/deckpilot-agent"
        );
    }
}
