use deckpilot_device::{ControlId, ModelCatalog, models};

use super::*;
use crate::testing::{echo_type, with_projects};
use crate::{
    ActionParameters, ControlBinding, LayoutPage, ParameterValue, Project, ProjectKey, Workflow,
};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn action(name: &str, parameters: ActionParameters) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: echo_type(),
        parameters,
        description: String::new(),
        icon: None,
        label: None,
    }
}

fn layout_binding(layout: &mut DeviceLayout, control: ControlId, target: BindingTarget) {
    let home = layout.home_page;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(ControlBinding::new(control, Trigger::Press, target));
}

#[test]
fn references_are_derived_from_the_bindings() {
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let a = ActionInstanceId::new();
    let w = WorkflowId::new();
    layout_binding(&mut layout, key(0), BindingTarget::action(a));
    layout_binding(&mut layout, key(1), BindingTarget::action(a));
    layout_binding(&mut layout, key(2), BindingTarget::Workflow { workflow: w });
    let folder = LayoutPage::new("Folder");
    let folder_id = folder.id;
    layout.pages.push(folder);
    layout_binding(
        &mut layout,
        key(3),
        BindingTarget::OpenPage { page: folder_id },
    );
    assert_eq!(layout.bound_actions(), vec![a]);
    assert_eq!(layout.bound_workflows(), vec![w]);
    let requirements = layout.requirements();
    assert_eq!(requirements.controls.len(), 4);
    assert!(
        requirements
            .controls
            .iter()
            .all(|c| c.triggers == vec![Trigger::Press])
    );
}

#[test]
fn a_layout_is_only_usable_on_its_own_model() {
    let catalog = ModelCatalog::builtin();
    let mk2 = models::stream_deck_mk2();
    let xl = models::stream_deck_xl();
    let plus = models::stream_deck_plus();
    let mut layout = DeviceLayout::new("Main", mk2.id.clone());
    layout_binding(&mut layout, key(14), BindingTarget::Back);

    assert_eq!(compatibility(&layout, &mk2).fit, ModelFit::Native);
    // The XL has key 14, the Stream Deck + does not: neither may use it.
    let on_xl = compatibility(&layout, &xl);
    assert_eq!(on_xl.fit, ModelFit::Fits);
    assert!(!on_xl.fit.is_usable());
    let on_plus = compatibility(&layout, &plus);
    assert_eq!(
        on_plus.fit,
        ModelFit::Missing {
            controls: vec![key(14)],
            triggers: 0
        }
    );
    assert!(on_plus.explanation.contains("lacks 1 control"));

    let report = compatibility_report(&layout, &catalog);
    assert_eq!(report[0].fit, ModelFit::Native);
    assert_eq!(report.len(), catalog.iter().count());

    let unknown = DeviceLayout::new(
        "Old",
        deckpilot_device::DeviceModelId::new("vendor.unknown"),
    );
    let report = compatibility_report(&unknown, &catalog);
    assert_eq!(report[0].fit, ModelFit::UnknownModel);
}

#[test]
fn the_closure_follows_actions_named_in_parameters() {
    let a = action("A", ActionParameters::new());
    let b = action(
        "B",
        ActionParameters::new().with(
            "states",
            ParameterValue::Text(format!("action:{}\nworkflow:{}", a.id, WorkflowId::new())),
        ),
    );
    let c = action(
        "C",
        ActionParameters::new().with("states", ParameterValue::Text(format!("action:{}", b.id))),
    );
    let unrelated = action("D", ActionParameters::new());
    let library = vec![a.clone(), b.clone(), c.clone(), unrelated];
    assert_eq!(action_closure([c.id], &library), vec![c.id, b.id, a.id]);
    // Unknown ids are left out; nothing loops.
    assert_eq!(action_closure([ActionInstanceId::new()], &library), vec![]);
    let mut looping = a.clone();
    looping.parameters =
        ActionParameters::new().with("states", ParameterValue::Text(format!("action:{}", c.id)));
    let library = vec![looping.clone(), b.clone(), c.clone()];
    assert_eq!(action_closure([c.id], &library).len(), 3);
}

#[test]
fn workflows_and_projects_of_another_profile_are_unavailable_not_used() {
    let mut work = with_projects("Work", &["SHOP"]);
    let shop = work.projects[0].id;
    let deploy = Workflow::new("Deploy");
    let deploy_id = deploy.id;
    work.workflows.push(deploy);
    let select = action(
        "Select SHOP",
        ActionParameters::new().with("project", ParameterValue::Project(shop)),
    );
    work.actions.push(select.clone());
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout_binding(
        &mut layout,
        key(0),
        BindingTarget::Workflow {
            workflow: deploy_id,
        },
    );
    layout_binding(&mut layout, key(1), BindingTarget::action(select.id));
    assert_eq!(availability(&layout, &work).len(), 0);

    let mut home = with_projects("Home", &["HOUSE"]);
    home.actions.push(select);
    let found = availability(&layout, &home);
    assert_eq!(found.len(), 2);
    assert!(matches!(found[0], Unavailable::Workflow { workflow, .. } if workflow == deploy_id));
    assert!(matches!(found[1], Unavailable::Project { control, .. } if control == key(1)));
    assert!(found[0].to_string().contains("does not have"));
    let _ = Project::new(ProjectKey::new("X").unwrap(), "X");
}

fn ids(n: usize) -> Vec<LayoutId> {
    (0..n).map(|_| LayoutId::new()).collect()
}

#[test]
fn history_goes_forward_and_back_in_order() {
    let [main, teams, meeting] = ids(3)[..] else {
        unreachable!()
    };
    let page = PageId::new();
    let mut history = LayoutHistory::default();
    assert!(history.switch(Some((main, vec![page])), teams, HistoryMode::Push));
    assert!(history.switch(Some((teams, vec![])), meeting, HistoryMode::Push));
    assert_eq!(history.layouts(), vec![main, teams]);
    assert_eq!(history.back(|_| true).unwrap().layout, teams);
    let entry = history.back(|_| true).unwrap();
    assert_eq!((entry.layout, entry.pages), (main, vec![page]));
    // Empty: Back has nowhere to go.
    assert!(history.back(|_| true).is_none());
    assert_eq!(history.len(), 0);
}

#[test]
fn switching_to_the_shown_layout_or_back_into_history_never_duplicates() {
    let [main, teams, meeting] = ids(3)[..] else {
        unreachable!()
    };
    let mut history = LayoutHistory::default();
    assert!(!history.switch(Some((main, vec![])), main, HistoryMode::Push));
    assert_eq!(history.len(), 0);
    history.switch(Some((main, vec![])), teams, HistoryMode::Push);
    history.switch(Some((teams, vec![])), meeting, HistoryMode::Push);
    // Meeting → Main unwinds: Main is not in the history twice.
    assert!(history.switch(Some((meeting, vec![])), main, HistoryMode::Push));
    assert_eq!(history.len(), 0);
    history.switch(Some((main, vec![])), teams, HistoryMode::Push);
    history.switch(Some((teams, vec![])), meeting, HistoryMode::Push);
    // Meeting → Teams unwinds to Teams; Back then goes to Main.
    history.switch(Some((meeting, vec![])), teams, HistoryMode::Push);
    assert_eq!(history.layouts(), vec![main]);
}

#[test]
fn history_modes_and_bounds() {
    let layouts = ids(MAX_LAYOUT_HISTORY + 5);
    let mut history = LayoutHistory::default();
    for pair in layouts.windows(2) {
        history.switch(Some((pair[0], vec![])), pair[1], HistoryMode::Push);
    }
    assert_eq!(history.len(), MAX_LAYOUT_HISTORY);
    assert_eq!(history.layouts()[0], layouts[4]);

    let [a, b, c] = ids(3)[..] else {
        unreachable!()
    };
    let mut history = LayoutHistory::default();
    history.switch(Some((a, vec![])), b, HistoryMode::Replace);
    assert_eq!(history.len(), 0);
    history.switch(Some((b, vec![])), c, HistoryMode::Push);
    history.switch(Some((c, vec![])), a, HistoryMode::Clear);
    assert_eq!(history.len(), 0);
    assert_eq!(HistoryMode::parse("replace"), Some(HistoryMode::Replace));
    assert_eq!(HistoryMode::parse("other"), None);
}

#[test]
fn back_skips_layouts_that_are_gone() {
    let [a, b, c] = ids(3)[..] else {
        unreachable!()
    };
    let mut history = LayoutHistory::default();
    history.switch(Some((a, vec![])), b, HistoryMode::Push);
    history.switch(Some((b, vec![])), c, HistoryMode::Push);
    // B was unassigned: Back goes to A.
    assert_eq!(history.back(|layout| layout != b).unwrap().layout, a);
    history.switch(Some((a, vec![])), b, HistoryMode::Push);
    history.retain(|layout| layout != a);
    assert_eq!(history.len(), 0);
}
