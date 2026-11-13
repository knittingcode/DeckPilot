//! Gestures end to end with exact timings (`docs/gestures-and-work-session.md`,
//! sections 2 and 3, workstream T): a real agent with a simulated MK.2
//! whose layout binds click, double click and long press on one key, each
//! to its own binding, next to keys bound to press and release only.
//!
//! Deterministic by construction: a click or a double click is queued as
//! one batch, so its edges share their read time; a long press is held
//! until the agent reports it, then released. The engine times every
//! gesture from those read times, so the delays it reports are exact. A
//! click on a click-only key (it fires on its press) is the barrier
//! before checking that nothing else happened: the device's inputs are
//! handled in order.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_agent::GestureHooks;
use deckpilot_core::gesture::{
    BoundGestures, Gesture, GestureFired, GestureTimings, KeyGestureSettings,
};
use deckpilot_core::{
    ActionInstanceId, ActionParameters, BindingTarget, ControlBinding, DeviceLayout, PageId,
    ParameterValue, Profile, RuntimeOutcome, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::{AgentClient, AgentEvent};
use deckpilot_modules::general;
use support::{TIMEOUT, TempDir, TestAgent, data_dir, registry};

/// Press: "down"; release: "up" (bindings from before gestures).
const EDGES: ControlId = ControlId::Key { index: 0 };
/// Click, double click and long press, with the key's own timings.
const GESTURES: ControlId = ControlId::Key { index: 2 };
/// Click and long press, with this computer's default timings.
const DEFAULTS: ControlId = ControlId::Key { index: 3 };
/// Click only: fires on the press; the barrier.
const MARK: ControlId = ControlId::Key { index: 4 };

const KEY_DOUBLE_MS: u16 = 250;
const KEY_LONG_MS: u16 = 450;
/// This computer's defaults (Settings › Devices), not the built-in ones.
const MACHINE: GestureTimings = GestureTimings {
    double_click_ms: 400,
    long_press_ms: 600,
};

/// Every dispatched gesture with the message its action logged.
#[derive(Default)]
struct Recorder(Mutex<Vec<(GestureFired, String)>>);

impl GestureHooks for Recorder {
    fn bindings(
        &self,
        _device: &DeviceId,
        _page: Option<PageId>,
        _control: ControlId,
    ) -> Option<(BoundGestures, GestureTimings)> {
        None
    }

    fn fired(&self, _fired: &GestureFired) {}

    fn dispatched(&self, fired: &GestureFired, outcome: &RuntimeOutcome) {
        let message = match outcome {
            RuntimeOutcome::Action { result, .. } => result.message.clone(),
            other => format!("{other:?}"),
        };
        self.0.lock().unwrap().push((fired.clone(), message));
    }
}

fn log(profile: &mut Profile, name: &str, message: &str) -> ActionInstanceId {
    let instance = registry()
        .create_instance(
            name,
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text(message.into())),
        )
        .unwrap();
    let id = instance.id;
    profile.actions.push(instance);
    id
}

fn profile() -> Profile {
    let mut profile = Profile::new("Gestures");
    let action =
        |profile: &mut Profile, name: &str| BindingTarget::action(log(profile, name, name));
    let down = action(&mut profile, "down");
    let up = action(&mut profile, "up");
    let click = action(&mut profile, "click");
    let double = action(&mut profile, "double");
    let long = action(&mut profile, "long");
    let tap = action(&mut profile, "tap");
    let hold = action(&mut profile, "hold");
    let mark = action(&mut profile, "mark");
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let page = &mut layout.pages[0];
    page.bindings = vec![
        ControlBinding::new(EDGES, Trigger::Press, down),
        ControlBinding::new(EDGES, Trigger::Release, up),
        ControlBinding::new(GESTURES, Trigger::Click, click),
        ControlBinding::new(GESTURES, Trigger::DoubleClick, double),
        ControlBinding::new(GESTURES, Trigger::LongPress, long),
        ControlBinding::new(DEFAULTS, Trigger::Click, tap),
        ControlBinding::new(DEFAULTS, Trigger::LongPress, hold),
        ControlBinding::new(MARK, Trigger::Click, mark),
    ];
    page.gestures = vec![KeyGestureSettings {
        control: GESTURES,
        double_click_ms: Some(KEY_DOUBLE_MS),
        long_press_ms: Some(KEY_LONG_MS),
    }];
    profile.layouts.push(layout);
    profile
}

struct Setup {
    agent: TestAgent,
    hooks: Arc<Recorder>,
    device: DeviceId,
    /// Gestures checked so far.
    checked: AtomicUsize,
    _dir: TempDir,
}

async fn setup() -> Setup {
    let dir = TempDir::new("deckpilot-gesture-e2e");
    let (paths, key) = data_dir(&dir, &profile(), &[]);
    let store = support::open_store(&paths, &key);
    deckpilot_storage::gestures::set_gesture_defaults(&store, MACHINE).unwrap();
    drop(store);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "GESTURE-E2E");
    let hooks = Arc::new(Recorder::default());
    let agent = TestAgent::start_with_gestures(&paths, &key, &mock, None, Some(hooks.clone()));
    agent.unlock().await;
    let deadline = Instant::now() + TIMEOUT;
    while !agent
        .status()
        .await
        .devices
        .iter()
        .any(|d| d.id == device && d.state == ConnectionState::Connected)
    {
        assert!(Instant::now() < deadline, "the MK.2 did not connect");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Setup {
        agent,
        hooks,
        device,
        checked: AtomicUsize::new(0),
        _dir: dir,
    }
}

impl Setup {
    fn seen(&self) -> Vec<(GestureFired, String)> {
        self.hooks.0.lock().unwrap().clone()
    }

    /// Waits until `count` gestures were dispatched; returns the last.
    async fn dispatched(&self, count: usize) -> (GestureFired, String) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let seen = self.seen();
            if seen.len() >= count {
                return seen[count - 1].clone();
            }
            assert!(Instant::now() < deadline, "no gesture {count}: {seen:#?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Queues `presses` press-release pairs of `key` as one read.
    fn taps(&self, key: ControlId, presses: usize) {
        let edges: Vec<_> = (0..presses)
            .flat_map(|_| [(key, ControlInput::Press), (key, ControlInput::Release)])
            .collect();
        self.agent.mock.send_inputs(&self.device, &edges);
    }

    fn send(&self, key: ControlId, input: ControlInput) {
        self.agent.mock.send_input(&self.device, key, input);
    }

    /// The next gesture: `key`'s `gesture`, which logged `message`, and
    /// how long after the first press it was recognised.
    async fn expect(&self, key: ControlId, gesture: Gesture, message: &str) -> Duration {
        let count = self.checked.fetch_add(1, Ordering::SeqCst) + 1;
        let (fired, logged) = self.dispatched(count).await;
        assert_eq!(
            (fired.control, fired.gesture, logged.as_str()),
            (key, gesture, message),
            "{fired:#?}"
        );
        fired.at - fired.pressed_at
    }

    /// Returns once every input sent so far has been handled, and checks
    /// that nothing but the gestures checked so far was dispatched.
    async fn nothing_more(&self) {
        self.taps(MARK, 1);
        let count = self.checked.fetch_add(1, Ordering::SeqCst) + 1;
        let (fired, logged) = self.dispatched(count).await;
        assert_eq!(
            (fired.control, logged.as_str()),
            (MARK, "mark"),
            "{:#?}",
            self.seen()
        );
        assert_eq!(fired.at, fired.pressed_at, "click-only fires on the press");
    }
}

fn ms(ms: u16) -> Duration {
    Duration::from_millis(ms.into())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_gesture_runs_its_own_binding_with_the_configured_timings() {
    let s = setup().await;

    // One tap: a click, once the key's double-click window has passed.
    s.taps(GESTURES, 1);
    assert_eq!(
        s.expect(GESTURES, Gesture::Click, "click").await,
        ms(KEY_DOUBLE_MS)
    );
    // Two taps: one double click, on the second press, and no click.
    s.taps(GESTURES, 2);
    assert_eq!(
        s.expect(GESTURES, Gesture::DoubleClick, "double").await,
        Duration::ZERO
    );
    s.nothing_more().await;
    // A hold: one long press at the key's threshold, while still down;
    // the release does nothing.
    s.send(GESTURES, ControlInput::Press);
    assert_eq!(
        s.expect(GESTURES, Gesture::LongPress, "long").await,
        ms(KEY_LONG_MS)
    );
    s.send(GESTURES, ControlInput::Release);
    s.nothing_more().await;

    // A key without timings of its own uses this computer's defaults.
    // Its double click is not bound, so a click fires on the release,
    // without waiting.
    s.taps(DEFAULTS, 1);
    assert_eq!(
        s.expect(DEFAULTS, Gesture::Click, "tap").await,
        Duration::ZERO
    );
    s.send(DEFAULTS, ControlInput::Press);
    assert_eq!(
        s.expect(DEFAULTS, Gesture::LongPress, "hold").await,
        ms(MACHINE.long_press_ms)
    );
    s.send(DEFAULTS, ControlInput::Release);
    // A quick second tap on that key is a second click, not a double.
    s.taps(DEFAULTS, 2);
    s.expect(DEFAULTS, Gesture::Click, "tap").await;
    s.expect(DEFAULTS, Gesture::Click, "tap").await;
    s.nothing_more().await;
    s.agent.shutdown().await;
}

/// The next `actionCompleted` event's action name.
async fn next_action(events: &mut AgentClient) -> String {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("an action completed in time")
            .unwrap();
        if let AgentEvent::ActionCompleted { name, .. } = event {
            return name;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn press_and_release_bindings_act_on_each_edge_as_before() {
    let s = setup().await;
    let mut events = s.agent.events().await;

    // The press acts while the key is still down, the release on its own.
    s.send(EDGES, ControlInput::Press);
    assert_eq!(next_action(&mut events).await, "down");
    s.send(EDGES, ControlInput::Release);
    assert_eq!(next_action(&mut events).await, "up");
    // A tap and a quick double tap are edges too: each one acts, in order,
    // and no gesture is recognised.
    s.taps(EDGES, 1);
    s.taps(EDGES, 2);
    for expected in ["down", "up", "down", "up", "down", "up"] {
        assert_eq!(next_action(&mut events).await, expected);
    }
    // Held past every gesture threshold: still nothing but the edges.
    s.send(EDGES, ControlInput::Press);
    assert_eq!(next_action(&mut events).await, "down");
    tokio::time::sleep(ms(MACHINE.long_press_ms) + Duration::from_millis(100)).await;
    s.send(EDGES, ControlInput::Release);
    assert_eq!(next_action(&mut events).await, "up");
    s.nothing_more().await;
    assert_eq!(next_action(&mut events).await, "mark");
    s.agent.shutdown().await;
}
