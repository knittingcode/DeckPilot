//! Key state (plugin API 1.2): what a plugin reports about a key bound to
//! one of its actions, and what the host draws from it.
//!
//! **The plugin describes, the host draws.** A [`KeyState`] is a small
//! typed description: two short texts, marks from closed sets, a
//! percentage and a short series. It never carries pixels, fonts, markup,
//! colours outside the palette or anything about a device. The agent's
//! renderer draws it at the device's resolution, or not at all.
//!
//! The host asks through the optional guest export
//! [`EXPORT_STATE`](crate::guest::EXPORT_STATE) (`dp_state`), with the same
//! input as `dp_invoke` ([`InvokeInput`](crate::guest::InvokeInput): the
//! action and the key's parameters), and reads a [`StateOutcome`] of at
//! most [`MAX_STATE_OUTPUT`] bytes. Inside a state call only the
//! operations of [`allowed_in_state`] are answered: a state call reads, it
//! never reaches the network, notifies, opens a page or writes.

use serde::{Deserialize, Serialize};

use crate::CapabilityRequest;
use crate::limits::{
    MAX_CODE_LEN, MAX_MESSAGE_LEN, MAX_STATE_BADGE, MAX_STATE_SERIES, MAX_STATE_TEXT,
};

/// The status mark in a corner of the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateIndicator {
    Ok,
    Warning,
    Error,
    Running,
    Unknown,
}

/// The key's background, from the agent's fixed palette. The colours the
/// user chose for the key win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateAccent {
    Blue,
    Green,
    Amber,
    Red,
    Purple,
    Teal,
    Grey,
}

/// A shape the agent draws above the key's title. A closed set: a name
/// that is not listed makes the state invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateGlyph {
    Check,
    Cross,
    Play,
    Pause,
    Stop,
    Timer,
    Bell,
    Up,
    Down,
    Dot,
    Circle,
    Square,
    Diamond,
    Triangle,
    Star,
    Heart,
}

/// What a key shows. Every member is optional; the default leaves the key
/// as the user designed it. Unknown members are refused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyState {
    /// Replaces the key's title; at most [`MAX_STATE_TEXT`] characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Replaces the key's second line; at most [`MAX_STATE_TEXT`]
    /// characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indicator: Option<StateIndicator>,
    /// 0 to 100, a bar along the bottom edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    /// On or off, a mark along the top edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggle: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<StateAccent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<StateGlyph>,
    /// A short label in the top left corner; at most [`MAX_STATE_BADGE`]
    /// characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    /// Values 0 to 100, oldest first, drawn as a small chart; at most
    /// [`MAX_STATE_SERIES`] points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<u8>>,
}

/// Which bound a [`KeyState`] breaks. Names the member, never its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateInvalid(pub &'static str);

impl std::fmt::Display for StateInvalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is out of bounds", self.0)
    }
}

impl std::error::Error for StateInvalid {}

impl KeyState {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub fn indicator(mut self, indicator: StateIndicator) -> Self {
        self.indicator = Some(indicator);
        self
    }

    #[must_use]
    pub fn progress(mut self, percent: u8) -> Self {
        self.progress = Some(percent);
        self
    }

    #[must_use]
    pub fn toggle(mut self, on: bool) -> Self {
        self.toggle = Some(on);
        self
    }

    #[must_use]
    pub fn accent(mut self, accent: StateAccent) -> Self {
        self.accent = Some(accent);
        self
    }

    #[must_use]
    pub fn glyph(mut self, glyph: StateGlyph) -> Self {
        self.glyph = Some(glyph);
        self
    }

    #[must_use]
    pub fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    #[must_use]
    pub fn series(mut self, series: impl Into<Vec<u8>>) -> Self {
        self.series = Some(series.into());
        self
    }

    /// Checks every bound of plugin API 1.2. Both sides apply it: the SDK
    /// before the state leaves the plugin, the host and the supervisor
    /// when it arrives.
    pub fn validate(&self) -> Result<(), StateInvalid> {
        let text = |value: &Option<String>, max: usize, member: &'static str| match value {
            Some(text) if text.chars().count() > max || text.chars().any(char::is_control) => {
                Err(StateInvalid(member))
            }
            _ => Ok(()),
        };
        text(&self.label, MAX_STATE_TEXT, "label")?;
        text(&self.detail, MAX_STATE_TEXT, "detail")?;
        text(&self.badge, MAX_STATE_BADGE, "badge")?;
        if self.progress.is_some_and(|percent| percent > 100) {
            return Err(StateInvalid("progress"));
        }
        if self.series.as_ref().is_some_and(|points| {
            points.len() > MAX_STATE_SERIES || points.iter().any(|p| *p > 100)
        }) {
            return Err(StateInvalid("series"));
        }
        Ok(())
    }
}

/// Why a plugin has no state for a key: a code (`[a-z][a-z0-9-]*`, like
/// an action's) and a message for the user. Shown as an error mark on the
/// key; never logged with its message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateFailure {
    pub code: String,
    pub message: String,
}

/// The output of one `dp_state` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum StateOutcome {
    Ok(KeyState),
    Error(StateFailure),
}

/// The state was not valid (unknown member or name, a bound, too large,
/// not JSON).
pub const STATE_INVALID: &str = "invalid-state";
/// The module has no `dp_state`, or the action no state.
pub const STATE_UNSUPPORTED: &str = "unsupported";
/// The plugin may not report key state (`device:state` not granted).
pub const STATE_NOT_PERMITTED: &str = "not-permitted";

impl StateOutcome {
    /// A failure with `code` and `message`, cut to the ABI's bounds; a
    /// malformed code becomes [`STATE_INVALID`].
    pub fn failed(code: &str, message: impl Into<String>) -> Self {
        let well_formed = !code.is_empty()
            && code.len() <= MAX_CODE_LEN
            && code.starts_with(|c: char| c.is_ascii_lowercase())
            && code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        Self::Error(StateFailure {
            code: if well_formed { code } else { STATE_INVALID }.to_owned(),
            message: message
                .into()
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_MESSAGE_LEN)
                .collect(),
        })
    }

    /// The outcome with every bound checked: a state out of bounds becomes
    /// an [`STATE_INVALID`] failure that names the member, and a failure is
    /// brought within the bounds of [`StateOutcome::failed`].
    #[must_use]
    pub fn checked(self) -> Self {
        match self {
            Self::Ok(state) => match state.validate() {
                Ok(()) => Self::Ok(state),
                Err(invalid) => Self::failed(STATE_INVALID, invalid.to_string()),
            },
            Self::Error(failure) => Self::failed(&failure.code, failure.message),
        }
    }

    /// Parses the bytes a guest returned from `dp_state`; anything that is
    /// not a valid outcome is an [`STATE_INVALID`] failure.
    pub fn from_guest(bytes: &[u8]) -> Self {
        match serde_json::from_slice::<Self>(bytes) {
            Ok(outcome) => outcome.checked(),
            Err(_) => Self::failed(STATE_INVALID, "the state does not parse"),
        }
    }
}

/// Whether `request` is answered inside a state call: reading the
/// plugin's own storage and settings, the time, and logging. Everything
/// else (the network, notifications, opening pages, writing storage,
/// publishing variables, random bytes, the run context) is refused, so a
/// state call cannot change or send anything.
pub fn allowed_in_state(request: &CapabilityRequest) -> bool {
    matches!(
        request,
        CapabilityRequest::StorageGet(_)
            | CapabilityRequest::StorageKeys(_)
            | CapabilityRequest::SettingsGet(_)
            | CapabilityRequest::TimeNow(_)
            | CapabilityRequest::Log(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Log, LogLevel, NoArgs, RandomBytes, StorageKey, StoragePrefix};

    #[test]
    fn a_state_is_small_typed_and_closed() {
        let state = KeyState::new()
            .label("12:40")
            .detail("Focus")
            .indicator(StateIndicator::Running)
            .progress(48)
            .toggle(true)
            .accent(StateAccent::Green)
            .glyph(StateGlyph::Timer)
            .badge("3")
            .series([10, 40, 35, 80]);
        state.validate().unwrap();
        let json = serde_json::to_string(&StateOutcome::Ok(state.clone())).unwrap();
        assert_eq!(
            json,
            r#"{"ok":{"label":"12:40","detail":"Focus","indicator":"running","progress":48,"toggle":true,"accent":"green","glyph":"timer","badge":"3","series":[10,40,35,80]}}"#
        );
        assert_eq!(
            StateOutcome::from_guest(json.as_bytes()),
            StateOutcome::Ok(state)
        );
        // Nothing set: an empty object, which leaves the key as it is.
        assert_eq!(
            serde_json::to_string(&StateOutcome::Ok(KeyState::new())).unwrap(),
            r#"{"ok":{}}"#
        );

        // No pixels, markup, colours or names outside the sets.
        for bad in [
            r#"{"ok":{"pixels":"iVBORw0KGgo="}}"#,
            r#"{"ok":{"image":{"png":"x"}}}"#,
            r#"{"ok":{"html":"<b>x</b>"}}"#,
            r##"{"ok":{"accent":"#ff0000"}}"##,
            r#"{"ok":{"accent":"pink"}}"#,
            r#"{"ok":{"glyph":"skull"}}"#,
            r#"{"ok":{"indicator":"loading"}}"#,
            r#"{"ok":{"progress":-1}}"#,
            r#"{"ok":{"progress":300}}"#,
            r#"{"ok":{"series":[1,2,256]}}"#,
            r#"{"ok":{"label":7}}"#,
            r#"{"ok":{},"error":{"code":"x","message":"y"}}"#,
            r#"{"state":{}}"#,
            "not json",
            "",
        ] {
            let StateOutcome::Error(failure) = StateOutcome::from_guest(bad.as_bytes()) else {
                panic!("{bad}")
            };
            assert_eq!(failure.code, STATE_INVALID, "{bad}");
        }
    }

    #[test]
    fn bounds_are_checked_and_name_the_member_only() {
        let long = "x".repeat(MAX_STATE_TEXT + 1);
        for (state, member) in [
            (KeyState::new().label(long.clone()), "label"),
            (KeyState::new().detail(long.clone()), "detail"),
            (KeyState::new().label("a\nb"), "label"),
            (KeyState::new().badge("12345"), "badge"),
            (KeyState::new().badge("\u{7}"), "badge"),
            (KeyState::new().progress(101), "progress"),
            (
                KeyState::new().series(vec![0; MAX_STATE_SERIES + 1]),
                "series",
            ),
            (KeyState::new().series([0, 101]), "series"),
        ] {
            assert_eq!(state.validate(), Err(StateInvalid(member)));
            let StateOutcome::Error(failure) = StateOutcome::Ok(state).checked() else {
                panic!("{member}")
            };
            assert_eq!(failure.code, STATE_INVALID);
            assert_eq!(failure.message, format!("{member} is out of bounds"));
        }
        // At the bounds: characters, not bytes.
        KeyState::new()
            .label("ż".repeat(MAX_STATE_TEXT))
            .detail("x".repeat(MAX_STATE_TEXT))
            .badge("9999")
            .progress(100)
            .series(vec![100; MAX_STATE_SERIES])
            .validate()
            .unwrap();

        // Failures: a code like an action's, a bounded message.
        let failure = |code: &str, message: &str| match StateOutcome::failed(code, message) {
            StateOutcome::Error(failure) => (failure.code, failure.message),
            StateOutcome::Ok(_) => unreachable!(),
        };
        assert_eq!(
            failure("not-configured", "Choose a city"),
            ("not-configured".into(), "Choose a city".into())
        );
        for code in ["", "Bad", "9x", "a b", "a_b", &"a".repeat(MAX_CODE_LEN + 1)] {
            assert_eq!(failure(code, "m").0, STATE_INVALID, "{code}");
        }
        let (_, message) = failure("x", &"m\n".repeat(MAX_MESSAGE_LEN));
        assert_eq!(message, "m".repeat(MAX_MESSAGE_LEN));
        let parsed = StateOutcome::from_guest(br#"{"error":{"code":"Bad Code","message":"m"}}"#);
        assert_eq!(parsed, StateOutcome::failed(STATE_INVALID, "m"));
    }

    #[test]
    fn a_state_call_only_reads() {
        let get = |key: &str| CapabilityRequest::StorageGet(StorageKey { key: key.into() });
        for allowed in [
            get("board"),
            CapabilityRequest::StorageKeys(StoragePrefix { prefix: None }),
            CapabilityRequest::SettingsGet(NoArgs {}),
            CapabilityRequest::TimeNow(NoArgs {}),
            CapabilityRequest::Log(Log {
                level: LogLevel::Debug,
                message: "m".into(),
            }),
        ] {
            assert!(allowed_in_state(&allowed), "{}", allowed.op());
        }
        let refused: Vec<CapabilityRequest> = [
            r#"{"op":"http.fetch","args":{"method":"GET","url":"https://example.com/"}}"#,
            r#"{"op":"notify","args":{"title":"t","body":"b"}}"#,
            r#"{"op":"storage.set","args":{"key":"k","value":1}}"#,
            r#"{"op":"storage.delete","args":{"key":"k"}}"#,
            r#"{"op":"storage.usage","args":{}}"#,
            r#"{"op":"credentials.has","args":{"credential":"token"}}"#,
            r#"{"op":"variables.publish","args":{"id":"x","value":1}}"#,
            r#"{"op":"variables.clear","args":{}}"#,
            r#"{"op":"url.open","args":{"url":"https://example.com/"}}"#,
            r#"{"op":"run.context","args":{}}"#,
        ]
        .iter()
        .map(|json| serde_json::from_str(json).unwrap())
        .chain([CapabilityRequest::RandomBytes(RandomBytes { length: 8 })])
        .collect();
        // Every operation is in one list or the other.
        assert_eq!(refused.len() + 5, 16);
        for request in refused {
            assert!(!allowed_in_state(&request), "{}", request.op());
        }
    }
}
