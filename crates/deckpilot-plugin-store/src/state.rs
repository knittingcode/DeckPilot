//! The lifecycle state machine of an installed plugin.
//!
//! ```text
//!              install                enable
//!  (none) ──▶ Installing ──commit──▶ Disabled ◀──────▶ Enabled
//!               │ abort                │  ▲   disable    │
//!               ▼                      │  │              │
//!             (none)                   ▼  │              ▼
//!                       Updating / RollingBack (transient, back to a
//!                       stable state)        Broken  (integrity failure)
//!                       Uninstalling ──▶ (none)  Quarantined (revoked)
//! ```
//!
//! Stable states: `Disabled`, `Enabled`, `Broken`, `Quarantined`. The
//! transient states exist only while a journal entry is open
//! (`Installing`, `Updating`, `Uninstalling`) or inside one transaction
//! (`RollingBack`); recovery never leaves a plugin in one of them.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    /// Files are being written; the plugin is not usable.
    Installing,
    /// Installed and not running. The state after install.
    Disabled,
    /// Installed and allowed to run.
    Enabled,
    /// A new version is being unpacked; the active version is unchanged.
    Updating,
    /// The previous version is being made active.
    RollingBack,
    /// Files and records are being removed.
    Uninstalling,
    /// The active version's files do not match their integrity record.
    Broken,
    /// The registry revoked the active version (`disable`).
    Quarantined,
}

impl PluginState {
    pub const ALL: [Self; 8] = [
        Self::Installing,
        Self::Disabled,
        Self::Enabled,
        Self::Updating,
        Self::RollingBack,
        Self::Uninstalling,
        Self::Broken,
        Self::Quarantined,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installing => "installing",
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Updating => "updating",
            Self::RollingBack => "rolling_back",
            Self::Uninstalling => "uninstalling",
            Self::Broken => "broken",
            Self::Quarantined => "quarantined",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }

    /// `Disabled`, `Enabled`, `Broken` or `Quarantined`.
    pub fn is_stable(self) -> bool {
        matches!(
            self,
            Self::Disabled | Self::Enabled | Self::Broken | Self::Quarantined
        )
    }

    /// Whether the plugin may run in this state.
    pub fn is_runnable(self) -> bool {
        self == Self::Enabled
    }

    /// Whether `self → to` is an allowed transition. Every state change of
    /// the store goes through this table.
    pub fn can_transition_to(self, to: Self) -> bool {
        use PluginState::{
            Broken, Disabled, Enabled, Installing, Quarantined, RollingBack, Uninstalling, Updating,
        };
        match self {
            Installing => to == Disabled,
            Disabled => matches!(
                to,
                Enabled | Updating | RollingBack | Uninstalling | Quarantined | Broken
            ),
            Enabled => matches!(
                to,
                Disabled | Updating | RollingBack | Uninstalling | Quarantined | Broken
            ),
            Broken => matches!(to, Updating | RollingBack | Uninstalling | Quarantined),
            Quarantined => matches!(to, Disabled | Updating | RollingBack | Uninstalling),
            Updating => matches!(to, Disabled | Enabled | Broken | Quarantined),
            RollingBack => matches!(to, Disabled | Enabled),
            Uninstalling => false,
        }
    }

    /// Whether the plugin's records may be deleted from this state (an
    /// aborted install or a finished uninstall).
    pub fn can_be_removed(self) -> bool {
        matches!(self, Self::Installing | Self::Uninstalling)
    }
}

impl fmt::Display for PluginState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::PluginState::{self, *};

    #[test]
    fn names_round_trip() {
        for state in PluginState::ALL {
            assert_eq!(PluginState::parse(state.as_str()), Some(state));
        }
        assert_eq!(PluginState::parse("active"), None);
    }

    #[test]
    fn transition_table_is_exact() {
        let allowed = [
            (Installing, Disabled),
            (Disabled, Enabled),
            (Enabled, Disabled),
            (Disabled, Updating),
            (Enabled, Updating),
            (Broken, Updating),
            (Quarantined, Updating),
            (Updating, Disabled),
            (Updating, Enabled),
            (Updating, Broken),
            (Updating, Quarantined),
            (Disabled, RollingBack),
            (Enabled, RollingBack),
            (Broken, RollingBack),
            (Quarantined, RollingBack),
            (RollingBack, Disabled),
            (RollingBack, Enabled),
            (Disabled, Uninstalling),
            (Enabled, Uninstalling),
            (Broken, Uninstalling),
            (Quarantined, Uninstalling),
            (Disabled, Quarantined),
            (Enabled, Quarantined),
            (Broken, Quarantined),
            (Quarantined, Disabled),
            (Disabled, Broken),
            (Enabled, Broken),
        ];
        for from in PluginState::ALL {
            for to in PluginState::ALL {
                assert_eq!(
                    from.can_transition_to(to),
                    allowed.contains(&(from, to)),
                    "{from} -> {to}"
                );
            }
        }
    }

    #[test]
    fn no_self_loops_and_no_way_out_of_quarantine_except_disabled() {
        for state in PluginState::ALL {
            assert!(!state.can_transition_to(state), "{state}");
        }
        assert!(!Quarantined.can_transition_to(Enabled));
        assert!(!Broken.can_transition_to(Enabled));
        assert!(!Installing.can_transition_to(Enabled));
        assert!(!Uninstalling.can_transition_to(Disabled));
    }

    #[test]
    fn transient_states_are_not_stable() {
        for state in [Installing, Updating, RollingBack, Uninstalling] {
            assert!(!state.is_stable());
        }
        assert!(Enabled.is_runnable());
        assert!(!Disabled.is_runnable());
        assert!(Installing.can_be_removed() && Uninstalling.can_be_removed());
        assert!(!Enabled.can_be_removed());
    }
}
