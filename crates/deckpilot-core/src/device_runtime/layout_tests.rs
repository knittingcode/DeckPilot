//! Layout navigation in the device runtime: *Switch layout* and *Back to
//! previous layout* per profile and device, with shared layouts, several
//! devices, profile switches, restarts and layouts that went away.

use deckpilot_device::{ControlInput, models};

use super::*;
use crate::testing::{Echo, registry};
use crate::{
    ActionHandler, ActionParameters, ActionType, ActionTypeId, DeckRequest, ExecutionContext,
    HistoryMode, LayoutPage, Os, ParameterValue,
};

/// `test.layout`: `switch:<layout>[:<page>][:replace|clear]` or `back`.
struct Navigate(Echo);

fn navigate_type() -> ActionTypeId {
    ActionTypeId::new("test.layout").unwrap()
}

impl ActionHandler for Navigate {
    fn definition(&self) -> &ActionType {
        self.0.definition()
    }

    fn execute(&self, parameters: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        let message = parameters.text("message").unwrap_or_default();
        let request = if message == "back" {
            DeckRequest::PreviousLayout
        } else {
            let mut parts = message.trim_start_matches("switch:").split(':');
            let layout = parts.next().unwrap().parse().unwrap();
            let mut page = None;
            let mut history = HistoryMode::Push;
            for part in parts {
                match HistoryMode::parse(part) {
                    Some(mode) => history = mode,
                    None => page = Some(part.parse().unwrap()),
                }
            }
            DeckRequest::SwitchLayout {
                layout,
                page,
                history,
            }
        };
        ActionResult::success("asked").with_request(request)
    }
}

fn runtime() -> Runtime {
    let mut registry = registry();
    registry
        .register(Box::new(Navigate(Echo::with_id("test.layout"))))
        .unwrap();
    Runtime::new(
        Engine::new(registry),
        ModelCatalog::builtin(),
        MachineConfig::new(Os::current()),
    )
}

fn mk2() -> DeviceModelId {
    DeviceModelId::new(models::STREAM_DECK_MK2_ID)
}

fn device(serial: &str) -> DeviceId {
    DeviceId {
        model: mk2(),
        serial: serial.into(),
    }
}

fn connect(runtime: &mut Runtime, device: &DeviceId) {
    runtime
        .handle_event(&DeviceEvent::Connected {
            device: device.clone(),
        })
        .unwrap();
}

fn press(runtime: &mut Runtime, device: &DeviceId, index: u16) -> ActionResult {
    match runtime
        .handle_event(&DeviceEvent::Input {
            device: device.clone(),
            control: ControlId::Key { index },
            input: ControlInput::Press,
        })
        .unwrap()
    {
        RuntimeOutcome::Action { result, .. } => result,
        other => panic!("expected an action, got {other:?}"),
    }
}

fn shown(runtime: &Runtime, device: &DeviceId) -> String {
    runtime.layout_of(device).unwrap().name.clone()
}

/// Layouts of a set: each binds key `i` to switch to the `i`-th layout of
/// `names`, key 14 to Back to previous layout.
struct Set {
    layouts: Vec<DeviceLayout>,
    actions: Vec<crate::ActionInstance>,
}

fn layout_set(names: &[&str]) -> Set {
    let mut layouts: Vec<DeviceLayout> = names
        .iter()
        .map(|name| {
            let mut layout = DeviceLayout::new(*name, mk2());
            layout.pages.push(LayoutPage::new("Second"));
            layout
        })
        .collect();
    let mut actions = Vec::new();
    let mut make = |message: String| {
        let instance = crate::ActionInstance {
            id: crate::ActionInstanceId::new(),
            name: message.clone(),
            action_type: navigate_type(),
            parameters: ActionParameters::new().with("message", ParameterValue::Text(message)),
            description: String::new(),
            icon: None,
            label: None,
        };
        let id = instance.id;
        actions.push(instance);
        id
    };
    let targets: Vec<crate::ActionInstanceId> = layouts
        .iter()
        .map(|layout| make(format!("switch:{}", layout.id)))
        .collect();
    let back = make("back".into());
    for layout in &mut layouts {
        let home = layout.home_page;
        let page = layout.page_mut(home).unwrap();
        for (index, target) in targets.iter().enumerate() {
            page.bindings.push(ControlBinding::new(
                ControlId::Key {
                    index: u16::try_from(index).unwrap(),
                },
                Trigger::Press,
                BindingTarget::action(*target),
            ));
        }
        page.bindings.push(ControlBinding::new(
            ControlId::Key { index: 14 },
            Trigger::Press,
            BindingTarget::action(back),
        ));
    }
    Set { layouts, actions }
}

/// A profile that uses every layout of `set` (the first is its layout for
/// the MK.2), with the set's actions.
fn profile_with(name: &str, set: &Set) -> Profile {
    let mut profile = Profile::new(name);
    profile.layouts = set.layouts.clone();
    profile.actions = set.actions.clone();
    profile
}

#[test]
fn main_teams_meeting_and_back_twice() {
    let set = layout_set(&["Main", "Teams", "Meeting"]);
    let mut runtime = runtime();
    runtime.set_profile(Some(profile_with("Work", &set)), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    assert_eq!(shown(&runtime, &deck), "Main");

    let result = press(&mut runtime, &deck, 1);
    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.data["layoutName"], "Teams");
    assert_eq!(shown(&runtime, &deck), "Teams");
    press(&mut runtime, &deck, 2);
    assert_eq!(shown(&runtime, &deck), "Meeting");
    assert_eq!(
        runtime.layout_history(&deck),
        vec![set.layouts[0].id, set.layouts[1].id]
    );

    press(&mut runtime, &deck, 14);
    assert_eq!(shown(&runtime, &deck), "Teams");
    press(&mut runtime, &deck, 14);
    assert_eq!(shown(&runtime, &deck), "Main");
    // Empty history: a documented no-op.
    let result = press(&mut runtime, &deck, 14);
    assert!(result.is_success());
    assert_eq!(result.message, "there is no previous layout");
    assert_eq!(shown(&runtime, &deck), "Main");
    assert!(runtime.take_device_layouts_changed());
}

#[test]
fn switching_to_the_shown_layout_and_rapid_presses_add_nothing() {
    let set = layout_set(&["Main", "Teams"]);
    let mut runtime = runtime();
    runtime.set_profile(Some(profile_with("Work", &set)), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    let result = press(&mut runtime, &deck, 0);
    assert_eq!(result.message, "layout Main is shown");
    assert_eq!(runtime.layout_history(&deck).len(), 0);
    for _ in 0..10 {
        press(&mut runtime, &deck, 1);
    }
    assert_eq!(shown(&runtime, &deck), "Teams");
    assert_eq!(runtime.layout_history(&deck), vec![set.layouts[0].id]);
}

#[test]
fn back_returns_to_the_page_that_was_shown() {
    let set = layout_set(&["Main", "Teams"]);
    let mut runtime = runtime();
    let mut profile = profile_with("Work", &set);
    runtime.set_profile(Some(profile.clone()), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    // Main's second page has no keys; go there by switching with a page.
    let second = set.layouts[0].pages[1].id;
    let to_second = crate::ActionInstance {
        parameters: ActionParameters::new().with(
            "message",
            ParameterValue::Text(format!("switch:{}:{second}", set.layouts[0].id)),
        ),
        ..set.actions[0].clone()
    };
    profile.actions[0] = to_second;
    runtime.reload_profile(profile);
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(second));
    // Leave Main from its second page by switching on a device that shows
    // Teams' bindings: use the runtime directly.
    let mut result = ActionResult::success("asked").with_request(DeckRequest::SwitchLayout {
        layout: set.layouts[1].id,
        page: None,
        history: HistoryMode::Push,
    });
    runtime.apply_page_request(&deck, &mut result);
    assert_eq!(shown(&runtime, &deck), "Teams");
    assert_eq!(runtime.current_page(&deck), Some(set.layouts[1].home_page));
    press(&mut runtime, &deck, 14);
    assert_eq!(shown(&runtime, &deck), "Main");
    assert_eq!(runtime.current_page(&deck), Some(second));
}

#[test]
fn unassigned_deleted_and_incompatible_targets_are_refused() {
    let set = layout_set(&["Main", "Teams"]);
    let mut runtime = runtime();
    // Teams is not assigned to this profile.
    let mut profile = profile_with("Work", &set);
    profile.layouts.truncate(1);
    runtime.set_profile(Some(profile), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    let result = press(&mut runtime, &deck, 1);
    assert_eq!(result.code.as_deref(), Some(LAYOUT_UNAVAILABLE));
    assert_eq!(shown(&runtime, &deck), "Main");
    assert_eq!(runtime.layout_history(&deck).len(), 0);

    // A Stream Deck + layout on an MK.2.
    let plus = DeviceLayout::new("Dials", DeviceModelId::new(models::STREAM_DECK_PLUS_ID));
    let mut profile = runtime.profile().unwrap().clone();
    profile.layouts.push(plus.clone());
    runtime.reload_profile(profile);
    let mut result = ActionResult::success("asked").with_request(DeckRequest::SwitchLayout {
        layout: plus.id,
        page: None,
        history: HistoryMode::Push,
    });
    runtime.apply_page_request(&deck, &mut result);
    assert_eq!(result.code.as_deref(), Some(LAYOUT_INCOMPATIBLE));
    assert!(
        result.message.contains("Stream Deck +"),
        "{}",
        result.message
    );
    assert_eq!(shown(&runtime, &deck), "Main");
    assert_ne!(runtime.profile().unwrap().name.len(), 0);
}

#[test]
fn a_layout_that_goes_away_is_forgotten_and_back_skips_it() {
    let set = layout_set(&["Main", "Teams", "Meeting"]);
    let mut runtime = runtime();
    let mut profile = profile_with("Work", &set);
    runtime.set_profile(Some(profile.clone()), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    press(&mut runtime, &deck, 1);
    press(&mut runtime, &deck, 2);
    // Teams is unassigned, then Meeting (shown) too.
    profile.layouts.remove(1);
    runtime.reload_profile(profile.clone());
    assert_eq!(runtime.layout_history(&deck), vec![set.layouts[0].id]);
    assert_eq!(shown(&runtime, &deck), "Meeting");
    profile.layouts.remove(1);
    runtime.reload_profile(profile);
    // The device falls back to the profile's layout for the model, at its
    // home page; nothing refers to Meeting any more.
    assert_eq!(shown(&runtime, &deck), "Main");
    assert_eq!(runtime.current_page(&deck), Some(set.layouts[0].home_page));
    assert_eq!(runtime.device_layouts().len(), 0);
}

#[test]
fn devices_and_profiles_keep_their_own_layouts_and_histories() {
    // One set of layouts shared by two profiles.
    let set = layout_set(&["Main", "Teams", "Media"]);
    let work = profile_with("Work", &set);
    let home = profile_with("Home", &set);
    let mut runtime = runtime();
    runtime.set_profile(Some(work.clone()), None);
    let (a, b) = (device("A"), device("B"));
    connect(&mut runtime, &a);
    connect(&mut runtime, &b);

    // Work: A goes Main → Teams; B stays on Main.
    press(&mut runtime, &a, 1);
    assert_eq!(shown(&runtime, &a), "Teams");
    assert_eq!(shown(&runtime, &b), "Main");

    // Home: both start on Main; A goes Main → Media.
    runtime.set_profile(Some(home.clone()), None);
    assert_eq!(shown(&runtime, &a), "Main");
    assert_eq!(runtime.layout_history(&a).len(), 0);
    press(&mut runtime, &a, 2);
    assert_eq!(shown(&runtime, &a), "Media");

    // Back in Work: A shows Teams again, and Back goes to Main, not Media.
    runtime.set_profile(Some(work), None);
    assert_eq!(shown(&runtime, &a), "Teams");
    assert_eq!(runtime.layout_history(&a), vec![set.layouts[0].id]);
    press(&mut runtime, &a, 14);
    assert_eq!(shown(&runtime, &a), "Main");
    // The profile never changed with a layout.
    assert_eq!(runtime.profile().unwrap().name, "Work");

    runtime.set_profile(Some(home), None);
    press(&mut runtime, &a, 14);
    assert_eq!(shown(&runtime, &a), "Main");
    assert_eq!(runtime.profile().unwrap().name, "Home");
}

#[test]
fn a_restart_restores_the_layouts_but_not_the_history() {
    let set = layout_set(&["Main", "Teams"]);
    let profile = profile_with("Work", &set);
    let mut runtime = runtime();
    runtime.set_profile(Some(profile.clone()), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    press(&mut runtime, &deck, 1);
    let saved = runtime.device_layouts();
    assert_eq!(saved, vec![(profile.id, deck.clone(), set.layouts[1].id)]);

    let mut restarted = self::runtime();
    // A saved entry for a layout that is gone is ignored.
    let gone = (profile.id, device("B"), LayoutId::new());
    restarted.set_profile(Some(profile), None);
    restarted.restore_device_layouts(saved.into_iter().chain([gone]));
    connect(&mut restarted, &deck);
    connect(&mut restarted, &device("B"));
    assert_eq!(shown(&restarted, &deck), "Teams");
    assert_eq!(shown(&restarted, &device("B")), "Main");
    assert_eq!(restarted.layout_history(&deck).len(), 0);
    assert_eq!(restarted.device_layouts().len(), 1);
    let result = press(&mut restarted, &deck, 14);
    assert_eq!(result.message, "there is no previous layout");
}

#[test]
fn faces_and_status_follow_the_layout_the_device_shows() {
    let set = layout_set(&["Main", "Teams"]);
    let mut runtime = runtime();
    runtime.set_profile(Some(profile_with("Work", &set)), None);
    let deck = device("A");
    connect(&mut runtime, &deck);
    press(&mut runtime, &deck, 1);
    match runtime.device_layout_status(&deck) {
        LayoutStatus::Compatible { layout, name } => {
            assert_eq!((layout, name.as_str()), (set.layouts[1].id, "Teams"));
        }
        other => panic!("{other:?}"),
    }
    // The model's layout is still Main for devices that did not switch.
    assert_eq!(runtime.layout_for(&mk2()).unwrap().name, "Main");
    assert_ne!(runtime.faces(&deck).len(), 0);
}

#[test]
fn layout_requests_need_a_device() {
    let set = layout_set(&["Main", "Teams"]);
    let mut runtime = runtime();
    runtime.set_profile(Some(profile_with("Work", &set)), None);
    let result = runtime.run_action(set.actions[1].id).unwrap();
    assert_eq!(result.code.as_deref(), Some(NO_DEVICE));
}
