//! The editor's live preview end to end (protocol 16,
//! `docs/editor-navigation-and-simulation.md`): a real agent with simulated
//! MK.2s (`MockBackend`) and temporary data. The unsaved working layout is
//! drawn by the device with the renderer's frames and runs on presses;
//! navigation goes both ways; saving, reverting, closing the connection,
//! locking and restarting leave the device on the saved configuration
//! where they should; nothing is written to the database while previewing.
//! A Stream Deck plugged into the machine is never opened.

mod support;

use std::time::Duration;

use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, LayoutId, LayoutPage, PageId,
    ParameterValue, Profile, ProfileId, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ControlId, DeviceId, Frame, models};
use deckpilot_ipc::live_preview::{
    DeviceNavigation, EditorNavigation, EndReason, LivePreviewEvent, PART_BUDGET, patch, reason,
};
use deckpilot_ipc::{AgentClient, ClientError, ClientKind, Request, Response};
use deckpilot_modules::{deck, general};
use support::{TIMEOUT, TempDir, TestAgent, data_dir};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn bind(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, target)
}

fn labelled(index: u16, target: BindingTarget, label: &str) -> ControlBinding {
    let mut binding = bind(index, target);
    binding.label = Some(label.into());
    binding
}

fn instance(
    profile: &mut Profile,
    name: &str,
    kind: deckpilot_core::ActionTypeId,
    parameters: &ActionParameters,
) -> deckpilot_core::ActionInstanceId {
    let instance = support::registry()
        .create_instance(name, kind, parameters)
        .unwrap();
    let id = instance.id;
    profile.actions.push(instance);
    id
}

/// Main: Home (key 0 logs, "Saved"; key 1 opens the folder Tools; key 2
/// turns the page; key 3 switches to Second; key 4 logs, "Other"), Two (a
/// top-level page). Second: key 14 goes back to the previous layout.
struct Ids {
    profile: ProfileId,
    main: LayoutId,
    second: LayoutId,
    home: PageId,
    tools: PageId,
    two: PageId,
}

fn profile() -> (Profile, Ids) {
    let mut profile = Profile::new("Live");
    let log = instance(
        &mut profile,
        "Log",
        general::log_id(),
        &ActionParameters::new().with("message", ParameterValue::Text("saved".into())),
    );
    let next = instance(
        &mut profile,
        "Next",
        deck::next_page_id(),
        &ActionParameters::new(),
    );
    let mut second = DeviceLayout::new("Second", models::stream_deck_mk2().id);
    let to_second = instance(
        &mut profile,
        "To Second",
        deck::switch_layout_id(),
        &ActionParameters::new().with(deck::LAYOUT, ParameterValue::Text(second.id.to_string())),
    );
    let back = instance(
        &mut profile,
        "Back",
        deck::previous_layout_id(),
        &ActionParameters::new(),
    );
    let second_home = second.home_page;
    second
        .page_mut(second_home)
        .unwrap()
        .bindings
        .push(bind(14, BindingTarget::action(back)));
    let mut main = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let home = main.home_page;
    let tools = LayoutPage::folder("Tools", home);
    let two = LayoutPage::new("Two");
    let (tools_id, two_id) = (tools.id, two.id);
    main.pages.extend([tools, two]);
    main.explicit_tree = true;
    main.page_mut(home).unwrap().bindings.extend([
        labelled(0, BindingTarget::action(log), "Saved"),
        bind(1, BindingTarget::OpenPage { page: tools_id }),
        bind(2, BindingTarget::action(next)),
        bind(3, BindingTarget::action(to_second)),
        labelled(4, BindingTarget::action(log), "Other"),
    ]);
    let ids = Ids {
        profile: profile.id,
        main: main.id,
        second: second.id,
        home,
        tools: tools_id,
        two: two_id,
    };
    profile.layouts = vec![main, second];
    profile.activate_layout(ids.main).unwrap();
    (profile, ids)
}

struct Setup {
    agent: TestAgent,
    mock: MockBackend,
    device: DeviceId,
    profile: Profile,
    ids: Ids,
    _dir: TempDir,
}

async fn setup() -> Setup {
    let dir = TempDir::new("deckpilot-live-preview");
    let (profile, ids) = profile();
    let (paths, key) = data_dir(&dir, &profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "LIVE-A");
    let agent = TestAgent::start_unlocked(&paths, &key, &mock, None).await;
    let setup = Setup {
        agent,
        mock,
        device,
        profile,
        ids,
        _dir: dir,
    };
    let saved = setup.saved();
    assert!(
        setup.shows(&setup.device, &saved, setup.ids.home, 0),
        "the saved key"
    );
    setup
}

/// The frame the renderer draws for `control` of `page` of `layout`, as
/// the MK.2 shows it (its default look).
fn expected(profile: &Profile, layout: &DeviceLayout, page: PageId, control: ControlId) -> Frame {
    let model = models::stream_deck_mk2();
    let spec = model.capabilities.keys.and_then(|keys| keys.image).unwrap();
    let faces = deckpilot_core::page_faces(
        &support::registry(),
        layout,
        page,
        profile,
        None,
        &[control],
        Some(&model.capabilities),
    );
    deckpilot_render::render_for(
        &faces[0].1,
        spec.width,
        spec.height,
        &deckpilot_render::Icons::from_profile(profile),
        &model.effective_appearance(),
    )
}

impl Setup {
    fn saved(&self) -> DeviceLayout {
        self.profile.layout(self.ids.main).unwrap().clone()
    }

    /// The saved layout with key 0's label changed.
    fn titled(&self, label: &str) -> DeviceLayout {
        let mut layout = self.saved();
        let home = layout.page_mut(self.ids.home).unwrap();
        home.bindings
            .iter_mut()
            .find(|binding| binding.control == key(0))
            .unwrap()
            .label = Some(label.into());
        layout
    }

    /// Whether `device` comes to show key `index` of `page` as `layout`
    /// has it.
    fn shows(&self, device: &DeviceId, layout: &DeviceLayout, page: PageId, index: u16) -> bool {
        let frame = expected(&self.profile, layout, page, key(index));
        self.mock.wait_for(device, TIMEOUT, |frames, _| {
            frames.get(&key(index)) == Some(&frame)
        })
    }

    async fn live(&self) -> Live {
        self.live_on(&self.device).await
    }

    async fn live_on(&self, device: &DeviceId) -> Live {
        let mut client = self.agent.client().await;
        let response = client
            .request(Request::BeginLivePreview {
                device: device.clone(),
                profile: self.ids.profile,
                layout: self.ids.main,
                navigation: None,
            })
            .await
            .unwrap();
        let Response::LivePreviewStarted {
            session,
            navigation,
        } = response
        else {
            panic!("{response:?}")
        };
        assert!(navigation.in_scope);
        assert!(navigation.connected);
        Live {
            client,
            session,
            revision: 0,
            sent: None,
        }
    }

    async fn page_id(&self) -> Option<PageId> {
        self.agent
            .status()
            .await
            .devices
            .into_iter()
            .find(|d| d.id == self.device)
            .and_then(|d| d.page_id)
    }

    async fn wait_page(&self, page: PageId) {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        while self.page_id().await != Some(page) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the device did not show {page}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

/// An editor's session.
struct Live {
    client: AgentClient,
    session: u64,
    revision: u64,
    sent: Option<DeviceLayout>,
}

impl Live {
    /// Sends `layout` as the next revision, in parts, as the app does.
    async fn apply(&mut self, layout: &DeviceLayout) -> Result<Response, ClientError> {
        self.revision += 1;
        let parts = patch(self.sent.as_ref(), layout, PART_BUDGET).unwrap();
        let count = parts.len();
        let mut last = None;
        for (index, part) in parts.into_iter().enumerate() {
            last = Some(
                self.client
                    .request(Request::ApplyLivePreview {
                        session: self.session,
                        revision: self.revision,
                        part: Box::new(part),
                        last: index + 1 == count,
                    })
                    .await?,
            );
        }
        self.sent = Some(layout.clone());
        Ok(last.unwrap())
    }

    async fn navigate(&mut self, navigation: EditorNavigation) -> DeviceNavigation {
        match self
            .client
            .request(Request::NavigateLivePreview {
                session: self.session,
                navigation,
            })
            .await
            .unwrap()
        {
            Response::LivePreviewNavigated { navigation, .. } => navigation,
            other => panic!("{other:?}"),
        }
    }

    async fn event(&mut self) -> LivePreviewEvent {
        tokio::time::timeout(TIMEOUT, self.client.next_live_preview())
            .await
            .expect("a live preview message")
            .unwrap()
    }

    /// The next `navigated` whose stack is `stack`.
    async fn navigated_to(&mut self, stack: &[PageId]) -> DeviceNavigation {
        loop {
            if let LivePreviewEvent::Navigated { navigation, .. } = self.event().await
                && navigation.stack == stack
            {
                return navigation;
            }
        }
    }

    async fn quiet(&mut self) -> bool {
        tokio::time::timeout(Duration::from_millis(300), self.client.next_live_preview())
            .await
            .is_err()
    }
}

fn reason_of(result: &Result<Response, ClientError>) -> Option<String> {
    match result {
        Err(ClientError::Agent(error)) => error.reason.clone(),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsaved_title_reaches_the_device_as_the_renderer_draws_it() {
    let s = setup().await;
    let mut live = s.live().await;
    let before = s.mock.control_writes(&s.device);
    let unsaved = s.titled("Unsaved");
    let applied = live.apply(&unsaved).await.unwrap();
    assert!(matches!(
        applied,
        Response::LivePreviewApplied {
            applied: true,
            revision: 1,
            ..
        }
    ));
    assert!(
        s.shows(&s.device, &unsaved, s.ids.home, 0),
        "the unsaved title"
    );
    // Only key 0 was drawn again.
    let after = s.mock.control_writes(&s.device);
    for (control, writes) in &after {
        if *control != key(0) {
            assert_eq!(before.get(control), Some(writes), "{control:?}");
        }
    }
    // An unchanged revision draws nothing.
    let writes = s.mock.image_writes(&s.device);
    live.apply(&unsaved).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(s.mock.image_writes(&s.device), writes);
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rapid_edits_end_on_the_latest_frame_without_flooding_the_device() {
    let s = setup().await;
    let mut live = s.live().await;
    let before = s.mock.control_writes(&s.device);
    for round in 0..40 {
        live.apply(&s.titled(&format!("Edit {round}")))
            .await
            .unwrap();
    }
    assert!(s.shows(&s.device, &s.titled("Edit 39"), s.ids.home, 0));
    let key0 = s
        .mock
        .control_writes(&s.device)
        .get(&key(0))
        .copied()
        .unwrap_or(0)
        - before.get(&key(0)).copied().unwrap_or(0);
    assert!((1..=40).contains(&key0), "{key0} writes");
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_revisions_are_refused_and_change_nothing() {
    let s = setup().await;
    let mut live = s.live().await;
    live.revision = 5;
    live.apply(&s.titled("Five")).await.unwrap();
    live.revision = 2;
    let stale = live.apply(&s.titled("Two")).await;
    assert_eq!(reason_of(&stale).as_deref(), Some(reason::STALE_REVISION));
    assert!(s.shows(&s.device, &s.titled("Five"), s.ids.home, 0));
    // An unknown session.
    let unknown = live
        .client
        .request(Request::EndLivePreview { session: 999 })
        .await;
    assert_eq!(
        reason_of(&unknown).as_deref(),
        Some(reason::UNKNOWN_SESSION)
    );
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_physical_press_runs_the_unsaved_binding_and_the_editor_hears_where_it_went() {
    let s = setup().await;
    let mut live = s.live().await;
    // Unsaved: key 5 opens a new folder inside Tools' parent (home).
    let mut working = s.saved();
    let fresh = LayoutPage::folder("Fresh", s.ids.home);
    let fresh_id = fresh.id;
    working.pages.push(fresh);
    working
        .page_mut(s.ids.home)
        .unwrap()
        .bindings
        .push(bind(5, BindingTarget::OpenPage { page: fresh_id }));
    live.apply(&working).await.unwrap();
    s.mock.tap(&s.device, key(5));
    let went = live.navigated_to(&[s.ids.home, fresh_id]).await;
    assert!(went.in_scope);
    // The protected Back of the unsaved folder.
    s.mock.tap(&s.device, key(0));
    live.navigated_to(&[s.ids.home]).await;
    // A folder key, the page key, the layout switch and Layout Back.
    s.mock.tap(&s.device, key(1));
    live.navigated_to(&[s.ids.home, s.ids.tools]).await;
    s.mock.tap(&s.device, key(0));
    live.navigated_to(&[s.ids.home]).await;
    s.mock.tap(&s.device, key(2));
    live.navigated_to(&[s.ids.two]).await;
    live.navigate(EditorNavigation::Show {
        layout: s.ids.main,
        stack: vec![s.ids.home],
    })
    .await;
    s.mock.tap(&s.device, key(3));
    let second_home = s.profile.layout(s.ids.second).unwrap().home_page;
    let away = live.navigated_to(&[second_home]).await;
    assert_eq!(away.layout, Some(s.ids.second));
    assert!(!away.in_scope, "another layout: the preview is suspended");
    s.mock.tap(&s.device, key(14));
    let back = live.navigated_to(&[s.ids.home]).await;
    assert_eq!(back.layout, Some(s.ids.main));
    assert!(back.in_scope);
    // The working layout shows again.
    assert!(s.shows(&s.device, &working, s.ids.home, 5));
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsaved_assignment_runs_on_a_physical_press() {
    let s = setup().await;
    let mut events = s.agent.events().await;
    let mut live = s.live().await;
    // Unsaved: key 0 turns the page (with an override) instead of logging.
    let next = s
        .profile
        .actions
        .iter()
        .find(|action| action.name == "Next")
        .unwrap()
        .id;
    let mut working = s.saved();
    let binding = working
        .page_mut(s.ids.home)
        .unwrap()
        .bindings
        .iter_mut()
        .find(|binding| binding.control == key(0))
        .unwrap();
    binding.target = BindingTarget::Action {
        action: next,
        overrides: ActionParameters::new().with(deck::WRAP_AROUND, ParameterValue::Boolean(false)),
    };
    live.apply(&working).await.unwrap();
    s.mock.tap(&s.device, key(0));
    live.navigated_to(&[s.ids.two]).await;
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("the action ran")
            .unwrap();
        if let deckpilot_ipc::AgentEvent::ActionCompleted { action, status, .. } = event {
            assert_eq!(action, next, "the unsaved action, not the saved one");
            assert_eq!(status, deckpilot_core::ActionStatus::Succeeded);
            break;
        }
    }
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_editor_navigates_the_device_without_running_anything_or_an_echo() {
    let s = setup().await;
    let mut live = s.live().await;
    let mut working = s.saved();
    let sub = LayoutPage::folder("Sub", s.ids.tools);
    let sub_id = sub.id;
    working.pages.push(sub);
    live.apply(&working).await.unwrap();
    let shown = live
        .navigate(EditorNavigation::Show {
            layout: s.ids.main,
            stack: vec![s.ids.home, s.ids.tools, sub_id],
        })
        .await;
    assert_eq!(shown.stack, [s.ids.home, s.ids.tools, sub_id]);
    s.wait_page(sub_id).await;
    assert!(live.quiet().await, "no echo of the editor's own navigation");
    // A page that does not exist.
    let refused = live
        .client
        .request(Request::NavigateLivePreview {
            session: live.session,
            navigation: EditorNavigation::Show {
                layout: s.ids.main,
                stack: vec![PageId::new()],
            },
        })
        .await;
    assert_eq!(reason_of(&refused).as_deref(), Some(reason::UNKNOWN_PAGE));
    // Another layout, then Layout Back.
    let second_home = s.profile.layout(s.ids.second).unwrap().home_page;
    let away = live
        .navigate(EditorNavigation::Show {
            layout: s.ids.second,
            stack: vec![second_home],
        })
        .await;
    assert_eq!(away.layout, Some(s.ids.second));
    let back = live.navigate(EditorNavigation::LayoutBack).await;
    assert_eq!(back.layout, Some(s.ids.main));
    assert!(back.in_scope);
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saving_keeps_the_page_and_draws_nothing() {
    let s = setup().await;
    let mut live = s.live().await;
    let unsaved = s.titled("To be saved");
    live.apply(&unsaved).await.unwrap();
    live.navigate(EditorNavigation::Show {
        layout: s.ids.main,
        stack: vec![s.ids.home, s.ids.tools],
    })
    .await;
    s.wait_page(s.ids.tools).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let writes = s.mock.image_writes(&s.device);
    // Saved as the app saves: the database, then a reload.
    s.agent
        .edit_profile(s.ids.profile, |profile| {
            *profile
                .layouts
                .iter_mut()
                .find(|layout| layout.id == unsaved.id)
                .unwrap() = unsaved.clone();
        })
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.page_id().await, Some(s.ids.tools));
    assert_eq!(
        s.mock.image_writes(&s.device),
        writes,
        "nothing drawn again"
    );
    assert!(live.quiet().await);
    // Ending the session now changes nothing either.
    live.client
        .request(Request::EndLivePreview {
            session: live.session,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(s.mock.image_writes(&s.device), writes);
    assert_eq!(s.page_id().await, Some(s.ids.tools));
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revert_restores_the_saved_frames_and_the_nearest_page() {
    let s = setup().await;
    let mut live = s.live().await;
    let mut working = s.titled("Edited");
    let sub = LayoutPage::folder("Sub", s.ids.tools);
    let sub_id = sub.id;
    working.pages.push(sub);
    live.apply(&working).await.unwrap();
    live.navigate(EditorNavigation::Show {
        layout: s.ids.main,
        stack: vec![s.ids.home, s.ids.tools, sub_id],
    })
    .await;
    // Revert: the saved layout as the next revision.
    live.apply(&s.saved()).await.unwrap();
    let nearest = live.navigated_to(&[s.ids.home, s.ids.tools]).await;
    assert!(nearest.in_scope);
    live.navigate(EditorNavigation::Show {
        layout: s.ids.main,
        stack: vec![s.ids.home],
    })
    .await;
    assert!(s.shows(&s.device, &s.saved(), s.ids.home, 0));
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_connection_leaves_the_saved_configuration_and_a_new_one_resumes() {
    let s = setup().await;
    let mut live = s.live().await;
    let unsaved = s.titled("Unsaved");
    live.apply(&unsaved).await.unwrap();
    assert!(s.shows(&s.device, &unsaved, s.ids.home, 0));
    // The editor quits or crashes: its connection closes.
    drop(live);
    assert!(
        s.shows(&s.device, &s.saved(), s.ids.home, 0),
        "the saved title"
    );
    // The app reconnects and sends the latest revision again, in full.
    let mut again = s.live().await;
    again.apply(&unsaved).await.unwrap();
    assert!(s.shows(&s.device, &unsaved, s.ids.home, 0));
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_an_agent_restart_the_device_shows_the_saved_layout_until_resent() {
    let s = setup().await;
    let mut live = s.live().await;
    let unsaved = s.titled("Unsaved");
    live.apply(&unsaved).await.unwrap();
    assert!(s.shows(&s.device, &unsaved, s.ids.home, 0));
    let (paths, key) = (s.agent.paths.clone(), s.agent.key.clone());
    let Setup {
        agent,
        mock,
        device,
        profile,
        ids,
        _dir: dir,
    } = s;
    agent.shutdown().await;
    assert!(matches!(
        live.client.next_live_preview().await,
        Err(ClientError::Closed | ClientError::Io(_))
    ));
    let agent = TestAgent::start_unlocked(&paths, &key, &mock, None).await;
    let s = Setup {
        agent,
        mock,
        device,
        profile,
        ids,
        _dir: dir,
    };
    assert!(
        s.shows(&s.device, &s.saved(), s.ids.home, 0),
        "nothing unsaved survives"
    );
    let mut resumed = s.live().await;
    resumed.apply(&unsaved).await.unwrap();
    assert!(s.shows(&s.device, &unsaved, s.ids.home, 0));
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn devices_are_isolated_and_busy_for_another_connection() {
    let s = setup().await;
    let other = s.mock.plug(&models::stream_deck_mk2(), "LIVE-B");
    assert!(s.shows(&other, &s.saved(), s.ids.home, 0));
    let mut live = s.live().await;
    let unsaved = s.titled("Only A");
    live.apply(&unsaved).await.unwrap();
    assert!(s.shows(&s.device, &unsaved, s.ids.home, 0));
    // B uses the same layout and keeps the saved one.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(s.shows(&other, &s.saved(), s.ids.home, 0));
    // A second window cannot preview on A; on B it can.
    let mut second = s.agent.client().await;
    let busy = second
        .request(Request::BeginLivePreview {
            device: s.device.clone(),
            profile: s.ids.profile,
            layout: s.ids.main,
            navigation: None,
        })
        .await;
    assert_eq!(reason_of(&busy).as_deref(), Some(reason::DEVICE_BUSY));
    let mut on_b = s.live_on(&other).await;
    on_b.apply(&s.titled("Only B")).await.unwrap();
    assert!(s.shows(&other, &s.titled("Only B"), s.ids.home, 0));
    assert!(s.shows(&s.device, &unsaved, s.ids.home, 0));
    // A session of another profile, or another model's layout, is refused.
    let inactive = second
        .request(Request::BeginLivePreview {
            device: s.device.clone(),
            profile: ProfileId::new(),
            layout: s.ids.main,
            navigation: None,
        })
        .await;
    assert_eq!(
        reason_of(&inactive).as_deref(),
        Some(reason::INACTIVE_PROFILE)
    );
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previewing_writes_nothing_to_the_database() {
    let s = setup().await;
    let mut live = s.live().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let files = || {
        let database = &s.agent.paths.database;
        let mut found = Vec::new();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let path = std::path::PathBuf::from(format!("{}{suffix}", database.display()));
            if let Ok(meta) = std::fs::metadata(&path) {
                found.push((suffix, meta.len(), meta.modified().ok()));
            }
        }
        found
    };
    let before = files();
    let mut working = s.saved();
    let sub = LayoutPage::folder("Sub", s.ids.tools);
    let sub_id = sub.id;
    working.pages.push(sub);
    for round in 0..10 {
        let mut edited = working.clone();
        edited.pages[0].bindings[0].label = Some(format!("Round {round}"));
        live.apply(&edited).await.unwrap();
        live.navigate(EditorNavigation::Show {
            layout: s.ids.main,
            stack: vec![s.ids.home, s.ids.tools, sub_id],
        })
        .await;
        live.navigate(EditorNavigation::Show {
            layout: s.ids.main,
            stack: vec![s.ids.home],
        })
        .await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(files(), before);
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locking_ends_the_session() {
    let s = setup().await;
    let mut live = s.live().await;
    live.apply(&s.titled("Unsaved")).await.unwrap();
    s.agent.request(Request::Lock).await.unwrap();
    loop {
        if let LivePreviewEvent::Ended { reason, session } = live.event().await {
            assert_eq!(reason, EndReason::Locked);
            assert_eq!(session, live.session);
            break;
        }
    }
    let refused = live.apply(&s.titled("Locked")).await;
    assert!(refused.is_err());
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mock_device_reports_its_frames_and_navigation() {
    let s = setup().await;
    let inspect = || async {
        match s
            .agent
            .request(Request::InspectMockDevice {
                device: s.device.clone(),
            })
            .await
            .unwrap()
        {
            Response::MockDevice { inspection } => *inspection,
            other => panic!("{other:?}"),
        }
    };
    let before = inspect().await;
    assert_eq!(before.navigation.stack, [s.ids.home]);
    assert_eq!(before.frames.len(), 15);
    let hash = |inspection: &deckpilot_ipc::live_preview::MockDeviceInspection, index| {
        inspection
            .frames
            .iter()
            .find(|frame| frame.control == key(index))
            .and_then(|frame| frame.hash.clone())
    };
    let mut live = s.live().await;
    let unsaved = s.titled("Hash");
    live.apply(&unsaved).await.unwrap();
    assert!(s.shows(&s.device, &unsaved, s.ids.home, 0));
    let after = inspect().await;
    assert_ne!(hash(&after, 0), hash(&before, 0));
    assert_eq!(hash(&after, 4), hash(&before, 4));
    assert!(after.navigation.in_scope);
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_15_connection_cannot_preview() {
    let s = setup().await;
    let mut old = AgentClient::connect_with_protocol(&s.agent.paths, ClientKind::Desktop, 15)
        .await
        .unwrap();
    let refused = old
        .request(Request::BeginLivePreview {
            device: s.device.clone(),
            profile: s.ids.profile,
            layout: s.ids.main,
            navigation: None,
        })
        .await;
    assert!(
        matches!(refused, Err(ClientError::Unsupported { needs: 16, .. })),
        "{refused:?}"
    );
    s.agent.shutdown().await;
}
