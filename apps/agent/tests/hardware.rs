//! Device lifecycle in the agent with simulated Stream Deck MK.2s:
//! unplugging and replugging (also while keys are being redrawn), several
//! devices at once, and what the keys show across agent restarts. These
//! check the agent's handling, not the USB driver; no hardware is used.

mod support;

use std::time::Duration;

use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, LayoutPage, Profile, Project,
    ProjectKey, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceBackend, DeviceId, Frame, models};
use deckpilot_ipc::{AgentStatus, Request};
use deckpilot_modules::general;
use deckpilot_modules::projects::{ProjectCatalog, actions};
use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

/// SHOP, CMR and CMS; "Next project" on key 0, "Announce" on key 1, and a
/// "Folder" page opened by key 5 that has "Next project" on key 2.
fn profile() -> Profile {
    let registry = support::registry();
    let mut profile = Profile::new("Work");
    for code in ["SHOP", "CMR", "CMS"] {
        profile
            .add_project(Project::new(ProjectKey::new(code).unwrap(), code))
            .unwrap();
    }
    profile.default_project = Some(profile.projects[0].id);
    let next = registry
        .create_instance("Next project", actions::next_id(), &ActionParameters::new())
        .unwrap();
    let next_id = next.id;
    profile.actions.push(next);
    profile.actions.push(
        registry
            .create_instance(
                "Announce",
                general::log_id(),
                &ActionParameters::new().with(
                    "message",
                    deckpilot_core::ParameterValue::Text("{project.key}".into()),
                ),
            )
            .unwrap(),
    );
    let mut layout = DeviceLayout::generate("Main", &models::stream_deck_mk2(), &profile);
    let mut folder = LayoutPage::new("Folder");
    folder.bindings.push(ControlBinding::new(
        key(2),
        Trigger::Press,
        BindingTarget::action(next_id),
    ));
    layout.pages[0].bindings.push(ControlBinding::new(
        key(5),
        Trigger::Press,
        BindingTarget::OpenPage { page: folder.id },
    ));
    layout.pages.push(folder);
    profile.layouts.push(layout);
    profile
}

async fn wait_status(agent: &TestAgent, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        if let Ok(deckpilot_ipc::Response::Status { status }) =
            agent.request(Request::GetStatus).await
            && predicate(&status)
        {
            return *status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not reached in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn page<'a>(status: &'a AgentStatus, device: &DeviceId) -> Option<&'a str> {
    status
        .devices
        .iter()
        .find(|d| d.id == *device)
        .and_then(|d| d.page.as_deref())
}

fn connected(status: &AgentStatus, device: &DeviceId) -> bool {
    status
        .devices
        .iter()
        .any(|d| d.id == *device && d.state == ConnectionState::Connected)
}

fn project(status: &AgentStatus) -> Option<&str> {
    status.active_project.as_ref().map(|p| p.key.as_str())
}

async fn wait_brightness(mock: &MockBackend, device: &DeviceId, percent: u8) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while mock.brightness(device) != Some(percent) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "brightness {percent} not applied, is {:?}",
            mock.brightness(device)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits until `device` shows exactly `expected` and returns it.
fn wait_frames(
    mock: &MockBackend,
    device: &DeviceId,
    expected: &std::collections::BTreeMap<ControlId, Frame>,
) -> bool {
    mock.wait_for(device, TIMEOUT, |frames, _| frames == expected)
}

/// Waits until the profile's home page (more than the "Locked" notice) is
/// shown and returns its frames.
fn home_frames(
    mock: &MockBackend,
    device: &DeviceId,
) -> std::collections::BTreeMap<ControlId, Frame> {
    assert!(mock.wait_for(device, TIMEOUT, |frames, _| {
        frames.contains_key(&key(0)) && frames.contains_key(&key(1)) && frames.contains_key(&key(5))
    }));
    mock.frames(device)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replugged_device_gets_its_page_brightness_and_images_back() {
    let dir = TempDir::new("deckpilot-hw");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    let home = home_frames(&mock, &device);

    agent
        .request(Request::SetBrightness { percent: 35 })
        .await
        .unwrap();
    wait_brightness(&mock, &device, 35).await;
    mock.tap(&device, key(5));
    wait_status(&agent, |s| page(s, &device) == Some("Folder")).await;
    assert!(
        mock.wait_for(&device, TIMEOUT, |frames, _| frames.contains_key(&key(2))
            && !frames.contains_key(&key(1)))
    );
    let folder = mock.frames(&device);

    mock.unplug(&device);
    wait_status(&agent, |s| !connected(s, &device)).await;
    // Plugging in again starts the device blank, at its default brightness.
    mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let status = wait_status(&agent, |s| connected(s, &device)).await;
    assert_eq!(page(&status, &device), Some("Folder"), "page restored");
    assert!(wait_frames(&mock, &device, &folder), "images restored");
    wait_brightness(&mock, &device, 35).await;

    // Navigation continues from the restored page: the automatic Back key
    // returns home.
    mock.tap(&device, key(0));
    wait_status(&agent, |s| page(s, &device) == Some("Home")).await;
    assert!(wait_frames(&mock, &device, &home));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unplugging_while_keys_are_redrawn_recovers() {
    let dir = TempDir::new("deckpilot-hw");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    home_frames(&mock, &device);

    // Every press of key 0 switches the project and redraws key 0; unplug
    // and replug in the middle of a burst of them.
    for round in 0..5 {
        for _ in 0..4 {
            mock.tap(&device, key(0));
        }
        if round % 2 == 0 {
            mock.unplug(&device);
            tokio::time::sleep(Duration::from_millis(5)).await;
            mock.plug(&models::stream_deck_mk2(), "MK2-A");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let status = wait_status(&agent, |s| connected(s, &device)).await;
    assert_eq!(page(&status, &device), Some("Home"));

    // Once things settle, the device shows the faces of the current state:
    // compare with a second device plugged in now, which is drawn fresh.
    let reference = mock.plug(&models::stream_deck_mk2(), "MK2-REF");
    wait_status(&agent, |s| connected(s, &reference)).await;
    let expected = home_frames(&mock, &reference);
    assert!(
        wait_frames(&mock, &device, &expected),
        "the replugged device shows the current faces"
    );

    // And input works on the latest connection.
    let before = project(&agent.status().await).map(str::to_owned);
    mock.tap(&device, key(0));
    wait_status(&agent, |s| project(s).map(str::to_owned) != before).await;
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn several_devices_have_their_own_pages_and_connections() {
    let dir = TempDir::new("deckpilot-hw");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mock = MockBackend::new();
    let a = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let b = mock.plug(&models::stream_deck_mk2(), "MK2-B");
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    let home = home_frames(&mock, &a);
    assert!(wait_frames(&mock, &b, &home), "both show the same layout");

    agent
        .request(Request::SetBrightness { percent: 60 })
        .await
        .unwrap();
    wait_brightness(&mock, &a, 60).await;
    wait_brightness(&mock, &b, 60).await;

    // Opening the folder on A leaves B on its home page.
    mock.tap(&a, key(5));
    let status = wait_status(&agent, |s| page(s, &a) == Some("Folder")).await;
    assert_eq!(page(&status, &b), Some("Home"));
    assert!(mock.wait_for(&a, TIMEOUT, |frames, _| !frames.contains_key(&key(1))));
    assert_eq!(mock.frames(&b), home, "B is not redrawn with A's page");

    // Key 2 is bound only in the folder: pressing it on B (home, unbound)
    // does nothing, pressing it on A runs the action.
    let start = project(&agent.status().await).map(str::to_owned);
    mock.tap(&b, key(2));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(project(&agent.status().await).map(str::to_owned), start);
    mock.tap(&a, key(2));
    wait_status(&agent, |s| project(s).map(str::to_owned) != start).await;

    // Losing A does not disturb B.
    mock.unplug(&a);
    let status = wait_status(&agent, |s| !connected(s, &a)).await;
    assert!(connected(&status, &b));
    assert_eq!(page(&status, &b), Some("Home"));
    mock.tap(&b, key(5));
    wait_status(&agent, |s| page(s, &b) == Some("Folder")).await;
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_are_blanked_on_shutdown_and_stale_images_replaced_on_start() {
    let dir = TempDir::new("deckpilot-hw");
    let (paths, key_material) = support::data_dir(&dir, &profile(), &[]);
    let mut mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");

    // A previous agent that crashed left an image on every key.
    let mut stale = mock.open(&device).unwrap();
    for index in 0..15 {
        stale
            .set_image(key(index), &Frame::filled(72, 72, [200, 0, 200]))
            .unwrap();
    }
    stale.flush().unwrap();
    drop(stale);
    assert_eq!(mock.frames(&device).len(), 15);

    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    assert!(
        mock.wait_for(&device, TIMEOUT, |frames, _| {
            frames.keys().copied().collect::<Vec<_>>() == vec![key(0), key(1), key(5)]
        }),
        "unbound keys are cleared, got {:?}",
        mock.frames(&device).keys()
    );
    let home = mock.frames(&device);
    let stale_frame = Frame::filled(72, 72, [200, 0, 200]);
    assert!(home.values().all(|frame| *frame != stale_frame));
    agent.shutdown().await;
    assert!(
        mock.frames(&device).is_empty(),
        "a stopped agent leaves the keys blank"
    );

    // A new agent draws the layout again on the same device.
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    assert!(wait_frames(&mock, &device, &home));
    agent.shutdown().await;
}
