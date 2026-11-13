use deckpilot_core::trigger::{LifecycleEvent, WorkflowTrigger};
use deckpilot_core::variables::{
    VariableDefinition, VariableKey, VariableRef, VariableScope, VariableType,
};
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, OnFailure, ParameterValue, Profile, Trigger, Workflow,
};
use deckpilot_device::{ControlId, models};

use super::{VariableUse, variable_references};
use crate::{ConfigStore, SqliteStore, StorageError};

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

fn action(name: &str, parameters: ActionParameters) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: ActionTypeId::new("general.log").unwrap(),
        parameters,
        description: String::new(),
        icon: None,
        label: None,
    }
}

/// `paths.root` used in every kind of place; `paths.other` nowhere, only
/// as literal text.
fn profile() -> Profile {
    let mut profile = Profile::new("Work");
    let reference = action(
        "Reference",
        ActionParameters::new().with(
            "message",
            ParameterValue::Reference(VariableRef::Shared(key("paths.root"))),
        ),
    );
    let template = action(
        "Template",
        ActionParameters::new().with(
            "message",
            ParameterValue::Template("cd {{ shared.paths.root }}".into()),
        ),
    );
    let mut labelled = action(
        "Literal",
        // Plain text is never parsed: no use.
        ActionParameters::new().with(
            "message",
            ParameterValue::Text("{{ shared.paths.other }}".into()),
        ),
    );
    labelled.label = Some("{{ shared.paths.root }}".into());
    let ids = (reference.id, template.id, labelled.id);
    profile.actions.extend([reference, template, labelled]);

    let mut steps = Workflow::new("Steps").step(ids.2, OnFailure::Stop);
    steps.steps[0].overrides = ActionParameters::new().with(
        "message",
        ParameterValue::Reference(VariableRef::Shared(key("paths.root"))),
    );
    profile.workflows.push(steps);
    let mut graph = Workflow::new("Graph");
    graph.graph = Some(
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "say", "type": "action", "action": ids.2,
                 "mappings": {"level": {"type": "variable", "reference": "shared.paths.root"},
                              "message": {"type": "template",
                                          "template": "at {{ shared.paths.root }}"}}},
            ],
            "edges": [{"from": "start", "to": "say"}],
        }))
        .unwrap(),
    );
    graph
        .triggers
        .push(WorkflowTrigger::lifecycle(LifecycleEvent::AgentStarted));
    profile.workflows.push(graph);

    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let mut binding = ControlBinding::new(
        ControlId::Key { index: 0 },
        Trigger::Press,
        BindingTarget::action(ids.0),
    );
    binding.label = Some("Root: {{ shared.paths.root }}".into());
    layout.pages[0].bindings.push(binding);
    // A gesture binding's parameter overrides.
    layout.pages[0].bindings.push(ControlBinding::new(
        ControlId::Key { index: 1 },
        Trigger::LongPress,
        BindingTarget::Action {
            action: ids.2,
            overrides: ActionParameters::new().with(
                "message",
                ParameterValue::Template("in {{ shared.paths.root }}".into()),
            ),
        },
    ));
    profile.layouts.push(layout);
    profile
}

#[test]
fn every_kind_of_use_is_found() {
    let profile = profile();
    let uses = variable_references(&profile, &key("paths.root"));
    let paths: Vec<(&str, &str)> = uses
        .iter()
        .map(|u| (u.owner_name.as_str(), u.path.as_str()))
        .collect();
    assert_eq!(
        paths,
        [
            ("Reference", "parameters.message"),
            ("Template", "parameters.message"),
            ("Literal", "label"),
            ("Steps", "steps[0].overrides.message"),
            ("Graph", "graph.nodes[1].mappings.level"),
            ("Graph", "graph.nodes[1].mappings.message"),
            ("Graph", "triggers[0]"),
            ("Main", "Home › label"),
            ("Main", "Home › target.overrides.message"),
        ]
    );
    assert!(matches!(
        &uses[5].site,
        VariableUse::Workflow { node: Some(node), .. } if node.as_str() == "say"
    ));
    assert!(matches!(
        uses[6].site,
        VariableUse::Trigger { index: 0, .. }
    ));
    // Literal text that looks like a reference is no use.
    assert_eq!(
        variable_references(&profile, &key("paths.other")),
        Vec::new()
    );
}

fn store() -> (SqliteStore, Profile) {
    let store = SqliteStore::open_in_memory().unwrap();
    let profile = profile();
    store.save_profile(&profile).unwrap();
    for name in ["paths.root", "paths.other"] {
        store
            .save_shared_variable(&VariableDefinition::new(
                key(name),
                VariableType::String,
                &[VariableScope::Global],
            ))
            .unwrap();
    }
    (store, profile)
}

#[test]
fn a_used_variable_is_never_deleted() {
    let (store, mut profile) = store();
    let refused = store.delete_shared_variable(&key("paths.root"));
    let Err(StorageError::VariableInUse(uses)) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(uses.len(), 9);
    assert_eq!(uses[0].profile_name, "Work");
    assert!(store.shared_variable(&key("paths.root")).unwrap().is_some());
    // The unused one goes.
    assert!(store.delete_shared_variable(&key("paths.other")).unwrap());

    // Once every use is removed, the variable can be deleted.
    profile.actions.iter_mut().for_each(|a| {
        a.parameters = ActionParameters::new();
        a.label = None;
    });
    profile.workflows.clear();
    profile.layouts.clear();
    store.save_profile(&profile).unwrap();
    assert!(store.delete_shared_variable(&key("paths.root")).unwrap());
}

#[test]
fn the_type_of_a_used_variable_cannot_change() {
    let (store, _) = store();
    let mut root = VariableDefinition::new(
        key("paths.root"),
        VariableType::Integer,
        &[VariableScope::Global],
    );
    assert!(matches!(
        store.save_shared_variable(&root),
        Err(StorageError::VariableInUse(_))
    ));
    // Other changes are fine.
    root.value_type = VariableType::String;
    root.label = "Root".into();
    store.save_shared_variable(&root).unwrap();
    // An unused variable may change its type.
    let other = VariableDefinition::new(
        key("paths.other"),
        VariableType::Integer,
        &[VariableScope::Global],
    );
    store.save_shared_variable(&other).unwrap();
}

#[test]
fn uses_in_every_profile_are_listed() {
    let (store, profile) = store();
    let copy = profile.duplicate("Home");
    store.save_profile(&copy).unwrap();
    let uses = store
        .shared_variable_references(&key("paths.root"))
        .unwrap();
    assert_eq!(uses.len(), 18);
    let mut profiles: Vec<_> = uses.iter().map(|u| u.profile).collect();
    profiles.dedup();
    assert_eq!(profiles.len(), 2);
}
