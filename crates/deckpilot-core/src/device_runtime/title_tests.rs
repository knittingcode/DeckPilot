//! Key titles of plugin actions (plugin API 1.8, `docs/key-text.md`, "Title
//! precedence"): the user's title beats the plugin's dynamic title, which
//! beats the plugin's suggested title, which beats the action's name; the
//! plugin's suggested position fills in only what the key does not set.

use deckpilot_device::models;

use super::*;
use crate::graph::NodeSpec;
use crate::{
    ActionHandler, ActionParameters, ActionTitle, ActionType, ActionTypeId, ButtonState,
    ExecutionContext, KeyAppearance, StateSnapshot, TitleAlign,
};

struct Plain(ActionType);

impl ActionHandler for Plain {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(&self, _: &ActionParameters, _: &ExecutionContext<'_>) -> ActionResult {
        ActionResult::success("ran")
    }
}

fn definition(id: &str, title: Option<ActionTitle>) -> ActionType {
    ActionType {
        editor_navigation: None,
        title,
        id: ActionTypeId::new(id).unwrap(),
        name: "CPU monitor".into(),
        description: String::new(),
        category: "Test".into(),
        parameters: Vec::new(),
        requires_project: false,
        deprecated: false,
        controls: None,
        node: NodeSpec::default(),
    }
}

struct Setup {
    registry: ActionRegistry,
    profile: Profile,
    layout: DeviceLayout,
}

/// One key (0) running an action of `id`, with `label` on the key and
/// `action_label` on the action.
fn setup(id: &str, label: Option<&str>, action_label: Option<&str>) -> Setup {
    let suggestions = ActionTitle {
        default: Some("CPU".into()),
        dynamic: true,
        position: Some(TitleAlign::Top),
        supports: None,
    };
    let mut registry = ActionRegistry::default();
    registry
        .register(Box::new(Plain(definition(id, Some(suggestions)))))
        .unwrap();
    let mut profile = Profile::new("Work");
    let mut action = registry
        .create_instance(
            "CPU monitor",
            ActionTypeId::new(id).unwrap(),
            &ActionParameters::new(),
        )
        .unwrap();
    action.label = action_label.map(Into::into);
    let action_id = action.id;
    profile.actions.push(action);
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(ControlBinding {
            control: ControlId::Key { index: 0 },
            trigger: Trigger::Press,
            target: BindingTarget::action(action_id),
            label: label.map(Into::into),
            appearance: None,
        });
    Setup {
        registry,
        profile,
        layout,
    }
}

fn face(setup: &Setup) -> KeyFace {
    let controls = [ControlId::Key { index: 0 }];
    page_faces(
        &setup.registry,
        &setup.layout,
        setup.layout.home_page,
        &setup.profile,
        None,
        &controls,
        None,
    )
    .remove(0)
    .1
}

fn live(face: KeyFace, label: &str) -> KeyFace {
    face.with_snapshot(&StateSnapshot::Ready {
        state: ButtonState {
            label: Some(label.into()),
            ..ButtonState::default()
        },
    })
}

#[test]
fn a_plugins_suggested_title_and_dynamic_title_give_way_to_the_users() {
    // Nothing set by the user: the plugin's suggested title, then its
    // dynamic one.
    let plain = setup("acme.system.cpu", None, None);
    assert_eq!(face(&plain).title, "CPU");
    assert_eq!(live(face(&plain), "42 %").title, "42 %");

    // The key's own title wins over both.
    let titled = setup("acme.system.cpu", Some("Load"), None);
    assert_eq!(face(&titled).title, "Load");
    assert_eq!(live(face(&titled), "42 %").title, "Load");
    // As does the action's own label.
    let labelled = setup("acme.system.cpu", None, Some("Processor"));
    assert_eq!(live(face(&labelled), "42 %").title, "Processor");

    // An action renamed by the user shows its name, not the suggestion.
    let mut renamed = setup("acme.system.cpu", None, None);
    renamed.profile.actions[0].name = "My CPU".into();
    assert_eq!(face(&renamed).title, "My CPU");
}

#[test]
fn built_in_module_states_still_replace_the_title() {
    // Built-in types keep what they always did: their state's label
    // replaces the title, labelled or not.
    let titled = setup("system.cpu", Some("Load"), None);
    assert_eq!(live(face(&titled), "42 %").title, "42 %");
}

#[test]
fn a_plugins_suggested_position_fills_in_only_what_the_key_does_not_set() {
    let mut s = setup("acme.system.cpu", None, None);
    assert_eq!(
        face(&s).appearance.unwrap().title_align,
        Some(TitleAlign::Top)
    );
    s.layout.pages[0].bindings[0].appearance = Some(KeyAppearance {
        title_align: Some(TitleAlign::Bottom),
        bold: Some(true),
        ..KeyAppearance::default()
    });
    let look = face(&s).appearance.unwrap();
    assert_eq!(look.title_align, Some(TitleAlign::Bottom));
    assert_eq!(look.bold, Some(true));
    // The suggestion is never written into the layout.
    assert_eq!(
        s.layout.pages[0].bindings[0]
            .appearance
            .as_ref()
            .unwrap()
            .title_align,
        Some(TitleAlign::Bottom)
    );
}

#[test]
fn a_shared_layout_shows_the_same_title_look_on_two_models() {
    let mut s = setup("acme.system.cpu", Some("Load"), None);
    let look = KeyAppearance {
        bold: Some(true),
        italic: Some(true),
        underline: Some(false),
        font_family: Some(crate::FontFamilyId::new("inter")),
        title_align: Some(TitleAlign::Middle),
        ..KeyAppearance::default()
    };
    s.layout.pages[0].bindings[0].appearance = Some(look.clone());
    let controls = [ControlId::Key { index: 0 }];
    let on = |model: deckpilot_device::DeviceModel| {
        page_faces(
            &s.registry,
            &s.layout,
            s.layout.home_page,
            &s.profile,
            None,
            &controls,
            Some(&model.capabilities),
        )
        .remove(0)
        .1
    };
    let mk2 = on(models::stream_deck_mk2());
    let xl = on(models::stream_deck_xl());
    assert_eq!(mk2.appearance, Some(look));
    assert_eq!(mk2, xl);
    assert_eq!(mk2.title, "Load");
}
