//! Key state of plugin actions (plugin API 1.2,
//! `docs/marketplace/plugin-key-state.md` section 4): a plugin action as
//! one more provider of the agent's live key states
//! (`service/states.rs`), with the budgets a plugin needs on top.
//!
//! The agent's scheduler already asks only for keys shown on a device, off
//! the agent's task, never more often than the policy allows, and redraws
//! only on change. Here, for plugins:
//!
//! - **A floor of 5 s** between two state calls for the same key
//!   ([`policy`]), whatever the manifest asks for.
//! - **One state call per plugin at a time** ([`StateBook::begin`]): the
//!   keys of a plugin take turns, each waiting at most [`TURN_WAIT`] for
//!   the others' short calls. None is queued behind a running action
//!   (`Lane::submit_idle`): a key whose plugin is busy keeps what it
//!   showed, and a key press never waits behind more than one state call.
//! - **2 s per state call** ([`STATE_CALL_TIMEOUT`]); past it the host is
//!   stopped like one whose action ran out of time.
//! - **Back-off**: three failures in a row for a key stop its calls for
//!   5 s, then 10 s, doubling up to 5 minutes; meanwhile the key shows its
//!   static face with the error mark.
//! - **Quiet fallbacks**: a plugin that may not report state
//!   (`device:state` not granted) or has none for the action leaves the
//!   key as the user designed it, without an error mark, and is asked
//!   again only after [`QUIET`].
//!
//! The plugin's [`KeyState`] is mapped to the core's [`ButtonState`]
//! member by member; the core sanitizes it again and the renderer draws
//! it. Nothing of a plugin is decoded as an image.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_core::{Accent, ButtonState, Glyph, Indicator, RefreshPolicy, StateError};
use deckpilot_plugin_abi::state::{KeyState, StateAccent, StateGlyph, StateIndicator};
use deckpilot_plugin_manifest::{RefreshMode, StateRefresh};

/// The shortest interval between two state calls for the same key.
pub(crate) const MIN_INTERVAL: Duration = Duration::from_secs(5);
/// Wall clock of one state call.
pub(crate) const STATE_CALL_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a key waits for its plugin's other state calls before it
/// keeps what it showed: three calls' worth.
pub(crate) const TURN_WAIT: Duration = Duration::from_secs(6);
/// Failures in a row for a key before its calls are paused.
const FAILURES_BEFORE_BACKOFF: u32 = 3;
const BACKOFF_BASE: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);
/// How long a plugin without the grant, or an action without state, is
/// left alone.
const QUIET: Duration = Duration::from_secs(30);
/// Keys remembered at most; the longest untouched are dropped first.
const MAX_KEYS: usize = 512;

/// The refresh policy of a manifest's `state`, never below
/// [`MIN_INTERVAL`] (the core caps it at 24 hours).
pub(crate) fn policy(refresh: &StateRefresh) -> RefreshPolicy {
    let interval = Duration::from_secs(refresh.interval_seconds).max(MIN_INTERVAL);
    match refresh.refresh {
        RefreshMode::OnEvents => RefreshPolicy::on_events(interval),
        RefreshMode::Every => RefreshPolicy::every(interval),
    }
}

/// A plugin's state as the core's, member by member.
pub(crate) fn button_state(state: KeyState) -> ButtonState {
    ButtonState {
        label: state.label,
        detail: state.detail,
        indicator: state.indicator.map(|indicator| match indicator {
            StateIndicator::Ok => Indicator::Ok,
            StateIndicator::Warning => Indicator::Warning,
            StateIndicator::Error => Indicator::Error,
            StateIndicator::Running => Indicator::Running,
            StateIndicator::Unknown => Indicator::Unknown,
        }),
        progress: state.progress,
        toggle: state.toggle,
        accent: state.accent.map(|accent| match accent {
            StateAccent::Blue => Accent::Blue,
            StateAccent::Green => Accent::Green,
            StateAccent::Amber => Accent::Amber,
            StateAccent::Red => Accent::Red,
            StateAccent::Purple => Accent::Purple,
            StateAccent::Teal => Accent::Teal,
            StateAccent::Grey => Accent::Grey,
        }),
        glyph: state.glyph.map(|glyph| match glyph {
            StateGlyph::Check => Glyph::Check,
            StateGlyph::Cross => Glyph::Cross,
            StateGlyph::Play => Glyph::Play,
            StateGlyph::Pause => Glyph::Pause,
            StateGlyph::Stop => Glyph::Stop,
            StateGlyph::Timer => Glyph::Timer,
            StateGlyph::Bell => Glyph::Bell,
            StateGlyph::Up => Glyph::Up,
            StateGlyph::Down => Glyph::Down,
            StateGlyph::Dot => Glyph::Dot,
            StateGlyph::Circle => Glyph::Circle,
            StateGlyph::Square => Glyph::Square,
            StateGlyph::Diamond => Glyph::Diamond,
            StateGlyph::Triangle => Glyph::Triangle,
            StateGlyph::Star => Glyph::Star,
            StateGlyph::Heart => Glyph::Heart,
        }),
        badge: state.badge,
        series: state.series,
    }
}

/// Whose state: the plugin, the action and the key's parameters (JSON).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KeyId {
    pub plugin: String,
    pub action: String,
    pub params: String,
}

/// How one state call ended.
#[derive(Debug)]
pub(crate) enum Asked {
    /// The plugin's description.
    State(ButtonState),
    /// Nothing to show and nothing wrong: not granted, or no state.
    Quiet,
    /// The plugin was running something else; nothing was asked.
    Busy,
    /// An error, a timeout, an invalid state, a crash.
    Failed(StateError),
}

type Answer = Result<ButtonState, StateError>;

/// Whether a key's state is asked for now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Turn {
    /// Yes, and no other state call of the plugin runs meanwhile.
    Ask,
    /// Another state call of the plugin is under way: wait for the turn
    /// (state calls are short) or show what the key showed last.
    Busy(Answer),
    /// The key backs off, or its plugin reports no state: show what it
    /// showed last, without asking.
    Paused(Answer),
}

#[derive(Debug)]
struct Entry {
    last: Option<Answer>,
    failures: u32,
    retry_at: Option<Instant>,
    touched: Instant,
}

#[derive(Debug, Default)]
struct Book {
    /// Plugins with a state call under way.
    asking: BTreeSet<String>,
    keys: BTreeMap<KeyId, Entry>,
}

/// What the agent remembers about plugin key states between calls: whose
/// turn it is, what each key last showed and which keys are backing off.
/// Cheap to clone; every clone is the same book.
#[derive(Debug, Clone, Default)]
pub(crate) struct StateBook(Arc<Mutex<Book>>);

/// Nothing to show: the key as the user designed it.
const fn nothing() -> ButtonState {
    ButtonState {
        label: None,
        detail: None,
        indicator: None,
        progress: None,
        toggle: None,
        accent: None,
        glyph: None,
        badge: None,
        series: None,
    }
}

fn backoff(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(FAILURES_BEFORE_BACKOFF).min(16);
    BACKOFF_BASE.saturating_mul(1 << doublings).min(BACKOFF_MAX)
}

impl StateBook {
    fn lock(&self) -> std::sync::MutexGuard<'_, Book> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Before a state call for `key`: whether to ask now. On
    /// [`Turn::Ask`] the caller has the plugin's turn and **must** call
    /// [`StateBook::finish`].
    pub fn begin(&self, key: &KeyId, now: Instant) -> Turn {
        let mut book = self.lock();
        if book.keys.len() >= MAX_KEYS && !book.keys.contains_key(key) {
            let oldest = book
                .keys
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                book.keys.remove(&oldest);
            }
        }
        let asking = book.asking.contains(&key.plugin);
        let entry = book.keys.entry(key.clone()).or_insert(Entry {
            last: None,
            failures: 0,
            retry_at: None,
            touched: now,
        });
        entry.touched = now;
        let last = entry.last.clone().unwrap_or(Ok(nothing()));
        if entry.retry_at.is_some_and(|at| now < at) {
            return Turn::Paused(last);
        }
        if asking {
            return Turn::Busy(last);
        }
        book.asking.insert(key.plugin.clone());
        Turn::Ask
    }

    /// After the state call [`StateBook::begin`] allowed: records how it
    /// ended and returns what the key shows.
    pub fn finish(&self, key: &KeyId, asked: Asked, now: Instant) -> Answer {
        let mut book = self.lock();
        book.asking.remove(&key.plugin);
        let Some(entry) = book.keys.get_mut(key) else {
            return Ok(nothing());
        };
        match asked {
            Asked::State(state) => {
                entry.failures = 0;
                entry.retry_at = None;
                entry.last = Some(Ok(state));
            }
            Asked::Quiet => {
                entry.failures = 0;
                entry.retry_at = Some(now + QUIET);
                entry.last = Some(Ok(nothing()));
            }
            Asked::Busy => {}
            Asked::Failed(error) => {
                entry.failures = entry.failures.saturating_add(1);
                if entry.failures >= FAILURES_BEFORE_BACKOFF {
                    entry.retry_at = Some(now + backoff(entry.failures));
                }
                entry.last = Some(Err(error));
            }
        }
        entry.last.clone().unwrap_or(Ok(nothing()))
    }

    /// Forgets everything about `plugin`: after it was disabled, updated
    /// or uninstalled, its keys start afresh.
    pub fn forget(&self, plugin: &str) {
        let mut book = self.lock();
        book.keys.retain(|key, _| key.plugin != plugin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(plugin: &str, params: &str) -> KeyId {
        KeyId {
            plugin: plugin.into(),
            action: "cell".into(),
            params: params.into(),
        }
    }

    fn failed() -> Asked {
        Asked::Failed(StateError::new("timeout", "no answer"))
    }

    #[test]
    fn intervals_have_a_floor_of_five_seconds() {
        let refresh = |refresh, interval_seconds| StateRefresh {
            refresh,
            interval_seconds,
        };
        for asked in [0, 1, 4] {
            assert_eq!(
                policy(&refresh(RefreshMode::Every, asked)),
                RefreshPolicy::every(MIN_INTERVAL)
            );
            assert_eq!(
                policy(&refresh(RefreshMode::OnEvents, asked)),
                RefreshPolicy::on_events(MIN_INTERVAL)
            );
        }
        assert_eq!(
            policy(&refresh(RefreshMode::Every, 60)),
            RefreshPolicy::every(Duration::from_secs(60))
        );
        // Only `every` polls; the core caps the interval at a day.
        assert_eq!(
            policy(&refresh(RefreshMode::OnEvents, 60)).expires_after(),
            None
        );
        assert_eq!(
            policy(&refresh(RefreshMode::Every, 10_000_000)).min_interval(),
            deckpilot_core::MAX_REFRESH_INTERVAL
        );
    }

    #[test]
    fn a_plugin_state_maps_member_by_member() {
        let state: KeyState = serde_json::from_value(serde_json::json!({
            "label": "12:40", "detail": "Focus", "indicator": "running", "progress": 48,
            "toggle": true, "accent": "green", "glyph": "timer", "badge": "3",
            "series": [10, 40, 35, 80]
        }))
        .unwrap();
        // The same names on both sides: the core's JSON is the plugin's.
        assert_eq!(
            serde_json::to_value(button_state(state.clone())).unwrap(),
            serde_json::to_value(&state).unwrap()
        );
        assert_eq!(button_state(KeyState::new()), ButtonState::default());
        // Every name of the closed sets has a counterpart.
        for accent in ["blue", "green", "amber", "red", "purple", "teal", "grey"] {
            let state: KeyState =
                serde_json::from_value(serde_json::json!({ "accent": accent })).unwrap();
            let mapped = serde_json::to_value(button_state(state)).unwrap();
            assert_eq!(mapped["accent"], accent);
        }
        for glyph in Glyph::ALL {
            let name = serde_json::to_value(glyph).unwrap();
            let state: KeyState =
                serde_json::from_value(serde_json::json!({ "glyph": name })).unwrap();
            assert_eq!(button_state(state).glyph, Some(glyph));
        }
        for indicator in ["ok", "warning", "error", "running", "unknown"] {
            let state: KeyState =
                serde_json::from_value(serde_json::json!({ "indicator": indicator })).unwrap();
            let mapped = serde_json::to_value(button_state(state)).unwrap();
            assert_eq!(mapped["indicator"], indicator);
        }
    }

    #[test]
    fn one_state_call_per_plugin_at_a_time() {
        let book = StateBook::default();
        let now = Instant::now();
        let (a, b, other) = (
            key("acme.board", "a"),
            key("acme.board", "b"),
            key("acme.other", "a"),
        );
        assert_eq!(book.begin(&a, now), Turn::Ask);
        // Another key of the same plugin does not wait: it shows what it
        // showed last (nothing yet). Another plugin has its own turn.
        assert_eq!(book.begin(&b, now), Turn::Busy(Ok(ButtonState::default())));
        assert_eq!(book.begin(&other, now), Turn::Ask);
        let shown = ButtonState::new().with_label("7");
        assert_eq!(
            book.finish(&a, Asked::State(shown.clone()), now),
            Ok(shown.clone())
        );
        // Now it is `b`'s turn; `a` keeps its state while `b` is asked.
        assert_eq!(book.begin(&b, now), Turn::Ask);
        assert_eq!(book.begin(&a, now), Turn::Busy(Ok(shown.clone())));
        // A plugin busy with an action: nothing changes for the key.
        assert_eq!(
            book.finish(&b, Asked::Busy, now),
            Ok(ButtonState::default())
        );
        assert_eq!(book.begin(&a, now), Turn::Ask);
        assert_eq!(book.finish(&a, Asked::Busy, now), Ok(shown));
    }

    #[test]
    fn repeated_failures_back_off_and_a_good_state_ends_it() {
        let book = StateBook::default();
        let mut now = Instant::now();
        let a = key("acme.board", "a");
        let ask = |book: &StateBook, now: Instant, asked: Asked| {
            assert_eq!(book.begin(&a, now), Turn::Ask, "the key is asked");
            book.finish(&a, asked, now)
        };
        // Two failures: shown as failed, asked again at the next interval.
        for _ in 0..2 {
            assert!(ask(&book, now, failed()).is_err());
        }
        // The third starts the back-off: 5 s, then 10 s, 20 s … 5 minutes.
        let mut expected = Duration::from_secs(5);
        for _ in 0..10 {
            assert!(ask(&book, now, failed()).is_err());
            let before = now + expected.checked_sub(Duration::from_millis(1)).unwrap();
            assert!(
                matches!(book.begin(&a, before), Turn::Paused(Err(_))),
                "not asked within {expected:?}, and shown as unavailable"
            );
            now += expected;
            expected = (expected * 2).min(Duration::from_secs(300));
        }
        assert_eq!(expected, Duration::from_secs(300));
        // A state ends it at once.
        let shown = ButtonState::new().with_label("ok");
        assert_eq!(ask(&book, now, Asked::State(shown.clone())), Ok(shown));
        assert_eq!(book.begin(&a, now), Turn::Ask);
        assert!(book.finish(&a, failed(), now).is_err());
        assert_eq!(book.begin(&a, now), Turn::Ask, "one failure does not pause");
        let _ = book.finish(&a, Asked::Busy, now);

        // Not granted or no state: the static face, without an error
        // mark, and left alone for a while.
        let quiet = key("acme.board", "quiet");
        assert_eq!(book.begin(&quiet, now), Turn::Ask);
        assert_eq!(
            book.finish(&quiet, Asked::Quiet, now),
            Ok(ButtonState::default())
        );
        assert_eq!(
            book.begin(&quiet, now + Duration::from_secs(29)),
            Turn::Paused(Ok(ButtonState::default()))
        );
        assert_eq!(book.begin(&quiet, now + Duration::from_secs(30)), Turn::Ask);
        assert!(book.finish(&quiet, Asked::Quiet, now).is_ok());

        // Forgetting a plugin drops its keys and their back-off.
        // One failure is on record already; two more start a back-off.
        assert!(ask(&book, now, failed()).is_err());
        assert!(ask(&book, now, failed()).is_err());
        assert!(matches!(book.begin(&a, now), Turn::Paused(Err(_))));
        book.forget("acme.board");
        assert_eq!(book.begin(&a, now), Turn::Ask);
    }

    #[test]
    fn the_book_is_bounded() {
        let book = StateBook::default();
        let start = Instant::now();
        for i in 0..(MAX_KEYS + 50) {
            let key = key(&format!("acme.p{i}"), "a");
            let now = start + Duration::from_millis(u64::try_from(i).unwrap());
            assert_eq!(book.begin(&key, now), Turn::Ask);
            assert!(
                book.finish(&key, Asked::State(ButtonState::default()), now)
                    .is_ok()
            );
        }
        assert_eq!(book.lock().keys.len(), MAX_KEYS);
        // The longest untouched went first.
        assert!(!book.lock().keys.contains_key(&key("acme.p0", "a")));
        assert!(book.lock().asking.is_empty());
    }
}
