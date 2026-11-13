use deckpilot_core::trigger::{LifecycleEvent, WorkflowTrigger};
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, OnFailure, ParameterValue, Profile, Trigger, Workflow,
};
use deckpilot_device::{ControlId, models};

use super::{ReferenceSite, Selection};
use crate::{ConfigStore, SqliteStore, StorageError};

fn action(name: &str, action_type: &str) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: ActionTypeId::new(action_type).unwrap(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    }
}

fn message() -> ActionParameters {
    ActionParameters::new().with("message", ParameterValue::Text("hi".into()))
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

struct Demo {
    profile: Profile,
    hello: ActionInstanceId,
    bye: ActionInstanceId,
    other: ActionInstanceId,
    unused: ActionInstanceId,
}

/// `hello` is on two keys (a press, and a long press with overrides), in
/// two steps (one with overrides) and in a graph node (with overrides) of
/// a workflow with a trigger.
fn demo() -> Demo {
    let mut profile = Profile::new("Demo");
    let hello = action("Hello", "general.log");
    let bye = action("Bye", "general.log");
    let other = action("Docs", "projects.documentation");
    let unused = action("Unused", "general.log");
    let ids = (hello.id, bye.id, other.id, unused.id);
    profile.actions.extend([hello, bye, other, unused]);
    let (hello, bye, other, unused) = ids;

    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let page = &mut layout.pages[0];
    for (index, action) in [(0, hello), (1, bye)] {
        page.bindings.push(ControlBinding::new(
            key(index),
            Trigger::Press,
            BindingTarget::action(action),
        ));
    }
    page.bindings.push(ControlBinding::new(
        key(2),
        Trigger::LongPress,
        BindingTarget::Action {
            action: hello,
            overrides: message(),
        },
    ));
    profile.layouts.push(layout);

    let mut steps = Workflow::new("Steps")
        .step(hello, OnFailure::Continue)
        .step(bye, OnFailure::Continue)
        .step(hello, OnFailure::Stop);
    steps.steps[2].overrides = message();
    profile.workflows.push(steps);

    let mut graph = Workflow::new("Graph");
    graph.graph = Some(
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "say", "type": "action", "action": hello, "label": "Say",
                 "overrides": {"message": {"type": "text", "value": "x"}}},
            ],
            "edges": [{"from": "start", "to": "say"}],
        }))
        .unwrap(),
    );
    graph
        .triggers
        .push(WorkflowTrigger::lifecycle(LifecycleEvent::AgentStarted));
    profile.workflows.push(graph);
    Demo {
        profile,
        hello,
        bye,
        other,
        unused,
    }
}

fn saved() -> (SqliteStore, Demo) {
    let store = SqliteStore::open_in_memory().unwrap();
    let demo = demo();
    store.save_profile(&demo.profile).unwrap();
    (store, demo)
}

#[test]
fn every_use_is_listed_with_triggers_of_the_workflows() {
    let (store, demo) = saved();
    let references = store
        .action_references(demo.profile.id, demo.hello)
        .unwrap();
    let kinds: Vec<&str> = references
        .iter()
        .map(|r| match r.site {
            ReferenceSite::Binding { .. } => "binding",
            ReferenceSite::Step { .. } => "step",
            ReferenceSite::Node { .. } => "node",
            ReferenceSite::Trigger { .. } => "trigger",
        })
        .collect();
    assert_eq!(
        kinds,
        ["binding", "binding", "step", "step", "node", "trigger"]
    );
    assert_eq!(references[0].owner_name, "Main");
    assert_eq!(references[0].detail.as_deref(), Some("Home"));
    assert_eq!(references[4].detail.as_deref(), Some("Say"));
    assert!(references[5].trigger.is_some());
    assert_eq!(
        store
            .action_references(demo.profile.id, demo.unused)
            .unwrap(),
        Vec::new()
    );
}

#[test]
fn a_used_action_is_never_deleted() {
    let (store, demo) = saved();
    let refused = store.delete_action(demo.profile.id, demo.hello);
    let Err(StorageError::ActionInUse(references)) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(references.len(), 6);
    // Nothing changed: the action and every binding are still there.
    assert_eq!(
        store.load_profile(demo.profile.id).unwrap().unwrap(),
        demo.profile
    );
    // An unused action is deleted.
    let deleted = store.delete_action(demo.profile.id, demo.unused).unwrap();
    assert_eq!(deleted.id, demo.unused);
    let profile = store.load_profile(demo.profile.id).unwrap().unwrap();
    assert!(profile.action(demo.unused).is_none());
    assert!(matches!(
        store.delete_action(demo.profile.id, demo.unused),
        Err(StorageError::Invalid(_))
    ));
}

#[test]
fn replacing_moves_every_use_and_then_the_action_can_go() {
    let (store, demo) = saved();
    let change = store
        .replace_action_references(demo.profile.id, demo.hello, demo.bye, &Selection::All)
        .unwrap();
    assert_eq!(change.changed.len(), 5);
    // Same action type: overrides are kept.
    assert_eq!(change.cleared_settings, Vec::new());
    let profile = store.load_profile(demo.profile.id).unwrap().unwrap();
    assert_eq!(super::action_references(&profile, demo.hello), Vec::new());
    assert_eq!(profile.workflows[0].steps[2].overrides, message());
    let bindings = &profile.layouts[0].pages[0].bindings;
    assert_eq!(bindings.len(), 3);
    assert!(
        bindings[..2]
            .iter()
            .all(|b| b.target == BindingTarget::action(demo.bye))
    );
    // The long press keeps its overrides.
    assert_eq!(
        bindings[2].target,
        BindingTarget::Action {
            action: demo.bye,
            overrides: message(),
        }
    );
    store.delete_action(demo.profile.id, demo.hello).unwrap();
    // The profile is still consistent.
    assert_eq!(
        store
            .load_profile(demo.profile.id)
            .unwrap()
            .unwrap()
            .validate(&deckpilot_device::ModelCatalog::builtin()),
        Vec::new()
    );
}

#[test]
fn replacing_with_another_type_clears_the_old_types_settings() {
    let (store, demo) = saved();
    let change = store
        .replace_action_references(demo.profile.id, demo.hello, demo.other, &Selection::All)
        .unwrap();
    // The long press binding, the step and the node had settings of the
    // old type.
    assert_eq!(change.cleared_settings.len(), 3);
    assert!(matches!(
        &change.cleared_settings[0],
        ReferenceSite::Binding { control, trigger: Trigger::LongPress, .. } if *control == key(2)
    ));
    let profile = store.load_profile(demo.profile.id).unwrap().unwrap();
    assert_eq!(
        profile.layouts[0].pages[0].bindings[2].target,
        BindingTarget::action(demo.other)
    );
    assert!(profile.workflows[0].steps[2].overrides.is_empty());
    let node = &profile.workflows[1].graph.as_ref().unwrap().nodes[1];
    assert!(matches!(
        &node.kind,
        deckpilot_core::graph::NodeKind::Action { action, overrides, .. }
            if *action == demo.other && overrides.is_empty()
    ));
}

#[test]
fn only_the_selected_uses_change() {
    let (store, demo) = saved();
    let sites: Vec<_> = store
        .action_references(demo.profile.id, demo.hello)
        .unwrap()
        .into_iter()
        .map(|r| r.site)
        .collect();
    let change = store
        .replace_action_references(
            demo.profile.id,
            demo.hello,
            demo.bye,
            &Selection::Sites(vec![sites[1].clone()]),
        )
        .unwrap();
    assert_eq!(change.changed, [sites[1].clone()]);
    let left = store
        .action_references(demo.profile.id, demo.hello)
        .unwrap();
    assert_eq!(left.len(), 5);
    assert!(!left.iter().any(|r| r.site == sites[1]));

    // A site that no longer uses the action, or a trigger, is refused and
    // nothing changes.
    let before = store.load_profile(demo.profile.id).unwrap();
    for site in [sites[1].clone(), sites[5].clone()] {
        assert!(matches!(
            store.replace_action_references(
                demo.profile.id,
                demo.hello,
                demo.bye,
                &Selection::Sites(vec![sites[0].clone(), site]),
            ),
            Err(StorageError::Invalid(_))
        ));
    }
    assert_eq!(store.load_profile(demo.profile.id).unwrap(), before);
    // Replacing with itself or a missing action is refused.
    for to in [demo.hello, ActionInstanceId::new()] {
        assert!(matches!(
            store.replace_action_references(demo.profile.id, demo.hello, to, &Selection::All),
            Err(StorageError::Invalid(_))
        ));
    }
}

#[test]
fn unbinding_clears_keys_and_steps_but_not_graph_nodes() {
    let (store, demo) = saved();
    let before = store.load_profile(demo.profile.id).unwrap();
    // `All` includes the graph node: refused as a whole.
    assert!(matches!(
        store.unbind_action(demo.profile.id, demo.hello, &Selection::All),
        Err(StorageError::Invalid(_))
    ));
    assert_eq!(store.load_profile(demo.profile.id).unwrap(), before);

    let sites: Vec<_> = store
        .action_references(demo.profile.id, demo.hello)
        .unwrap()
        .into_iter()
        .map(|r| r.site)
        .filter(|site| {
            !matches!(
                site,
                ReferenceSite::Node { .. } | ReferenceSite::Trigger { .. }
            )
        })
        .collect();
    assert_eq!(sites.len(), 4);
    let change = store
        .unbind_action(demo.profile.id, demo.hello, &Selection::Sites(sites))
        .unwrap();
    assert_eq!(change.changed.len(), 4);
    let profile = store.load_profile(demo.profile.id).unwrap().unwrap();
    // The key of `bye` is untouched; the keys of `hello` are empty.
    let bindings = &profile.layouts[0].pages[0].bindings;
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].control, key(1));
    // Both steps of `hello` are gone, `bye`'s step stays.
    let steps: Vec<_> = profile.workflows[0]
        .steps
        .iter()
        .map(|s| s.action)
        .collect();
    assert_eq!(steps, [demo.bye]);
    // The node remains, so deleting is still refused, naming it.
    let Err(StorageError::ActionInUse(left)) = store.delete_action(demo.profile.id, demo.hello)
    else {
        panic!("still used by the node");
    };
    assert!(matches!(left[0].site, ReferenceSite::Node { .. }));
}

#[test]
fn unknown_profiles_are_refused() {
    let store = SqliteStore::open_in_memory().unwrap();
    assert!(matches!(
        store.action_references(deckpilot_core::ProfileId::new(), ActionInstanceId::new()),
        Err(StorageError::UnknownProfile)
    ));
    assert!(matches!(
        store.delete_action(deckpilot_core::ProfileId::new(), ActionInstanceId::new()),
        Err(StorageError::UnknownProfile)
    ));
}

/// A work session key uses no action: it is never listed, and replacing,
/// unbinding or deleting actions leaves it as it is.
#[test]
fn stateful_keys_are_not_action_uses_and_survive_changes() {
    use deckpilot_core::stateful::StatefulBinding;

    let store = SqliteStore::open_in_memory().unwrap();
    let mut demo = demo();
    let session = BindingTarget::Stateful(StatefulBinding::work_session());
    let page = &mut demo.profile.layouts[0].pages[0];
    page.bindings.extend([
        ControlBinding::new(key(9), Trigger::Click, session.clone()),
        ControlBinding::new(key(9), Trigger::LongPress, session.clone()),
    ]);
    store.save_profile(&demo.profile).unwrap();
    let id = demo.profile.id;
    let stateful = |store: &SqliteStore| -> Vec<BindingTarget> {
        store.load_profile(id).unwrap().unwrap().layouts[0].pages[0]
            .bindings
            .iter()
            .filter(|b| b.control == key(9))
            .map(|b| b.target.clone())
            .collect()
    };

    for action in [demo.hello, demo.bye, demo.other, demo.unused] {
        assert!(store.action_references(id, action).unwrap().iter().all(
            |r| !matches!(&r.site, ReferenceSite::Binding { control, .. } if *control == key(9))
        ));
    }
    store.delete_action(id, demo.unused).unwrap();
    let sites: Vec<_> = store
        .action_references(id, demo.bye)
        .unwrap()
        .into_iter()
        .map(|r| r.site)
        .collect();
    store
        .unbind_action(id, demo.bye, &Selection::Sites(sites))
        .unwrap();
    assert_eq!(stateful(&store), [session.clone(), session]);
}
