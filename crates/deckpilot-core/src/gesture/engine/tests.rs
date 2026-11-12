//! The engine with synthetic instants: every time is `base + n ms`, so the
//! tests are exact and need no clock.

use std::time::{Duration, Instant};

use deckpilot_device::{ControlId, ControlInput, DeviceId, DeviceModelId};

use super::*;
use crate::gesture::{
    BoundGestures, DEFAULT_DOUBLE_CLICK_MS, DEFAULT_LONG_PRESS_MS, Gesture, GestureCancel,
    GestureTimings,
};

const CLICK: BoundGestures = BoundGestures {
    click: true,
    double_click: false,
    long_press: false,
};
const CLICK_LONG: BoundGestures = BoundGestures {
    click: true,
    double_click: false,
    long_press: true,
};
const CLICK_DOUBLE: BoundGestures = BoundGestures {
    click: true,
    double_click: true,
    long_press: false,
};
const ALL: BoundGestures = BoundGestures {
    click: true,
    double_click: true,
    long_press: true,
};

/// One engine with a fixed origin; times are milliseconds after it.
struct Bench {
    engine: GestureEngine,
    base: Instant,
}

impl Bench {
    fn new() -> Self {
        Self {
            engine: GestureEngine::new(),
            base: Instant::now(),
        }
    }

    fn t(&self, ms: u64) -> Instant {
        self.base + Duration::from_millis(ms)
    }

    fn input(
        &mut self,
        ms: u64,
        device: &DeviceId,
        key: u16,
        input: ControlInput,
        context: KeyContext,
    ) -> Vec<Seen> {
        let at = self.t(ms);
        let out = self
            .engine
            .on_input(at, device, 1, control(key), input, || context);
        self.seen(out)
    }

    fn press(&mut self, ms: u64, key: u16, bound: BoundGestures) -> Vec<Seen> {
        self.input(ms, &device("A"), key, ControlInput::Press, ctx(bound))
    }

    fn release(&mut self, ms: u64, key: u16, bound: BoundGestures) -> Vec<Seen> {
        self.input(ms, &device("A"), key, ControlInput::Release, ctx(bound))
    }

    fn advance(&mut self, ms: u64) -> Vec<Seen> {
        let out = self.engine.advance(self.t(ms));
        self.seen(out)
    }

    fn deadline(&self) -> Option<u64> {
        self.engine
            .next_deadline()
            .map(|at| u64::try_from((at - self.base).as_millis()).unwrap())
    }

    fn seen(&self, out: Vec<GestureOutput>) -> Vec<Seen> {
        let ms = |at: Instant| u64::try_from((at - self.base).as_millis()).unwrap();
        out.into_iter()
            .map(|output| match output {
                GestureOutput::Edge {
                    device,
                    control,
                    input,
                    at,
                    ..
                } => Seen::Edge(device.serial, index(control), input, ms(at)),
                GestureOutput::Fired(fired) => Seen::Fired(
                    fired.device.serial,
                    index(fired.control),
                    fired.gesture,
                    ms(fired.at),
                ),
            })
            .collect()
    }
}

/// An output with the device serial, key index and milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Edge(String, u16, ControlInput, u64),
    Fired(String, u16, Gesture, u64),
}

fn fired(key: u16, gesture: Gesture, ms: u64) -> Seen {
    Seen::Fired("A".into(), key, gesture, ms)
}

fn device(serial: &str) -> DeviceId {
    DeviceId {
        model: DeviceModelId::new("elgato.streamdeck.mk2"),
        serial: serial.into(),
    }
}

fn control(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn index(control: ControlId) -> u16 {
    match control {
        ControlId::Key { index } => index,
        other => panic!("unexpected control {other:?}"),
    }
}

fn ctx(bound: BoundGestures) -> KeyContext {
    KeyContext {
        bound,
        timings: GestureTimings::default(),
        page: None,
        generation: 1,
    }
}

#[test]
fn raw_keys_pass_their_edges_through_at_once() {
    let mut bench = Bench::new();
    let raw = BoundGestures::default();
    assert_eq!(
        bench.press(10, 0, raw),
        [Seen::Edge("A".into(), 0, ControlInput::Press, 10)]
    );
    assert_eq!(bench.engine.pending(), 0, "raw keys have no state");
    assert_eq!(bench.deadline(), None);
    assert_eq!(
        bench.release(5000, 0, raw),
        [Seen::Edge("A".into(), 0, ControlInput::Release, 5000)]
    );
    // A stray release of a raw key passes too, as it does today.
    assert_eq!(
        bench.release(6000, 0, raw),
        [Seen::Edge("A".into(), 0, ControlInput::Release, 6000)]
    );
    // Rotation is never a gesture, even on a control in gesture mode.
    let rotate = ControlInput::Rotate { delta: 1 };
    let out = bench.input(7000, &device("A"), 0, rotate, ctx(ALL));
    assert_eq!(out, [Seen::Edge("A".into(), 0, rotate, 7000)]);
}

#[test]
fn a_lone_click_fires_on_the_press() {
    let mut bench = Bench::new();
    assert_eq!(bench.press(10, 0, CLICK), [fired(0, Gesture::Click, 10)]);
    assert_eq!(bench.deadline(), None, "nothing waits");
    // Holding long and releasing does nothing more.
    assert_eq!(bench.advance(10_000), []);
    assert_eq!(bench.release(10_000, 0, CLICK), []);
    assert_eq!(bench.engine.pending(), 0);
}

#[test]
fn with_long_press_bound_click_fires_on_a_release_below_the_threshold() {
    for (hold, expected) in [
        (0, Gesture::Click),
        (DEFAULT_LONG_PRESS_MS - 1, Gesture::Click),
        (DEFAULT_LONG_PRESS_MS, Gesture::LongPress),
        (DEFAULT_LONG_PRESS_MS + 1, Gesture::LongPress),
    ] {
        let mut bench = Bench::new();
        let hold = u64::from(hold);
        assert_eq!(bench.press(100, 3, CLICK_LONG), []);
        assert_eq!(
            bench.deadline(),
            Some(100 + u64::from(DEFAULT_LONG_PRESS_MS))
        );
        // No advance: the release itself finds the passed threshold.
        let out = bench.release(100 + hold, 3, CLICK_LONG);
        let at = if expected == Gesture::LongPress {
            100 + u64::from(DEFAULT_LONG_PRESS_MS)
        } else {
            100 + hold
        };
        assert_eq!(out, [fired(3, expected, at)], "hold {hold} ms");
        assert_eq!(bench.engine.pending(), 0);
    }
}

#[test]
fn a_long_press_fires_once_at_the_threshold_and_its_release_never_clicks() {
    let mut bench = Bench::new();
    bench.press(0, 1, ALL);
    assert_eq!(bench.advance(699), []);
    assert_eq!(bench.advance(700), [fired(1, Gesture::LongPress, 700)]);
    assert_eq!(bench.deadline(), None, "one long press per hold");
    assert_eq!(bench.advance(1400), []);
    assert_eq!(bench.advance(5000), []);
    assert_eq!(bench.release(5000, 1, ALL), [], "the release is swallowed");
    assert_eq!(bench.advance(6000), []);
    assert_eq!(bench.engine.pending(), 0);
}

#[test]
fn a_long_hold_is_a_click_when_long_press_is_not_bound() {
    let mut bench = Bench::new();
    bench.press(0, 0, CLICK_DOUBLE);
    assert_eq!(bench.deadline(), None, "no long-press deadline");
    assert_eq!(bench.advance(5000), []);
    assert_eq!(bench.release(5000, 0, CLICK_DOUBLE), []);
    assert_eq!(bench.advance(5301), [fired(0, Gesture::Click, 5300)]);

    // Click alone with nothing else: the press fired already.
    let long_only = BoundGestures {
        long_press: true,
        ..BoundGestures::default()
    };
    let mut bench = Bench::new();
    bench.press(0, 0, long_only);
    assert_eq!(bench.release(200, 0, long_only), [], "click is not bound");
    assert_eq!(bench.engine.pending(), 0);
}

#[test]
fn the_double_click_window_is_measured_from_the_first_release() {
    let window = u64::from(DEFAULT_DOUBLE_CLICK_MS);
    for (gap, double) in [
        (0, true),
        (window - 1, true),
        (window, true),
        (window + 1, false),
    ] {
        let mut bench = Bench::new();
        bench.press(0, 2, CLICK_DOUBLE);
        assert_eq!(bench.release(80, 2, CLICK_DOUBLE), [], "single click waits");
        assert_eq!(bench.deadline(), Some(80 + window));
        let out = bench.press(80 + gap, 2, CLICK_DOUBLE);
        if double {
            assert_eq!(out, [fired(2, Gesture::DoubleClick, 80 + gap)], "gap {gap}");
            assert_eq!(bench.release(90 + gap, 2, CLICK_DOUBLE), []);
            assert_eq!(bench.advance(5000), []);
            assert_eq!(bench.engine.pending(), 0);
        } else {
            // The first sequence ends as a click; the press starts a new one.
            assert_eq!(out, [fired(2, Gesture::Click, 80 + window)], "gap {gap}");
            assert_eq!(bench.engine.pending(), 1);
        }
    }
}

#[test]
fn the_window_ends_in_a_click_without_a_second_press() {
    let mut bench = Bench::new();
    bench.press(0, 0, CLICK_DOUBLE);
    bench.release(50, 0, CLICK_DOUBLE);
    assert_eq!(bench.advance(349), []);
    assert_eq!(
        bench.advance(350),
        [],
        "the window includes its last instant"
    );
    assert_eq!(bench.advance(351), [fired(0, Gesture::Click, 350)]);
    assert_eq!(bench.engine.pending(), 0);
    assert_eq!(bench.deadline(), None);
}

#[test]
fn double_click_fires_on_the_second_press_and_nothing_follows() {
    let mut bench = Bench::new();
    bench.press(0, 4, ALL);
    bench.release(100, 4, ALL);
    assert_eq!(
        bench.press(250, 4, ALL),
        [fired(4, Gesture::DoubleClick, 250)]
    );
    // Holding the second press past the long-press time does nothing.
    assert_eq!(bench.deadline(), None);
    assert_eq!(bench.advance(5000), []);
    assert_eq!(bench.release(5000, 4, ALL), []);
    assert_eq!(bench.engine.pending(), 0);
}

#[test]
fn double_click_alone_ignores_single_clicks() {
    let double = BoundGestures {
        double_click: true,
        ..BoundGestures::default()
    };
    let mut bench = Bench::new();
    bench.press(0, 0, double);
    bench.release(50, 0, double);
    assert_eq!(bench.advance(1000), []);
    assert_eq!(bench.engine.pending(), 0);
    bench.press(2000, 0, double);
    bench.release(2050, 0, double);
    assert_eq!(
        bench.press(2100, 0, double),
        [fired(0, Gesture::DoubleClick, 2100)]
    );
}

#[test]
fn per_key_timings_are_used() {
    let mut bench = Bench::new();
    let context = KeyContext {
        bound: ALL,
        timings: GestureTimings {
            double_click_ms: 150,
            long_press_ms: 1200,
        },
        page: None,
        generation: 1,
    };
    bench.input(0, &device("A"), 0, ControlInput::Press, context);
    assert_eq!(bench.deadline(), Some(1200));
    bench.input(1199, &device("A"), 0, ControlInput::Release, context);
    assert_eq!(bench.deadline(), Some(1349));
    assert_eq!(bench.advance(1350), [fired(0, Gesture::Click, 1349)]);
}

#[test]
fn the_context_is_captured_at_the_first_press() {
    let mut bench = Bench::new();
    let first = KeyContext {
        page: Some(crate::PageId::new()),
        generation: 7,
        ..ctx(CLICK_LONG)
    };
    let at = bench.t(0);
    bench
        .engine
        .on_input(at, &device("A"), 1, control(0), ControlInput::Press, || {
            first
        });
    // The release brings a different (raw) context: the sequence keeps its own.
    let out = bench.engine.on_input(
        bench.t(100),
        &device("A"),
        1,
        control(0),
        ControlInput::Release,
        || KeyContext::raw(None, 8),
    );
    let [GestureOutput::Fired(fired)] = out.as_slice() else {
        panic!("expected one gesture, got {out:?}");
    };
    assert_eq!(fired.gesture, Gesture::Click);
    assert_eq!(fired.page, first.page);
    assert_eq!(fired.generation, 7);
    assert_eq!(fired.pressed_at, at);
    assert_eq!(fired.connection, 1);
    assert_eq!(fired.device, device("A"));
}

#[test]
fn a_release_without_a_sequence_is_dropped_in_gesture_mode() {
    let mut bench = Bench::new();
    assert_eq!(bench.release(0, 0, ALL), []);
    assert_eq!(bench.engine.pending(), 0);
}

#[test]
fn impossible_edges_are_ignored() {
    let mut bench = Bench::new();
    bench.press(0, 0, ALL);
    assert_eq!(bench.press(10, 0, ALL), [], "press while down");
    bench.release(20, 0, ALL);
    assert_eq!(bench.release(30, 0, ALL), [], "release while up");
    assert_eq!(bench.advance(321), [fired(0, Gesture::Click, 320)]);
}

#[test]
fn disconnect_cancels_holds_and_open_windows() {
    let mut bench = Bench::new();
    bench.press(0, 0, ALL); // held
    bench.press(0, 1, CLICK_DOUBLE);
    bench.release(10, 1, CLICK_DOUBLE); // within the window
    assert_eq!(bench.engine.pending(), 2);
    bench
        .engine
        .cancel(&device("A"), GestureCancel::Disconnected);
    assert_eq!(bench.engine.pending(), 0);
    assert_eq!(bench.deadline(), None);
    assert_eq!(bench.advance(10_000), []);
    // A release from the old hold arrives late: nothing fires.
    assert_eq!(bench.release(10_000, 0, ALL), []);
}

#[test]
fn a_new_connection_does_not_continue_an_old_sequence() {
    let mut bench = Bench::new();
    let a = device("A");
    let context = ctx(CLICK_LONG);
    bench
        .engine
        .on_input(bench.t(0), &a, 1, control(0), ControlInput::Press, || {
            context
        });
    // The device came back before the old connection's end was handled.
    let out = bench.engine.on_input(
        bench.t(100),
        &a,
        2,
        control(0),
        ControlInput::Release,
        || context,
    );
    assert_eq!(
        out,
        [],
        "the release belongs to no sequence of connection 2"
    );
    assert_eq!(bench.engine.pending(), 0, "connection 1 was forgotten");
    assert_eq!(bench.advance(10_000), []);
}

#[test]
fn a_page_change_swallows_held_keys_and_drops_open_windows() {
    for reason in [
        GestureCancel::PageChanged,
        GestureCancel::ProfileChanged,
        GestureCancel::Locked,
    ] {
        let mut bench = Bench::new();
        bench.press(0, 0, ALL); // held: would long press at 700
        bench.press(0, 1, CLICK_DOUBLE);
        bench.release(10, 1, CLICK_DOUBLE); // would click at 310
        bench.engine.cancel(&device("A"), reason);
        assert_eq!(
            bench.engine.pending(),
            1,
            "{reason:?}: the held key remains"
        );
        assert_eq!(bench.deadline(), None, "{reason:?}");
        assert_eq!(bench.advance(5000), [], "{reason:?}");
        assert_eq!(
            bench.release(5000, 0, ALL),
            [],
            "{reason:?}: release never clicks"
        );
        assert_eq!(bench.engine.pending(), 0);
        // The next sequence works normally.
        bench.press(6000, 0, CLICK);
        assert_eq!(bench.engine.pending(), 1);
    }
}

#[test]
fn cancel_touches_only_the_named_device() {
    let mut bench = Bench::new();
    let (a, b) = (device("A"), device("B"));
    bench.input(0, &a, 0, ControlInput::Press, ctx(CLICK_LONG));
    bench.input(0, &b, 0, ControlInput::Press, ctx(CLICK_LONG));
    bench.engine.cancel(&a, GestureCancel::PageChanged);
    assert_eq!(
        bench.advance(700),
        [Seen::Fired("B".into(), 0, Gesture::LongPress, 700)]
    );
    bench.engine.cancel_all(GestureCancel::Disconnected);
    assert_eq!(bench.engine.pending(), 0);
}

#[test]
fn devices_are_independent() {
    let mut bench = Bench::new();
    let (a, b) = (device("A"), device("B"));
    // The same key on two devices, interleaved.
    bench.input(0, &a, 5, ControlInput::Press, ctx(ALL));
    bench.input(20, &b, 5, ControlInput::Press, ctx(ALL));
    bench.input(60, &a, 5, ControlInput::Release, ctx(ALL));
    let out = bench.input(100, &b, 5, ControlInput::Release, ctx(ALL));
    assert_eq!(out, []);
    let out = bench.input(200, &a, 5, ControlInput::Press, ctx(ALL));
    assert_eq!(out, [Seen::Fired("A".into(), 5, Gesture::DoubleClick, 200)]);
    assert_eq!(
        bench.advance(401),
        [Seen::Fired("B".into(), 5, Gesture::Click, 400)]
    );
    bench.input(210, &a, 5, ControlInput::Release, ctx(ALL));
    assert_eq!(bench.engine.pending(), 0);
}

#[test]
fn an_input_handles_its_devices_passed_deadlines_first() {
    let mut bench = Bench::new();
    let (a, b) = (device("A"), device("B"));
    bench.input(0, &a, 0, ControlInput::Press, ctx(CLICK_LONG)); // long at 700
    bench.input(0, &b, 0, ControlInput::Press, ctx(CLICK_LONG)); // long at 700
    // The service task was busy; device A's next input was read at 900.
    let out = bench.input(
        900,
        &a,
        1,
        ControlInput::Press,
        ctx(BoundGestures::default()),
    );
    assert_eq!(
        out,
        [
            Seen::Fired("A".into(), 0, Gesture::LongPress, 700),
            Seen::Edge("A".into(), 1, ControlInput::Press, 900),
        ]
    );
    // Device B was not advanced by A's input; B's release read at 650
    // (delivered late) is still a click.
    let out = bench.input(650, &b, 0, ControlInput::Release, ctx(CLICK_LONG));
    assert_eq!(out, [Seen::Fired("B".into(), 0, Gesture::Click, 650)]);
}

#[test]
fn a_slow_caller_does_not_turn_a_click_into_a_long_press() {
    // The press and release were read 50 ms apart; the service task gets to
    // them two seconds later. The stamps, not the processing time, count.
    let mut bench = Bench::new();
    assert_eq!(bench.press(0, 0, CLICK_LONG), []);
    assert_eq!(
        bench.release(50, 0, CLICK_LONG),
        [fired(0, Gesture::Click, 50)]
    );
    assert_eq!(bench.advance(2050), []);
}

#[test]
fn deadlines_fire_in_order() {
    let mut bench = Bench::new();
    bench.press(0, 2, CLICK_LONG); // 700
    bench.press(0, 1, CLICK_DOUBLE);
    bench.release(100, 1, CLICK_DOUBLE); // 400
    bench.press(50, 0, CLICK_LONG); // 750
    assert_eq!(bench.deadline(), Some(400));
    assert_eq!(
        bench.advance(800),
        [
            fired(1, Gesture::Click, 400),
            fired(2, Gesture::LongPress, 700),
            fired(0, Gesture::LongPress, 750),
        ]
    );
}

/// A small deterministic generator, so the property test needs no crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const DEVICES: [&str; 2] = ["A", "B"];
const KEYS: u16 = 4;
const SLOTS: usize = DEVICES.len() * KEYS as usize;

#[derive(Default, Clone)]
struct Track {
    down: bool,
    /// Gestures fired in the current sequence.
    fired: usize,
}

/// Random sequences on two devices with four keys each, checked as they
/// go.
struct Property {
    rng: Rng,
    bench: Bench,
    connection: [u64; 2],
    contexts: Vec<KeyContext>,
    tracks: Vec<Track>,
}

impl Property {
    fn new(seed: u64) -> Self {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let contexts = (0..SLOTS)
            .map(|_| {
                let bound = BoundGestures {
                    click: rng.chance(70),
                    double_click: rng.chance(50),
                    long_press: rng.chance(50),
                };
                KeyContext {
                    bound: if bound.any() { bound } else { CLICK },
                    timings: GestureTimings {
                        double_click_ms: 150 + u16::try_from(rng.below(851)).unwrap(),
                        long_press_ms: 300 + u16::try_from(rng.below(4701)).unwrap(),
                    },
                    page: None,
                    generation: 1,
                }
            })
            .collect();
        Self {
            rng,
            bench: Bench::new(),
            connection: [1; 2],
            contexts,
            tracks: vec![Track::default(); SLOTS],
        }
    }

    fn slot(d: usize, k: u16) -> usize {
        d * usize::from(KEYS) + usize::from(k)
    }

    /// One random step at `now`: a cancel or an edge of a random key.
    fn step(&mut self, now: u64) {
        let at = self.bench.t(now);
        // The caller feeds inputs read up to `now`, then advances.
        let outputs = self.bench.engine.advance(at);
        self.check(&outputs);
        let d = usize::try_from(self.rng.below(2)).unwrap();
        if self.rng.chance(5) {
            let reason = [
                GestureCancel::Disconnected,
                GestureCancel::PageChanged,
                GestureCancel::ProfileChanged,
                GestureCancel::Locked,
            ][usize::try_from(self.rng.below(4)).unwrap()];
            self.bench.engine.cancel(&device(DEVICES[d]), reason);
            if reason == GestureCancel::Disconnected {
                // The device reconnects with every key up.
                for k in 0..KEYS {
                    self.tracks[Self::slot(d, k)].down = false;
                }
                self.connection[d] += 1;
            }
        } else {
            let k = u16::try_from(self.rng.below(u64::from(KEYS))).unwrap();
            let slot = Self::slot(d, k);
            // Mostly real edges, sometimes a repeated one.
            let repeat = self.rng.chance(3);
            let pressed = if repeat {
                self.tracks[slot].down
            } else {
                !self.tracks[slot].down
            };
            self.input(at, d, k, pressed);
        }
        self.close_idle();
        assert!(self.bench.engine.pending() <= SLOTS, "bounded state");
    }

    fn input(&mut self, at: Instant, d: usize, k: u16, pressed: bool) {
        let slot = Self::slot(d, k);
        self.tracks[slot].down = pressed;
        let input = if pressed {
            ControlInput::Press
        } else {
            ControlInput::Release
        };
        self.close_idle();
        let context = self.contexts[slot];
        let outputs = self.bench.engine.on_input(
            at,
            &device(DEVICES[d]),
            self.connection[d],
            control(k),
            input,
            || context,
        );
        self.check(&outputs);
    }

    /// Releases every key and passes every deadline.
    fn finish(&mut self, now: u64) {
        for d in 0..DEVICES.len() {
            for k in 0..KEYS {
                if self.tracks[Self::slot(d, k)].down {
                    self.input(self.bench.t(now), d, k, false);
                }
            }
        }
        let outputs = self.bench.engine.advance(self.bench.t(now + 10_000));
        self.check(&outputs);
        self.close_idle();
    }

    fn check(&mut self, outputs: &[GestureOutput]) {
        for output in outputs {
            let GestureOutput::Fired(fired) = output else {
                panic!("gesture-mode keys never pass edges: {output:?}");
            };
            let d = DEVICES
                .iter()
                .position(|s| *s == fired.device.serial)
                .unwrap();
            let slot = Self::slot(d, index(fired.control));
            assert!(
                self.contexts[slot].bound.has(fired.gesture),
                "{:?} fired but is not bound",
                fired.gesture
            );
            let track = &mut self.tracks[slot];
            track.fired += 1;
            assert_eq!(track.fired, 1, "more than one gesture in one sequence");
        }
    }

    /// Ends the sequences of keys the engine has forgotten.
    fn close_idle(&mut self) {
        for (d, serial) in DEVICES.iter().enumerate() {
            for k in 0..KEYS {
                let idle =
                    self.bench
                        .engine
                        .is_idle(&device(serial), self.connection[d], control(k));
                if idle {
                    self.tracks[Self::slot(d, k)].fired = 0;
                }
            }
        }
    }
}

#[test]
fn every_sequence_resolves_to_at_most_one_gesture() {
    for seed in 1..=200_u64 {
        let mut property = Property::new(seed);
        let mut now = 0_u64;
        for _ in 0..400 {
            now += property.rng.below(1500);
            property.step(now);
        }
        property.finish(now + 10_000);
        let engine = &property.bench.engine;
        assert_eq!(engine.pending(), 0, "seed {seed}: everything ends idle");
        assert_eq!(engine.next_deadline(), None);
    }
}

#[derive(Default)]
struct Sequence {
    active: bool,
    fired: usize,
}

impl Sequence {
    /// Counts `fired` and checks the sequence once the key is idle.
    /// Returns 1 when a sequence ended.
    fn settle(&mut self, bench: &Bench, fired: usize, bound: BoundGestures) -> usize {
        self.fired += fired;
        if bench.engine.is_idle(&device("A"), 1, control(0)) {
            let ended = self.active || self.fired > 0;
            if ended {
                assert_eq!(self.fired, 1, "{bound:?}");
            }
            *self = Self::default();
            usize::from(ended)
        } else {
            self.active = true;
            0
        }
    }
}

#[test]
fn an_uncancelled_sequence_with_click_bound_fires_exactly_once() {
    // With click bound, every way a sequence can end produces one gesture.
    let mut rng = Rng(0x5EED);
    let mut ended = 0;
    for _ in 0..500 {
        let bound = BoundGestures {
            click: true,
            double_click: rng.chance(50),
            long_press: rng.chance(50),
        };
        let mut bench = Bench::new();
        let mut seq = Sequence::default();
        let mut now = 0;
        for _ in 0..=rng.below(3) {
            now += rng.below(600);
            let out = bench.advance(now);
            ended += seq.settle(&bench, out.len(), bound);
            let out = bench.press(now, 0, bound);
            ended += seq.settle(&bench, out.len(), bound);
            now += rng.below(1500);
            let out = bench.advance(now);
            ended += seq.settle(&bench, out.len(), bound);
            let out = bench.release(now, 0, bound);
            ended += seq.settle(&bench, out.len(), bound);
        }
        let out = bench.advance(now + 5000);
        ended += seq.settle(&bench, out.len(), bound);
        assert!(!seq.active, "every sequence ended");
    }
    assert!(ended >= 500, "sequences were checked");
}
