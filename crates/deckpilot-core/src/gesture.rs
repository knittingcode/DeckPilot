//! Key gestures: single click, double click and long press.
//!
//! Contract only (see `docs/gestures-and-work-session.md`): the types the
//! gesture engine, the layout model and the gesture editor share. Nothing
//! uses them yet; key bindings still fire on the raw press or release.
//!
//! The timing rules, which the engine implements:
//!
//! - A key whose bindings use gestures resolves every press-to-release
//!   sequence to **at most one** gesture.
//! - Single click waits for the double-click window only when the key has a
//!   double-click binding, and for the release only when it has a
//!   long-press binding ([`ClickTiming`]).
//! - A long press fires once, when the hold reaches its threshold; the
//!   release after it never clicks.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::ControlId;

/// A gesture a key binding can react to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Gesture {
    Click,
    DoubleClick,
    LongPress,
}

impl Gesture {
    pub const ALL: [Self; 3] = [Self::Click, Self::DoubleClick, Self::LongPress];
}

/// Default time from the first release to the second press.
pub const DEFAULT_DOUBLE_CLICK_MS: u16 = 300;
/// Default hold that makes a press a long press.
pub const DEFAULT_LONG_PRESS_MS: u16 = 700;
/// Allowed double-click windows, in milliseconds.
pub const DOUBLE_CLICK_RANGE_MS: std::ops::RangeInclusive<u16> = 150..=1000;
/// Allowed long-press thresholds, in milliseconds.
pub const LONG_PRESS_RANGE_MS: std::ops::RangeInclusive<u16> = 300..=5000;

/// The timings one key uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GestureTimings {
    pub double_click_ms: u16,
    pub long_press_ms: u16,
}

impl Default for GestureTimings {
    fn default() -> Self {
        Self {
            double_click_ms: DEFAULT_DOUBLE_CLICK_MS,
            long_press_ms: DEFAULT_LONG_PRESS_MS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GestureTimingError {
    #[error("the double-click window must be between {min} and {max} ms, not {value} ms")]
    DoubleClick { value: u16, min: u16, max: u16 },
    #[error("the long-press time must be between {min} and {max} ms, not {value} ms")]
    LongPress { value: u16, min: u16, max: u16 },
}

impl GestureTimings {
    /// Checks both values against their allowed ranges.
    pub fn validate(self) -> Result<Self, GestureTimingError> {
        if !DOUBLE_CLICK_RANGE_MS.contains(&self.double_click_ms) {
            return Err(GestureTimingError::DoubleClick {
                value: self.double_click_ms,
                min: *DOUBLE_CLICK_RANGE_MS.start(),
                max: *DOUBLE_CLICK_RANGE_MS.end(),
            });
        }
        if !LONG_PRESS_RANGE_MS.contains(&self.long_press_ms) {
            return Err(GestureTimingError::LongPress {
                value: self.long_press_ms,
                min: *LONG_PRESS_RANGE_MS.start(),
                max: *LONG_PRESS_RANGE_MS.end(),
            });
        }
        Ok(self)
    }
}

/// Timings set for one key of a page; unset values use the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyGestureSettings {
    pub control: ControlId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_click_ms: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_press_ms: Option<u16>,
}

impl KeyGestureSettings {
    /// The key's timings over `defaults`, validated.
    pub fn timings(&self, defaults: GestureTimings) -> Result<GestureTimings, GestureTimingError> {
        GestureTimings {
            double_click_ms: self.double_click_ms.unwrap_or(defaults.double_click_ms),
            long_press_ms: self.long_press_ms.unwrap_or(defaults.long_press_ms),
        }
        .validate()
    }
}

/// Which gestures a key has bindings for, as the engine sees it when a
/// press starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct BoundGestures {
    pub click: bool,
    pub double_click: bool,
    pub long_press: bool,
}

/// When a single click is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClickTiming {
    /// On the press: nothing else could follow, so there is no delay.
    OnPress,
    /// On the release, if the hold stayed below the long-press time.
    OnRelease,
    /// When the double-click window after the release ends without a
    /// second press.
    AfterDoubleClickWindow,
}

impl BoundGestures {
    pub fn any(self) -> bool {
        self.click || self.double_click || self.long_press
    }

    pub fn has(self, gesture: Gesture) -> bool {
        match gesture {
            Gesture::Click => self.click,
            Gesture::DoubleClick => self.double_click,
            Gesture::LongPress => self.long_press,
        }
    }

    /// Single click is delayed only as far as the other bound gestures
    /// require.
    pub fn click_timing(self) -> ClickTiming {
        if self.double_click {
            ClickTiming::AfterDoubleClickWindow
        } else if self.long_press {
            ClickTiming::OnRelease
        } else {
            ClickTiming::OnPress
        }
    }
}

/// Why pending gestures of a device were dropped without firing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GestureCancel {
    /// The device was disconnected (its connection ended).
    Disconnected,
    /// The device shows another page now.
    PageChanged,
    /// The profile or its layout was changed or reloaded.
    ProfileChanged,
    /// DeckPilot was locked.
    Locked,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(
            GestureTimings::default().validate(),
            Ok(GestureTimings {
                double_click_ms: 300,
                long_press_ms: 700
            })
        );
    }

    #[test]
    fn timings_outside_their_ranges_are_refused() {
        let too_fast = GestureTimings {
            double_click_ms: 100,
            ..GestureTimings::default()
        };
        assert!(matches!(
            too_fast.validate(),
            Err(GestureTimingError::DoubleClick { value: 100, .. })
        ));
        let too_long = GestureTimings {
            long_press_ms: 6000,
            ..GestureTimings::default()
        };
        assert!(matches!(
            too_long.validate(),
            Err(GestureTimingError::LongPress { value: 6000, .. })
        ));
        for edge in [
            GestureTimings {
                double_click_ms: 150,
                long_press_ms: 300,
            },
            GestureTimings {
                double_click_ms: 1000,
                long_press_ms: 5000,
            },
        ] {
            assert_eq!(edge.validate(), Ok(edge));
        }
    }

    #[test]
    fn key_settings_fall_back_to_the_defaults() {
        let settings = KeyGestureSettings {
            control: ControlId::Key { index: 3 },
            double_click_ms: None,
            long_press_ms: Some(1200),
        };
        assert_eq!(
            settings.timings(GestureTimings::default()),
            Ok(GestureTimings {
                double_click_ms: 300,
                long_press_ms: 1200
            })
        );
        let json = serde_json::to_value(settings).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"control": {"kind": "key", "index": 3}, "longPressMs": 1200})
        );
    }

    #[test]
    fn click_waits_only_for_what_is_bound() {
        let click = BoundGestures {
            click: true,
            ..BoundGestures::default()
        };
        assert_eq!(click.click_timing(), ClickTiming::OnPress);
        let with_long = BoundGestures {
            long_press: true,
            ..click
        };
        assert_eq!(with_long.click_timing(), ClickTiming::OnRelease);
        let with_double = BoundGestures {
            double_click: true,
            ..click
        };
        assert_eq!(
            with_double.click_timing(),
            ClickTiming::AfterDoubleClickWindow
        );
        let all = BoundGestures {
            click: true,
            double_click: true,
            long_press: true,
        };
        assert_eq!(all.click_timing(), ClickTiming::AfterDoubleClickWindow);
        assert!(Gesture::ALL.into_iter().all(|g| all.has(g)));
        assert!(!BoundGestures::default().any());
    }

    #[test]
    fn gestures_serialize_in_camel_case() {
        let names: Vec<String> = Gesture::ALL
            .into_iter()
            .map(|g| {
                serde_json::to_value(g)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(names, ["click", "doubleClick", "longPress"]);
    }
}
