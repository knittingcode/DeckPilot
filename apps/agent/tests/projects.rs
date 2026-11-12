//! `projects.cycle` and `projects.current` end to end: a simulated Stream
//! Deck, real IPC and an encrypted database. Covers concurrent presses,
//! what the keys show after a switch, a reconnect and a profile change, and
//! profiles whose projects change underneath the active one.

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use deckpilot_core::{
    ActionParameters, ActionStatus, DeviceLayout, ParameterValue, Profile, ProfileId, Project,
    ProjectId, ProjectKey,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceId, Frame, models};
use deckpilot_ipc::{AgentClient, AgentEvent, ClientKind, Request, Response};
use deckpilot_modules::projects::actions::{self, DIRECTION, NEXT_DIRECTION, PREVIOUS_DIRECTION};
use deckpilot_storage::ConfigStore;
use support::{TIMEOUT, TempDir, TestAgent, data_dir, registry};

/// Key 0: cycle forward, key 1: cycle back, key 2: current project.
const NEXT_KEY: ControlId = ControlId::Key { index: 0 };
const PREVIOUS_KEY: ControlId = ControlId::Key { index: 1 };
const CURRENT_KEY: ControlId = ControlId::Key { index: 2 };

/// A profile with `keys` as projects and a generated MK.2 layout with the
/// three project keys.
fn profile(name: &str, keys: &[&str]) -> Profile {
    let registry = registry();
    let mut profile = Profile::new(name);
    for key in keys {
        profile
            .projects
            .push(Project::new(ProjectKey::new(*key).unwrap(), *key));
    }
    let direction =
        |value: &str| ActionParameters::new().with(DIRECTION, ParameterValue::Choice(value.into()));
    profile.actions.extend([
        registry
            .create_instance("Next", actions::cycle_id(), &direction(NEXT_DIRECTION))
            .unwrap(),
        registry
            .create_instance(
                "Previous",
                actions::cycle_id(),
                &direction(PREVIOUS_DIRECTION),
            )
            .unwrap(),
        registry
            .create_instance("Project", actions::current_id(), &ActionParameters::new())
            .unwrap(),
    ]);
    let layout = DeviceLayout::generate("Main", &models::stream_deck_mk2(), &profile);
    profile.layouts.push(layout);
    profile
}

struct Deck {
    agent: TestAgent,
    device: DeviceId,
    profile: Profile,
    _dir: TempDir,
}

async fn start(keys: &[&str]) -> Deck {
    let dir = TempDir::new("deckpilot-projects");
    let profile = profile("Work", keys);
    let (paths, key) = data_dir(&dir, &profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "PROJECTS-1");
    let agent = TestAgent::start_unlocked(&paths, &key, &mock, None).await;
    let deck = Deck {
        agent,
        device,
        profile,
        _dir: dir,
    };
    // The layout is shown (the "Locked" notice uses a single key).
    deck.wait_frames(|frames| frames.len() > 1);
    deck
}

impl Deck {
    fn wait_frames(&self, predicate: impl Fn(&BTreeMap<ControlId, Frame>) -> bool) {
        assert!(
            self.agent
                .mock
                .wait_for(&self.device, TIMEOUT, |frames, _| predicate(frames)),
            "the device did not show the expected keys"
        );
    }

    fn frame(&self, control: ControlId) -> Frame {
        self.agent.mock.frames(&self.device)[&control].clone()
    }

    /// Waits until the agent reports `expected` as the active project.
    async fn wait_active(&self, expected: Option<ProjectId>) {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let status = self.agent.status().await;
            if status.active_project.as_ref().map(|p| p.id) == expected {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "active project is {:?}",
                status.active_project.map(|p| p.key)
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_disconnected(&self) {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let status = self.agent.status().await;
            if !status
                .devices
                .iter()
                .any(|d| d.state == ConnectionState::Connected)
            {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the device is still connected"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn stored(&self, profile: ProfileId) -> Option<ProjectId> {
        self.agent.store().active_project(profile).unwrap()
    }

    /// Presses `control` and waits until the key `watch` shows a new face.
    fn press_and_redraw(&self, control: ControlId, watch: ControlId) -> Frame {
        let before = self.frame(watch);
        self.agent.mock.tap(&self.device, control);
        self.wait_frames(|frames| frames.get(&watch).is_some_and(|f| *f != before));
        self.frame(watch)
    }
}

/// Counts `ActionCompleted` events until `count` arrived; all must succeed.
async fn completed(events: &mut AgentClient, count: usize) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let mut seen = 0;
    while seen < count {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("not every press completed in time")
            .unwrap();
        if let AgentEvent::ActionCompleted { status, name, .. } = event {
            assert_eq!(status, ActionStatus::Succeeded, "{name}");
            seen += 1;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_presses_advance_exactly_one_project_each() {
    // Seven presses queued on the device at once, and five requests from
    // separate clients at the same time.
    const PRESSES: usize = 7;
    const REQUESTS: usize = 5;
    let keys = ["A", "B", "C", "D", "E"];
    let deck = start(&keys).await;
    let mut events = deck.agent.events().await;
    let next = deck.profile.actions[0].id;

    for _ in 0..PRESSES {
        deck.agent.mock.tap(&deck.device, NEXT_KEY);
    }
    let requests: Vec<_> = (0..REQUESTS)
        .map(|_| {
            let paths = deck.agent.paths.clone();
            tokio::spawn(async move {
                let mut client = AgentClient::connect(&paths, ClientKind::Desktop)
                    .await
                    .unwrap();
                match client.request(Request::RunAction { action: next }).await {
                    Ok(Response::ActionResult { result }) => result,
                    other => panic!("unexpected {other:?}"),
                }
            })
        })
        .collect();
    for request in requests {
        let result = tokio::time::timeout(TIMEOUT, request)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_success(), "{}", result.message);
    }
    completed(&mut events, PRESSES + REQUESTS).await;

    let expected = deck.profile.projects[(PRESSES + REQUESTS) % keys.len()].id;
    deck.wait_active(Some(expected)).await;
    assert_eq!(deck.stored(deck.profile.id), Some(expected));
    // Nothing else is pending: the selection stays.
    tokio::time::sleep(Duration::from_millis(200)).await;
    deck.wait_active(Some(expected)).await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_show_the_project_after_a_cycle_a_reconnect_and_a_profile_change() {
    let deck = start(&["SHOP", "CMR", "CMS"]).await;
    let [cmr, cms] = [1, 2].map(|i| deck.profile.projects[i].id);
    let mut events = deck.agent.events().await;

    let shop_face = deck.frame(NEXT_KEY);
    let shop_current = deck.frame(CURRENT_KEY);
    // A press redraws the keys that show the project right away, and
    // subscribers are told about the new project.
    let cmr_face = deck.press_and_redraw(NEXT_KEY, NEXT_KEY);
    deck.wait_frames(|frames| frames.get(&CURRENT_KEY).is_some_and(|f| *f != shop_current));
    let cmr_current = deck.frame(CURRENT_KEY);
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("no state event for the new project")
            .unwrap();
        if let AgentEvent::State { status } = event
            && status.active_project.as_ref().map(|p| p.id) == Some(cmr)
        {
            break;
        }
    }
    assert_eq!(deck.stored(deck.profile.id), Some(cmr));
    assert_ne!(cmr_face, shop_face);
    let third_face = deck.press_and_redraw(NEXT_KEY, NEXT_KEY);
    assert_ne!(third_face, cmr_face);
    assert_ne!(third_face, shop_face);
    // Back to CMR with the other key: the same face as before.
    assert_eq!(deck.press_and_redraw(PREVIOUS_KEY, NEXT_KEY), cmr_face);
    deck.wait_active(Some(cmr)).await;

    // Reconnecting: the device starts blank and shows the current project.
    deck.agent.mock.unplug(&deck.device);
    deck.wait_disconnected().await;
    deck.agent
        .mock
        .plug(&models::stream_deck_mk2(), "PROJECTS-1");
    deck.wait_frames(|frames| {
        frames.get(&NEXT_KEY) == Some(&cmr_face) && frames.get(&CURRENT_KEY) == Some(&cmr_current)
    });

    // Another profile in between: its own projects switch, this profile's
    // selection is kept and shown again when it comes back.
    let home = profile("Home", &["BLOG", "WIKI"]);
    deck.agent.store().save_profile(&home).unwrap();
    deck.agent
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    deck.wait_active(Some(home.projects[0].id)).await;
    deck.wait_frames(|frames| frames.get(&NEXT_KEY).is_some_and(|f| *f != cmr_face));
    deck.press_and_redraw(NEXT_KEY, NEXT_KEY);
    deck.wait_active(Some(home.projects[1].id)).await;
    assert_eq!(deck.stored(home.id), Some(home.projects[1].id));
    assert_eq!(deck.stored(deck.profile.id), Some(cmr));

    deck.agent
        .request(Request::SetActiveProfile {
            profile: deck.profile.id,
        })
        .await
        .unwrap();
    deck.wait_active(Some(cmr)).await;
    deck.wait_frames(|frames| frames.get(&NEXT_KEY) == Some(&cmr_face));
    // The next press continues from this profile's project.
    assert_eq!(deck.press_and_redraw(NEXT_KEY, NEXT_KEY), third_face);
    deck.wait_active(Some(cms)).await;
    deck.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projects_changed_underneath_the_active_one() {
    let deck = start(&["SHOP", "CMR", "CMS"]).await;
    let [shop, cmr, cms] = [0, 1, 2].map(|i| deck.profile.projects[i].id);
    let shop_face = deck.frame(NEXT_KEY);
    deck.press_and_redraw(NEXT_KEY, NEXT_KEY);
    deck.wait_active(Some(cmr)).await;

    // Reordered to CMS, CMR, SHOP: the next press follows the new order.
    deck.agent
        .edit_profile(deck.profile.id, |profile| profile.projects.reverse())
        .await;
    deck.press_and_redraw(NEXT_KEY, NEXT_KEY);
    deck.wait_active(Some(shop)).await;
    assert_eq!(deck.frame(NEXT_KEY), shop_face);

    // The active project is deleted: the first project becomes active and
    // is shown at once; the next press continues from it.
    deck.agent
        .edit_profile(deck.profile.id, |profile| {
            profile.projects.retain(|p| p.id != shop);
        })
        .await;
    deck.wait_active(Some(cms)).await;
    assert_eq!(deck.stored(deck.profile.id), Some(cms));
    deck.wait_frames(|frames| frames.get(&NEXT_KEY).is_some_and(|f| *f != shop_face));
    deck.press_and_redraw(NEXT_KEY, NEXT_KEY);
    deck.wait_active(Some(cmr)).await;

    // One project left: a press succeeds and changes nothing.
    deck.agent
        .edit_profile(deck.profile.id, |profile| {
            profile.projects.retain(|p| p.id == cmr);
        })
        .await;
    let mut events = deck.agent.events().await;
    deck.agent.mock.tap(&deck.device, NEXT_KEY);
    completed(&mut events, 1).await;
    deck.wait_active(Some(cmr)).await;

    // No projects: the press fails with no project active.
    deck.agent
        .edit_profile(deck.profile.id, |profile| profile.projects.clear())
        .await;
    deck.wait_active(None).await;
    deck.agent.mock.tap(&deck.device, NEXT_KEY);
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("the press did not complete")
            .unwrap();
        if let AgentEvent::ActionCompleted { status, .. } = event {
            assert_eq!(status, ActionStatus::Failed);
            break;
        }
    }
    deck.wait_active(None).await;
    deck.agent.shutdown().await;
}
