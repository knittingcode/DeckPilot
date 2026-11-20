//! Simulated presses end to end (`simulatePress`, protocol 14,
//! `docs/editor-navigation-and-simulation.md`): a real agent with a
//! simulated MK.2. The app's press goes through the agent's own gesture
//! engine and dispatch on a lane of its own, with the key's timings, after
//! the agent checked what the device shows; a press that may change
//! something outside DeckPilot waits for `confirmed`, and the computer's
//! settings still decide whether it runs.

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_agent::GestureHooks;
use deckpilot_core::gesture::{
    BoundGestures, Gesture, GestureFired, GestureTimings, KeyGestureSettings,
};
use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActionStatus, BindingTarget, ControlBinding, DeviceLayout,
    ExecutionTrigger, LayoutId, LayoutPage, PageId, ParameterValue, Profile, ProfileId,
    RuntimeOutcome, Trigger, WorkflowId,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::simulation::{
    PressKind, SimulatedTrigger as T, SimulationOutcome, SimulationRefusal,
};
use deckpilot_ipc::{AgentClient, AgentEvent, ClientError, ClientKind, Request, Response};
use deckpilot_modules::general;
use support::{TIMEOUT, TempDir, TestAgent, data_dir, registry};

/// Press: "down"; release: "up" (no gestures).
const EDGES: ControlId = ControlId::Key { index: 0 };
/// Click, double click and long press, with the key's own timings.
const GESTURES: ControlId = ControlId::Key { index: 2 };
/// Click and double click only.
const CLICKS: ControlId = ControlId::Key { index: 5 };
/// A workflow that logs.
const WORKFLOW: ControlId = ControlId::Key { index: 6 };
/// `system.run-command`.
const COMMAND: ControlId = ControlId::Key { index: 7 };
/// A workflow with a `system.run-command` step.
const COMMAND_WORKFLOW: ControlId = ControlId::Key { index: 8 };
/// `http.request` with POST.
const POST: ControlId = ControlId::Key { index: 9 };
/// Opens the folder page.
const FOLDER: ControlId = ControlId::Key { index: 10 };

const KEY_DOUBLE_MS: u16 = 250;
const KEY_LONG_MS: u16 = 450;
/// How long the agent holds a simulated key for a tap.
const TAP_MS: u64 = 40;
/// The connection id bit of the simulated lane.
const SIMULATED: u64 = 1 << 63;

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

fn instance(
    profile: &mut Profile,
    name: &str,
    kind: deckpilot_core::ActionTypeId,
    parameters: &ActionParameters,
) -> ActionInstanceId {
    let instance = registry()
        .create_instance(name, kind, parameters)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let id = instance.id;
    profile.actions.push(instance);
    id
}

fn log(profile: &mut Profile, name: &str) -> BindingTarget {
    BindingTarget::action(instance(
        profile,
        name,
        general::log_id(),
        &ActionParameters::new().with("message", ParameterValue::Text(name.into())),
    ))
}

fn workflow(profile: &mut Profile, name: &str, action: ActionInstanceId) -> WorkflowId {
    let workflow = support::graph_workflow(
        name,
        support::graph(
            &serde_json::json!([
                {"id": "start", "type": "trigger"},
                {"id": "step", "type": "action", "action": action},
            ]),
            &serde_json::json!([{"from": "start", "to": "step"}]),
        ),
    );
    let id = workflow.id;
    profile.workflows.push(workflow);
    id
}

struct Ids {
    profile: ProfileId,
    layout: LayoutId,
    home: PageId,
    folder: PageId,
    logged: WorkflowId,
}

fn profile() -> (Profile, Ids) {
    use deckpilot_modules::{http, system};
    let mut profile = Profile::new("Simulated");
    let down = log(&mut profile, "down");
    let up = log(&mut profile, "up");
    let click = log(&mut profile, "click");
    let double = log(&mut profile, "double");
    let long = log(&mut profile, "long");
    let one = log(&mut profile, "one");
    let two = log(&mut profile, "two");
    let BindingTarget::Action { action: step, .. } = log(&mut profile, "step") else {
        unreachable!()
    };
    let logged = workflow(&mut profile, "Logged", step);
    let command = instance(
        &mut profile,
        "Deploy",
        system::actions::id(system::actions::RUN_COMMAND),
        &ActionParameters::new().with(
            "program",
            ParameterValue::Resource(deckpilot_core::ResourceRef::Fixed {
                key: deckpilot_core::ResourceKey::new("tools.shell").unwrap(),
            }),
        ),
    );
    let deploying = workflow(&mut profile, "Release", command);
    let post = instance(
        &mut profile,
        "Webhook",
        deckpilot_core::ActionTypeId::new(http::actions::REQUEST).unwrap(),
        &ActionParameters::new()
            .with("method", ParameterValue::Choice("POST".into()))
            .with(
                "url",
                ParameterValue::Text("https://hooks.example.invalid/x".into()),
            ),
    );
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let home = layout.home_page;
    let mut folder = LayoutPage::new("Tools");
    folder.parent = Some(home);
    let folder_id = folder.id;
    layout.pages.push(folder);
    let page = layout.page_mut(home).unwrap();
    page.bindings = vec![
        ControlBinding::new(EDGES, Trigger::Press, down),
        ControlBinding::new(EDGES, Trigger::Release, up),
        ControlBinding::new(GESTURES, Trigger::Click, click),
        ControlBinding::new(GESTURES, Trigger::DoubleClick, double),
        ControlBinding::new(GESTURES, Trigger::LongPress, long),
        ControlBinding::new(CLICKS, Trigger::Click, one),
        ControlBinding::new(CLICKS, Trigger::DoubleClick, two),
        ControlBinding::new(
            WORKFLOW,
            Trigger::Press,
            BindingTarget::Workflow { workflow: logged },
        ),
        ControlBinding::new(COMMAND, Trigger::Press, BindingTarget::action(command)),
        ControlBinding::new(
            COMMAND_WORKFLOW,
            Trigger::Press,
            BindingTarget::Workflow {
                workflow: deploying,
            },
        ),
        ControlBinding::new(POST, Trigger::Press, BindingTarget::action(post)),
        ControlBinding::new(
            FOLDER,
            Trigger::Press,
            BindingTarget::OpenPage { page: folder_id },
        ),
    ];
    page.gestures = vec![KeyGestureSettings {
        control: GESTURES,
        double_click_ms: Some(KEY_DOUBLE_MS),
        long_press_ms: Some(KEY_LONG_MS),
    }];
    let ids = Ids {
        profile: profile.id,
        layout: layout.id,
        home,
        folder: folder_id,
        logged,
    };
    profile.layouts.push(layout);
    (profile, ids)
}

struct Setup {
    agent: TestAgent,
    hooks: Arc<Recorder>,
    device: DeviceId,
    ids: Ids,
    _dir: TempDir,
}

async fn setup() -> Setup {
    setup_with(|_, _| {}).await
}

/// [`setup`] with `change` made to the stored profile first.
async fn setup_with(change: impl FnOnce(&mut Profile, &Ids)) -> Setup {
    let dir = TempDir::new("deckpilot-simulate-press");
    let (mut profile, ids) = profile();
    change(&mut profile, &ids);
    let (paths, key) = data_dir(&dir, &profile, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "SIMULATE");
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
        ids,
        _dir: dir,
    }
}

impl Setup {
    fn request(&self, control: ControlId, kind: PressKind, confirmed: bool) -> Request {
        self.request_on(self.ids.home, control, kind, confirmed)
    }

    fn request_on(
        &self,
        page: PageId,
        control: ControlId,
        kind: PressKind,
        confirmed: bool,
    ) -> Request {
        Request::SimulatePress {
            device: self.device.clone(),
            profile: self.ids.profile,
            layout: self.ids.layout,
            page,
            control,
            kind,
            confirmed,
        }
    }

    async fn send(&self, request: Request) -> SimulationOutcome {
        match self.agent.request(request).await.unwrap() {
            Response::SimulatedPress { outcome } => outcome,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn press(&self, control: ControlId, kind: PressKind) -> SimulationOutcome {
        self.send(self.request(control, kind, false)).await
    }

    /// Runs a press and returns what it dispatched.
    async fn ran(&self, control: ControlId, kind: PressKind) -> Vec<T> {
        match self.press(control, kind).await {
            SimulationOutcome::Ran { dispatched } => dispatched,
            other => panic!("{control:?} {kind:?}: {other:?}"),
        }
    }

    fn seen(&self) -> Vec<(GestureFired, String)> {
        self.hooks.0.lock().unwrap().clone()
    }

    /// Waits until `count` gestures were dispatched.
    async fn gestures(&self, count: usize) -> Vec<(GestureFired, String)> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let seen = self.seen();
            if seen.len() >= count {
                return seen;
            }
            assert!(Instant::now() < deadline, "no gesture {count}: {seen:#?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn page(&self) -> Option<String> {
        self.agent
            .status()
            .await
            .devices
            .into_iter()
            .find(|d| d.id == self.device)
            .and_then(|d| d.page)
    }
}

/// The names of the next `count` actions that completed.
async fn completed(events: &mut AgentClient, count: usize) -> Vec<(String, ActionStatus)> {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let mut seen = Vec::new();
    while seen.len() < count {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("an action completed in time")
            .unwrap();
        if let AgentEvent::ActionCompleted { name, status, .. } = event {
            seen.push((name, status));
        }
    }
    seen
}

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn presses_are_recognized_like_physical_ones() {
    let s = setup().await;
    let mut events = s.agent.events().await;

    // A key without gestures gets its edges: press, then release.
    assert_eq!(
        s.ran(EDGES, PressKind::Single).await,
        [T::Press, T::Release]
    );
    let names: Vec<String> = completed(&mut events, 2)
        .await
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["down", "up"]);

    // A gesture key: a single press is a click once the key's own window
    // after the release has passed.
    assert_eq!(s.ran(GESTURES, PressKind::Single).await, [T::Click]);
    let seen = s.gestures(1).await;
    let (fired, message) = &seen[0];
    assert_eq!((fired.gesture, message.as_str()), (Gesture::Click, "click"));
    assert_eq!(
        fired.at - fired.pressed_at,
        ms(TAP_MS) + ms(KEY_DOUBLE_MS.into())
    );
    assert_ne!(fired.connection & SIMULATED, 0, "on the simulated lane");

    // A double press: one double click, never two clicks.
    assert_eq!(s.ran(GESTURES, PressKind::Double).await, [T::DoubleClick]);
    assert_eq!(s.ran(CLICKS, PressKind::Double).await, [T::DoubleClick]);
    // A long press: at the key's threshold, while held; the release
    // does nothing.
    assert_eq!(s.ran(GESTURES, PressKind::Long).await, [T::LongPress]);
    // Without a long-press binding, a long press is a click.
    assert_eq!(s.ran(CLICKS, PressKind::Long).await, [T::Click]);
    let seen = s.gestures(5).await;
    let got: Vec<(ControlId, Gesture, &str)> = seen[1..]
        .iter()
        .map(|(f, m)| (f.control, f.gesture, m.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (GESTURES, Gesture::DoubleClick, "double"),
            (CLICKS, Gesture::DoubleClick, "two"),
            (GESTURES, Gesture::LongPress, "long"),
            (CLICKS, Gesture::Click, "one"),
        ]
    );
    let long = &seen[3].0;
    assert_eq!(long.at - long.pressed_at, ms(KEY_LONG_MS.into()));

    // A workflow key starts its workflow, recorded as simulated.
    assert_eq!(
        s.ran(WORKFLOW, PressKind::Single).await,
        [T::Press, T::Release]
    );
    let runs = s
        .agent
        .wait_for_runs(s.ids.logged, |runs| !runs.is_empty())
        .await;
    assert_eq!(runs[0].trigger, ExecutionTrigger::Simulated);
    s.agent.shutdown().await;
}

/// A sequence of the physical key goes on untouched while the app
/// simulates presses of the same key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn physical_input_is_not_disturbed() {
    let s = setup().await;
    // The physical key goes down and stays down…
    s.agent
        .mock
        .send_input(&s.device, GESTURES, ControlInput::Press);
    tokio::time::sleep(ms(20)).await;
    // …while the app presses it twice.
    assert_eq!(s.ran(GESTURES, PressKind::Double).await, [T::DoubleClick]);
    // The physical release ends the physical sequence as a click, after
    // the key's window: neither a double click nor dropped.
    s.agent
        .mock
        .send_input(&s.device, GESTURES, ControlInput::Release);
    let seen = s.gestures(2).await;
    let lanes: Vec<(Gesture, bool)> = seen
        .iter()
        .map(|(f, _)| (f.gesture, f.connection & SIMULATED != 0))
        .collect();
    assert_eq!(
        lanes,
        [(Gesture::DoubleClick, true), (Gesture::Click, false)],
        "{seen:#?}"
    );
    tokio::time::sleep(ms(u64::from(KEY_DOUBLE_MS) + 100)).await;
    assert_eq!(s.seen().len(), 2, "{:#?}", s.seen());
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn presses_are_refused_unless_the_device_shows_what_the_app_shows() {
    let s = setup().await;
    let refused = |outcome: SimulationOutcome| match outcome {
        SimulationOutcome::Refused { code, .. } => code,
        other => panic!("not refused: {other:?}"),
    };
    let with = |change: &dyn Fn(&mut Request)| {
        let mut request = s.request(GESTURES, PressKind::Single, true);
        change(&mut request);
        request
    };
    macro_rules! field {
        ($field:ident, $value:expr) => {
            with(&|r: &mut Request| {
                if let Request::SimulatePress { $field, .. } = r {
                    *$field = $value;
                }
            })
        };
    }
    assert_eq!(
        refused(s.send(field!(profile, ProfileId::new())).await),
        SimulationRefusal::WrongProfile
    );
    assert_eq!(
        refused(s.send(field!(layout, LayoutId::new())).await),
        SimulationRefusal::WrongLayout
    );
    assert_eq!(
        refused(s.send(field!(page, s.ids.folder)).await),
        SimulationRefusal::WrongPage
    );
    assert_eq!(
        refused(s.send(field!(control, ControlId::Key { index: 99 })).await),
        SimulationRefusal::UnknownControl
    );
    let elsewhere = DeviceId {
        model: s.device.model.clone(),
        serial: "NOT-CONNECTED".into(),
    };
    assert_eq!(
        refused(s.send(field!(device, elsewhere.clone())).await),
        SimulationRefusal::DeviceNotConnected
    );
    assert!(s.seen().is_empty(), "nothing ran: {:#?}", s.seen());

    // A folder key navigates; the app then has to follow the device, and
    // the automatic Back key (not a binding) brings it home.
    assert_eq!(
        s.ran(FOLDER, PressKind::Single).await,
        [T::Press, T::Release]
    );
    assert_eq!(s.page().await.as_deref(), Some("Tools"));
    assert_eq!(
        refused(s.press(GESTURES, PressKind::Single).await),
        SimulationRefusal::WrongPage
    );
    let back = s.request_on(
        s.ids.folder,
        ControlId::Key { index: 0 },
        PressKind::Single,
        false,
    );
    assert!(matches!(s.send(back).await, SimulationOutcome::Ran { .. }));
    assert_eq!(s.page().await.as_deref(), Some("Home"));

    // Locked: refused, and no unlock prompt is involved.
    s.agent.request(Request::Lock).await.unwrap();
    assert_eq!(
        refused(s.press(GESTURES, PressKind::Single).await),
        SimulationRefusal::Locked
    );
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changes_outside_deckpilot_need_confirmation_and_the_settings_still_apply() {
    let s = setup().await;
    let mut events = s.agent.events().await;
    let ask = |outcome: SimulationOutcome| match outcome {
        SimulationOutcome::ConfirmationRequired { action_names, .. } => action_names,
        other => panic!("ran without confirmation: {other:?}"),
    };
    assert_eq!(ask(s.press(COMMAND, PressKind::Single).await), ["Deploy"]);
    assert_eq!(
        ask(s.press(COMMAND_WORKFLOW, PressKind::Single).await),
        ["Release", "Deploy"]
    );
    assert_eq!(ask(s.press(POST, PressKind::Single).await), ["Webhook"]);
    // Navigation, logging and other DeckPilot-only keys need none.
    assert!(matches!(
        s.press(EDGES, PressKind::Single).await,
        SimulationOutcome::Ran { .. }
    ));
    let (name, _) = completed(&mut events, 1).await.remove(0);
    assert_eq!(name, "down");
    let _ = completed(&mut events, 1).await;

    // Confirmed, it runs — and "Allow running commands" is still off on
    // this computer, so the command is refused as from a real key.
    assert!(matches!(
        s.send(s.request(COMMAND, PressKind::Single, true)).await,
        SimulationOutcome::Ran { dispatched } if dispatched == [T::Press, T::Release]
    ));
    let (name, status) = completed(&mut events, 1).await.remove(0);
    assert_eq!(
        (name.as_str(), status),
        ("Deploy", ActionStatus::Failed),
        "the setting gates it"
    );
    s.agent.shutdown().await;
}

/// An app of protocol 13 cannot simulate presses; everything else works.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_13_app_cannot_simulate() {
    let s = setup().await;
    let mut old = AgentClient::connect_with_protocol(&s.agent.paths, ClientKind::Desktop, 13)
        .await
        .unwrap();
    assert!(!old.supports(deckpilot_ipc::Capability::SimulateInput));
    assert!(matches!(
        old.request(s.request(GESTURES, PressKind::Single, true))
            .await,
        Err(ClientError::Unsupported { needs: 14, .. })
    ));
    assert!(matches!(
        old.request(Request::GetStatus).await,
        Ok(Response::Status { .. })
    ));
    tokio::time::sleep(ms(u64::from(KEY_DOUBLE_MS) + 100)).await;
    assert!(s.seen().is_empty(), "nothing ran: {:#?}", s.seen());
    s.agent.shutdown().await;
}

/// A folder key (`deckpilot_core::folder_control`) only opens its folder, on
/// the press, simulated exactly as pressed on the device: no gesture waits
/// for a double click, bindings a not yet migrated layout still has on the
/// key never run (not even after a confirmation), and recovered ones
/// neither.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_key_opens_on_the_press_simulated_as_physically() {
    let s = setup_with(|profile, ids| {
        let stale = log(profile, "stale");
        let command = profile
            .actions
            .iter()
            .find(|a| a.name == "Deploy")
            .unwrap()
            .id;
        let layout = &mut profile.layouts[0];
        layout.recovered.push(deckpilot_core::RecoveredBindings {
            page: ids.home,
            control: FOLDER,
            folder: ids.folder,
            bindings: vec![ControlBinding::new(FOLDER, Trigger::Click, stale.clone())],
            gestures: None,
            reason: deckpilot_core::RecoveryReason::FolderControl,
        });
        let page = layout.page_mut(ids.home).unwrap();
        page.bindings.extend([
            ControlBinding::new(FOLDER, Trigger::DoubleClick, stale),
            ControlBinding::new(FOLDER, Trigger::LongPress, BindingTarget::action(command)),
        ]);
    })
    .await;
    let home_again = |s: &Setup| {
        s.agent.mock.tap(&s.device, ControlId::Key { index: 0 });
    };
    for kind in [PressKind::Single, PressKind::Double, PressKind::Long] {
        // Raw edges, at once, without asking for the command (a double
        // press is two raw taps, the second on the folder's page, as on the
        // device): never a gesture.
        let ran = s.ran(FOLDER, kind).await;
        assert_eq!(ran[..2], [T::Press, T::Release], "{kind:?}");
        assert!(
            ran.iter().all(|t| matches!(t, T::Press | T::Release)),
            "{kind:?}: {ran:?}"
        );
        assert_eq!(s.page().await.as_deref(), Some("Tools"), "{kind:?}");
        home_again(&s);
        let deadline = Instant::now() + TIMEOUT;
        while s.page().await.as_deref() != Some("Home") {
            assert!(Instant::now() < deadline, "not back home");
            tokio::time::sleep(ms(5)).await;
        }
    }
    // The same key pressed on the device.
    s.agent.mock.tap(&s.device, FOLDER);
    let deadline = Instant::now() + TIMEOUT;
    while s.page().await.as_deref() != Some("Tools") {
        assert!(
            Instant::now() < deadline,
            "the physical press did not open it"
        );
        tokio::time::sleep(ms(5)).await;
    }
    // No gesture was dispatched: nothing but the open ran.
    assert!(s.seen().is_empty(), "{:#?}", s.seen());
    s.agent.shutdown().await;
}
