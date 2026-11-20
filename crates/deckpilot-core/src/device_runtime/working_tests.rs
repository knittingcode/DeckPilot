//! Working layouts of the editor's live preview, and the editor's
//! navigation, in the device runtime (`working.rs`): a clock the tests
//! move, two simulated MK.2s.

use std::sync::Mutex;
use std::time::Duration;

use deckpilot_device::{ControlInput, models};

use super::*;
use crate::testing::{Echo, registry};
use crate::{
    ActionHandler, ActionParameters, ActionType, ActionTypeId, AutoExit, DeckRequest,
    ExecutionContext, HistoryMode, LayoutPage, Os, ParameterValue,
};

/// `test.switch`: switches to the layout in `message`.
struct Switch(Echo);

impl ActionHandler for Switch {
    fn definition(&self) -> &ActionType {
        self.0.definition()
    }

    fn execute(&self, parameters: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        let layout = parameters.text("message").unwrap().parse().unwrap();
        ActionResult::success("asked").with_request(DeckRequest::SwitchLayout {
            layout,
            page: None,
            history: HistoryMode::Push,
        })
    }
}

#[derive(Clone)]
struct TestClock(Arc<Mutex<Instant>>);

impl TestClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }

    fn advance(&self, seconds: u64) -> Instant {
        let mut now = self.0.lock().unwrap();
        *now += Duration::from_secs(seconds);
        *now
    }
}

fn device(serial: &str) -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
        serial: serial.into(),
    }
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn bind(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, target)
}

/// Main: Home (key 0 runs Run, key 1 opens Tools, key 2 switches to
/// Second), Tools (a folder with the reserved Back on key 0, Auto Exit
/// after 10 s; key 2 runs Run). Second: an empty layout.
struct Fixture {
    profile: Profile,
    main: LayoutId,
    second: LayoutId,
    home: PageId,
    tools: PageId,
    run: ActionInstanceId,
}

fn fixture() -> Fixture {
    let mut profile = Profile::new("Work");
    let run = crate::ActionInstance {
        id: ActionInstanceId::new(),
        name: "Run".into(),
        action_type: crate::testing::echo_type(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    };
    let run_id = run.id;
    let second = DeviceLayout::new("Second", device("A").model);
    let switch = crate::ActionInstance {
        id: ActionInstanceId::new(),
        name: "Switch".into(),
        action_type: ActionTypeId::new("test.switch").unwrap(),
        parameters: ActionParameters::new()
            .with("message", ParameterValue::Text(second.id.to_string())),
        description: String::new(),
        icon: None,
        label: None,
    };
    let switch_id = switch.id;
    profile.actions.extend([run, switch]);
    let mut main = DeviceLayout::new("Main", device("A").model);
    let home = main.home_page;
    let mut tools = LayoutPage::folder("Tools", home);
    tools.auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 10,
    });
    tools.bindings.push(bind(2, BindingTarget::action(run_id)));
    let tools_id = tools.id;
    main.pages.push(tools);
    main.page_mut(home).unwrap().bindings.extend([
        bind(0, BindingTarget::action(run_id)),
        bind(1, BindingTarget::OpenPage { page: tools_id }),
        bind(2, BindingTarget::action(switch_id)),
    ]);
    main.explicit_tree = true;
    let (main_id, second_id) = (main.id, second.id);
    profile.layouts.extend([main, second]);
    Fixture {
        profile,
        main: main_id,
        second: second_id,
        home,
        tools: tools_id,
        run: run_id,
    }
}

fn started(f: &Fixture, clock: &TestClock) -> (Runtime, DeviceId, DeviceId) {
    let mut registry = registry();
    registry
        .register(Box::new(Switch(Echo::with_id("test.switch"))))
        .unwrap();
    let mut runtime = Runtime::new(
        Engine::new(registry),
        ModelCatalog::builtin(),
        MachineConfig::new(Os::current()),
    );
    let clock = clock.clone();
    runtime.set_clock(move || clock.now());
    runtime.set_profile(Some(f.profile.clone()), None);
    let (a, b) = (device("A"), device("B"));
    for deck in [&a, &b] {
        runtime
            .handle_event(&DeviceEvent::Connected {
                device: deck.clone(),
            })
            .unwrap();
    }
    (runtime, a, b)
}

fn clock() -> TestClock {
    TestClock(Arc::new(Mutex::new(Instant::now())))
}

fn press(runtime: &mut Runtime, deck: &DeviceId, index: u16) -> RuntimeOutcome {
    runtime
        .handle_event(&DeviceEvent::Input {
            device: deck.clone(),
            control: key(index),
            input: ControlInput::Press,
        })
        .unwrap()
}

fn main_of(f: &Fixture) -> DeviceLayout {
    f.profile.layout(f.main).unwrap().clone()
}

fn face(runtime: &Runtime, deck: &DeviceId, index: u16) -> KeyFace {
    runtime
        .faces(deck)
        .into_iter()
        .find(|(control, _)| *control == key(index))
        .unwrap()
        .1
}

fn stack(runtime: &Runtime, deck: &DeviceId) -> Vec<PageId> {
    runtime.navigation_state(deck).stack
}

#[test]
fn a_working_layout_is_shown_and_run_by_its_device_only() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, b) = started(&f, &clock);
    let saved_face = face(&runtime, &a, 0);
    // Unsaved: key 0 opens Tools instead of running Run, and Tools is
    // renamed: a folder key shows its folder's name, its one title (a
    // title of the binding's own is not shown).
    let mut working = main_of(&f);
    working.page_mut(f.tools).unwrap().name = "Unsaved".into();
    let home = working.page_mut(f.home).unwrap();
    home.bindings.retain(|binding| binding.control != key(0));
    let mut folder = bind(0, BindingTarget::OpenPage { page: f.tools });
    folder.label = Some("Stale title".into());
    home.bindings.push(folder);
    runtime.set_working_layout(&a, f.profile.id, working.clone());
    assert_eq!(face(&runtime, &a, 0).title, "Unsaved");
    // The other device shows the same layout as saved.
    assert_eq!(face(&runtime, &b, 0), saved_face);
    assert_eq!(runtime.layout_of(&a), Some(&working));
    assert_eq!(runtime.saved_layout_of(&a), f.profile.layout(f.main));
    // A press runs the unsaved binding on A, the saved one on B.
    assert!(matches!(
        press(&mut runtime, &a, 0),
        RuntimeOutcome::Navigated { page, .. } if page == f.tools
    ));
    assert!(matches!(
        press(&mut runtime, &b, 0),
        RuntimeOutcome::Action { action, .. } if action == f.run
    ));
    // Dropped: the saved layout again, on the page shown (it exists).
    assert!(runtime.clear_working_layout(&a));
    assert!(!runtime.clear_working_layout(&a));
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
    press(&mut runtime, &a, 0);
    assert_eq!(face(&runtime, &a, 0), saved_face);
}

#[test]
fn unsaved_overrides_run_on_a_physical_press() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, _) = started(&f, &clock);
    let mut working = main_of(&f);
    let binding = working
        .page_mut(f.home)
        .unwrap()
        .bindings
        .iter_mut()
        .find(|binding| binding.control == key(0))
        .unwrap();
    binding.target = BindingTarget::Action {
        action: f.run,
        overrides: ActionParameters::new().with("message", ParameterValue::Text("unsaved".into())),
    };
    runtime.set_working_layout(&a, f.profile.id, working);
    match press(&mut runtime, &a, 0) {
        RuntimeOutcome::Action { result, .. } => assert_eq!(result.message, "unsaved"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_working_layout_of_another_profile_or_layout_is_inert() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, _) = started(&f, &clock);
    let saved = face(&runtime, &a, 0);
    let mut working = main_of(&f);
    working.page_mut(f.home).unwrap().bindings[0].label = Some("Other profile".into());
    runtime.set_working_layout(&a, ProfileId::new(), working.clone());
    assert_eq!(face(&runtime, &a, 0), saved);
    // For Main, but the device shows Second: suspended until it is back.
    runtime.set_working_layout(&a, f.profile.id, working);
    press(&mut runtime, &a, 2);
    assert_eq!(runtime.navigation_state(&a).layout, Some(f.second));
    runtime.navigate(&a, &EditorTarget::LayoutBack).unwrap();
    assert_eq!(face(&runtime, &a, 0).title, "Other profile");
}

#[test]
fn editor_navigation_runs_nothing_and_starts_no_countdown() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, b) = started(&f, &clock);
    runtime
        .navigate(
            &a,
            &EditorTarget::Show {
                layout: f.main,
                stack: vec![f.home, f.tools],
            },
        )
        .unwrap();
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
    assert_eq!(runtime.next_folder_exit(), None);
    // The other device stays where it is.
    assert_eq!(stack(&runtime, &b), [f.home]);
    // Entering by a key starts the countdown; it ends back home.
    press(&mut runtime, &b, 1);
    assert!(runtime.next_folder_exit().is_some());
    let now = clock.advance(11);
    for exit in runtime.due_folder_exits(now) {
        runtime.exit_folder(&exit);
    }
    assert_eq!(stack(&runtime, &b), [f.home]);
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
    // Refusals change nothing.
    let unknown = EditorTarget::Show {
        layout: f.main,
        stack: vec![PageId::new()],
    };
    assert_eq!(
        runtime.navigate(&a, &unknown),
        Err(NavigationError::UnknownPage)
    );
    let other = EditorTarget::Show {
        layout: LayoutId::new(),
        stack: vec![f.home],
    };
    assert_eq!(
        runtime.navigate(&a, &other),
        Err(NavigationError::UnknownLayout)
    );
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
}

#[test]
fn device_navigation_is_reported_in_both_directions() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, _) = started(&f, &clock);
    // A folder key, then the protected Back.
    press(&mut runtime, &a, 1);
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
    press(&mut runtime, &a, 0);
    assert_eq!(stack(&runtime, &a), [f.home]);
    // Auto Exit.
    press(&mut runtime, &a, 1);
    let now = clock.advance(11);
    for exit in runtime.due_folder_exits(now) {
        runtime.exit_folder(&exit);
    }
    assert_eq!(stack(&runtime, &a), [f.home]);
    // Switch layout by a key, then Layout Back from the editor.
    press(&mut runtime, &a, 2);
    let state = runtime.navigation_state(&a);
    assert_eq!(state.layout, Some(f.second));
    assert_eq!(state.profile, Some(f.profile.id));
    assert!(state.connected);
    runtime.navigate(&a, &EditorTarget::LayoutBack).unwrap();
    assert_eq!(runtime.navigation_state(&a).layout, Some(f.main));
    assert_eq!(
        runtime.navigate(&a, &EditorTarget::LayoutBack),
        Err(NavigationError::NoPreviousLayout)
    );
    // The editor switches layouts: the device can go back with Layout Back.
    let home_of_second = f.profile.layout(f.second).unwrap().home_page;
    runtime
        .navigate(
            &a,
            &EditorTarget::Show {
                layout: f.second,
                stack: vec![home_of_second],
            },
        )
        .unwrap();
    assert_eq!(runtime.navigation_state(&a).layout, Some(f.second));
    assert_eq!(runtime.layout_history(&a), [f.main]);
    // Disconnected: the parked stack, and no navigation.
    runtime
        .handle_event(&DeviceEvent::Disconnected { device: a.clone() })
        .unwrap();
    let parked = runtime.navigation_state(&a);
    assert!(!parked.connected);
    assert_eq!(parked.stack, [home_of_second]);
    assert_eq!(
        runtime.navigate(&a, &EditorTarget::LayoutBack),
        Err(NavigationError::NotConnected)
    );
}

#[test]
fn unsaved_pages_are_navigable_and_a_revert_goes_to_the_nearest_page() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, _) = started(&f, &clock);
    let mut working = main_of(&f);
    let sub = LayoutPage::folder("Sub", f.tools);
    let sub_id = sub.id;
    working.pages.push(sub);
    runtime.set_working_layout(&a, f.profile.id, working.clone());
    runtime
        .navigate(
            &a,
            &EditorTarget::Show {
                layout: f.main,
                stack: vec![f.home, f.tools, sub_id],
            },
        )
        .unwrap();
    assert_eq!(runtime.current_page(&a), Some(sub_id));
    // Never remembered: only saved pages are.
    assert_eq!(runtime.remembered_pages(), []);
    // Revert (the saved layout as the next revision): the folder it was in.
    runtime.set_working_layout(&a, f.profile.id, main_of(&f));
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
    // Editing on: the page again, then the session ends without saving.
    runtime.set_working_layout(&a, f.profile.id, working);
    runtime
        .navigate(
            &a,
            &EditorTarget::Show {
                layout: f.main,
                stack: vec![f.home, f.tools, sub_id],
            },
        )
        .unwrap();
    runtime.clear_working_layout(&a);
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
}

#[test]
fn a_reload_keeps_the_nearest_page_that_is_still_there() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, b) = started(&f, &clock);
    press(&mut runtime, &a, 1);
    // Saved meanwhile: unchanged pages keep the device where it is.
    runtime.reload_profile(f.profile.clone());
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
    // Tools deleted elsewhere: home, the page it was opened from.
    let mut profile = f.profile.clone();
    profile
        .layouts
        .iter_mut()
        .find(|layout| layout.id == f.main)
        .unwrap()
        .pages
        .retain(|page| page.id != f.tools);
    runtime.reload_profile(profile);
    assert_eq!(stack(&runtime, &a), [f.home]);
    assert_eq!(stack(&runtime, &b), [f.home]);
}

#[test]
fn saving_the_working_layout_keeps_navigation_and_faces() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, _) = started(&f, &clock);
    let mut working = main_of(&f);
    working.page_mut(f.tools).unwrap().bindings[0].label = Some("Saved soon".into());
    runtime.set_working_layout(&a, f.profile.id, working.clone());
    press(&mut runtime, &a, 1);
    let faces = runtime.faces(&a);
    // Saved: the profile now has the working layout; nothing moves.
    let mut profile = f.profile.clone();
    *profile
        .layouts
        .iter_mut()
        .find(|layout| layout.id == f.main)
        .unwrap() = working;
    runtime.reload_profile(profile);
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
    assert_eq!(runtime.faces(&a), faces);
    runtime.clear_working_layout(&a);
    assert_eq!(runtime.faces(&a), faces);
    assert_eq!(stack(&runtime, &a), [f.home, f.tools]);
}

#[test]
fn auto_exit_follows_the_working_layout() {
    let f = fixture();
    let clock = clock();
    let (mut runtime, a, _) = started(&f, &clock);
    // Unsaved: Auto Exit of Tools off.
    let mut working = main_of(&f);
    working.page_mut(f.tools).unwrap().auto_exit = Some(AutoExit::disabled());
    runtime.set_working_layout(&a, f.profile.id, working);
    press(&mut runtime, &a, 1);
    assert_eq!(runtime.next_folder_exit(), None);
}
