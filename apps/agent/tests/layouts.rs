//! Shared layouts and layout navigation against a real agent with
//! simulated Stream Deck MK.2s (`MockBackend`) and temporary data: *Switch
//! layout* and *Back to previous layout* per device and profile, a layout
//! two profiles share, what the app's status and the device show, what
//! survives a restart, layouts that are removed or do not fit, and rapid
//! presses. A Stream Deck plugged into the machine is never opened.

mod support;

use std::time::Duration;

use deckpilot_core::{
    ActionParameters, ActionStatus, BindingTarget, ControlBinding, DeviceLayout, FaceStyle,
    KeyFace, LayoutId, LayoutStatus, ParameterValue, Profile, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceId, models};
use deckpilot_ipc::{AgentClient, AgentEvent, AgentStatus, Request};
use deckpilot_modules::{deck, general};
use deckpilot_storage::ConfigStore;
use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

/// Key 0 switches to Main, key 1 to Teams, key 2 to Meeting, key 3 to
/// Dials (a Stream Deck + layout), key 14 goes back; key 5 logs the
/// layout's name.
struct Set {
    main: DeviceLayout,
    teams: DeviceLayout,
    meeting: DeviceLayout,
    dials: DeviceLayout,
    actions: Vec<deckpilot_core::ActionInstance>,
}

fn instance(
    name: &str,
    action_type: deckpilot_core::ActionTypeId,
    parameters: &ActionParameters,
) -> deckpilot_core::ActionInstance {
    support::registry()
        .create_instance(name, action_type, parameters)
        .unwrap()
}

fn switch(name: &str, layout: LayoutId) -> deckpilot_core::ActionInstance {
    instance(
        name,
        deck::switch_layout_id(),
        &ActionParameters::new().with(deck::LAYOUT, ParameterValue::Text(layout.to_string())),
    )
}

fn layout_set() -> Set {
    let mk2 = models::stream_deck_mk2().id;
    let mut main = DeviceLayout::new("Main", mk2.clone());
    let mut teams = DeviceLayout::new("Teams", mk2.clone());
    let mut meeting = DeviceLayout::new("Meeting", mk2);
    let dials = DeviceLayout::new("Dials", models::stream_deck_plus().id);
    let to = [
        switch("To Main", main.id),
        switch("To Teams", teams.id),
        switch("To Meeting", meeting.id),
        switch("To Dials", dials.id),
    ];
    let back = instance(
        "Back to previous layout",
        deck::previous_layout_id(),
        &ActionParameters::new(),
    );
    let mut actions: Vec<_> = to.to_vec();
    actions.push(back.clone());
    for layout in [&mut main, &mut teams, &mut meeting] {
        let name = instance(
            &format!("In {}", layout.name),
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text(layout.name.clone())),
        );
        let home = layout.home_page;
        let page = layout.page_mut(home).unwrap();
        for (index, action) in to.iter().enumerate() {
            page.bindings.push(ControlBinding::new(
                key(u16::try_from(index).unwrap()),
                Trigger::Press,
                BindingTarget::action(action.id),
            ));
        }
        page.bindings.extend([
            ControlBinding::new(key(5), Trigger::Press, BindingTarget::action(name.id)),
            ControlBinding::new(key(14), Trigger::Press, BindingTarget::action(back.id)),
        ]);
        actions.push(name);
    }
    Set {
        main,
        teams,
        meeting,
        dials,
        actions,
    }
}

/// A profile that uses every layout of `set` (Main first, its default).
fn profile(name: &str, set: &Set) -> Profile {
    let mut profile = Profile::new(name);
    profile.actions.clone_from(&set.actions);
    profile.layouts = vec![
        set.main.clone(),
        set.teams.clone(),
        set.meeting.clone(),
        set.dials.clone(),
    ];
    profile.activate_layout(set.main.id).unwrap();
    profile
}

fn shown(status: &AgentStatus, device: &DeviceId) -> Option<String> {
    status
        .devices
        .iter()
        .find(|d| d.id == *device)
        .and_then(|d| match &d.layout {
            LayoutStatus::Compatible { name, .. } | LayoutStatus::Invalid { name, .. } => {
                Some(name.clone())
            }
            _ => None,
        })
}

async fn wait_status(agent: &TestAgent, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if predicate(&status) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() <= deadline,
            "condition not reached in time; devices: {:?}",
            status.devices
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_layout(agent: &TestAgent, device: &DeviceId, name: &str) {
    wait_status(agent, |s| shown(s, device).as_deref() == Some(name)).await;
}

async fn connected(agent: &TestAgent, device: &DeviceId) {
    wait_status(agent, |s| {
        s.devices
            .iter()
            .any(|d| d.id == *device && d.state == ConnectionState::Connected && d.page.is_some())
    })
    .await;
}

/// The next finished action: its name and status.
async fn completed(events: &mut AgentClient) -> (String, ActionStatus) {
    loop {
        let event = tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .expect("an event in time")
            .unwrap();
        if let AgentEvent::ActionCompleted { name, status, .. } = event {
            return (name, status);
        }
    }
}

/// The device shows an action key titled `title` on `index`.
fn shows(mock: &MockBackend, device: &DeviceId, index: u16, title: &str) -> bool {
    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    // The log action's type icon, as keys are drawn icon first.
    let icon = support::registry()
        .definition(&general::log_id())
        .and_then(|definition| definition.node.icon.clone());
    let frame = deckpilot_agent::render::render_with(
        &KeyFace::new(FaceStyle::Action, title).with_symbol(icon),
        spec.width,
        spec.height,
        &deckpilot_agent::render::Icons::default(),
    );
    mock.wait_for(device, TIMEOUT, |frames, _| {
        frames.get(&key(index)) == Some(&frame)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn layouts_switch_and_go_back_per_device_and_profile() {
    let dir = TempDir::new("deckpilot-layouts-switch");
    let set = layout_set();
    let work = profile("Work", &set);
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    // Home uses the same layouts: one record each, shared.
    let home = Profile::new("Home");
    {
        let store = support::open_store(&paths, &db_key);
        store.save_profile(&home).unwrap();
        for layout in [&set.main, &set.teams, &set.meeting] {
            store.assign_layout(home.id, layout.id).unwrap();
        }
        assert_eq!(store.layout_users(set.main.id).unwrap().len(), 2);
    }
    let mock = MockBackend::new();
    let (a, b) = (
        mock.plug(&models::stream_deck_mk2(), "MK2-A"),
        mock.plug(&models::stream_deck_mk2(), "MK2-B"),
    );
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &a).await;
    connected(&agent, &b).await;
    let mut events = agent.events().await;
    wait_layout(&agent, &a, "Main").await;

    // Main → Teams → Meeting on A; B stays on Main.
    mock.tap(&a, key(1));
    wait_layout(&agent, &a, "Teams").await;
    assert_eq!(
        completed(&mut events).await,
        ("To Teams".to_owned(), ActionStatus::Succeeded)
    );
    assert!(shows(&mock, &a, 5, "In Teams"), "the device draws Teams");
    mock.tap(&a, key(2));
    wait_layout(&agent, &a, "Meeting").await;
    assert_eq!(shown(&agent.status().await, &b).as_deref(), Some("Main"));
    // The key of the new layout runs.
    mock.tap(&a, key(5));
    loop {
        if completed(&mut events).await.0 == "In Meeting" {
            break;
        }
    }

    // Back twice: Teams, then Main; then there is nothing to go back to.
    mock.tap(&a, key(14));
    wait_layout(&agent, &a, "Teams").await;
    mock.tap(&a, key(14));
    wait_layout(&agent, &a, "Main").await;
    mock.tap(&a, key(14));
    loop {
        let (name, status) = completed(&mut events).await;
        if name == "Back to previous layout" {
            assert_eq!(status, ActionStatus::Succeeded);
            break;
        }
    }
    assert_eq!(shown(&agent.status().await, &a).as_deref(), Some("Main"));

    // A Stream Deck + layout is refused on an MK.2: nothing changes.
    mock.tap(&a, key(3));
    loop {
        let (name, status) = completed(&mut events).await;
        if name == "To Dials" {
            assert_eq!(status, ActionStatus::Failed);
            break;
        }
    }
    assert_eq!(shown(&agent.status().await, &a).as_deref(), Some("Main"));

    // Work: A on Teams. Home: A starts on Main and goes to Meeting; the
    // layouts are shared, the way back is not.
    mock.tap(&a, key(1));
    wait_layout(&agent, &a, "Teams").await;
    agent
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    wait_layout(&agent, &a, "Main").await;
    mock.tap(&a, key(2));
    wait_layout(&agent, &a, "Meeting").await;
    // Dials is not assigned to Home: refused, the profile stays.
    mock.tap(&a, key(3));
    loop {
        let (name, status) = completed(&mut events).await;
        if name == "To Dials" {
            assert_eq!(status, ActionStatus::Failed);
            break;
        }
    }
    let status = agent.status().await;
    assert_eq!(status.active_profile.unwrap().name, "Home");
    agent
        .request(Request::SetActiveProfile { profile: work.id })
        .await
        .unwrap();
    wait_layout(&agent, &a, "Teams").await;
    mock.tap(&a, key(14));
    wait_layout(&agent, &a, "Main").await;
    assert_eq!(agent.status().await.active_profile.unwrap().name, "Work");
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_layout_survives_a_restart_and_follows_the_app() {
    let dir = TempDir::new("deckpilot-layouts-restart");
    let set = layout_set();
    let work = profile("Work", &set);
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    mock.tap(&device, key(1));
    wait_layout(&agent, &device, "Teams").await;
    // Rapid presses: one switch, no duplicate history.
    for _ in 0..5 {
        mock.tap(&device, key(2));
    }
    wait_layout(&agent, &device, "Meeting").await;
    agent.shutdown().await;
    let saved = support::open_store(&paths, &db_key)
        .device_layouts()
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].2, set.meeting.id);

    // A new agent: the device shows Meeting again; the way back is gone.
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    wait_layout(&agent, &device, "Meeting").await;
    let mut events = agent.events().await;
    mock.tap(&device, key(14));
    assert_eq!(
        completed(&mut events).await,
        (
            "Back to previous layout".to_owned(),
            ActionStatus::Succeeded
        )
    );
    assert_eq!(
        shown(&agent.status().await, &device).as_deref(),
        Some("Meeting")
    );

    // The app removes Meeting from the profile: the device shows the
    // profile's default again, and nothing refers to Meeting.
    let store = agent.store();
    store.unassign_layout(work.id, set.meeting.id).unwrap();
    agent.request(Request::ReloadConfiguration).await.unwrap();
    wait_layout(&agent, &device, "Main").await;
    assert_eq!(store.device_layouts().unwrap().len(), 0);
    assert!(store.shared_layout(set.meeting.id).unwrap().is_some());

    // The app makes Teams the default: the device follows.
    mock.tap(&device, key(2));
    store.set_default_layout(work.id, set.teams.id).unwrap();
    agent.request(Request::ReloadConfiguration).await.unwrap();
    wait_layout(&agent, &device, "Teams").await;
    agent.shutdown().await;
}
