//! Key gestures end to end: simulated MK.2s, the device threads and the
//! service task. The engine's exact timing is tested in the core with
//! synthetic instants; these tests check the wiring with real time and
//! generous margins.
//!
//! Layouts cannot bind gestures yet, so the hooks put keys 0 to 3 in
//! gesture mode on every page and record what fires. Key 5 stays raw: on
//! the home page it opens a folder, in the folder it goes back.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_agent::GestureHooks;
use deckpilot_core::gesture::{BoundGestures, Gesture, GestureFired, GestureTimings};
use deckpilot_core::{
    BindingTarget, ControlBinding, DeviceLayout, LayoutPage, PageId, Profile, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use support::{TIMEOUT, TempDir, TestAgent, data_dir};

/// Click on the press.
const CLICK_KEY: u16 = 0;
/// Click and a 500 ms long press.
const LONG_KEY: u16 = 1;
/// Click and a 1 s double click.
const DOUBLE_KEY: u16 = 2;
/// Click and a 5 s long press: a hold that surely outlasts what a test
/// does meanwhile.
const HOLD_KEY: u16 = 3;
/// Raw: opens the folder, and goes back from it.
const FOLDER_KEY: u16 = 5;

const LONG_MS: u16 = 500;
const DOUBLE_MS: u16 = 1000;

/// What fired, as the hooks saw it on the service task.
#[derive(Debug, Clone)]
struct Seen {
    serial: String,
    key: u16,
    gesture: Gesture,
    page: Option<PageId>,
    thread: String,
    held: Duration,
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
    /// Pages seen when a gesture key's sequence could start, by device
    /// serial.
    pages: Mutex<Vec<(String, u16, Option<PageId>)>>,
    /// How long the hook blocks the service task on the next click of
    /// `CLICK_KEY`, as a slow synchronous action would.
    block_next: Mutex<Option<Duration>>,
    blocking: Mutex<bool>,
}

impl GestureHooks for Recorder {
    fn bindings(
        &self,
        device: &DeviceId,
        page: Option<PageId>,
        control: ControlId,
    ) -> Option<(BoundGestures, GestureTimings)> {
        let ControlId::Key { index } = control else {
            return None;
        };
        let bound = match index {
            CLICK_KEY => BoundGestures {
                click: true,
                ..BoundGestures::default()
            },
            LONG_KEY | HOLD_KEY => BoundGestures {
                click: true,
                long_press: true,
                ..BoundGestures::default()
            },
            DOUBLE_KEY => BoundGestures {
                click: true,
                double_click: true,
                ..BoundGestures::default()
            },
            _ => return None,
        };
        let long_press_ms = if index == HOLD_KEY { 5000 } else { LONG_MS };
        self.pages
            .lock()
            .unwrap()
            .push((device.serial.clone(), index, page));
        Some((
            bound,
            GestureTimings {
                double_click_ms: DOUBLE_MS,
                long_press_ms,
            },
        ))
    }

    fn fired(&self, fired: &GestureFired) {
        let ControlId::Key { index } = fired.control else {
            panic!("only keys have gestures here");
        };
        self.seen.lock().unwrap().push(Seen {
            serial: fired.device.serial.clone(),
            key: index,
            gesture: fired.gesture,
            page: fired.page,
            thread: std::thread::current().name().unwrap_or("").to_owned(),
            held: fired.at - fired.pressed_at,
        });
        let block = if index == CLICK_KEY {
            self.block_next.lock().unwrap().take()
        } else {
            None
        };
        if let Some(block) = block {
            *self.blocking.lock().unwrap() = true;
            std::thread::sleep(block);
            *self.blocking.lock().unwrap() = false;
        }
    }
}

impl Recorder {
    fn seen(&self) -> Vec<(String, u16, Gesture)> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|s| (s.serial.clone(), s.key, s.gesture))
            .collect()
    }

    fn presses(&self) -> usize {
        self.pages.lock().unwrap().len()
    }

    fn last_page(&self) -> Option<PageId> {
        self.pages.lock().unwrap().last().and_then(|p| p.2)
    }
}

struct Setup {
    agent: TestAgent,
    profile: deckpilot_core::ProfileId,
    hooks: Arc<Recorder>,
    home: PageId,
    folder: PageId,
    _dir: TempDir,
}

fn profile() -> (Profile, PageId, PageId) {
    let mut profile = Profile::new("Gestures");
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let home = layout.home_page;
    let mut folder = LayoutPage::new("Folder");
    folder.parent = Some(home);
    let folder_id = folder.id;
    folder.bindings.push(ControlBinding::new(
        key(FOLDER_KEY),
        Trigger::Press,
        BindingTarget::Back,
    ));
    layout.pages[0].bindings.push(ControlBinding::new(
        key(FOLDER_KEY),
        Trigger::Press,
        BindingTarget::OpenPage { page: folder_id },
    ));
    layout.pages.push(folder);
    profile.layouts.push(layout);
    (profile, home, folder_id)
}

async fn setup(serials: &[&str]) -> (Setup, Vec<DeviceId>) {
    let dir = TempDir::new("deckpilot-gestures");
    let (profile, home, folder) = profile();
    let (paths, db_key) = data_dir(&dir, &profile, &[]);
    let mock = MockBackend::new();
    let devices: Vec<DeviceId> = serials
        .iter()
        .map(|serial| mock.plug(&models::stream_deck_mk2(), serial))
        .collect();
    let hooks = Arc::new(Recorder::default());
    let agent = TestAgent::start_with_gestures(&paths, &db_key, &mock, None, Some(hooks.clone()));
    agent.unlock().await;
    let setup = Setup {
        agent,
        profile: profile.id,
        hooks,
        home,
        folder,
        _dir: dir,
    };
    for device in &devices {
        setup.wait_connected(device, true).await;
    }
    (setup, devices)
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

impl Setup {
    fn press(&self, device: &DeviceId, index: u16) {
        self.agent
            .mock
            .send_input(device, key(index), ControlInput::Press);
    }

    fn release(&self, device: &DeviceId, index: u16) {
        self.agent
            .mock
            .send_input(device, key(index), ControlInput::Release);
    }

    fn tap(&self, device: &DeviceId, index: u16) {
        self.agent.mock.tap(device, key(index));
    }

    /// Waits until `predicate` holds for what fired.
    async fn wait_for(&self, what: &str, predicate: impl Fn(&[(String, u16, Gesture)]) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let seen = self.hooks.seen();
            if predicate(&seen) {
                return;
            }
            assert!(Instant::now() < deadline, "{what}: saw {seen:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until the agent has looked up `count` gesture-key presses.
    async fn wait_presses(&self, count: usize) {
        let deadline = Instant::now() + TIMEOUT;
        while self.hooks.presses() < count {
            assert!(Instant::now() < deadline, "the presses were not handled");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn wait_connected(&self, device: &DeviceId, connected: bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let status = self.agent.status().await;
            let is = status
                .devices
                .iter()
                .any(|d| d.id == *device && d.state == ConnectionState::Connected);
            if is == connected {
                return;
            }
            assert!(Instant::now() < deadline, "{device} connected: {is}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Lets every pending window and threshold pass, then checks that
    /// exactly `expected` fired.
    async fn settle_and_expect(&self, expected: &[(String, u16, Gesture)]) {
        tokio::time::sleep(Duration::from_millis(u64::from(DOUBLE_MS) + 500)).await;
        assert_eq!(self.hooks.seen(), expected);
    }
}

fn g(serial: &str, key: u16, gesture: Gesture) -> (String, u16, Gesture) {
    (serial.to_owned(), key, gesture)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn click_double_click_and_long_press_fire_on_the_service_task() {
    let (setup, devices) = setup(&["G-1"]).await;
    let d = &devices[0];

    // Only a click bound: it fires on the press, while the key is down.
    setup.press(d, CLICK_KEY);
    setup
        .wait_for("click on the press", |s| {
            s == [g("G-1", CLICK_KEY, Gesture::Click)]
        })
        .await;
    setup.release(d, CLICK_KEY);

    // A long press fires while the key is held, once; the release is quiet.
    setup.press(d, LONG_KEY);
    setup
        .wait_for("long press", |s| {
            s.len() == 2 && s[1] == g("G-1", LONG_KEY, Gesture::LongPress)
        })
        .await;
    tokio::time::sleep(Duration::from_millis(u64::from(LONG_MS) * 3)).await;
    setup.release(d, LONG_KEY);

    // A quick tap of the same key is a click.
    setup.tap(d, LONG_KEY);
    // Two taps within the window are one double click.
    setup.tap(d, DOUBLE_KEY);
    setup.tap(d, DOUBLE_KEY);
    // One tap is a click once the window has passed.
    setup
        .wait_for("click and double click", |s| s.len() >= 4)
        .await;
    tokio::time::sleep(Duration::from_millis(u64::from(DOUBLE_MS) + 300)).await;
    setup.tap(d, DOUBLE_KEY);
    setup
        .settle_and_expect(&[
            g("G-1", CLICK_KEY, Gesture::Click),
            g("G-1", LONG_KEY, Gesture::LongPress),
            g("G-1", LONG_KEY, Gesture::Click),
            g("G-1", DOUBLE_KEY, Gesture::DoubleClick),
            g("G-1", DOUBLE_KEY, Gesture::Click),
        ])
        .await;

    let seen = setup.hooks.seen.lock().unwrap().clone();
    for s in &seen {
        assert!(
            !s.thread.starts_with("device-"),
            "{:?} dispatched on the device thread {}",
            s.gesture,
            s.thread
        );
        assert_eq!(s.page, Some(setup.home));
    }
    let long = &seen[1];
    assert_eq!(
        long.held,
        Duration::from_millis(LONG_MS.into()),
        "at the threshold"
    );
    setup.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_page_change_drops_pending_gestures_and_raw_keys_act_on_the_press() {
    let (setup, devices) = setup(&["G-1"]).await;
    let d = &devices[0];

    // A click waiting for its double-click window; then a raw key opens
    // the folder, at once.
    setup.tap(d, DOUBLE_KEY);
    setup.tap(d, FOLDER_KEY);
    setup.press(d, CLICK_KEY);
    setup
        .wait_for("click in the folder", |s| !s.is_empty())
        .await;
    setup.release(d, CLICK_KEY);
    assert_eq!(
        setup.hooks.last_page(),
        Some(setup.folder),
        "the raw key navigated"
    );

    // A key held while the page changes neither clicks on release nor
    // reaches its long press.
    setup.press(d, HOLD_KEY);
    setup.wait_presses(3).await;
    setup.tap(d, FOLDER_KEY); // back home
    setup.release(d, HOLD_KEY);
    setup
        .settle_and_expect(&[g("G-1", CLICK_KEY, Gesture::Click)])
        .await;

    // A reload of the profile drops pending gestures too.
    setup.tap(d, DOUBLE_KEY);
    setup.wait_presses(4).await;
    setup.agent.edit_profile(setup.profile, |_| {}).await;
    setup
        .settle_and_expect(&[g("G-1", CLICK_KEY, Gesture::Click)])
        .await;
    setup.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnect_cancels_pending_gestures() {
    let (setup, devices) = setup(&["G-1"]).await;
    let d = &devices[0];

    // One key held, one waiting for its double-click window.
    setup.press(d, HOLD_KEY);
    setup.tap(d, DOUBLE_KEY);
    setup.wait_presses(2).await;
    setup.agent.mock.unplug(d);
    setup.wait_connected(d, false).await;
    setup.agent.mock.plug(&models::stream_deck_mk2(), "G-1");
    setup.wait_connected(d, true).await;
    // The new connection does not continue the old hold.
    setup.release(d, HOLD_KEY);
    setup.settle_and_expect(&[]).await;

    // The device works normally afterwards.
    setup.tap(d, LONG_KEY);
    setup
        .wait_for("click after reconnecting", |s| {
            s == [g("G-1", LONG_KEY, Gesture::Click)]
        })
        .await;
    setup.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_devices_are_independent() {
    let (setup, devices) = setup(&["G-A", "G-B"]).await;
    let (a, b) = (&devices[0], &devices[1]);

    // The same key: held on A, tapped on B, tapped twice on B's double key
    // while A's double key waits for its window.
    setup.press(a, LONG_KEY);
    setup.tap(b, LONG_KEY);
    setup.tap(a, DOUBLE_KEY);
    setup.tap(b, DOUBLE_KEY);
    setup.tap(b, DOUBLE_KEY);
    setup
        .wait_for("A's long press", |s| {
            s.contains(&g("G-A", LONG_KEY, Gesture::LongPress))
        })
        .await;
    setup.release(a, LONG_KEY);
    // B navigates; A's pending click is not affected.
    setup.tap(b, FOLDER_KEY);
    tokio::time::sleep(Duration::from_millis(u64::from(DOUBLE_MS) + 500)).await;

    let mut by_device: BTreeMap<String, Vec<(u16, Gesture)>> = BTreeMap::new();
    for (serial, key, gesture) in setup.hooks.seen() {
        by_device.entry(serial).or_default().push((key, gesture));
    }
    assert_eq!(
        by_device["G-A"],
        [(LONG_KEY, Gesture::LongPress), (DOUBLE_KEY, Gesture::Click)]
    );
    assert_eq!(
        by_device["G-B"],
        [
            (LONG_KEY, Gesture::Click),
            (DOUBLE_KEY, Gesture::DoubleClick)
        ]
    );

    // Unplugging B leaves A's pending gestures alone.
    setup.tap(a, DOUBLE_KEY);
    setup.press(b, HOLD_KEY);
    setup.wait_presses(6).await;
    setup.agent.mock.unplug(b);
    setup.wait_connected(b, false).await;
    setup
        .wait_for("A's click after B left", |s| {
            s.iter()
                .filter(|x| **x == g("G-A", DOUBLE_KEY, Gesture::Click))
                .count()
                == 2
        })
        .await;
    assert_eq!(setup.hooks.seen().len(), 5, "B's hold fired nothing");
    setup.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_dispatch_does_not_turn_a_click_into_a_long_press() {
    let (setup, devices) = setup(&["G-1"]).await;
    let d = &devices[0];
    // The next click blocks the service task for well over the long-press
    // time, as a slow synchronous action does.
    *setup.hooks.block_next.lock().unwrap() = Some(Duration::from_millis(u64::from(LONG_MS) * 4));
    // The press of a quick tap is handled...
    setup.press(d, LONG_KEY);
    setup.wait_presses(1).await;
    // ...then another key's slow click blocks the task, and the tap's
    // release is read meanwhile, well below the threshold.
    setup.tap(d, CLICK_KEY);
    setup.release(d, LONG_KEY);
    let deadline = Instant::now() + TIMEOUT;
    while !*setup.hooks.blocking.lock().unwrap() {
        assert!(Instant::now() < deadline, "the click did not block");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    setup.wait_for("the tap", |s| s.len() == 2).await;
    tokio::time::sleep(Duration::from_millis(u64::from(LONG_MS) * 2)).await;
    assert_eq!(
        setup.hooks.seen(),
        [
            g("G-1", CLICK_KEY, Gesture::Click),
            g("G-1", LONG_KEY, Gesture::Click)
        ]
    );
    setup.agent.shutdown().await;
}
