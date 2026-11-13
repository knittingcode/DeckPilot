//! The gesture engine: turns press and release edges into gestures.
//!
//! A pure state machine (see `docs/gestures-and-work-session.md`, section
//! 2). It never reads a clock: every call carries the time, which the
//! device thread took when the input was read. Its owner sleeps until
//! [`GestureEngine::next_deadline`] and then calls
//! [`GestureEngine::advance`].
//!
//! Each key is tracked per device and connection, so devices are
//! independent and a reconnected device never continues an old sequence.
//! A key in raw mode (no gesture bound) has no state: its edges pass
//! through unchanged and at once. A key in gesture mode resolves each
//! sequence from its first press until it is idle again to at most one
//! gesture. There is at most one entry, and so at most one deadline, per
//! key that is not idle.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use deckpilot_device::{ControlId, ControlInput, DeviceId};

use super::{BoundGestures, ClickTiming, Gesture, GestureCancel, GestureTimings};
use crate::PageId;

/// What a key is bound to when a sequence starts, captured with its first
/// press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyContext {
    /// Gestures bound to the key. None: raw mode.
    pub bound: BoundGestures,
    pub timings: GestureTimings,
    /// The page the device showed at the press.
    pub page: Option<PageId>,
    /// The layout generation at the press; it changes when the profile or
    /// its layout is changed or reloaded.
    pub generation: u64,
}

impl KeyContext {
    /// A key without gesture bindings: its edges pass through.
    pub fn raw(page: Option<PageId>, generation: u64) -> Self {
        Self {
            page,
            generation,
            ..Self::default()
        }
    }
}

/// A gesture that was recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GestureFired {
    pub device: DeviceId,
    /// The device connection the sequence came from.
    pub connection: u64,
    pub control: ControlId,
    pub gesture: Gesture,
    /// The page and generation captured at the first press; the caller
    /// dispatches the gesture only if both still match.
    pub page: Option<PageId>,
    pub generation: u64,
    /// When the sequence's first press was read.
    pub pressed_at: Instant,
    /// When the gesture was recognized: an input's time, or the deadline
    /// that passed.
    pub at: Instant,
}

/// What the engine reports, in the order it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GestureOutput {
    /// An edge of a raw-mode key, unchanged and without delay.
    Edge {
        device: DeviceId,
        connection: u64,
        control: ControlId,
        input: ControlInput,
        at: Instant,
    },
    Fired(GestureFired),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// First press is down. `long` is when it becomes a long press, if one
    /// is bound.
    Down { long: Option<Instant> },
    /// Released once, waiting until `until` (inclusive) for a second
    /// press.
    Wait { until: Instant },
    /// A gesture fired or was cancelled while the key is down: the release
    /// ends the sequence and does nothing else.
    Swallow,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    context: KeyContext,
    pressed_at: Instant,
    phase: Phase,
}

impl Entry {
    /// When this entry acts without input, if ever.
    fn deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::Down { long } => long,
            Phase::Wait { until } => Some(until),
            Phase::Swallow => None,
        }
    }

    /// Whether the deadline has passed at `now`. A long press is reached
    /// at its threshold; the double-click window includes its last
    /// instant.
    fn due(&self, now: Instant) -> bool {
        match self.phase {
            Phase::Down { long: Some(long) } => now >= long,
            Phase::Wait { until } => now > until,
            Phase::Down { long: None } | Phase::Swallow => false,
        }
    }
}

type Key = (DeviceId, u64, ControlId);

/// Gesture state of every key that is not idle.
#[derive(Debug, Default)]
pub struct GestureEngine {
    keys: BTreeMap<Key, Entry>,
}

impl GestureEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one input read at `at`. `context` describes the key's
    /// bindings now; it is called only for a key without a sequence in
    /// progress, and captured when the input starts one.
    ///
    /// Deadlines of the same device that passed by `at` are handled first,
    /// so a late call never reorders an input and a deadline. Inputs of
    /// one device must come in the order they were read.
    pub fn on_input(
        &mut self,
        at: Instant,
        device: &DeviceId,
        connection: u64,
        control: ControlId,
        input: ControlInput,
        context: impl FnOnce() -> KeyContext,
    ) -> Vec<GestureOutput> {
        // A newer connection replaces the old one: its keys are forgotten.
        self.keys
            .retain(|(d, c, _), _| d != device || *c >= connection);
        let mut out = self.advance_where(at, |(d, c, _)| d == device && *c == connection);
        let key = (device.clone(), connection, control);
        let edge = || GestureOutput::Edge {
            device: device.clone(),
            connection,
            control,
            input,
            at,
        };
        let pressed = match input {
            ControlInput::Press => true,
            ControlInput::Release => false,
            // Rotation and touch have no gestures.
            _ => {
                out.push(edge());
                return out;
            }
        };
        let Some(entry) = self.keys.get_mut(&key) else {
            let context = context();
            if !context.bound.any() {
                out.push(edge());
            } else if pressed {
                out.extend(self.start(key, at, context));
            }
            // A release without a sequence (the key was down before the
            // connection, or before its page showed gestures) is dropped.
            return out;
        };
        match (entry.phase, pressed) {
            (Phase::Down { .. }, false) => {
                let context = entry.context;
                let pressed_at = entry.pressed_at;
                if context.bound.double_click {
                    let window = Duration::from_millis(context.timings.double_click_ms.into());
                    entry.phase = Phase::Wait { until: at + window };
                } else {
                    self.keys.remove(&key);
                    if context.bound.click {
                        out.push(fired(&key, &context, Gesture::Click, pressed_at, at));
                    }
                }
            }
            (Phase::Wait { .. }, true) => {
                // Within the window, or `advance_where` would have ended it.
                entry.phase = Phase::Swallow;
                let entry = *entry;
                out.push(fired(
                    &key,
                    &entry.context,
                    Gesture::DoubleClick,
                    entry.pressed_at,
                    at,
                ));
            }
            (Phase::Swallow, false) => {
                self.keys.remove(&key);
            }
            // A press while down or a release while up cannot come from a
            // key; ignore it rather than guess.
            (Phase::Down { .. } | Phase::Swallow, true) | (Phase::Wait { .. }, false) => {}
        }
        out
    }

    fn start(&mut self, key: Key, at: Instant, context: KeyContext) -> Option<GestureOutput> {
        let bound = context.bound;
        if bound.click_timing() == ClickTiming::OnPress {
            // Only a click is bound: nothing could follow, so no delay.
            let out = fired(&key, &context, Gesture::Click, at, at);
            self.keys.insert(
                key,
                Entry {
                    context,
                    pressed_at: at,
                    phase: Phase::Swallow,
                },
            );
            return Some(out);
        }
        let long = bound
            .long_press
            .then(|| at + Duration::from_millis(context.timings.long_press_ms.into()));
        self.keys.insert(
            key,
            Entry {
                context,
                pressed_at: at,
                phase: Phase::Down { long },
            },
        );
        None
    }

    /// Handles every deadline that passed by `now`, in deadline order.
    /// The caller passes in every input read before `now` first.
    pub fn advance(&mut self, now: Instant) -> Vec<GestureOutput> {
        self.advance_where(now, |_| true)
    }

    fn advance_where(&mut self, now: Instant, filter: impl Fn(&Key) -> bool) -> Vec<GestureOutput> {
        let mut due: Vec<(Instant, Key)> = self
            .keys
            .iter()
            .filter(|(key, entry)| filter(key) && entry.due(now))
            .filter_map(|(key, entry)| entry.deadline().map(|at| (at, key.clone())))
            .collect();
        due.sort();
        let mut out = Vec::new();
        for (deadline, key) in due {
            let Some(entry) = self.keys.get_mut(&key) else {
                continue;
            };
            let context = entry.context;
            let pressed_at = entry.pressed_at;
            match entry.phase {
                Phase::Down { .. } => {
                    // Once per hold: the release is swallowed.
                    entry.phase = Phase::Swallow;
                    out.push(fired(
                        &key,
                        &context,
                        Gesture::LongPress,
                        pressed_at,
                        deadline,
                    ));
                }
                Phase::Wait { .. } => {
                    self.keys.remove(&key);
                    if context.bound.click {
                        out.push(fired(&key, &context, Gesture::Click, pressed_at, deadline));
                    }
                }
                Phase::Swallow => {}
            }
        }
        out
    }

    /// The earliest instant at which [`GestureEngine::advance`] may fire
    /// something.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.keys.values().filter_map(Entry::deadline).min()
    }

    /// Drops the pending gestures of `device` (every connection). Keys
    /// that are down still swallow their release, so it cannot click on
    /// what is shown now. A disconnect forgets the device's keys.
    pub fn cancel(&mut self, device: &DeviceId, reason: GestureCancel) {
        self.cancel_where(reason, |d| d == device);
    }

    /// [`GestureEngine::cancel`] for every device.
    pub fn cancel_all(&mut self, reason: GestureCancel) {
        self.cancel_where(reason, |_| true);
    }

    fn cancel_where(&mut self, reason: GestureCancel, filter: impl Fn(&DeviceId) -> bool) {
        self.keys.retain(|(device, _, _), entry| {
            if !filter(device) {
                return true;
            }
            if reason == GestureCancel::Disconnected {
                return false;
            }
            match entry.phase {
                Phase::Down { .. } | Phase::Swallow => {
                    entry.phase = Phase::Swallow;
                    true
                }
                Phase::Wait { .. } => false,
            }
        });
    }

    /// Number of keys that are not idle.
    pub fn pending(&self) -> usize {
        self.keys.len()
    }

    /// Whether the key has no sequence in progress.
    pub fn is_idle(&self, device: &DeviceId, connection: u64, control: ControlId) -> bool {
        !self
            .keys
            .contains_key(&(device.clone(), connection, control))
    }
}

fn fired(
    key: &Key,
    context: &KeyContext,
    gesture: Gesture,
    pressed_at: Instant,
    at: Instant,
) -> GestureOutput {
    GestureOutput::Fired(GestureFired {
        device: key.0.clone(),
        connection: key.1,
        control: key.2,
        gesture,
        page: context.page,
        generation: context.generation,
        pressed_at,
        at,
    })
}

#[cfg(test)]
mod tests;
