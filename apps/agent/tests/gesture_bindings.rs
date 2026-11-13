//! Gesture bindings end to end: layouts that bind click, double click and
//! long press on a simulated MK.2, dispatched by the agent through
//! `Runtime::dispatch_gesture` (`docs/gestures-and-work-session.md`,
//! section 3). The engine's exact timing is tested in the core; these
//! tests use real time with generous margins.
//!
//! The hooks supply no bindings (the page's are used); they only record
//! what each dispatched gesture did.

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_agent::GestureHooks;
use deckpilot_core::gesture::{
    BoundGestures, Gesture, GestureFired, GestureTimings, KeyGestureSettings,
};
use deckpilot_core::variables::{
    ScopeKey, VariableDefinition, VariableKey, VariableRef, VariableScope, VariableType,
    VariableValue,
};
use deckpilot_core::{
    ActionInstanceId, ActionParameters, BindingTarget, ControlBinding, DeviceLayout, OnFailure,
    PageId, ParameterValue, Profile, RuntimeOutcome, Trigger, Workflow, WorkflowId,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::{AgentClient, AgentEvent, AgentPaths};
use deckpilot_modules::general;
use deckpilot_storage::ConfigStore;
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use support::{TIMEOUT, TempDir, TestAgent, data_dir, registry};

/// Click: "click"; double click: the same action with the shared variable
/// `team.name`; long press: the same action with "long".
const GESTURE_KEY: u16 = 2;
/// Click: "tap"; long press: the workflow.
const WORKFLOW_KEY: u16 = 3;
/// A press binding from before gestures: fires on the press.
const LEGACY_KEY: u16 = 0;

const DOUBLE_MS: u16 = 1000;
const LONG_MS: u16 = 500;

/// What a dispatched gesture did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Did {
    Ran(String),
    Workflow(WorkflowId),
    Navigated(PageId),
    Nothing,
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<(u16, Gesture, Did)>>,
}

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
        let ControlId::Key { index } = fired.control else {
            panic!("only keys have gestures here");
        };
        let did = match outcome {
            RuntimeOutcome::Action { result, .. } => Did::Ran(result.message.clone()),
            RuntimeOutcome::WorkflowRequested { workflow } => Did::Workflow(*workflow),
            RuntimeOutcome::Navigated { page, .. } => Did::Navigated(*page),
            _ => Did::Nothing,
        };
        self.seen.lock().unwrap().push((index, fired.gesture, did));
    }
}

impl Recorder {
    fn seen(&self) -> Vec<(u16, Gesture, Did)> {
        self.seen.lock().unwrap().clone()
    }
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
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

fn message(value: ParameterValue) -> ActionParameters {
    ActionParameters::new().with("message", value)
}

struct Setup {
    agent: TestAgent,
    hooks: Arc<Recorder>,
    device: DeviceId,
    say: ActionInstanceId,
    workflow: WorkflowId,
    _dir: TempDir,
}

fn profile() -> (Profile, ActionInstanceId, WorkflowId) {
    let mut profile = Profile::new("Gesture bindings");
    let say = log(&mut profile, "Say", "click");
    let legacy = log(&mut profile, "Legacy", "legacy");
    let tap = log(&mut profile, "Tap", "tap");
    let workflow = Workflow::new("Held").step(say, OnFailure::Stop);
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);

    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let page = &mut layout.pages[0];
    let gesture = key(GESTURE_KEY);
    page.bindings = vec![
        ControlBinding::new(
            key(LEGACY_KEY),
            Trigger::Press,
            BindingTarget::action(legacy),
        ),
        ControlBinding::new(gesture, Trigger::Click, BindingTarget::action(say)),
        ControlBinding::new(
            gesture,
            Trigger::DoubleClick,
            BindingTarget::Action {
                action: say,
                overrides: message(ParameterValue::Reference(
                    VariableRef::parse("shared.team.name").unwrap(),
                )),
            },
        ),
        ControlBinding::new(
            gesture,
            Trigger::LongPress,
            BindingTarget::Action {
                action: say,
                overrides: message(ParameterValue::Text("long".into())),
            },
        ),
        ControlBinding::new(
            key(WORKFLOW_KEY),
            Trigger::Click,
            BindingTarget::action(tap),
        ),
        ControlBinding::new(
            key(WORKFLOW_KEY),
            Trigger::LongPress,
            BindingTarget::Workflow {
                workflow: workflow_id,
            },
        ),
    ];
    page.gestures = vec![
        KeyGestureSettings {
            control: gesture,
            double_click_ms: Some(DOUBLE_MS),
            long_press_ms: Some(LONG_MS),
        },
        KeyGestureSettings {
            control: key(WORKFLOW_KEY),
            double_click_ms: None,
            long_press_ms: Some(LONG_MS),
        },
    ];
    profile.layouts.push(layout);
    (profile, say, workflow_id)
}

async fn setup() -> Setup {
    let dir = TempDir::new("deckpilot-gesture-bindings");
    let (profile, say, workflow) = profile();
    let (paths, db_key) = data_dir(&dir, &profile, &[]);
    let store = support::open_store(&paths, &db_key);
    let name = VariableKey::new("team.name").unwrap();
    store
        .save_shared_variable(&VariableDefinition::new(
            name.clone(),
            VariableType::String,
            &[VariableScope::Global],
        ))
        .unwrap();
    store
        .set_shared_variable_value(
            &name,
            &ScopeKey::Global,
            &VariableValue::String("crew".into()),
        )
        .unwrap();
    drop(store);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "BINDINGS-1");
    let hooks = Arc::new(Recorder::default());
    let agent = TestAgent::start_with_gestures(&paths, &db_key, &mock, None, Some(hooks.clone()));
    agent.unlock().await;
    let setup = Setup {
        agent,
        hooks,
        device,
        say,
        workflow,
        _dir: dir,
    };
    wait_connected(&setup.agent, &setup.device).await;
    setup
}

async fn wait_connected(agent: &TestAgent, device: &DeviceId) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if status
            .devices
            .iter()
            .any(|d| d.id == *device && d.state == ConnectionState::Connected)
        {
            return;
        }
        assert!(Instant::now() < deadline, "{device} did not connect");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

impl Setup {
    fn send(&self, index: u16, input: ControlInput) {
        self.agent.mock.send_input(&self.device, key(index), input);
    }

    fn tap(&self, index: u16) {
        self.agent.mock.tap(&self.device, key(index));
    }

    async fn wait_for(&self, what: &str, predicate: impl Fn(&[(u16, Gesture, Did)]) -> bool) {
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

    /// Lets every window and threshold pass, then checks what was
    /// dispatched.
    async fn settle_and_expect(&self, expected: &[(u16, Gesture, Did)]) {
        tokio::time::sleep(Duration::from_millis(u64::from(DOUBLE_MS) + 500)).await;
        assert_eq!(self.hooks.seen(), expected);
    }
}

fn ran(index: u16, gesture: Gesture, text: &str) -> (u16, Gesture, Did) {
    (index, gesture, Did::Ran(text.into()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn click_double_click_and_long_press_run_their_own_bindings() {
    let s = setup().await;

    // One tap: a click, after the double-click window.
    s.tap(GESTURE_KEY);
    s.wait_for("click", |seen| !seen.is_empty()).await;
    // Two taps: one double click, with the shared variable's value in
    // place of the saved message.
    s.tap(GESTURE_KEY);
    s.tap(GESTURE_KEY);
    s.wait_for("double click", |seen| seen.len() >= 2).await;
    // A hold: one long press while the key is down, with its own
    // override; holding on and releasing does nothing more, and never
    // clicks.
    s.send(GESTURE_KEY, ControlInput::Press);
    s.wait_for("long press", |seen| seen.len() >= 3).await;
    tokio::time::sleep(Duration::from_millis(u64::from(LONG_MS) * 2)).await;
    s.send(GESTURE_KEY, ControlInput::Release);
    s.settle_and_expect(&[
        ran(GESTURE_KEY, Gesture::Click, "click"),
        ran(GESTURE_KEY, Gesture::DoubleClick, "crew"),
        ran(GESTURE_KEY, Gesture::LongPress, "long"),
    ])
    .await;

    // The saved instance is unchanged, and no instance was added.
    let stored = s
        .agent
        .store()
        .load_profile(s.agent.status().await.active_profile.unwrap().id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.actions.len(), 3);
    assert_eq!(
        stored.action(s.say).unwrap().parameters.text("message"),
        Some("click")
    );
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_press_starts_a_workflow_and_a_click_does_not() {
    let s = setup().await;
    // Long press bound: the click fires on the release.
    s.tap(WORKFLOW_KEY);
    s.wait_for("click", |seen| !seen.is_empty()).await;
    s.send(WORKFLOW_KEY, ControlInput::Press);
    s.wait_for("long press", |seen| seen.len() >= 2).await;
    s.send(WORKFLOW_KEY, ControlInput::Release);
    s.agent
        .wait_for_runs(s.workflow, |runs| {
            runs.len() == 1 && runs[0].state == deckpilot_core::ExecutionState::Succeeded
        })
        .await;
    s.settle_and_expect(&[
        ran(WORKFLOW_KEY, Gesture::Click, "tap"),
        (WORKFLOW_KEY, Gesture::LongPress, Did::Workflow(s.workflow)),
    ])
    .await;
    assert_eq!(s.agent.runs(s.workflow).await.len(), 1);
    s.agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_press_binding_from_before_gestures_fires_on_the_press() {
    let s = setup().await;
    let mut events = s.agent.events().await;
    // Pressed and held: the action runs without waiting for the release.
    s.send(LEGACY_KEY, ControlInput::Press);
    let completed = next_action(&mut events).await;
    assert!(
        matches!(&completed, AgentEvent::ActionCompleted { name, .. } if name == "Legacy"),
        "{completed:?}"
    );
    s.send(LEGACY_KEY, ControlInput::Release);
    // No gesture is involved.
    s.settle_and_expect(&[]).await;
    s.agent.shutdown().await;
}

async fn next_action(events: &mut AgentClient) -> AgentEvent {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("no action completed in time")
            .unwrap();
        if matches!(event, AgentEvent::ActionCompleted { .. }) {
            return event;
        }
    }
}

/// The database of 0.6.0's validation seed, upgraded by this agent: its
/// keys still act on the press, as they did.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_of_a_0_6_0_database_still_act_on_the_press() {
    let dir = TempDir::new("deckpilot-gesture-legacy");
    let paths = AgentPaths::in_dir(dir.path().join("data"));
    paths.create_dirs().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/deckpilot-storage/tests/fixtures/v0.6.0");
    std::fs::copy(fixture.join("deckpilot.db"), &paths.database).unwrap();
    std::fs::copy(
        fixture.join("deckpilot.vault.json"),
        paths.data_dir.join("deckpilot.vault.json"),
    )
    .unwrap();
    let db_key = Vault::new(&paths.database)
        .unlock(&Password::new("deckpilot-dev"))
        .unwrap();
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "LEGACY-1");
    let hooks = Arc::new(Recorder::default());
    let agent = TestAgent::start_with_gestures(&paths, &db_key, &mock, None, Some(hooks.clone()));
    agent.unlock().await;
    wait_connected(&agent, &device).await;
    let mut events = agent.events().await;

    // Key 0 of the seed's MK.2 layout switches to the next project.
    mock.send_input(&device, key(0), ControlInput::Press);
    let completed = next_action(&mut events).await;
    assert!(
        matches!(&completed, AgentEvent::ActionCompleted { .. }),
        "{completed:?}"
    );
    mock.send_input(&device, key(0), ControlInput::Release);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(hooks.seen(), []);
    agent.shutdown().await;
}
