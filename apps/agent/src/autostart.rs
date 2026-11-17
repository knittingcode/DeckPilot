//! Start the agent after login, using per-user mechanisms only:
//! a `LaunchAgent` on macOS, an XDG autostart entry on Linux and the
//! `HKEY_CURRENT_USER` Run key on Windows. No system service is installed.
//!
//! The mechanism follows the build's distribution channel
//! (docs/distribution/README.md, section 8, and the capability matrix):
//! GitHub, APT, Homebrew and `WinGet` builds are ordinary installations
//! and write these entries. Store builds must not: the Mac App Store's
//! sandbox does not allow a `LaunchAgent` file (its builds register a login
//! item with `SMAppService`), an MSIX package declares a
//! `windows.startupTask` instead of writing the Run key, and Flathub apps
//! ask the Background portal instead of writing an XDG entry. Those
//! mechanisms are not implemented yet, so store builds report autostart as
//! unsupported and never write anything ([`Strategy::Unsupported`]).

use std::path::{Path, PathBuf};

use auto_launch::{
    AutoLaunch, AutoLaunchBuilder, LinuxLaunchMode, MacOSLaunchMode, WindowsEnableMode,
};
use deckpilot_distribution::{Distribution, DistributionChannel};
use deckpilot_ipc::paths::{RUN_AGENT_ARG, appimage};

const APP_NAME: &str = "DeckPilot Agent";

/// How a build of a channel starts the agent at login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// A `LaunchAgent`, XDG autostart entry or Run key (`auto_launch`).
    LoginEntry,
    /// Not available in this build; the reason says why and what instead.
    Unsupported(&'static str),
}

impl Strategy {
    /// The strategy of `channel`.
    pub const fn for_channel(channel: DistributionChannel) -> Self {
        match channel {
            DistributionChannel::Github
            | DistributionChannel::Apt
            | DistributionChannel::Homebrew
            | DistributionChannel::Winget => Self::LoginEntry,
            DistributionChannel::MacAppStore => Self::Unsupported(
                "starting the agent at login is not available in the Mac App Store build yet \
                 (it needs a login item registered by the app); open DeckPilot to start it",
            ),
            DistributionChannel::MicrosoftStore => Self::Unsupported(
                "starting the agent at login is not available in the Microsoft Store build yet \
                 (it needs the package's startup task); open DeckPilot to start it",
            ),
            DistributionChannel::Flathub => Self::Unsupported(
                "starting the agent at login is not available in the Flathub build yet \
                 (it needs the Background portal); open DeckPilot to start it",
            ),
        }
    }
}

#[derive(Debug)]
pub struct Autostart {
    inner: Inner,
}

#[derive(Debug)]
enum Inner {
    Entry(AutoLaunch),
    Unsupported(&'static str),
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
    /// Autostart of the currently running agent, by the strategy of the
    /// build's channel ([`Strategy::for_channel`]).
    pub fn for_current_exe(distribution: &Distribution) -> Result<Self, String> {
        match Strategy::for_channel(distribution.distribution_channel()) {
            Strategy::LoginEntry => Self::login_entry(),
            Strategy::Unsupported(reason) => Ok(Self {
                inner: Inner::Unsupported(reason),
            }),
        }
    }

    /// A `LaunchAgent`, XDG entry or Run key for the running executable.
    fn login_entry() -> Result<Self, String> {
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
        Ok(Self {
            inner: Inner::Entry(inner),
        })
    }

    /// Whether this build can start the agent at login at all.
    pub fn is_supported(&self) -> bool {
        matches!(self.inner, Inner::Entry(_))
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

    /// Never enabled where unsupported.
    pub fn is_enabled(&self) -> Result<bool, String> {
        match &self.inner {
            Inner::Entry(entry) => entry.is_enabled().map_err(|e| e.to_string()),
            Inner::Unsupported(_) => Ok(false),
        }
    }

    /// Where unsupported, turning it off succeeds (there is nothing to
    /// remove) and turning it on fails with the reason; neither writes
    /// anything.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        match &self.inner {
            Inner::Entry(entry) => {
                let result = if enabled {
                    entry.enable()
                } else {
                    entry.disable()
                };
                result.map_err(|e| e.to_string())
            }
            Inner::Unsupported(_) if !enabled => Ok(()),
            Inner::Unsupported(reason) => Err((*reason).to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_distribution::{Architecture, Platform};

    #[test]
    fn ordinary_installations_write_login_entries_and_store_builds_do_not() {
        use DistributionChannel as C;
        for channel in [C::Github, C::Apt, C::Homebrew, C::Winget] {
            assert_eq!(
                Strategy::for_channel(channel),
                Strategy::LoginEntry,
                "{channel}"
            );
        }
        for channel in [C::MacAppStore, C::MicrosoftStore, C::Flathub] {
            assert!(
                matches!(Strategy::for_channel(channel), Strategy::Unsupported(_)),
                "{channel}"
            );
        }
        // Every channel has a strategy.
        assert_eq!(C::ALL.len(), 7);
    }

    #[test]
    fn a_store_build_reports_autostart_as_unsupported_and_writes_nothing() {
        let store = Distribution::new(
            DistributionChannel::MacAppStore,
            None,
            env!("CARGO_PKG_VERSION"),
            Platform::Macos,
            Architecture::Aarch64,
        )
        .unwrap();
        let autostart = Autostart::for_current_exe(&store).unwrap();
        assert!(!autostart.is_supported());
        assert_eq!(autostart.is_enabled(), Ok(false));
        assert_eq!(autostart.repair(), Ok(()));
        assert_eq!(autostart.set_enabled(false), Ok(()));
        let error = autostart.set_enabled(true).unwrap_err();
        assert!(error.contains("Mac App Store"), "{error}");
        assert_eq!(autostart.is_enabled(), Ok(false));
    }

    #[test]
    fn an_installed_agent_starts_itself() {
        // .deb and .rpm install to /usr/bin, a stable path.
        let (program, args) = command(Path::new("/usr/bin/deckpilot-agent"), None);
        assert_eq!(program, Path::new("/usr/bin/deckpilot-agent"));
        assert_eq!(args, Vec::<String>::new());
    }

    #[test]
    fn the_login_item_starts_the_agent_with_plugins_on_in_a_plugins_build() {
        // Neither entry passes a plugin switch (the AppImage passes on
        // nothing after RUN_AGENT_ARG), so the build's default applies:
        // plugins on with the `plugins` feature, off without it.
        for (exe, appimage) in [
            ("/usr/bin/deckpilot-agent", None),
            (
                "/tmp/.mount_DeckPiAbC123/usr/bin/deckpilot-agent",
                Some(PathBuf::from("/home/u/Apps/DeckPilot.AppImage")),
            ),
        ] {
            let (_, args) = command(Path::new(exe), appimage);
            let mut settings = crate::plugins::PluginSettings {
                enabled: crate::plugins::ENABLED_BY_DEFAULT,
                ..crate::plugins::PluginSettings::default()
            };
            for arg in args.iter().filter(|a| *a != RUN_AGENT_ARG) {
                assert!(!settings.apply_switch(arg), "{exe}: {args:?}");
            }
            assert_eq!(settings.enabled, cfg!(feature = "plugins"), "{exe}");
        }
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
