use deckpilot_device::{
    ControlId, DeviceCapabilities, EncoderSpec, TouchGesture, TouchSurface, models,
};

use super::*;
use crate::graph::NodeSpec;
use crate::{ActionTypeId, Trigger};
use InputCapability as I;

fn action(controls: Option<ActionControls>) -> ActionType {
    ActionType {
        editor_navigation: None,
        title: None,
        id: ActionTypeId::new("test.action").unwrap(),
        name: "Action".into(),
        description: String::new(),
        category: "Test".into(),
        parameters: Vec::new(),
        requires_project: false,
        node: NodeSpec::default(),
        deprecated: false,
        controls,
    }
}

fn key_only() -> ActionType {
    action(Some(ActionControls::keys()))
}

fn encoder_only() -> ActionType {
    action(Some(ActionControls::new().with(
        ControlKind::Encoder,
        [I::RotateClockwise, I::RotateCounterClockwise],
    )))
}

fn caps(capabilities: &DeviceCapabilities, control: ControlId) -> ControlCapabilities {
    capabilities.control_capabilities(control).unwrap()
}

fn mk2_key() -> ControlCapabilities {
    caps(
        &models::stream_deck_mk2().capabilities,
        ControlId::Key { index: 0 },
    )
}

fn plus(control: ControlId) -> ControlCapabilities {
    caps(&models::stream_deck_plus().capabilities, control)
}

const PLUS_KEY: ControlId = ControlId::Key { index: 0 };
const PLUS_DIAL: ControlId = ControlId::Encoder { index: 0 };
const PLUS_ZONE: ControlId = ControlId::TouchZone { index: 0 };

#[test]
fn an_mk2_key_accepts_press_actions() {
    for trigger in [
        Trigger::Press,
        Trigger::Release,
        Trigger::Click,
        Trigger::DoubleClick,
        Trigger::LongPress,
    ] {
        assert_eq!(
            compatibility(&key_only(), &mk2_key(), trigger),
            Compatibility::Compatible,
            "{trigger:?}"
        );
    }
}

#[test]
fn an_mk2_key_refuses_encoder_only_actions() {
    let result = compatibility(&encoder_only(), &mk2_key(), Trigger::Press);
    assert_eq!(
        result,
        Compatibility::Incompatible(IncompatibilityReason::ControlKindUnsupported {
            kind: ControlKind::Key,
            supported: vec![ControlKind::Encoder],
        })
    );
    assert_eq!(
        result.to_string(),
        "the action cannot be bound to a key; it works on encoders only"
    );
}

#[test]
fn a_plus_encoder_accepts_rotation_and_refuses_key_only_actions() {
    let dial = plus(PLUS_DIAL);
    for trigger in [Trigger::RotateClockwise, Trigger::RotateCounterClockwise] {
        assert!(compatibility(&encoder_only(), &dial, trigger).is_compatible());
        assert_eq!(
            compatibility(&key_only(), &dial, trigger),
            Compatibility::Incompatible(IncompatibilityReason::ControlKindUnsupported {
                kind: ControlKind::Encoder,
                supported: vec![ControlKind::Key],
            })
        );
    }
    // A press of the dial is not one of the encoder-only action's inputs.
    assert_eq!(
        compatibility(&encoder_only(), &dial, Trigger::Press),
        Compatibility::Incompatible(IncompatibilityReason::InputUnsupportedByAction {
            kind: ControlKind::Encoder,
            input: I::Press,
            supported: vec![I::RotateClockwise, I::RotateCounterClockwise],
        })
    );
}

#[test]
fn a_plus_key_refuses_encoder_only_actions() {
    let key = plus(PLUS_KEY);
    assert!(compatibility(&key_only(), &key, Trigger::Press).is_compatible());
    assert!(matches!(
        compatibility(&encoder_only(), &key, Trigger::Press),
        Compatibility::Incompatible(IncompatibilityReason::ControlKindUnsupported { .. })
    ));
}

#[test]
fn a_multi_control_action_uses_the_inputs_of_each_kind() {
    let both = action(Some(
        ActionControls::new()
            .with(ControlKind::Key, [I::Press])
            .with(ControlKind::Encoder, [I::RotateClockwise])
            .with(ControlKind::TouchZone, [I::Tap]),
    ));
    let key = plus(PLUS_KEY);
    let dial = plus(PLUS_DIAL);
    let zone = plus(PLUS_ZONE);
    assert!(compatibility(&both, &key, Trigger::Press).is_compatible());
    assert!(compatibility(&both, &dial, Trigger::RotateClockwise).is_compatible());
    let tap = Trigger::Touch {
        gesture: TouchGesture::Tap,
    };
    assert!(compatibility(&both, &zone, tap).is_compatible());
    // Each kind has its own inputs: a click is not declared for keys, a
    // press not for encoders, a swipe not for touch zones.
    assert!(matches!(
        compatibility(&both, &key, Trigger::Click),
        Compatibility::Incompatible(IncompatibilityReason::InputUnsupportedByAction {
            kind: ControlKind::Key,
            input: I::Click,
            ..
        })
    ));
    assert!(!compatibility(&both, &dial, Trigger::Press).is_compatible());
    assert!(!compatibility(&both, &dial, Trigger::RotateCounterClockwise).is_compatible());
    let swipe = Trigger::Touch {
        gesture: TouchGesture::Swipe,
    };
    assert!(!compatibility(&both, &zone, swipe).is_compatible());
}

#[test]
fn gestures_the_control_lacks_are_refused() {
    // A long touch on a surface that reports taps only.
    let tap_only = DeviceCapabilities {
        touch: Some(TouchSurface {
            zones: 1,
            image: None,
            gestures: vec![TouchGesture::Tap],
        }),
        ..DeviceCapabilities::default()
    };
    let touch = action(Some(
        ActionControls::new().with(ControlKind::TouchZone, [I::Tap, I::LongTouch]),
    ));
    let zone = caps(&tap_only, ControlId::TouchZone { index: 0 });
    let long_touch = Trigger::Touch {
        gesture: TouchGesture::LongPress,
    };
    assert_eq!(
        compatibility(&touch, &zone, long_touch),
        Compatibility::Incompatible(IncompatibilityReason::InputUnsupportedByControl {
            kind: ControlKind::TouchZone,
            input: I::LongTouch,
        })
    );
    // A double click on a dial that cannot be pressed.
    let fixed = DeviceCapabilities {
        encoders: Some(EncoderSpec {
            count: 1,
            pressable: false,
        }),
        ..DeviceCapabilities::default()
    };
    let dial = caps(&fixed, ControlId::Encoder { index: 0 });
    let steps = action(Some(
        ActionControls::keys_and_steps().with(ControlKind::Encoder, [I::DoubleClick]),
    ));
    let result = compatibility(&steps, &dial, Trigger::DoubleClick);
    assert_eq!(
        result,
        Compatibility::Incompatible(IncompatibilityReason::InputUnsupportedByControl {
            kind: ControlKind::Encoder,
            input: I::DoubleClick,
        })
    );
    assert_eq!(result.to_string(), "this encoder has no double-click");
    assert!(compatibility(&steps, &dial, Trigger::RotateClockwise).is_compatible());
}

#[test]
fn a_missing_capability_is_refused() {
    // The MK.2 has no dial; the catalog says so before compatibility is
    // asked.
    let mk2 = models::stream_deck_mk2().capabilities;
    assert!(
        mk2.control_capabilities(ControlId::Encoder { index: 0 })
            .is_none()
    );
    // A Stream Deck + touch zone with an action that supports keys and
    // encoders only.
    assert!(matches!(
        compatibility(
            &action(Some(ActionControls::keys_and_steps())),
            &plus(PLUS_ZONE),
            Trigger::Touch {
                gesture: TouchGesture::Tap
            }
        ),
        Compatibility::Incompatible(IncompatibilityReason::ControlKindUnsupported {
            kind: ControlKind::TouchZone,
            ..
        })
    ));
}

#[test]
fn a_type_without_a_declaration_gets_keys_only() {
    let legacy = action(None);
    assert_eq!(*legacy.effective_controls(), ActionControls::legacy());
    assert_eq!(
        ActionControls::legacy().kinds(),
        vec![ControlKind::Key],
        "never encoders or touch zones"
    );
    for trigger in [
        Trigger::Press,
        Trigger::Release,
        Trigger::Click,
        Trigger::DoubleClick,
        Trigger::LongPress,
    ] {
        assert!(legacy.compatibility(&mk2_key(), trigger).is_compatible());
        assert!(
            legacy
                .compatibility(&plus(PLUS_KEY), trigger)
                .is_compatible()
        );
    }
    assert!(
        !legacy
            .compatibility(&plus(PLUS_DIAL), Trigger::RotateClockwise)
            .is_compatible()
    );
    assert!(
        !legacy
            .compatibility(&plus(PLUS_DIAL), Trigger::Press)
            .is_compatible()
    );
    assert!(
        !legacy
            .compatibility(
                &plus(PLUS_ZONE),
                Trigger::Touch {
                    gesture: TouchGesture::Tap
                }
            )
            .is_compatible()
    );
}

#[test]
fn every_trigger_maps_to_one_input() {
    let pairs = [
        (Trigger::Press, I::Press),
        (Trigger::Release, I::Release),
        (Trigger::RotateClockwise, I::RotateClockwise),
        (Trigger::RotateCounterClockwise, I::RotateCounterClockwise),
        (
            Trigger::Touch {
                gesture: TouchGesture::Tap,
            },
            I::Tap,
        ),
        (
            Trigger::Touch {
                gesture: TouchGesture::LongPress,
            },
            I::LongTouch,
        ),
        (
            Trigger::Touch {
                gesture: TouchGesture::Swipe,
            },
            I::Swipe,
        ),
        (Trigger::Click, I::Click),
        (Trigger::DoubleClick, I::DoubleClick),
        (Trigger::LongPress, I::LongPress),
    ];
    for (trigger, input) in pairs {
        assert_eq!(trigger.input_capability(), input);
        // Gesture triggers map to the derived inputs, raw ones to native.
        assert_eq!(trigger.gesture().is_some(), input.is_derived());
    }
    let inputs: BTreeSet<_> = pairs.iter().map(|(_, input)| *input).collect();
    assert_eq!(inputs.len(), I::ALL.len());
}

#[test]
fn declarations_round_trip_and_older_payloads_decode() {
    let declared = action(Some(ActionControls::keys_and_steps()));
    let json = serde_json::to_value(&declared).unwrap();
    assert_eq!(
        json["controls"],
        serde_json::json!({
            "key": ["press", "release", "click", "double-click", "long-press"],
            "encoder": ["press", "rotate-clockwise", "rotate-counter-clockwise", "click"],
        })
    );
    let back: ActionType = serde_json::from_value(json).unwrap();
    assert_eq!(back, declared);

    // A type without a declaration does not send the field...
    let legacy = serde_json::to_value(action(None)).unwrap();
    assert!(legacy.get("controls").is_none());
    // ...and a payload from before declarations decodes without it.
    let old: ActionType = serde_json::from_value(serde_json::json!({
        "id": "acme.greet",
        "name": "Greet",
        "description": "",
        "category": "Fun",
        "parameters": [],
        "requiresProject": false,
    }))
    .unwrap();
    assert_eq!(old.controls, None);
    assert_eq!(*old.effective_controls(), ActionControls::legacy());

    // Touch zones are `touchZone`, as control ids name them.
    let touch: ActionControls =
        serde_json::from_value(serde_json::json!({"touchZone": ["tap", "long-touch"]})).unwrap();
    assert_eq!(
        touch,
        ActionControls::new().with(ControlKind::TouchZone, [I::Tap, I::LongTouch])
    );
    // Unknown inputs are refused, not dropped.
    assert!(
        serde_json::from_value::<ActionControls>(serde_json::json!({"key": ["poke"]})).is_err()
    );
}

#[test]
fn reasons_serialize_for_the_app() {
    let reason = IncompatibilityReason::InputUnsupportedByAction {
        kind: ControlKind::Encoder,
        input: I::Press,
        supported: vec![I::RotateClockwise],
    };
    assert_eq!(
        serde_json::to_value(&reason).unwrap(),
        serde_json::json!({
            "type": "inputUnsupportedByAction",
            "kind": "encoder",
            "input": "press",
            "supported": ["rotate-clockwise"],
        })
    );
    let compatibility = Compatibility::Incompatible(reason.clone());
    let json = serde_json::to_value(&compatibility).unwrap();
    assert_eq!(json["status"], "incompatible");
    assert_eq!(
        serde_json::from_value::<Compatibility>(json).unwrap(),
        compatibility
    );
}

#[test]
fn declaration_issues() {
    assert_eq!(ActionControls::keys().issues(), Vec::<String>::new());
    assert_eq!(
        ActionControls::keys_and_steps().issues(),
        Vec::<String>::new()
    );
    assert_eq!(ActionControls::new().issues().len(), 1);
    assert_eq!(
        ActionControls::new()
            .with(ControlKind::Key, [I::RotateClockwise])
            .issues(),
        vec!["key: a key has no rotate-clockwise".to_owned()]
    );
    assert_eq!(
        ActionControls::new()
            .with(ControlKind::Encoder, [])
            .issues()
            .len(),
        1
    );
}
