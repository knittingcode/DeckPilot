//! Update modes, check scheduling and when an update may be installed.

use serde::{Deserialize, Serialize};

use crate::channel::Channel;

/// Setting under which [`UpdatePreferences`] are stored (JSON).
pub const PREFERENCES_SETTING: &str = "update.preferences";

/// How updates are applied. The default only notifies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateMode {
    /// Tell the user; downloading and installing are manual.
    #[default]
    NotifyOnly,
    /// Download in the background, install after the user confirms.
    DownloadAndAsk,
    /// Download and install automatically once the deck is idle.
    InstallWhenIdle,
}

pub const MIN_INTERVAL_HOURS: u32 = 1;
pub const MAX_INTERVAL_HOURS: u32 = 24 * 30;
/// Minimum time without device input before an automatic installation.
pub const IDLE_SECONDS: u64 = 10 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdatePreferences {
    pub mode: UpdateMode,
    pub automatic_checks: bool,
    pub check_interval_hours: u32,
    /// Seconds since the Unix epoch of the last completed check.
    pub last_check: Option<u64>,
    /// Stable (the default) or prerelease; preferences stored before
    /// channels existed are stable.
    pub channel: Channel,
}

impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            mode: UpdateMode::NotifyOnly,
            automatic_checks: true,
            check_interval_hours: 24,
            last_check: None,
            channel: Channel::Stable,
        }
    }
}

impl UpdatePreferences {
    /// Clamps values that came from storage or the UI into supported ranges.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.check_interval_hours = self
            .check_interval_hours
            .clamp(MIN_INTERVAL_HOURS, MAX_INTERVAL_HOURS);
        self
    }

    /// Whether an automatic check should run at `now` (Unix seconds).
    pub fn is_check_due(&self, now: u64) -> bool {
        if !self.automatic_checks {
            return false;
        }
        let interval = u64::from(self.check_interval_hours) * 3600;
        self.last_check
            .is_none_or(|last| now.saturating_sub(last) >= interval || last > now)
    }
}

/// Facts needed to decide what to do with an available update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallContext {
    pub downloaded: bool,
    pub user_confirmed: bool,
    /// The agent is executing an action or workflow.
    pub agent_busy: bool,
    pub agent_last_input_at: Option<u64>,
    pub now: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallDecision {
    /// Show that an update is available.
    Notify,
    /// Download now.
    Download,
    /// Downloaded; wait for the user to confirm installation.
    AskUser,
    /// Downloaded; wait until the device has been idle long enough.
    WaitForIdle,
    InstallNow,
}

impl InstallContext {
    pub fn is_idle(&self) -> bool {
        !self.agent_busy
            && self
                .agent_last_input_at
                .is_none_or(|last| self.now.saturating_sub(last) >= IDLE_SECONDS)
    }
}

/// What to do next. An explicit user confirmation always installs; only
/// [`UpdateMode::InstallWhenIdle`] installs without one, and only when idle.
pub fn decide(mode: UpdateMode, context: &InstallContext) -> InstallDecision {
    if context.downloaded && context.user_confirmed && !context.agent_busy {
        return InstallDecision::InstallNow;
    }
    match (mode, context.downloaded) {
        (UpdateMode::NotifyOnly, false) => InstallDecision::Notify,
        (UpdateMode::DownloadAndAsk | UpdateMode::InstallWhenIdle, false) => {
            InstallDecision::Download
        }
        (UpdateMode::NotifyOnly | UpdateMode::DownloadAndAsk, true) => InstallDecision::AskUser,
        (UpdateMode::InstallWhenIdle, true) if context.is_idle() => InstallDecision::InstallNow,
        (UpdateMode::InstallWhenIdle, true) => InstallDecision::WaitForIdle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn context(downloaded: bool) -> InstallContext {
        InstallContext {
            downloaded,
            user_confirmed: false,
            agent_busy: false,
            agent_last_input_at: None,
            now: NOW,
        }
    }

    #[test]
    fn defaults_to_notify_only_with_daily_checks() {
        let prefs = UpdatePreferences::default();
        assert_eq!(prefs.mode, UpdateMode::NotifyOnly);
        assert!(prefs.is_check_due(NOW));
        let checked = UpdatePreferences {
            last_check: Some(NOW - 3600),
            ..prefs.clone()
        };
        assert!(!checked.is_check_due(NOW));
        assert!(checked.is_check_due(NOW + 23 * 3600));
        // A clock that went backwards does not block checks forever.
        assert!(
            UpdatePreferences {
                last_check: Some(NOW + 1000),
                ..prefs.clone()
            }
            .is_check_due(NOW)
        );
        let manual = UpdatePreferences {
            automatic_checks: false,
            ..prefs
        };
        assert!(!manual.is_check_due(NOW));
    }

    #[test]
    fn intervals_are_clamped() {
        let low = UpdatePreferences {
            check_interval_hours: 0,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(low.check_interval_hours, MIN_INTERVAL_HOURS);
        let high = UpdatePreferences {
            check_interval_hours: 99_999,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(high.check_interval_hours, MAX_INTERVAL_HOURS);
    }

    #[test]
    fn decisions_per_mode() {
        assert_eq!(
            decide(UpdateMode::NotifyOnly, &context(false)),
            InstallDecision::Notify
        );
        assert_eq!(
            decide(UpdateMode::NotifyOnly, &context(true)),
            InstallDecision::AskUser
        );
        assert_eq!(
            decide(UpdateMode::DownloadAndAsk, &context(false)),
            InstallDecision::Download
        );
        assert_eq!(
            decide(UpdateMode::DownloadAndAsk, &context(true)),
            InstallDecision::AskUser
        );
        assert_eq!(
            decide(UpdateMode::InstallWhenIdle, &context(false)),
            InstallDecision::Download
        );
        assert_eq!(
            decide(UpdateMode::InstallWhenIdle, &context(true)),
            InstallDecision::InstallNow
        );
    }

    #[test]
    fn automatic_installation_waits_for_idle_and_never_interrupts_work() {
        let recent = InstallContext {
            agent_last_input_at: Some(NOW - 60),
            ..context(true)
        };
        assert_eq!(
            decide(UpdateMode::InstallWhenIdle, &recent),
            InstallDecision::WaitForIdle
        );
        let busy = InstallContext {
            agent_busy: true,
            ..context(true)
        };
        assert_eq!(
            decide(UpdateMode::InstallWhenIdle, &busy),
            InstallDecision::WaitForIdle
        );
        let idle = InstallContext {
            agent_last_input_at: Some(NOW - IDLE_SECONDS),
            ..context(true)
        };
        assert_eq!(
            decide(UpdateMode::InstallWhenIdle, &idle),
            InstallDecision::InstallNow
        );

        // A confirmation installs in any mode, but not while work is running.
        let confirmed = InstallContext {
            user_confirmed: true,
            ..recent
        };
        assert_eq!(
            decide(UpdateMode::NotifyOnly, &confirmed),
            InstallDecision::InstallNow
        );
        let confirmed_busy = InstallContext {
            user_confirmed: true,
            ..busy
        };
        assert_eq!(
            decide(UpdateMode::NotifyOnly, &confirmed_busy),
            InstallDecision::AskUser
        );
    }

    #[test]
    fn preferences_tolerate_missing_fields() {
        let prefs: UpdatePreferences =
            serde_json::from_str(r#"{"mode":"downloadAndAsk"}"#).unwrap();
        assert_eq!(prefs.mode, UpdateMode::DownloadAndAsk);
        assert_eq!(prefs.check_interval_hours, 24);
        assert_eq!(prefs.channel, Channel::Stable);
        let pre: UpdatePreferences = serde_json::from_str(r#"{"channel":"prerelease"}"#).unwrap();
        assert_eq!(pre.channel, Channel::Prerelease);
        assert!(serde_json::from_str::<UpdatePreferences>(r#"{"channel":"nightly"}"#).is_err());
    }
}
