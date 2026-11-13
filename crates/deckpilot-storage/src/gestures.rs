//! The gesture timings of this computer: the double-click window and the
//! long-press time of every key whose page sets none (see
//! `docs/gestures-and-work-session.md`, section 3.1). A machine setting,
//! never exported.

use deckpilot_core::gesture::GestureTimings;

use crate::{ConfigStore, Result, StorageError};

const DEFAULTS: &str = "gestures.defaults";

/// The default timings. Missing, unreadable or out-of-range values (for
/// example from a newer version) fall back to the built-in defaults.
pub fn gesture_defaults(store: &impl ConfigStore) -> Result<GestureTimings> {
    Ok(store
        .setting(DEFAULTS)?
        .and_then(|value| serde_json::from_str::<GestureTimings>(&value).ok())
        .and_then(|timings| timings.validate().ok())
        .unwrap_or_default())
}

/// Stores the default timings; values outside the allowed ranges are
/// refused.
pub fn set_gesture_defaults(store: &impl ConfigStore, timings: GestureTimings) -> Result<()> {
    let timings = timings
        .validate()
        .map_err(|error| StorageError::Invalid(error.to_string()))?;
    store.set_setting(DEFAULTS, &serde_json::to_string(&timings)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteStore;

    #[test]
    fn defaults_round_trip_and_fall_back_safely() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(gesture_defaults(&store).unwrap(), GestureTimings::default());
        let timings = GestureTimings {
            double_click_ms: 450,
            long_press_ms: 1200,
        };
        set_gesture_defaults(&store, timings).unwrap();
        assert_eq!(gesture_defaults(&store).unwrap(), timings);

        let too_fast = GestureTimings {
            double_click_ms: 50,
            ..timings
        };
        assert!(matches!(
            set_gesture_defaults(&store, too_fast),
            Err(StorageError::Invalid(_))
        ));
        assert_eq!(gesture_defaults(&store).unwrap(), timings);

        // Written by something else: never used out of range.
        store
            .set_setting(DEFAULTS, r#"{"doubleClickMs":300,"longPressMs":60000}"#)
            .unwrap();
        assert_eq!(gesture_defaults(&store).unwrap(), GestureTimings::default());
        store.set_setting(DEFAULTS, "not json").unwrap();
        assert_eq!(gesture_defaults(&store).unwrap(), GestureTimings::default());
    }
}
