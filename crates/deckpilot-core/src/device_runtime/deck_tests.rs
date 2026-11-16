//! Deck requests in the device runtime: next and previous page along the
//! top-level pages, folders that close after use, the page a profile is
//! returned to, page placeholders in labels, and where a request is
//! refused.

use deckpilot_device::{ControlInput, models};

use super::*;
use crate::testing::{Echo, echo_type, registry};
use crate::{
    ActionHandler, ActionParameters, ActionType, ActionTypeId, Capability, DeckRequest,
    ExecutionContext, LayoutPage, ModuleId, OnFailure, Os, ParameterValue, ProfileTarget, Workflow,
    WorkflowStep,
};

/// `test.turn`: asks for what its `message` names.
struct Turn(Echo);

fn turn_type() -> ActionTypeId {
    ActionTypeId::new("test.turn").unwrap()
}

impl ActionHandler for Turn {
    fn definition(&self) -> &ActionType {
        self.0.definition()
    }

    fn execute(&self, parameters: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        let request = match parameters.text("message").unwrap_or_default() {
            "next" => DeckRequest::NextPage { wrap_around: true },
            "next-stop" => DeckRequest::NextPage { wrap_around: false },
            "previous" => DeckRequest::PreviousPage { wrap_around: true },
            "previous-stop" => DeckRequest::PreviousPage { wrap_around: false },
            "profile" => DeckRequest::SwitchProfile {
                target: ProfileTarget::Next,
                wrap_around: true,
            },
            "fail" => {
                return ActionResult::failure("failed")
                    .with_request(DeckRequest::NextPage { wrap_around: true });
            }
            _ => return ActionResult::success("nothing"),
        };
        ActionResult::success("asked").with_request(request)
    }
}

fn deck_registry() -> ActionRegistry {
    let mut registry = registry();
    registry
        .register(Box::new(Turn(Echo::with_id("test.turn"))))
        .unwrap();
    registry
}

fn runtime() -> Runtime {
    Runtime::new(
        Engine::new(deck_registry()),
        ModelCatalog::builtin(),
        MachineConfig::new(Os::current()),
    )
}

fn device(serial: &str) -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
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

fn press(runtime: &mut Runtime, device: &DeviceId, index: u16) -> RuntimeOutcome {
    runtime
        .handle_event(&DeviceEvent::Input {
            device: device.clone(),
            control: ControlId::Key { index },
            input: ControlInput::Press,
        })
        .unwrap()
}

fn bind(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(ControlId::Key { index }, Trigger::Press, target)
}

fn action(profile: &mut Profile, action_type: ActionTypeId, message: &str) -> BindingTarget {
    let instance = deck_registry()
        .create_instance(
            message,
            action_type,
            &ActionParameters::new().with("message", ParameterValue::Text(message.into())),
        )
        .unwrap();
    let target = BindingTarget::action(instance.id);
    profile.actions.push(instance);
    target
}

/// The pages of [`book`], by name.
struct Pages {
    home: PageId,
    two: PageId,
    three: PageId,
    folder: PageId,
    inner: PageId,
}

/// A profile whose layout has three top-level pages (Home, Two, Three), a
/// folder "Folder" in Two and a folder "Inner" in it. On every page: key 0
/// next, key 1 previous, key 2 next without wraparound, key 3 previous
/// without wraparound, key 4 an echo. Two: key 5 opens Folder. Folder: key
/// 5 opens Inner, key 6 the "Build" workflow.
fn book(name: &str) -> (Profile, Pages) {
    let mut profile = Profile::new(name);
    let next = action(&mut profile, turn_type(), "next");
    let previous = action(&mut profile, turn_type(), "previous");
    let next_stop = action(&mut profile, turn_type(), "next-stop");
    let previous_stop = action(&mut profile, turn_type(), "previous-stop");
    let echo = action(&mut profile, echo_type(), "echo");
    let mut workflow = Workflow::new("Build");
    workflow.steps.push(WorkflowStep::new(
        echo.action_id().unwrap(),
        OnFailure::Stop,
    ));
    let build = BindingTarget::Workflow {
        workflow: workflow.id,
    };
    profile.workflows.push(workflow);

    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    layout.explicit_tree = true;
    let keys = |page: &mut LayoutPage| {
        page.bindings.extend([
            bind(0, next.clone()),
            bind(1, previous.clone()),
            bind(2, next_stop.clone()),
            bind(3, previous_stop.clone()),
            bind(4, echo.clone()),
        ]);
    };
    let home = layout.home_page;
    keys(layout.page_mut(home).unwrap());
    let mut two = LayoutPage::new("Two");
    let mut three = LayoutPage::new("Three");
    let mut folder = LayoutPage::new("Folder");
    let mut inner = LayoutPage::new("Inner");
    folder.parent = Some(two.id);
    inner.parent = Some(folder.id);
    for page in [&mut two, &mut three, &mut folder, &mut inner] {
        keys(page);
    }
    two.bindings
        .push(bind(5, BindingTarget::OpenPage { page: folder.id }));
    folder
        .bindings
        .push(bind(5, BindingTarget::OpenPage { page: inner.id }));
    folder.bindings.push(bind(6, build));
    let pages = Pages {
        home,
        two: two.id,
        three: three.id,
        folder: folder.id,
        inner: inner.id,
    };
    // The folder sits between the pages in the list: only the tree counts.
    layout.pages.extend([two, folder, inner, three]);
    profile.layouts.push(layout);
    (profile, pages)
}

fn message(outcome: &RuntimeOutcome) -> &str {
    match outcome {
        RuntimeOutcome::Action { result, .. } => &result.message,
        other => panic!("not an action: {other:?}"),
    }
}

#[test]
fn next_and_previous_move_along_the_top_level_pages() {
    let (profile, pages) = book("Work");
    let layout = &profile.layouts[0];
    assert_eq!(
        layout.top_level_pages(),
        [pages.home, pages.two, pages.three]
    );
    assert_eq!(layout.page_position(pages.inner), Some((1, 3)));
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);

    let outcome = press(&mut runtime, &deck, 0);
    assert_eq!(message(&outcome), "page Two");
    let RuntimeOutcome::Action { result, .. } = &outcome else {
        unreachable!()
    };
    assert!(result.is_success());
    assert_eq!(result.request, None, "the request was applied");
    assert_eq!(result.data["pageName"], "Two");
    assert_eq!(result.data["pageId"], pages.two.to_string());
    assert_eq!(runtime.current_page(&deck), Some(pages.two));

    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(pages.three));
    // Past the last page: the first one, with wraparound.
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(pages.home));
    // And back before the first one: the last.
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(pages.three));
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(pages.two));
}

#[test]
fn without_wraparound_the_ends_stay_and_nothing_fails() {
    let (profile, pages) = book("Work");
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);

    let outcome = press(&mut runtime, &deck, 3);
    assert_eq!(message(&outcome), "there is no previous page");
    assert!(matches!(&outcome, RuntimeOutcome::Action { result, .. } if result.is_success()));
    assert_eq!(runtime.current_page(&deck), Some(pages.home));

    press(&mut runtime, &deck, 2);
    press(&mut runtime, &deck, 2);
    assert_eq!(runtime.current_page(&deck), Some(pages.three));
    let outcome = press(&mut runtime, &deck, 2);
    assert_eq!(message(&outcome), "there is no next page");
    assert_eq!(runtime.current_page(&deck), Some(pages.three));
}

#[test]
fn a_layout_with_one_page_has_nowhere_to_turn() {
    let mut profile = Profile::new("Solo");
    let next = action(&mut profile, turn_type(), "next");
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    // A folder of the home page, inferred from the key that opens it, is
    // not a page of the sequence.
    let folder = LayoutPage::new("Folder");
    layout.page_mut(home).unwrap().bindings.extend([
        bind(0, next),
        bind(1, BindingTarget::OpenPage { page: folder.id }),
    ]);
    layout.pages.push(folder);
    assert_eq!(layout.top_level_pages(), [home]);
    profile.layouts.push(layout);
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);
    let outcome = press(&mut runtime, &deck, 0);
    assert_eq!(message(&outcome), "there is no next page");
    assert_eq!(runtime.current_page(&deck), Some(home));
}

#[test]
fn inside_a_folder_turning_the_page_leaves_it() {
    let (profile, pages) = book("Work");
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);
    press(&mut runtime, &deck, 0);
    press(&mut runtime, &deck, 5);
    press(&mut runtime, &deck, 5);
    assert_eq!(runtime.current_page(&deck), Some(pages.inner));

    // Relative to Two, the page the folders belong to.
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(pages.three));
    // Nothing of the folders is left on the stack: Back has nowhere to go.
    assert!(matches!(
        runtime
            .dispatch(&deck, BindingTarget::Back, crate::RunOrigin::Key)
            .unwrap(),
        RuntimeOutcome::Navigated { page, .. } if page == pages.three
    ));

    press(&mut runtime, &deck, 1);
    press(&mut runtime, &deck, 5);
    assert_eq!(runtime.current_page(&deck), Some(pages.folder));
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(pages.home));
}

#[test]
fn devices_turn_their_own_pages() {
    let (profile, pages) = book("Work");
    let mut runtime = runtime();
    let (a, b) = (device("A"), device("B"));
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &a);
    connect(&mut runtime, &b);
    press(&mut runtime, &a, 0);
    assert_eq!(runtime.current_page(&a), Some(pages.two));
    assert_eq!(runtime.current_page(&b), Some(pages.home));
}

#[test]
fn a_folder_that_closes_returns_after_its_keys_ran_something() {
    let (mut profile, pages) = book("Work");
    profile.layouts[0]
        .page_mut(pages.folder)
        .unwrap()
        .auto_return = true;
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);
    press(&mut runtime, &deck, 0);
    let enter = |runtime: &mut Runtime| {
        press(runtime, &deck, 5);
        assert_eq!(runtime.current_page(&deck), Some(pages.folder));
    };

    // An action: it ran, then the folder closed.
    enter(&mut runtime);
    let outcome = press(&mut runtime, &deck, 4);
    assert_eq!(message(&outcome), "echo");
    assert_eq!(runtime.current_page(&deck), Some(pages.two));

    // A workflow: the folder closes when the run is asked for.
    enter(&mut runtime);
    assert!(matches!(
        press(&mut runtime, &deck, 6),
        RuntimeOutcome::WorkflowRequested { .. }
    ));
    assert_eq!(runtime.current_page(&deck), Some(pages.two));

    // Navigation keys do not close it: the inner folder opens.
    enter(&mut runtime);
    press(&mut runtime, &deck, 5);
    assert_eq!(runtime.current_page(&deck), Some(pages.inner));
    // The inner folder does not close by itself.
    press(&mut runtime, &deck, 4);
    assert_eq!(runtime.current_page(&deck), Some(pages.inner));
    // A key that is not bound does nothing.
    runtime
        .dispatch(&deck, BindingTarget::Back, crate::RunOrigin::Key)
        .unwrap();
    assert_eq!(runtime.current_page(&deck), Some(pages.folder));
    assert!(matches!(
        press(&mut runtime, &deck, 9),
        RuntimeOutcome::Ignored { .. }
    ));
    assert_eq!(runtime.current_page(&deck), Some(pages.folder));

    // An action that turned the page already left the folder: no second
    // step back from the new page.
    press(&mut runtime, &deck, 0);
    assert_eq!(runtime.current_page(&deck), Some(pages.three));
}

#[test]
fn a_page_outside_folders_never_closes_by_itself() {
    let (mut profile, pages) = book("Work");
    // The setting is ignored on a top-level page: there is no folder to
    // return to.
    profile.layouts[0]
        .page_mut(pages.three)
        .unwrap()
        .auto_return = true;
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);
    press(&mut runtime, &deck, 1);
    assert_eq!(runtime.current_page(&deck), Some(pages.three));
    press(&mut runtime, &deck, 4);
    assert_eq!(runtime.current_page(&deck), Some(pages.three));
}

#[test]
fn a_profile_is_returned_to_on_the_page_it_was_left_on() {
    let (work, pages) = book("Work");
    let (home, home_pages) = book("Home");
    let mut runtime = runtime();
    let (a, b) = (device("A"), device("B"));
    runtime.set_profile(Some(work.clone()), None);
    connect(&mut runtime, &a);
    connect(&mut runtime, &b);
    // A: inside the folder of Two. B: on Three.
    press(&mut runtime, &a, 0);
    press(&mut runtime, &a, 5);
    press(&mut runtime, &b, 1);
    assert_eq!(runtime.current_page(&a), Some(pages.folder));
    assert_eq!(runtime.current_page(&b), Some(pages.three));

    // Another profile starts on its home page.
    runtime.set_profile(Some(home.clone()), None);
    assert_eq!(runtime.current_page(&a), Some(home_pages.home));
    press(&mut runtime, &a, 0);
    assert_eq!(runtime.current_page(&a), Some(home_pages.two));

    // Back: each device on its page, the folder closed.
    runtime.set_profile(Some(work.clone()), None);
    assert_eq!(runtime.current_page(&a), Some(pages.two));
    assert_eq!(runtime.current_page(&b), Some(pages.three));
    runtime.set_profile(Some(home), None);
    assert_eq!(runtime.current_page(&a), Some(home_pages.two));
    assert_eq!(runtime.current_page(&b), Some(home_pages.home));

    // A page that is gone, or is inside a folder now: the home page.
    let mut changed = work;
    changed.layouts[0]
        .pages
        .retain(|page| page.id != pages.three);
    changed.layouts[0].page_mut(pages.two).unwrap().parent = Some(pages.home);
    runtime.set_profile(Some(changed), None);
    assert_eq!(runtime.current_page(&a), Some(pages.home));
    assert_eq!(runtime.current_page(&b), Some(pages.home));
}

#[test]
fn pages_of_an_earlier_run_are_restored_when_devices_connect() {
    let (work, pages) = book("Work");
    let mut first = runtime();
    let (a, b) = (device("A"), device("B"));
    first.set_profile(Some(work.clone()), None);
    connect(&mut first, &a);
    connect(&mut first, &b);
    press(&mut first, &a, 0);
    press(&mut first, &a, 5);
    let saved = first.remembered_pages();
    // The top-level page, not the folder. A device on its home page needs
    // no entry: nothing is saved for a deck that never left it.
    assert_eq!(saved, [(work.id, a.clone(), pages.two)]);
    // Back on the home page, the entry goes.
    let mut back = runtime();
    back.set_profile(Some(work.clone()), None);
    connect(&mut back, &a);
    assert_eq!(back.remembered_pages(), []);
    press(&mut back, &a, 0);
    assert_eq!(back.remembered_pages().len(), 1);
    press(&mut back, &a, 1);
    assert_eq!(back.remembered_pages(), []);

    let mut second = runtime();
    second.restore_pages(saved);
    second.set_profile(Some(work.clone()), None);
    connect(&mut second, &a);
    connect(&mut second, &b);
    assert_eq!(second.current_page(&a), Some(pages.two));
    assert_eq!(second.current_page(&b), Some(pages.home));

    // A saved page that does not exist is ignored.
    let mut third = runtime();
    third.restore_pages([(work.id, a.clone(), PageId::new())]);
    third.set_profile(Some(work), None);
    connect(&mut third, &a);
    assert_eq!(third.current_page(&a), Some(pages.home));
}

#[test]
fn a_device_that_was_unplugged_keeps_its_page_for_the_profile() {
    let (work, pages) = book("Work");
    let (home, _) = book("Home");
    let mut runtime = runtime();
    let a = device("A");
    runtime.set_profile(Some(work.clone()), None);
    connect(&mut runtime, &a);
    press(&mut runtime, &a, 0);
    runtime
        .handle_event(&DeviceEvent::Disconnected { device: a.clone() })
        .unwrap();
    // Unplugged while Work is left and entered again.
    runtime.set_profile(Some(home), None);
    runtime.set_profile(Some(work), None);
    connect(&mut runtime, &a);
    assert_eq!(runtime.current_page(&a), Some(pages.two));
}

#[test]
fn labels_show_where_the_device_is() {
    let (mut profile, pages) = book("Work");
    let layout = &mut profile.layouts[0];
    for page in &mut layout.pages {
        page.bindings.push(ControlBinding {
            label: Some("{page.number}/{page.count}".into()),
            ..bind(7, BindingTarget::Back)
        });
        page.bindings.push(ControlBinding {
            label: Some("{page.name} of {profile}".into()),
            ..bind(8, BindingTarget::Back)
        });
    }
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);
    let title = |runtime: &Runtime, index: u16| {
        runtime
            .faces(&deck)
            .into_iter()
            .find(|(control, _)| *control == ControlId::Key { index })
            .unwrap()
            .1
            .title
    };
    assert_eq!(title(&runtime, 7), "1/3");
    assert_eq!(title(&runtime, 8), "Home of {profile}");
    press(&mut runtime, &deck, 0);
    assert_eq!(title(&runtime, 7), "2/3");
    // In a folder: the position of its page, and the folder's own name.
    press(&mut runtime, &deck, 5);
    assert_eq!(runtime.current_page(&deck), Some(pages.folder));
    assert_eq!(title(&runtime, 7), "2/3");
    assert_eq!(title(&runtime, 8), "Folder of {profile}");
    // Labels without a page placeholder are untouched.
    assert_eq!(title(&runtime, 4), "echo");
}

#[test]
fn a_page_request_needs_a_device() {
    let (profile, pages) = book("Work");
    let next = profile.actions[0].id;
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);
    // "Run" in the app: no device asked, so no device moves.
    let result = runtime.run_action(next).unwrap();
    assert!(!result.is_success());
    assert_eq!(result.code.as_deref(), Some(NO_DEVICE));
    assert_eq!(result.request, None);
    assert_eq!(runtime.current_page(&deck), Some(pages.home));
}

#[test]
fn a_profile_switch_is_left_for_the_agent() {
    let (mut profile, pages) = book("Work");
    let switch = action(&mut profile, turn_type(), "profile");
    let switch_id = switch.action_id().unwrap();
    let home = profile.layouts[0].home_page;
    profile.layouts[0]
        .page_mut(home)
        .unwrap()
        .bindings
        .push(bind(9, switch));
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);
    let expected = Some(DeckRequest::SwitchProfile {
        target: ProfileTarget::Next,
        wrap_around: true,
    });
    // From a key and from the app: the runtime changes nothing and hands
    // the request on.
    let RuntimeOutcome::Action { result, .. } = press(&mut runtime, &deck, 9) else {
        panic!("not an action");
    };
    assert_eq!(result.deck_request(), expected);
    assert_eq!(runtime.current_page(&deck), Some(pages.home));
    assert_eq!(
        runtime.run_action(switch_id).unwrap().deck_request(),
        expected
    );
}

#[test]
fn a_request_is_refused_where_nothing_applies_it() {
    let (mut profile, pages) = book("Work");
    let next = profile.actions[0].id;
    let mut workflow = Workflow::new("Turns");
    workflow
        .steps
        .push(WorkflowStep::new(next, OnFailure::Stop));
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);

    // As a workflow step the action fails with a code instead of
    // reporting a success that did nothing.
    let report = runtime.run_workflow(workflow_id).unwrap();
    assert_eq!(report.status, crate::ActionStatus::Failed);
    let step = &report.steps[0].result;
    assert_eq!(step.code.as_deref(), Some(crate::runtime::NOT_FROM_A_KEY));
    assert_eq!(step.request, None);
    assert_eq!(runtime.current_page(&deck), Some(pages.home));
}

#[test]
fn a_failed_action_asks_for_nothing() {
    let (mut profile, pages) = book("Work");
    let failing = action(&mut profile, turn_type(), "fail");
    let home = profile.layouts[0].home_page;
    profile.layouts[0]
        .page_mut(home)
        .unwrap()
        .bindings
        .push(bind(9, failing));
    let mut runtime = runtime();
    let deck = device("A");
    runtime.set_profile(Some(profile), None);
    connect(&mut runtime, &deck);
    let RuntimeOutcome::Action { result, .. } = press(&mut runtime, &deck, 9) else {
        panic!("not an action");
    };
    assert!(!result.is_success());
    assert_eq!(result.request, None);
    assert_eq!(runtime.current_page(&deck), Some(pages.home));
}

#[test]
fn a_module_needs_the_capability_to_ask() {
    let profile = Profile::new("Work");
    let machine = MachineConfig::new(Os::Linux);
    let context = ExecutionContext::new(&profile, None, &machine);
    let module = ModuleId::new("test").unwrap();
    let register = |capabilities: &[Capability]| {
        let mut registry = ActionRegistry::default();
        registry
            .register_for_module(
                module.clone(),
                capabilities,
                Box::new(Turn(Echo::with_id("test.turn"))),
            )
            .unwrap();
        registry
    };
    let ask = ActionParameters::new().with("message", ParameterValue::Text("next".into()));
    let denied = register(&[Capability::ProfileRead]).dispatch(&turn_type(), &ask, &context);
    assert!(!denied.is_success());
    assert!(
        denied
            .message
            .contains("may not change pages and profiles on the deck"),
        "{}",
        denied.message
    );
    assert_eq!(denied.request, None);

    let allowed = register(&[Capability::DeckControl]).dispatch(&turn_type(), &ask, &context);
    assert_eq!(
        allowed.deck_request(),
        Some(DeckRequest::NextPage { wrap_around: true })
    );
}

#[test]
fn pages_without_the_setting_serialize_as_before() {
    let page = LayoutPage::new("Folder");
    let json = serde_json::to_value(&page).unwrap();
    assert!(json.get("autoReturn").is_none(), "{json}");
    let mut closing = page.clone();
    closing.auto_return = true;
    let json = serde_json::to_value(&closing).unwrap();
    assert_eq!(json["autoReturn"], true);
    assert_eq!(serde_json::from_value::<LayoutPage>(json).unwrap(), closing);
    // A page written before the setting existed reads as not closing.
    let old = serde_json::json!({ "id": page.id, "name": "Folder", "bindings": [] });
    assert!(
        !serde_json::from_value::<LayoutPage>(old)
            .unwrap()
            .auto_return
    );
}
