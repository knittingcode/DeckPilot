//! The `deck` module's actions against a real agent with simulated
//! devices (`MockBackend`) and temporary data: keys that turn pages, the
//! page indicator, folders that close after use, keys that switch
//! profiles, what survives a restart, and a configuration saved before any
//! of this existed. A Stream Deck plugged into the machine is never opened.

mod support;

use std::time::Duration;

use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActionStatus, BindingTarget, ControlBinding, DeviceLayout,
    FaceStyle, KeyFace, LayoutPage, ParameterValue, Profile, ProfileId, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceId, models};
use deckpilot_ipc::{AgentClient, AgentEvent, AgentPaths, AgentStatus, Request, Response};
use deckpilot_modules::{deck, general};
use deckpilot_storage::ConfigStore;
use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn press(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, target)
}

/// An action of `action_type` named `name`, added to `profile`.
fn action(
    profile: &mut Profile,
    name: &str,
    action_type: deckpilot_core::ActionTypeId,
    parameters: &ActionParameters,
) -> BindingTarget {
    let instance = support::registry()
        .create_instance(name, action_type, parameters)
        .unwrap();
    let target = BindingTarget::action(instance.id);
    profile.actions.push(instance);
    target
}

fn log(profile: &mut Profile, name: &str) -> BindingTarget {
    action(
        profile,
        name,
        general::log_id(),
        &ActionParameters::new().with("message", ParameterValue::Text(name.into())),
    )
}

fn wrap(on: bool) -> ActionParameters {
    ActionParameters::new().with(deck::WRAP_AROUND, ParameterValue::Boolean(on))
}

fn switch_to(target: &str, profile: Option<ProfileId>) -> ActionParameters {
    let parameters =
        ActionParameters::new().with(deck::TARGET, ParameterValue::Choice(target.into()));
    match profile {
        Some(id) => parameters.with(deck::PROFILE, ParameterValue::Text(id.to_string())),
        None => parameters,
    }
}

/// A profile with the pages Home, Two and Three, and in Two the folder
/// "Tools", which closes after use.
///
/// Every page: key 0 next page, key 1 previous page (both wrap around),
/// key 2 next page without wraparound, key 4 the page indicator, key 5
/// "`<name>` key" (a log action). Two: key 6 opens Tools. Tools: key 7
/// "In tools" (a log action).
fn book(name: &str) -> Profile {
    let mut profile = Profile::new(name);
    let next = action(&mut profile, "Next page", deck::next_page_id(), &wrap(true));
    let previous = action(
        &mut profile,
        "Previous page",
        deck::previous_page_id(),
        &wrap(true),
    );
    let next_stop = action(
        &mut profile,
        "Next page, no wrap",
        deck::next_page_id(),
        &wrap(false),
    );
    let indicator = action(
        &mut profile,
        "Page indicator",
        deck::page_indicator_id(),
        &ActionParameters::new(),
    );
    let own = log(&mut profile, &format!("{name} key"));
    let in_tools = log(&mut profile, "In tools");

    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.explicit_tree = true;
    let mut two = LayoutPage::new("Two");
    let three = LayoutPage::new("Three");
    let mut tools = LayoutPage::new("Tools");
    tools.parent = Some(two.id);
    tools.auto_return = true;
    tools.bindings.push(press(7, in_tools));
    two.bindings
        .push(press(6, BindingTarget::OpenPage { page: tools.id }));
    layout.pages.extend([two, three, tools]);
    for page in &mut layout.pages {
        if page.name == "Tools" {
            continue;
        }
        page.bindings.extend([
            press(0, next.clone()),
            press(1, previous.clone()),
            press(2, next_stop.clone()),
            press(4, indicator.clone()),
            press(5, own.clone()),
        ]);
    }
    profile.layouts.push(layout);
    profile
}

fn page(status: &AgentStatus, device: &DeviceId) -> Option<String> {
    status
        .devices
        .iter()
        .find(|d| d.id == *device)
        .and_then(|d| d.page.clone())
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
            "condition not reached in time; devices: {:?}, profile: {:?}",
            status.devices,
            status.active_profile
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_page(agent: &TestAgent, device: &DeviceId, name: &str) {
    wait_status(agent, |s| page(s, device).as_deref() == Some(name)).await;
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

async fn run(agent: &TestAgent, action: ActionInstanceId) -> deckpilot_core::ActionResult {
    match agent.request(Request::RunAction { action }).await.unwrap() {
        Response::ActionResult { result } => result,
        other => panic!("unexpected {other:?}"),
    }
}

fn action_id(profile: &Profile, name: &str) -> ActionInstanceId {
    profile.actions.iter().find(|a| a.name == name).unwrap().id
}

/// The frame of an action key with `title`, as the agent draws it.
fn titled(title: &str) -> deckpilot_device::Frame {
    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    deckpilot_agent::render::render_with(
        &KeyFace::new(FaceStyle::Action, title),
        spec.width,
        spec.height,
        &deckpilot_agent::render::Icons::default(),
    )
}

fn shows(mock: &MockBackend, device: &DeviceId, index: u16, title: &str) -> bool {
    let frame = titled(title);
    mock.wait_for(device, TIMEOUT, |frames, _| {
        frames.get(&key(index)) == Some(&frame)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_turn_pages_and_the_indicator_follows() {
    let dir = TempDir::new("deckpilot-deck-pages");
    let work = book("Work");
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    let mut events = agent.events().await;
    wait_page(&agent, &device, "Home").await;
    assert!(shows(&mock, &device, 4, "1/3"), "the indicator on Home");

    mock.tap(&device, key(0));
    wait_page(&agent, &device, "Two").await;
    assert_eq!(
        completed(&mut events).await,
        ("Next page".to_owned(), ActionStatus::Succeeded)
    );
    assert!(shows(&mock, &device, 4, "2/3"), "the indicator on Two");
    mock.tap(&device, key(0));
    wait_page(&agent, &device, "Three").await;
    assert!(shows(&mock, &device, 4, "3/3"), "the indicator on Three");

    // The last page without wraparound: it stays, and the key did not fail.
    mock.tap(&device, key(2));
    assert_eq!(
        completed(&mut events).await,
        ("Next page".to_owned(), ActionStatus::Succeeded)
    );
    assert_eq!(
        completed(&mut events).await,
        ("Next page, no wrap".to_owned(), ActionStatus::Succeeded)
    );
    assert_eq!(
        page(&agent.status().await, &device).as_deref(),
        Some("Three")
    );

    // With wraparound: the first page again; and backwards to the last.
    mock.tap(&device, key(0));
    wait_page(&agent, &device, "Home").await;
    mock.tap(&device, key(1));
    wait_page(&agent, &device, "Three").await;

    // From the app there is no device whose page could be meant; a
    // profile's page indicator can be run and does nothing.
    let refused = run(&agent, action_id(&work, "Next page")).await;
    assert_eq!(refused.status, ActionStatus::Failed);
    assert_eq!(refused.code.as_deref(), Some("no-device"));
    assert!(
        run(&agent, action_id(&work, "Page indicator"))
            .await
            .is_success()
    );
    assert_eq!(
        page(&agent.status().await, &device).as_deref(),
        Some("Three")
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_that_closes_returns_after_its_key_ran() {
    let dir = TempDir::new("deckpilot-deck-folder");
    let work = book("Work");
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    let mut events = agent.events().await;

    mock.tap(&device, key(0));
    wait_page(&agent, &device, "Two").await;
    completed(&mut events).await;
    mock.tap(&device, key(6));
    wait_page(&agent, &device, "Tools").await;
    // Its key runs, then the device is back on the folder's page.
    mock.tap(&device, key(7));
    assert_eq!(
        completed(&mut events).await,
        ("In tools".to_owned(), ActionStatus::Succeeded)
    );
    wait_page(&agent, &device, "Two").await;
    // The key of the page under the folder's key did not run as well.
    mock.tap(&device, key(5));
    assert_eq!(
        completed(&mut events).await,
        ("Work key".to_owned(), ActionStatus::Succeeded)
    );
    agent.shutdown().await;
}

/// Three profiles in the order the agent lists them (by name): Alpha, Beta
/// and Gamma. Alpha's Home: key 8 next profile, key 9 previous profile
/// without wraparound, key 10 switch to Gamma, key 11 switch to a profile
/// that does not exist.
fn three_profiles() -> (Profile, Profile, Profile) {
    let mut alpha = book("Alpha");
    let (beta, gamma) = (book("Beta"), book("Gamma"));
    let next = action(
        &mut alpha,
        "Next profile",
        deck::switch_profile_id(),
        &switch_to(deck::TARGET_NEXT, None),
    );
    let previous = action(
        &mut alpha,
        "Previous profile",
        deck::switch_profile_id(),
        &switch_to(deck::TARGET_PREVIOUS, None)
            .with(deck::WRAP_AROUND, ParameterValue::Boolean(false)),
    );
    let to_gamma = action(
        &mut alpha,
        "To Gamma",
        deck::switch_profile_id(),
        &switch_to(deck::TARGET_PROFILE, Some(gamma.id)),
    );
    let to_nowhere = action(
        &mut alpha,
        "To a deleted profile",
        deck::switch_profile_id(),
        &switch_to(deck::TARGET_PROFILE, Some(ProfileId::new())),
    );
    alpha.layouts[0].pages[0].bindings.extend([
        press(8, next),
        press(9, previous),
        press(10, to_gamma),
        press(11, to_nowhere),
    ]);
    (alpha, beta, gamma)
}

fn active(status: &AgentStatus) -> Option<&str> {
    status.active_profile.as_ref().map(|p| p.name.as_str())
}

async fn wait_profile(agent: &TestAgent, name: &str) -> AgentStatus {
    wait_status(agent, |s| active(s) == Some(name)).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_switches_the_profile_of_every_device() {
    let dir = TempDir::new("deckpilot-deck-profiles");
    let (alpha, beta, gamma) = three_profiles();
    let (paths, db_key) = support::data_dir(&dir, &alpha, &[]);
    let store = support::open_store(&paths, &db_key);
    store.save_profile(&beta).unwrap();
    store.save_profile(&gamma).unwrap();
    drop(store);
    let mock = MockBackend::new();
    let (a, b) = (
        mock.plug(&models::stream_deck_mk2(), "MK2-A"),
        mock.plug(&models::stream_deck_mk2(), "MK2-B"),
    );
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &a).await;
    connected(&agent, &b).await;
    let mut events = agent.events().await;
    let names: Vec<String> = agent
        .status()
        .await
        .profiles
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Alpha", "Beta", "Gamma"]);

    // The first of the list has no previous profile without wraparound:
    // nothing changes and the key does not fail.
    mock.tap(&a, key(9));
    assert_eq!(
        completed(&mut events).await,
        ("Previous profile".to_owned(), ActionStatus::Succeeded)
    );
    assert_eq!(active(&agent.status().await), Some("Alpha"));

    // A profile that does not exist: the key fails, nothing changes.
    mock.tap(&a, key(11));
    assert_eq!(
        completed(&mut events).await,
        ("To a deleted profile".to_owned(), ActionStatus::Failed)
    );
    assert_eq!(active(&agent.status().await), Some("Alpha"));
    let missing = run(&agent, action_id(&alpha, "To a deleted profile")).await;
    assert_eq!(missing.code.as_deref(), Some("profile-not-found"));
    assert_eq!(
        completed(&mut events).await,
        ("To a deleted profile".to_owned(), ActionStatus::Failed)
    );

    // Device B is on another page of Alpha when A's key switches.
    mock.tap(&b, key(0));
    wait_page(&agent, &b, "Two").await;
    completed(&mut events).await;
    mock.tap(&a, key(8));
    assert_eq!(
        completed(&mut events).await,
        ("Next profile".to_owned(), ActionStatus::Succeeded)
    );
    let status = wait_profile(&agent, "Beta").await;
    // Both devices show Beta, on its home page, and its keys run.
    assert_eq!(page(&status, &a).as_deref(), Some("Home"));
    assert_eq!(page(&status, &b).as_deref(), Some("Home"));
    mock.tap(&b, key(5));
    assert_eq!(
        completed(&mut events).await,
        ("Beta key".to_owned(), ActionStatus::Succeeded)
    );
    // The switch is saved: it is the active profile of the store.
    assert_eq!(agent.store().active_profile().unwrap(), Some(beta.id));

    // Back to Alpha over IPC, as the app does: each device is on the page
    // it showed there.
    agent
        .request(Request::SetActiveProfile { profile: alpha.id })
        .await
        .unwrap();
    let status = wait_profile(&agent, "Alpha").await;
    assert_eq!(page(&status, &a).as_deref(), Some("Home"));
    assert_eq!(page(&status, &b).as_deref(), Some("Two"));

    // One specific profile, from a key and from the app.
    mock.tap(&a, key(10));
    assert_eq!(
        completed(&mut events).await,
        ("To Gamma".to_owned(), ActionStatus::Succeeded)
    );
    wait_profile(&agent, "Gamma").await;
    agent
        .request(Request::SetActiveProfile { profile: alpha.id })
        .await
        .unwrap();
    wait_profile(&agent, "Alpha").await;
    let result = run(&agent, action_id(&alpha, "To Gamma")).await;
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.message, "profile Gamma");
    assert_eq!(result.data["profileId"], gamma.id.to_string());
    wait_profile(&agent, "Gamma").await;
    agent.shutdown().await;
}

fn brightness(change: &str, key: &str, value: i64) -> ActionParameters {
    ActionParameters::new()
        .with(deck::CHANGE, ParameterValue::Choice(change.into()))
        .with(key, ParameterValue::Integer(value))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_set_and_step_the_brightness_of_the_devices() {
    let dir = TempDir::new("deckpilot-deck-brightness");
    let mut work = Profile::new("Work");
    let set = action(
        &mut work,
        "Dim",
        deck::brightness_id(),
        &brightness(deck::CHANGE_SET, deck::LEVEL, 30),
    );
    let brighter = action(
        &mut work,
        "Brighter",
        deck::brightness_id(),
        &brightness(deck::CHANGE_BRIGHTER, deck::STEP, 50),
    );
    let dimmer = action(
        &mut work,
        "Dimmer",
        deck::brightness_id(),
        &brightness(deck::CHANGE_DIMMER, deck::STEP, 25),
    );
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.pages[0]
        .bindings
        .extend([press(0, set), press(1, brighter), press(2, dimmer)]);
    work.layouts.push(layout);
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    let mock = MockBackend::new();
    let (a, b) = (
        mock.plug(&models::stream_deck_mk2(), "MK2-A"),
        mock.plug(&models::stream_deck_mk2(), "MK2-B"),
    );
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &a).await;
    connected(&agent, &b).await;
    let level = |status: &AgentStatus| status.brightness;

    // A level: the setting, saved, and sent to every device.
    mock.tap(&a, key(0));
    wait_status(&agent, |s| level(s) == 30).await;
    assert_eq!(agent.store().device_brightness().unwrap(), Some(30));
    for device in [&a, &b] {
        assert!(
            wait_until(|| mock.brightness(device) == Some(30)).await,
            "{device} was not dimmed"
        );
    }
    // Steps from where it is, and never past the ends.
    mock.tap(&b, key(1));
    wait_status(&agent, |s| level(s) == 80).await;
    mock.tap(&b, key(1));
    wait_status(&agent, |s| level(s) == 100).await;
    mock.tap(&a, key(2));
    wait_status(&agent, |s| level(s) == 75).await;
    assert!(wait_until(|| mock.brightness(&b) == Some(75)).await);

    // From the app too: it needs no device. The result says the level.
    let result = run(&agent, action_id(&work, "Dim")).await;
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.message, "brightness 30 %");
    assert_eq!(result.data["brightness"], "30");
    agent.shutdown().await;

    // The level is the setting: a new agent starts with it.
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &a).await;
    assert_eq!(agent.status().await.brightness, 30);
    agent.shutdown().await;
}

/// Whether `condition` became true in time.
async fn wait_until(condition: impl Fn() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while !condition() {
        if tokio::time::Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    true
}

fn saved_pages(paths: &AgentPaths, key: &deckpilot_storage::DatabaseKey) -> Option<String> {
    support::open_store(paths, key)
        .setting("navigation.pages")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_profile_and_the_pages_survive_a_restart() {
    let dir = TempDir::new("deckpilot-deck-restart");
    let (alpha, beta, _) = three_profiles();
    let (paths, db_key) = support::data_dir(&dir, &alpha, &[]);
    support::open_store(&paths, &db_key)
        .save_profile(&beta)
        .unwrap();
    let mock = MockBackend::new();
    let (a, b) = (
        mock.plug(&models::stream_deck_mk2(), "MK2-A"),
        mock.plug(&models::stream_deck_mk2(), "MK2-B"),
    );
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &a).await;
    connected(&agent, &b).await;

    // Alpha: device B on Two. Then Beta from A's key; there A goes to
    // Three and B into the folder of Two.
    mock.tap(&b, key(0));
    wait_page(&agent, &b, "Two").await;
    mock.tap(&a, key(8));
    wait_profile(&agent, "Beta").await;
    mock.tap(&a, key(1));
    wait_page(&agent, &a, "Three").await;
    mock.tap(&b, key(0));
    wait_page(&agent, &b, "Two").await;
    mock.tap(&b, key(6));
    wait_page(&agent, &b, "Tools").await;
    agent.shutdown().await;
    let saved = saved_pages(&paths, &db_key).expect("the pages were saved");
    assert!(saved.contains(&alpha.id.to_string()), "{saved}");
    assert!(saved.contains(&beta.id.to_string()), "{saved}");

    // A new agent on the same data: Beta is active, A is on Three, and B
    // on the page of its folder, which is closed.
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &a).await;
    connected(&agent, &b).await;
    let status = wait_profile(&agent, "Beta").await;
    assert_eq!(page(&status, &a).as_deref(), Some("Three"));
    assert_eq!(page(&status, &b).as_deref(), Some("Two"));
    // And Alpha's pages are remembered across the restart too.
    agent
        .request(Request::SetActiveProfile { profile: alpha.id })
        .await
        .unwrap();
    let status = wait_profile(&agent, "Alpha").await;
    assert_eq!(page(&status, &a).as_deref(), Some("Home"));
    assert_eq!(page(&status, &b).as_deref(), Some("Two"));
    agent.shutdown().await;

    // A saved page that is gone, and saved pages that cannot be read: the
    // home page, and the agent starts.
    let store = support::open_store(&paths, &db_key);
    let mut changed = store.load_profile(alpha.id).unwrap().unwrap();
    changed.layouts[0]
        .pages
        .retain(|p| p.name != "Two" && p.name != "Tools");
    store.save_profile(&changed).unwrap();
    drop(store);
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &a).await;
    connected(&agent, &b).await;
    let status = wait_profile(&agent, "Alpha").await;
    assert_eq!(page(&status, &b).as_deref(), Some("Home"));
    // A goes to the page that is left, which is saved.
    mock.tap(&a, key(0));
    wait_page(&agent, &a, "Three").await;
    agent.shutdown().await;
    support::open_store(&paths, &db_key)
        .set_setting("navigation.pages", "not json")
        .unwrap();
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &a).await;
    let status = wait_profile(&agent, "Alpha").await;
    assert_eq!(page(&status, &a).as_deref(), Some("Home"));
    agent.shutdown().await;
}

/// A profile as DeckPilot 0.6.0-alpha.3 stored it: a layout whose home page
/// opens a folder (no stored page tree, no Back key in the folder), a log
/// action on each page. Nothing in it mentions pages' new settings.
fn alpha3_document() -> serde_json::Value {
    serde_json::json!({
        "id": "5d2f6c0e-7a0b-4a57-9d4f-3d3c5e1b9a01",
        "name": "From alpha 3",
        "description": "",
        "actions": [
            {
                "id": "0b6c0f2e-1f0a-4f0e-8a67-0c8f4b5a7d11",
                "name": "On home",
                "actionType": "general.log",
                "parameters": { "message": { "type": "text", "value": "On home" } }
            },
            {
                "id": "0b6c0f2e-1f0a-4f0e-8a67-0c8f4b5a7d12",
                "name": "In folder",
                "actionType": "general.log",
                "parameters": { "message": { "type": "text", "value": "In folder" } }
            }
        ],
        "workflows": [],
        "projects": [],
        "layouts": [
            {
                "id": "9a1d2c3b-4e5f-4a6b-8c7d-0e1f2a3b4c01",
                "name": "Main",
                "model": "elgato.streamdeck.mk2",
                "homePage": "9a1d2c3b-4e5f-4a6b-8c7d-0e1f2a3b4c02",
                "pages": [
                    {
                        "id": "9a1d2c3b-4e5f-4a6b-8c7d-0e1f2a3b4c02",
                        "name": "Home",
                        "bindings": [
                            {
                                "control": { "kind": "key", "index": 0 },
                                "trigger": { "type": "press" },
                                "target": {
                                    "type": "action",
                                    "action": "0b6c0f2e-1f0a-4f0e-8a67-0c8f4b5a7d11"
                                }
                            },
                            {
                                "control": { "kind": "key", "index": 5 },
                                "trigger": { "type": "press" },
                                "target": {
                                    "type": "openPage",
                                    "page": "9a1d2c3b-4e5f-4a6b-8c7d-0e1f2a3b4c03"
                                }
                            }
                        ]
                    },
                    {
                        "id": "9a1d2c3b-4e5f-4a6b-8c7d-0e1f2a3b4c03",
                        "name": "Folder",
                        "bindings": [
                            {
                                "control": { "kind": "key", "index": 2 },
                                "trigger": { "type": "press" },
                                "target": {
                                    "type": "action",
                                    "action": "0b6c0f2e-1f0a-4f0e-8a67-0c8f4b5a7d12"
                                }
                            }
                        ]
                    }
                ]
            }
        ]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_configuration_from_before_behaves_as_before_and_is_not_rewritten() {
    let dir = TempDir::new("deckpilot-deck-alpha3");
    let old: Profile = serde_json::from_value(alpha3_document()).unwrap();
    // Reading and writing it changes nothing in the document.
    assert_eq!(serde_json::to_value(&old).unwrap(), alpha3_document());
    let (paths, db_key) = support::data_dir(&dir, &old, &[]);
    assert_eq!(saved_pages(&paths, &db_key), None);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    connected(&agent, &device).await;
    let mut events = agent.events().await;

    // The folder opens, its key runs and it stays open (nothing closes by
    // itself), and the automatic Back on its first free key returns.
    mock.tap(&device, key(5));
    wait_page(&agent, &device, "Folder").await;
    mock.tap(&device, key(2));
    assert_eq!(
        completed(&mut events).await,
        ("In folder".to_owned(), ActionStatus::Succeeded)
    );
    assert_eq!(
        page(&agent.status().await, &device).as_deref(),
        Some("Folder")
    );
    mock.tap(&device, key(0));
    wait_page(&agent, &device, "Home").await;
    mock.tap(&device, key(0));
    assert_eq!(
        completed(&mut events).await,
        ("On home".to_owned(), ActionStatus::Succeeded)
    );

    // The deck actions are there without anything being installed or
    // enabled: the module is built in and on by default.
    let store = agent.store();
    // Nothing was saved for a deck that only opened a folder of its home
    // page: the settings of the old database are as they were.
    assert_eq!(store.setting("navigation.pages").unwrap(), None);
    assert!(
        !store
            .module_states()
            .unwrap()
            .contains_key(&deck::module_id()),
        "no state was stored for the module"
    );
    let mut with_next = store.load_profile(old.id).unwrap().unwrap();
    let next = action(
        &mut with_next,
        "Next page",
        deck::next_page_id(),
        &wrap(true),
    );
    with_next.layouts[0].pages[0].bindings.push(press(1, next));
    // A second top-level page to go to.
    with_next.layouts[0].pages.push(LayoutPage::new("Second"));
    store.save_profile(&with_next).unwrap();
    drop(store);
    agent.request(Request::ReloadConfiguration).await.unwrap();
    mock.tap(&device, key(1));
    wait_page(&agent, &device, "Second").await;
    agent.shutdown().await;

    // The agent never rewrote the profile: what is stored is what the test
    // saved, and the original document reads back unchanged without the
    // additions.
    let stored = support::open_store(&paths, &db_key)
        .load_profile(old.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored, with_next);
    let mut without = serde_json::to_value(&stored).unwrap();
    without["actions"].as_array_mut().unwrap().pop();
    without["layouts"][0]["pages"].as_array_mut().unwrap().pop();
    without["layouts"][0]["pages"][0]["bindings"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert_eq!(without, alpha3_document());
}
