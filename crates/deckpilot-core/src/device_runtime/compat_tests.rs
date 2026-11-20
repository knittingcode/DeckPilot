//! Action/control compatibility in the device runtime: a binding whose
//! action cannot run from its control is refused (nothing runs), reported
//! once, shown as unavailable, and kept; layouts of other models are
//! untouched (`docs/device-controls.md`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use deckpilot_device::{ControlInput, ControlKind, InputCapability, models};

use super::*;
use crate::graph::NodeSpec;
use crate::{
    ActionControls, ActionHandler, ActionParameters, ActionType, ActionTypeId, ExecutionContext,
    IncompatibilityReason, LayoutIssue, LayoutPage, Os,
};

/// Counts its runs.
struct Counting {
    definition: ActionType,
    runs: Arc<AtomicUsize>,
}

impl ActionHandler for Counting {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(&self, _: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        self.runs.fetch_add(1, Ordering::SeqCst);
        ActionResult::success("ran")
    }
}

fn definition(id: &str, controls: Option<ActionControls>) -> ActionType {
    ActionType {
        editor_navigation: None,
        title: None,
        id: ActionTypeId::new(id).unwrap(),
        name: id.into(),
        description: String::new(),
        category: "Test".into(),
        parameters: Vec::new(),
        requires_project: false,
        node: NodeSpec::default(),
        deprecated: false,
        controls,
    }
}

const KEY_ONLY: &str = "test.key-only";
const DIAL: &str = "test.dial";
const LEGACY: &str = "test.legacy";
const STEPS: &str = "test.steps";

struct Setup {
    runtime: Runtime,
    runs: BTreeMap<&'static str, Arc<AtomicUsize>>,
    actions: BTreeMap<&'static str, ActionInstanceId>,
}

impl Setup {
    fn runs(&self, id: &str) -> usize {
        self.runs[id].load(Ordering::SeqCst)
    }
}

fn types() -> Vec<(&'static str, Option<ActionControls>)> {
    vec![
        (KEY_ONLY, Some(ActionControls::keys())),
        (
            DIAL,
            Some(ActionControls::new().with(
                ControlKind::Encoder,
                [
                    InputCapability::RotateClockwise,
                    InputCapability::RotateCounterClockwise,
                ],
            )),
        ),
        (LEGACY, None),
        (STEPS, Some(ActionControls::keys_and_steps())),
    ]
}

fn plus() -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_PLUS_ID),
        serial: "PLUS".into(),
    }
}

fn mk2() -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
        serial: "MK2".into(),
    }
}

const DIAL_0: ControlId = ControlId::Encoder { index: 0 };
const DIAL_1: ControlId = ControlId::Encoder { index: 1 };
const DIAL_2: ControlId = ControlId::Encoder { index: 2 };
const KEY_0: ControlId = ControlId::Key { index: 0 };
const KEY_1: ControlId = ControlId::Key { index: 1 };
const KEY_2: ControlId = ControlId::Key { index: 2 };

/// A Stream Deck + layout with compatible and incompatible bindings, and
/// an MK.2 layout with key bindings only.
fn profile(actions: &BTreeMap<&'static str, ActionInstanceId>) -> Profile {
    let mut profile = Profile::new("Work");
    let run = |id: &str| BindingTarget::action(actions[id]);
    let mut layout = DeviceLayout::new("Plus", DeviceModelId::new(models::STREAM_DECK_PLUS_ID));
    layout.pages[0].bindings = vec![
        // Incompatible: a key-only action on a dial rotation.
        ControlBinding::new(DIAL_0, Trigger::RotateClockwise, run(KEY_ONLY)),
        // Compatible: the dial action on a rotation.
        ControlBinding::new(DIAL_1, Trigger::RotateClockwise, run(DIAL)),
        // Incompatible: an action from before declarations on a dial.
        ControlBinding::new(DIAL_2, Trigger::RotateCounterClockwise, run(LEGACY)),
        // Compatible: a key-only action on a key.
        ControlBinding::new(KEY_0, Trigger::Press, run(KEY_ONLY)),
        // Incompatible: the dial action on a key.
        ControlBinding::new(KEY_1, Trigger::Press, run(DIAL)),
        // Incompatible: a long press the dial action does not support.
        ControlBinding::new(KEY_2, Trigger::LongPress, run(DIAL)),
    ];
    let mut keys = DeviceLayout::new("Keys", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    keys.pages[0].bindings = vec![
        ControlBinding::new(KEY_0, Trigger::Press, run(LEGACY)),
        ControlBinding::new(KEY_1, Trigger::Click, run(KEY_ONLY)),
        ControlBinding::new(KEY_2, Trigger::Press, run(DIAL)),
    ];
    profile.layouts.extend([layout, keys]);
    profile
}

fn setup() -> Setup {
    let mut registry = ActionRegistry::default();
    let mut runs = BTreeMap::new();
    let mut actions = BTreeMap::new();
    let mut instances = Vec::new();
    for (id, controls) in types() {
        let count = Arc::new(AtomicUsize::new(0));
        registry
            .register(Box::new(Counting {
                definition: definition(id, controls),
                runs: count.clone(),
            }))
            .unwrap();
        runs.insert(id, count);
        let instance = registry
            .create_instance(id, ActionTypeId::new(id).unwrap(), &ActionParameters::new())
            .unwrap();
        actions.insert(id, instance.id);
        instances.push(instance);
    }
    let mut profile = profile(&actions);
    profile.actions = instances;
    let mut runtime = Runtime::new(
        Engine::new(registry),
        ModelCatalog::builtin(),
        MachineConfig::new(Os::current()),
    );
    runtime.set_profile(Some(profile), None);
    for device in [plus(), mk2()] {
        runtime
            .handle_event(&DeviceEvent::Connected { device })
            .unwrap();
    }
    Setup {
        runtime,
        runs,
        actions,
    }
}

fn input(device: &DeviceId, control: ControlId, input: ControlInput) -> DeviceEvent {
    DeviceEvent::Input {
        device: device.clone(),
        control,
        input,
    }
}

#[test]
fn an_encoder_rotation_bound_to_a_key_only_action_does_not_run_it() {
    let mut s = setup();
    let outcome = s
        .runtime
        .handle_event(&input(&plus(), DIAL_0, ControlInput::Rotate { delta: 1 }))
        .unwrap();
    assert_eq!(
        outcome,
        RuntimeOutcome::Unavailable {
            action: s.actions[KEY_ONLY],
            control: DIAL_0,
            trigger: Trigger::RotateClockwise,
            reason: IncompatibilityReason::ControlKindUnsupported {
                kind: ControlKind::Encoder,
                supported: vec![ControlKind::Key],
            },
            first: true,
        }
    );
    assert_eq!(s.runs(KEY_ONLY), 0);
    // Reported once: the second refusal is not the first.
    let again = s
        .runtime
        .handle_event(&input(&plus(), DIAL_0, ControlInput::Rotate { delta: 3 }))
        .unwrap();
    assert!(matches!(
        again,
        RuntimeOutcome::Unavailable { first: false, .. }
    ));
    assert_eq!(s.runs(KEY_ONLY), 0);

    // The same action runs from a key.
    let pressed = s
        .runtime
        .handle_event(&input(&plus(), KEY_0, ControlInput::Press))
        .unwrap();
    assert!(matches!(pressed, RuntimeOutcome::Action { .. }));
    assert_eq!(s.runs(KEY_ONLY), 1);
}

#[test]
fn compatible_bindings_run_and_incompatible_ones_never_do() {
    let mut s = setup();
    // The dial action runs from a rotation.
    let outcome = s
        .runtime
        .handle_event(&input(&plus(), DIAL_1, ControlInput::Rotate { delta: 1 }))
        .unwrap();
    assert!(matches!(outcome, RuntimeOutcome::Action { .. }));
    assert_eq!(s.runs(DIAL), 1);
    // ...not from a key, raw or as a gesture.
    let key = s
        .runtime
        .handle_event(&input(&plus(), KEY_1, ControlInput::Press))
        .unwrap();
    assert!(matches!(key, RuntimeOutcome::Unavailable { .. }));
    let gesture = s
        .runtime
        .dispatch_gesture(&plus(), KEY_2, Gesture::LongPress)
        .unwrap();
    assert!(matches!(
        gesture,
        RuntimeOutcome::Unavailable {
            trigger: Trigger::LongPress,
            first: true,
            ..
        }
    ));
    assert_eq!(s.runs(DIAL), 1);
    // A type without a declaration is keys only.
    let legacy = s
        .runtime
        .handle_event(&input(&plus(), DIAL_2, ControlInput::Rotate { delta: -1 }))
        .unwrap();
    assert!(matches!(legacy, RuntimeOutcome::Unavailable { .. }));
    assert_eq!(s.runs(LEGACY), 0);
    let on_a_key = s
        .runtime
        .handle_event(&input(&mk2(), KEY_0, ControlInput::Press))
        .unwrap();
    assert!(matches!(on_a_key, RuntimeOutcome::Action { .. }));
    assert_eq!(s.runs(LEGACY), 1);
}

#[test]
fn refusals_are_reported_again_after_a_reload_and_bindings_are_kept() {
    let mut s = setup();
    let before = s.runtime.profile().unwrap().clone();
    let rotate = input(&plus(), DIAL_0, ControlInput::Rotate { delta: 1 });
    assert!(matches!(
        s.runtime.handle_event(&rotate).unwrap(),
        RuntimeOutcome::Unavailable { first: true, .. }
    ));
    s.runtime.reload_profile(before.clone());
    assert!(matches!(
        s.runtime.handle_event(&rotate).unwrap(),
        RuntimeOutcome::Unavailable { first: true, .. }
    ));
    // Nothing was removed or converted.
    assert_eq!(s.runtime.profile().unwrap(), &before);
}

#[test]
fn unavailable_bindings_and_faces() {
    let s = setup();
    let found: Vec<(String, ControlId, Trigger)> = s
        .runtime
        .unavailable_bindings()
        .into_iter()
        .map(|(model, found)| (model.to_string(), found.control, found.trigger))
        .collect();
    let plus_model = models::STREAM_DECK_PLUS_ID.to_owned();
    let mk2_model = models::STREAM_DECK_MK2_ID.to_owned();
    assert_eq!(
        found,
        vec![
            (plus_model.clone(), DIAL_0, Trigger::RotateClockwise),
            (plus_model.clone(), DIAL_2, Trigger::RotateCounterClockwise),
            (plus_model.clone(), KEY_1, Trigger::Press),
            (plus_model, KEY_2, Trigger::LongPress),
            (mk2_model, KEY_2, Trigger::Press),
        ]
    );
    // The key whose binding cannot run shows as unavailable; the others
    // as their action.
    let faces: BTreeMap<ControlId, KeyFace> = s.runtime.faces(&plus()).into_iter().collect();
    assert_eq!(faces[&KEY_1].detail.as_deref(), Some("Unavailable"));
    assert_eq!(faces[&KEY_1].title, DIAL);
    assert_eq!(faces[&KEY_0].style, FaceStyle::Action);
    assert_eq!(faces[&KEY_0].detail, None);
    // No live state is asked for it.
    let shown: Vec<ControlId> = s
        .runtime
        .shown_targets(&plus())
        .into_iter()
        .map(|(control, _)| control)
        .collect();
    assert!(shown.contains(&KEY_0) && !shown.contains(&KEY_1));

    // The layout status explains.
    let LayoutStatus::Invalid { issues, .. } = s.runtime.layout_status(&plus().model) else {
        panic!("incompatible bindings are reported");
    };
    assert_eq!(issues.len(), 4);
    assert!(
        issues[0].contains("the action cannot be bound to an encoder; it works on keys only"),
        "{issues:?}"
    );
}

#[test]
fn layouts_of_other_models_are_untouched_when_devices_switch() {
    let mut s = setup();
    let before = s.runtime.profile().unwrap().layouts.clone();
    // The MK.2 disconnects, the Stream Deck + is used, the MK.2 comes back.
    s.runtime
        .handle_event(&DeviceEvent::Disconnected { device: mk2() })
        .unwrap();
    let _ = s
        .runtime
        .handle_event(&input(&plus(), DIAL_0, ControlInput::Rotate { delta: 1 }));
    let _ = s
        .runtime
        .handle_event(&input(&plus(), KEY_0, ControlInput::Press));
    s.runtime
        .handle_event(&DeviceEvent::Connected { device: mk2() })
        .unwrap();
    let clicked = s
        .runtime
        .dispatch_gesture(&mk2(), KEY_1, Gesture::Click)
        .unwrap();
    assert!(matches!(clicked, RuntimeOutcome::Action { .. }));
    // Each model keeps its own layout, byte for byte.
    let after = &s.runtime.profile().unwrap().layouts;
    assert_eq!(
        serde_json::to_string(after).unwrap(),
        serde_json::to_string(&before).unwrap()
    );
    assert!(matches!(
        s.runtime.layout_status(&mk2().model),
        LayoutStatus::Invalid { ref issues, .. } if issues.len() == 1
    ));
}

#[test]
fn a_restored_action_type_makes_its_bindings_available_again() {
    let mut s = setup();
    // The dial action's module is gone: its bindings are unavailable as
    // an unknown type, not as incompatible ones.
    let mut registry = ActionRegistry::default();
    for (id, controls) in types().into_iter().filter(|(id, _)| *id != DIAL) {
        registry
            .register(Box::new(Counting {
                definition: definition(id, controls),
                runs: s.runs[id].clone(),
            }))
            .unwrap();
    }
    s.runtime.set_engine(Engine::new(registry));
    assert!(
        s.runtime
            .unavailable_bindings()
            .iter()
            .all(|(_, found)| found.action != s.actions[DIAL])
    );
    // The type returns, now declared for keys too: the key binding runs.
    let mut registry = ActionRegistry::default();
    for (id, controls) in types() {
        let controls = if id == DIAL {
            controls.map(|c| c.with(ControlKind::Key, [InputCapability::Press]))
        } else {
            controls
        };
        registry
            .register(Box::new(Counting {
                definition: definition(id, controls),
                runs: s.runs[id].clone(),
            }))
            .unwrap();
    }
    s.runtime.set_engine(Engine::new(registry));
    let outcome = s
        .runtime
        .handle_event(&input(&plus(), KEY_1, ControlInput::Press))
        .unwrap();
    assert!(matches!(outcome, RuntimeOutcome::Action { .. }));
    assert_eq!(s.runs(DIAL), 1);
}

#[test]
fn layout_checks_report_incompatible_actions() {
    let s = setup();
    let profile = s.runtime.profile().unwrap();
    let registry = s.runtime.engine().registry();
    let catalog = ModelCatalog::builtin();
    let plus_layout = &profile.layouts[0];
    let issues = plus_layout.check_compatibility(&catalog, registry, profile);
    assert_eq!(issues.len(), 4);
    assert!(matches!(
        &issues[0],
        LayoutIssue::IncompatibleAction { control, trigger: Trigger::RotateClockwise, .. }
            if *control == DIAL_0
    ));
    // `validate` (no action types) is unchanged.
    assert_eq!(plus_layout.validate(&catalog, profile), Vec::new());

    // Saving: bindings that are stored stay; new ones are refused.
    let mut edited = plus_layout.clone();
    edited.pages[0].bindings.push(ControlBinding::new(
        ControlId::Encoder { index: 3 },
        Trigger::RotateClockwise,
        BindingTarget::action(s.actions[KEY_ONLY]),
    ));
    let (refused, kept) =
        edited.compatibility_for_save(Some(plus_layout), &catalog, registry, profile);
    assert_eq!(kept.len(), 4);
    assert_eq!(refused.len(), 1);
    assert!(matches!(
        &refused[0],
        LayoutIssue::IncompatibleAction {
            control: ControlId::Encoder { index: 3 },
            ..
        }
    ));
    // Unchanged, a stored layout always saves.
    let (refused, kept) =
        plus_layout.compatibility_for_save(Some(plus_layout), &catalog, registry, profile);
    assert_eq!(refused, Vec::new());
    assert_eq!(kept.len(), 4);
    // Moving an incompatible binding makes it new.
    let mut moved = plus_layout.clone();
    moved.pages[0].bindings[0].control = ControlId::Encoder { index: 3 };
    let (refused, kept) =
        moved.compatibility_for_save(Some(plus_layout), &catalog, registry, profile);
    assert_eq!((refused.len(), kept.len()), (1, 3));
    // A new layout: every incompatible binding is new.
    let (refused, kept) = plus_layout.compatibility_for_save(None, &catalog, registry, profile);
    assert_eq!((refused.len(), kept.len()), (4, 0));
    // Bindings that are not actions keep their rules.
    let mut pages = plus_layout.clone();
    let folder = LayoutPage::new("Folder");
    let folder_id = folder.id;
    pages.pages.push(folder);
    pages.pages[0].bindings = vec![
        ControlBinding::new(
            DIAL_0,
            Trigger::RotateClockwise,
            BindingTarget::OpenPage { page: folder_id },
        ),
        ControlBinding::new(DIAL_1, Trigger::RotateCounterClockwise, BindingTarget::Back),
    ];
    assert_eq!(
        pages.check_compatibility(&catalog, registry, profile),
        Vec::new()
    );
}
