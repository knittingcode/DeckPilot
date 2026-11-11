//! Live key state of Docker keys: whether Docker is available, how many
//! containers run, and whether a key's Compose project is up.
//!
//! Read with the same read-only commands as the actions (`docker version`,
//! `docker ps`, `docker compose ps`), at most every [`STATE_INTERVAL`]
//! while the key is shown. Never changes containers, whatever the key's
//! action does when pressed.

use std::time::Duration;

use deckpilot_core::{ButtonState, Indicator};

/// How often a shown key's Docker state is read at most.
pub const STATE_INTERVAL: Duration = Duration::from_secs(30);

/// `docker.status`: green when the engine answers, red otherwise.
pub fn availability(available: bool) -> ButtonState {
    if available {
        ButtonState::new().with_indicator(Indicator::Ok)
    } else {
        ButtonState::new()
            .with_detail("Unavailable")
            .with_indicator(Indicator::Error)
    }
}

/// `docker.containers`: how many run; green when any does.
pub fn containers(running: usize) -> ButtonState {
    ButtonState::new()
        .with_detail(format!("{running} running"))
        .with_indicator(if running > 0 {
            Indicator::Ok
        } else {
            Indicator::Unknown
        })
}

/// A Compose project: `running` of `total` containers up — green when all
/// are, amber when some are, grey when none are. With `toggle` (up and
/// down keys) also whether the project is up.
pub fn compose(running: usize, total: usize, toggle: bool) -> ButtonState {
    let indicator = if running == 0 {
        Indicator::Unknown
    } else if running >= total {
        Indicator::Ok
    } else {
        Indicator::Warning
    };
    let state = ButtonState::new()
        .with_detail(format!("{running}/{total} up"))
        .with_indicator(indicator);
    if toggle {
        state.with_toggle(running > 0 && running >= total)
    } else {
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_projects() {
        assert_eq!(
            compose(3, 3, false),
            ButtonState::new()
                .with_detail("3/3 up")
                .with_indicator(Indicator::Ok)
        );
        assert_eq!(compose(1, 3, false).indicator, Some(Indicator::Warning));
        assert_eq!(compose(0, 3, false).indicator, Some(Indicator::Unknown));
        assert_eq!(compose(0, 0, true).toggle, Some(false));
        assert_eq!(compose(2, 2, true).toggle, Some(true));
        assert_eq!(compose(1, 2, true).toggle, Some(false));
        assert_eq!(compose(1, 2, false).toggle, None);
    }

    #[test]
    fn engine_and_containers() {
        assert_eq!(availability(true).indicator, Some(Indicator::Ok));
        assert_eq!(availability(false).indicator, Some(Indicator::Error));
        assert_eq!(containers(2).detail.as_deref(), Some("2 running"));
        assert_eq!(containers(0).indicator, Some(Indicator::Unknown));
    }
}
