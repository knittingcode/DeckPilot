//! Scrolling (marquee) key titles on a simulated Stream Deck MK.2
//! (`MockBackend`, temporary data; a Stream Deck plugged into the machine is
//! never opened): only keys of the shown page whose title is too wide
//! scroll, at most 15 frames a second; switching pages, disconnecting and
//! locking stop them; going back and reconnecting start them again
//! (`docs/key-text.md`).

mod support;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, KeyAppearance, LayoutPage,
    ParameterValue, Profile, TextAppearance, TextMode, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceId, models};
use deckpilot_ipc::Request;
use deckpilot_modules::general;
use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn log_action(profile: &mut Profile, name: &str) -> BindingTarget {
    let action = support::registry()
        .create_instance(
            name,
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text(name.into())),
        )
        .unwrap();
    let target = BindingTarget::action(action.id);
    profile.actions.push(action);
    target
}

fn marquee() -> KeyAppearance {
    KeyAppearance {
        text: Some(TextAppearance {
            mode: TextMode::Marquee,
            speed: Some(90),
            pause: Some(150),
            ..TextAppearance::default()
        }),
        ..KeyAppearance::default()
    }
}

/// Home: key 1 scrolls ("Deploy the staging environment"), key 2 is a
/// marquee title that fits ("Mail"), key 3 a wrapped long title, key 5
/// opens a folder whose key 4 is plain (key 0 becomes its automatic Back).
fn profile() -> Profile {
    let mut profile = Profile::new("Work");
    let deploy = log_action(&mut profile, "Deploy");
    let mail = log_action(&mut profile, "Mail");
    let wrapped = log_action(&mut profile, "Wrapped");
    let inside = log_action(&mut profile, "Inside");
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let mut folder = LayoutPage::new("Folder");
    folder
        .bindings
        .push(ControlBinding::new(key(4), Trigger::Press, inside));
    layout.pages[0].bindings.extend([
        ControlBinding {
            label: Some("Deploy the staging environment".into()),
            appearance: Some(marquee()),
            ..ControlBinding::new(key(1), Trigger::Press, deploy)
        },
        ControlBinding {
            appearance: Some(marquee()),
            ..ControlBinding::new(key(2), Trigger::Press, mail)
        },
        ControlBinding {
            label: Some("Deploy the staging environment".into()),
            ..ControlBinding::new(key(3), Trigger::Press, wrapped)
        },
        ControlBinding::new(
            key(5),
            Trigger::Press,
            BindingTarget::OpenPage { page: folder.id },
        ),
    ]);
    layout.pages.push(folder);
    profile.layouts.push(layout);
    profile
}

/// The distinct images `control` showed and the image writes to the
/// device during `period`.
fn watch(
    mock: &MockBackend,
    device: &DeviceId,
    control: ControlId,
    period: Duration,
) -> (BTreeSet<Vec<u8>>, usize) {
    let writes = mock.image_writes(device);
    let mut seen = BTreeSet::new();
    let until = Instant::now() + period;
    while Instant::now() < until {
        if let Some(frame) = mock.frames(device).get(&control) {
            seen.insert(frame.rgb().to_vec());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    (seen, mock.image_writes(device) - writes)
}

async fn connected(agent: &TestAgent, connected: bool) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        let is = status
            .devices
            .iter()
            .any(|d| d.state == ConnectionState::Connected && d.page.is_some());
        if is == connected {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{status:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_visible_titles_that_are_too_wide_scroll() {
    let dir = TempDir::new("deckpilot-marquee");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    connected(&agent, true).await;
    assert!(mock.wait_for(&device, TIMEOUT, |frames, _| frames.contains_key(&key(5))));

    // Key 1 scrolls: many different images; the others stay as they are.
    let period = Duration::from_millis(1500);
    let (scrolling, writes) = watch(&mock, &device, key(1), period);
    assert!(scrolling.len() >= 5, "{} images", scrolling.len());
    for still in [key(2), key(3), key(5)] {
        let (images, _) = watch(&mock, &device, still, Duration::from_millis(300));
        assert_eq!(images.len(), 1, "{still:?} does not scroll");
    }
    // At most 15 frames a second, and only for the one key.
    let bound = (period.as_secs_f64() * f64::from(deckpilot_agent::marquee::MARQUEE_FPS)).ceil();
    #[expect(clippy::cast_precision_loss, reason = "a few dozen writes")]
    let measured = writes as f64;
    assert!(measured <= bound + 2.0, "{writes} writes in {period:?}");

    // Another page: nothing scrolls, nothing is written.
    mock.tap(&device, key(5));
    assert!(mock.wait_for(&device, TIMEOUT, |frames, _| frames.contains_key(&key(4))));
    std::thread::sleep(Duration::from_millis(200));
    let (_, writes) = watch(&mock, &device, key(1), Duration::from_millis(600));
    assert_eq!(writes, 0, "no frames on a page without scrolling keys");
    assert!(
        !mock.frames(&device).contains_key(&key(1)),
        "key 1 is blank there"
    );

    // Back: key 1 scrolls again.
    mock.tap(&device, key(0));
    let (scrolling, _) = watch(&mock, &device, key(1), Duration::from_millis(1200));
    assert!(
        scrolling.len() >= 4,
        "{} images after Back",
        scrolling.len()
    );

    // Disconnected: stopped. Reconnected: scrolling again.
    mock.unplug(&device);
    connected(&agent, false).await;
    let writes = mock.image_writes(&device);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        mock.image_writes(&device),
        writes,
        "nothing while disconnected"
    );
    mock.plug(&models::stream_deck_mk2(), "MK2-A");
    connected(&agent, true).await;
    let (scrolling, _) = watch(&mock, &device, key(1), Duration::from_millis(1500));
    assert!(
        scrolling.len() >= 4,
        "{} images after reconnecting",
        scrolling.len()
    );

    // Locked: the keys show the lock notice and stand still.
    agent.request(Request::Lock).await.unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let (images, writes) = watch(&mock, &device, key(1), Duration::from_millis(600));
    assert!(images.len() <= 1, "{} images while locked", images.len());
    assert_eq!(writes, 0, "nothing scrolls while locked");
    agent.shutdown().await;
}
