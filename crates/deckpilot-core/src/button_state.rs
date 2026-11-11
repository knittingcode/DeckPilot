//! Live state of keys: what a module reports about the action a key runs,
//! such as a repository's branch or whether a Compose project is up.
//!
//! A module opts in per action type through
//! [`ActionHandler::state_policy`](crate::ActionHandler::state_policy) and
//! [`ActionHandler::button_state`](crate::ActionHandler::button_state). The
//! agent asks for the state off the device loop, never more often than the
//! declared [`RefreshPolicy`] allows, caches it and redraws only keys whose
//! state changed. The renderer only knows the typed [`ButtonState`] (label,
//! detail, [`Indicator`], progress, toggle); it knows nothing about Git,
//! Docker or any other module.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The shortest interval any provider is refreshed at, whatever it
/// declares.
pub const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
/// The longest interval a provider may declare; longer ones are shortened.
pub const MAX_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Longest label and detail a state may set, in characters.
pub const MAX_STATE_TEXT: usize = 32;
/// Longest error message kept for a failed state, in characters.
pub const MAX_STATE_ERROR: usize = 120;

/// A small status mark drawn in a corner of the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Indicator {
    /// Everything is as expected (green).
    Ok,
    /// Needs attention but works (amber), e.g. uncommitted changes.
    Warning,
    /// Broken or unreachable (red).
    Error,
    /// Something is in progress (blue), e.g. a running workflow.
    Running,
    /// The state is not known (grey), e.g. nothing is running.
    Unknown,
    /// The first state is still being determined. Set by the agent; a
    /// provider does not need to return it.
    Loading,
}

/// What a module reports about a key's action. Every field is optional;
/// the default changes nothing about the key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ButtonState {
    /// Replaces the key's title, e.g. the branch name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Replaces the key's secondary line, e.g. `2/3 up`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indicator: Option<Indicator>,
    /// 0–100, drawn as a bar along the bottom edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    /// On/off state of a toggle-like action, drawn as a mark along the top
    /// edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggle: Option<bool>,
}

impl ButtonState {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub fn with_indicator(mut self, indicator: Indicator) -> Self {
        self.indicator = Some(indicator);
        self
    }

    #[must_use]
    pub fn with_progress(mut self, percent: u8) -> Self {
        self.progress = Some(percent);
        self
    }

    #[must_use]
    pub fn with_toggle(mut self, on: bool) -> Self {
        self.toggle = Some(on);
        self
    }

    /// The state with texts shortened to [`MAX_STATE_TEXT`] characters
    /// (without control characters) and progress capped at 100. Applied by
    /// the core to every state a module returns.
    #[must_use]
    pub fn sanitized(self) -> Self {
        let clean = |text: String| {
            let text: String = text
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_STATE_TEXT)
                .collect();
            let text = text.trim().to_owned();
            (!text.is_empty()).then_some(text)
        };
        Self {
            label: self.label.and_then(clean),
            detail: self.detail.and_then(clean),
            indicator: self.indicator,
            progress: self.progress.map(|p| p.min(100)),
            toggle: self.toggle,
        }
    }
}

/// Why a module could not determine a state. Shown on the key as an error
/// indicator; the message is kept short and is never logged, so it must
/// not contain secrets (the same rule as action messages).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateError {
    /// A stable code from the module's documented list, e.g.
    /// `not-a-repository`.
    pub code: String,
    pub message: String,
}

impl StateError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// The error with its message shortened to [`MAX_STATE_ERROR`]
    /// characters.
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            code: self.code.chars().take(64).collect(),
            message: self
                .message
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_STATE_ERROR)
                .collect(),
        }
    }
}

impl From<crate::ActionResult> for StateError {
    /// A failed action result, e.g. from a helper shared with `execute`.
    fn from(result: crate::ActionResult) -> Self {
        Self::new(
            result.code.unwrap_or_else(|| "failed".into()),
            result.message,
        )
    }
}

impl From<crate::CapabilityDenied> for StateError {
    fn from(denied: crate::CapabilityDenied) -> Self {
        Self::new("not-permitted", denied.to_string())
    }
}

/// When the agent asks a module for a key's state again.
///
/// Whatever the policy, a provider is called at most once per
/// [`RefreshPolicy::min_interval`] for the same key, and only while the
/// key is shown on a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RefreshPolicy {
    /// Only when something happened in DeckPilot: a key was pressed, the
    /// active project changed, the configuration was reloaded or a
    /// workflow ended. For states that only change through DeckPilot.
    OnEvents {
        #[serde(with = "millis")]
        min_interval: Duration,
    },
    /// Every `interval` while the key is shown, and on events, but never
    /// more often than every `interval`. For states that change outside
    /// DeckPilot, such as a repository or containers.
    Every {
        #[serde(with = "millis")]
        interval: Duration,
    },
}

impl RefreshPolicy {
    pub fn on_events(min_interval: Duration) -> Self {
        Self::OnEvents { min_interval }
    }

    pub fn every(interval: Duration) -> Self {
        Self::Every { interval }
    }

    /// The shortest time between two calls, bounded to
    /// [`MIN_REFRESH_INTERVAL`]..=[`MAX_REFRESH_INTERVAL`].
    pub fn min_interval(&self) -> Duration {
        let declared = match self {
            Self::OnEvents { min_interval } => *min_interval,
            Self::Every { interval } => *interval,
        };
        declared.clamp(MIN_REFRESH_INTERVAL, MAX_REFRESH_INTERVAL)
    }

    /// How long a state stays current without an event; `None`: until
    /// the next event.
    pub fn expires_after(&self) -> Option<Duration> {
        match self {
            Self::OnEvents { .. } => None,
            Self::Every { .. } => Some(self.min_interval()),
        }
    }
}

/// What the agent knows about a key's state at a moment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StateSnapshot {
    /// The first state has been requested and has not arrived yet.
    Loading,
    Ready {
        state: ButtonState,
    },
    Failed {
        error: StateError,
    },
}

mod millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(u64::try_from(value.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_are_bounded() {
        assert_eq!(
            RefreshPolicy::every(Duration::ZERO).min_interval(),
            MIN_REFRESH_INTERVAL
        );
        assert_eq!(
            RefreshPolicy::on_events(Duration::from_secs(5)).min_interval(),
            Duration::from_secs(5)
        );
        assert_eq!(
            RefreshPolicy::every(Duration::from_secs(10 * 24 * 3600)).min_interval(),
            MAX_REFRESH_INTERVAL
        );
        assert_eq!(
            RefreshPolicy::every(Duration::from_secs(30)).expires_after(),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            RefreshPolicy::on_events(Duration::from_secs(30)).expires_after(),
            None
        );
    }

    #[test]
    fn states_are_sanitized() {
        let state = ButtonState::new()
            .with_label("feature/a-very-long-branch-name-that-goes-on")
            .with_detail("  \n ")
            .with_progress(250)
            .sanitized();
        assert_eq!(state.label.as_deref().map(str::len), Some(MAX_STATE_TEXT));
        assert_eq!(state.detail, None);
        assert_eq!(state.progress, Some(100));
        let error = StateError::new("x", "a\nb".repeat(200)).sanitized();
        assert_eq!(error.message.chars().count(), MAX_STATE_ERROR);
        assert!(!error.message.contains('\n'));
    }

    #[test]
    fn serializes_compactly() {
        let policy = RefreshPolicy::every(Duration::from_secs(30));
        assert_eq!(
            serde_json::to_value(policy).unwrap(),
            serde_json::json!({"type": "every", "interval": 30_000})
        );
        let snapshot = StateSnapshot::Ready {
            state: ButtonState::new().with_indicator(Indicator::Ok),
        };
        assert_eq!(
            serde_json::to_value(&snapshot).unwrap(),
            serde_json::json!({"type": "ready", "state": {"indicator": "ok"}})
        );
    }
}
