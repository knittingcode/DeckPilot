//! Start the agent after login, using per-user mechanisms only:
//! a `LaunchAgent` on macOS, an XDG autostart entry on Linux and the
//! `HKEY_CURRENT_USER` Run key on Windows. No system service is installed.

use auto_launch::{
    AutoLaunch, AutoLaunchBuilder, LinuxLaunchMode, MacOSLaunchMode, WindowsEnableMode,
};

const APP_NAME: &str = "DeckPilot Agent";

#[derive(Debug)]
pub struct Autostart {
    inner: AutoLaunch,
}

impl Autostart {
    /// Autostart entry for the currently running executable.
    pub fn for_current_exe() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_str().ok_or("executable path is not valid UTF-8")?;
        let inner = AutoLaunchBuilder::new()
            .set_app_name(APP_NAME)
            .set_app_path(exe)
            .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
            .set_linux_launch_mode(LinuxLaunchMode::XdgAutostart)
            .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { inner })
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
