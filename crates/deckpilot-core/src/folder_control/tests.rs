//! Folder controls: the rule (by identifiers only), validation of saved
//! and edited layouts, normalization (migration 12 and imports) with the
//! recovered list, and moving or discarding recovered bindings.

use deckpilot_device::{ModelCatalog, models};

use super::*;
use crate::{
    ActionInstance, ActionInstanceId, ActionParameters, KeyAppearance, Profile, WorkflowId,
};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn on(index: u16, trigger: Trigger, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), trigger, target)
}

fn press(index: u16, target: BindingTarget) -> ControlBinding {
    on(index, Trigger::Press, target)
}

fn open(page: PageId) -> BindingTarget {
    BindingTarget::OpenPage { page }
}

fn capabilities() -> DeviceCapabilities {
    models::stream_deck_mk2().capabilities
}

/// Home: key 1 opens the folder Tools, key 2 opens the top-level page
/// Other, key 3 runs an action. Tools: key 1 opens the folder Docker
/// (nested). The tree is stored.
struct Fixture {
    profile: Profile,
    layout: DeviceLayout,
    home: PageId,
    tools: PageId,
    docker: PageId,
    other: PageId,
    run: ActionInstanceId,
}

fn fixture() -> Fixture {
    let run = ActionInstance {
        id: ActionInstanceId::new(),
        name: "Run".into(),
        action_type: crate::testing::echo_type(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    };
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let home = layout.home_page;
    let mut tools = LayoutPage::folder("Tools", home);
    let docker = LayoutPage::folder("Docker", tools.id);
    let other = LayoutPage::new("Other");
    let (tools_id, docker_id, other_id) = (tools.id, docker.id, other.id);
    tools.bindings.push(press(1, open(docker_id)));
    layout.pages.extend([tools, docker, other]);
    layout.page_mut(home).unwrap().bindings.extend([
        press(1, open(tools_id)),
        press(2, open(other_id)),
        press(3, BindingTarget::action(run.id)),
    ]);
    layout.explicit_tree = true;
    let mut profile = Profile::new("Work");
    let run_id = run.id;
    profile.actions.push(run);
    Fixture {
        profile,
        layout,
        home,
        tools: tools_id,
        docker: docker_id,
        other: other_id,
        run: run_id,
    }
}

fn issues(f: &Fixture, layout: &DeviceLayout) -> Vec<LayoutIssue> {
    layout
        .validate(&ModelCatalog::builtin(), &f.profile)
        .into_iter()
        .filter(|issue| issue.folder_control_reason().is_some())
        .collect()
}

#[test]
fn a_key_that_opens_a_folder_of_its_page_is_a_folder_control() {
    let f = fixture();
    let layout = &f.layout;
    assert_eq!(layout.folder_control(f.home, key(1)), Some(f.tools));
    // Nested: Docker is a folder of Tools.
    assert_eq!(layout.folder_control(f.tools, key(1)), Some(f.docker));
    // A top-level page is not a folder: an ordinary page key.
    assert_eq!(layout.folder_control(f.home, key(2)), None);
    // Actions, empty keys and unknown pages are not.
    assert_eq!(layout.folder_control(f.home, key(3)), None);
    assert_eq!(layout.folder_control(f.home, key(4)), None);
    assert_eq!(layout.folder_control(PageId::new(), key(1)), None);
    assert_eq!(layout.folder_controls(f.home), vec![(key(1), f.tools)]);
    assert_eq!(issues(&f, layout), []);
}

#[test]
fn the_rule_follows_identifiers_not_names_labels_or_icons() {
    let f = fixture();
    let mut layout = f.layout.clone();
    // A page named like a folder, a key labelled "Folder": no folder control.
    layout.page_mut(f.other).unwrap().name = "Tools".into();
    let binding = &mut layout.page_mut(f.home).unwrap().bindings[1];
    binding.label = Some("Folder".into());
    binding.appearance = Some(KeyAppearance::default());
    assert_eq!(layout.folder_control(f.home, key(2)), None);
    // Renaming the folder and relabelling its key change nothing.
    layout.page_mut(f.tools).unwrap().name = "Renamed".into();
    layout.page_mut(f.home).unwrap().bindings[0].label = Some("Other".into());
    assert_eq!(layout.folder_control(f.home, key(1)), Some(f.tools));
    // Moving Other into Home makes its key a folder control.
    layout.page_mut(f.other).unwrap().parent = Some(f.home);
    assert_eq!(layout.folder_control(f.home, key(2)), Some(f.other));
}

#[test]
fn inferred_folders_count_and_dials_never_do() {
    // Generated layouts store no tree: the page a key opens first is its
    // folder.
    let mut layout = DeviceLayout::new("Old", models::stream_deck_mk2().id);
    let home = layout.home_page;
    let more = LayoutPage::new("More");
    let more_id = more.id;
    layout.pages.push(more);
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(press(14, open(more_id)));
    assert!(!layout.explicit_tree);
    assert_eq!(layout.folder_control(home, key(14)), Some(more_id));
    // A dial or a touch zone that opens a folder keeps ordinary bindings.
    let dial = ControlId::Encoder { index: 0 };
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(ControlBinding::new(dial, Trigger::Press, open(more_id)));
    assert_eq!(layout.folder_control(home, dial), None);
}

#[test]
fn saving_refuses_every_other_binding_on_a_folder_control() {
    let f = fixture();
    let workflow = WorkflowId::new();
    let extras = [
        on(1, Trigger::DoubleClick, BindingTarget::action(f.run)),
        on(1, Trigger::LongPress, BindingTarget::action(f.run)),
        on(1, Trigger::Release, BindingTarget::action(f.run)),
        on(1, Trigger::Release, BindingTarget::Workflow { workflow }),
        on(1, Trigger::Release, BindingTarget::Back),
        on(1, Trigger::Release, open(f.other)),
    ];
    for extra in extras {
        let mut layout = f.layout.clone();
        layout
            .page_mut(f.home)
            .unwrap()
            .bindings
            .push(extra.clone());
        let found = issues(&f, &layout);
        assert!(
            matches!(
                found.as_slice(),
                [LayoutIssue::FolderControlBindings { control, folder, .. }]
                    if *control == key(1) && folder == "Tools"
            ),
            "{extra:?}: {found:?}"
        );
        assert_eq!(
            found[0].folder_control_reason(),
            Some("folderControl.extraBinding")
        );
    }
}

#[test]
fn a_folder_control_opens_on_the_press_only() {
    let f = fixture();
    for trigger in [
        Trigger::Click,
        Trigger::DoubleClick,
        Trigger::LongPress,
        Trigger::Release,
    ] {
        let mut layout = f.layout.clone();
        layout.page_mut(f.home).unwrap().bindings[0].trigger = trigger;
        let found = issues(&f, &layout);
        assert!(
            matches!(
                found.as_slice(),
                [LayoutIssue::FolderControlTrigger { trigger: t, .. }] if *t == trigger
            ),
            "{trigger:?}: {found:?}"
        );
        assert_eq!(
            found[0].folder_control_reason(),
            Some("folderControl.trigger")
        );
    }
}

#[test]
fn replacing_what_a_folder_control_opens_is_refused_clearing_and_moving_are_not() {
    let f = fixture();
    let stored = &f.layout;
    let edited = |change: &dyn Fn(&mut DeviceLayout)| {
        let mut layout = stored.clone();
        change(&mut layout);
        layout.folder_control_edits(Some(stored))
    };
    // Unchanged.
    assert_eq!(edited(&|_| {}), []);
    // Replaced by an action: refused.
    let replaced = edited(&|layout| {
        layout.page_mut(f.home).unwrap().bindings[0].target = BindingTarget::action(f.run);
    });
    assert!(
        matches!(
            replaced.as_slice(),
            [LayoutIssue::FolderControlReplaced { control, folder, .. }]
                if *control == key(1) && folder == "Tools"
        ),
        "{replaced:?}"
    );
    assert_eq!(
        replaced[0].folder_control_reason(),
        Some("folderControl.replaced")
    );
    // Replaced by a page key to another page: refused too.
    assert_eq!(
        edited(&|layout| {
            layout.page_mut(f.home).unwrap().bindings[0].target = open(f.other);
        })
        .len(),
        1
    );
    // Cleared: allowed (the folder stays, reachable from the editor).
    assert_eq!(
        edited(&|layout| {
            layout
                .page_mut(f.home)
                .unwrap()
                .bindings
                .retain(|b| b.control != key(1));
        }),
        []
    );
    // Swapped with the action key: the folder is still opened, allowed.
    assert_eq!(
        edited(&|layout| {
            let page = layout.page_mut(f.home).unwrap();
            page.bindings[0].control = key(3);
            page.bindings[2].control = key(1);
        }),
        []
    );
    // The folder deleted (its key reused) or moved to the top level: allowed.
    assert_eq!(
        edited(&|layout| {
            layout.pages.retain(|p| p.id != f.tools && p.id != f.docker);
            layout.page_mut(f.home).unwrap().bindings[0].target = BindingTarget::action(f.run);
        }),
        []
    );
    assert_eq!(
        edited(&|layout| {
            layout.page_mut(f.tools).unwrap().parent = None;
            layout.page_mut(f.home).unwrap().bindings[0].target = BindingTarget::action(f.run);
        }),
        []
    );
    // A new layout has nothing to compare with.
    assert_eq!(stored.folder_control_edits(None), []);
}

/// Home key 1 opens Tools on a click, with a double click and a long press
/// that run the action and its own timings; Tools key 1 opens Docker with a
/// release that runs a workflow.
fn mixed(f: &Fixture) -> DeviceLayout {
    let mut layout = f.layout.clone();
    let home = layout.page_mut(f.home).unwrap();
    home.bindings[0].trigger = Trigger::Click;
    home.bindings[0].label = Some("Tools".into());
    home.bindings.extend([
        on(1, Trigger::DoubleClick, BindingTarget::action(f.run)),
        on(1, Trigger::LongPress, BindingTarget::action(f.run)),
    ]);
    home.gestures.push(KeyGestureSettings {
        control: key(1),
        double_click_ms: Some(400),
        long_press_ms: None,
    });
    layout.page_mut(f.tools).unwrap().bindings.push(on(
        1,
        Trigger::Release,
        BindingTarget::Workflow {
            workflow: WorkflowId::new(),
        },
    ));
    layout
}

#[test]
fn normalizing_keeps_the_open_on_the_press_and_recovers_the_rest() {
    let f = fixture();
    let mut layout = mixed(&f);
    let report = layout.normalize_folder_controls();
    assert_eq!(
        report,
        vec![
            NormalizedFolderControl {
                page: f.home,
                control: key(1),
                folder: f.tools,
                recovered: 2,
            },
            NormalizedFolderControl {
                page: f.tools,
                control: key(1),
                folder: f.docker,
                recovered: 1,
            },
        ]
    );
    let home = layout.page(f.home).unwrap();
    let own: Vec<_> = home
        .bindings
        .iter()
        .filter(|b| b.control == key(1))
        .collect();
    // The open, on the press, with its label.
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].trigger, Trigger::Press);
    assert_eq!(own[0].target, open(f.tools));
    assert_eq!(own[0].label.as_deref(), Some("Tools"));
    assert_eq!(home.gestures, []);
    // Other keys are untouched.
    assert_eq!(home.bindings.len(), 3);
    assert_eq!(layout.recovered.len(), 2);
    let entry = &layout.recovered[0];
    assert_eq!(
        (entry.page, entry.control, entry.folder),
        (f.home, key(1), f.tools)
    );
    assert_eq!(entry.reason, RecoveryReason::FolderControl);
    assert_eq!(
        entry.bindings.iter().map(|b| b.trigger).collect::<Vec<_>>(),
        [Trigger::DoubleClick, Trigger::LongPress]
    );
    assert_eq!(entry.gestures.map(|g| g.double_click_ms), Some(Some(400)));
    assert_eq!(layout.recovered_of(f.tools, key(1)).len(), 1);
    // Their action stays with the layout.
    assert_eq!(layout.bound_actions(), vec![f.run]);
    assert_eq!(issues(&f, &layout), []);
    // Idempotent.
    let again = layout.clone();
    assert_eq!(layout.normalize_folder_controls(), []);
    assert_eq!(layout, again);
}

#[test]
fn normalizing_a_clean_layout_changes_nothing() {
    let f = fixture();
    let mut layout = f.layout.clone();
    layout.explicit_tree = false;
    let before = serde_json::to_string(&layout).unwrap();
    assert_eq!(layout.normalize_folder_controls(), []);
    assert_eq!(serde_json::to_string(&layout).unwrap(), before);
    assert!(!before.contains("recovered"));
}

#[test]
fn normalizing_an_inferred_tree_stores_it_first() {
    // Key 1 opens Tools and, on a release, a second folder: dropping that
    // binding must not move the second folder to the top level.
    let mut layout = DeviceLayout::new("Old", models::stream_deck_mk2().id);
    let home = layout.home_page;
    let first = LayoutPage::new("First");
    let second = LayoutPage::new("Second");
    let (first_id, second_id) = (first.id, second.id);
    layout.pages.extend([first, second]);
    layout.page_mut(home).unwrap().bindings.extend([
        press(1, open(first_id)),
        on(1, Trigger::Release, open(second_id)),
    ]);
    let tree = layout.page_parents();
    assert_eq!(tree.get(&second_id), Some(&home));
    let report = layout.normalize_folder_controls();
    assert_eq!(report.len(), 1);
    assert!(layout.explicit_tree);
    assert_eq!(layout.page_parents(), tree);
    assert_eq!(layout.folder_control(home, key(1)), Some(first_id));
}

#[test]
fn a_lone_open_on_another_trigger_moves_to_the_press_and_lone_timings_go() {
    let f = fixture();
    let mut layout = f.layout.clone();
    let home = layout.page_mut(f.home).unwrap();
    home.bindings[0].trigger = Trigger::Click;
    home.gestures.push(KeyGestureSettings {
        control: key(1),
        double_click_ms: Some(400),
        long_press_ms: None,
    });
    // Changed, but nothing recovered.
    assert_eq!(layout.normalize_folder_controls(), []);
    let home = layout.page(f.home).unwrap();
    assert_eq!(home.bindings[0].trigger, Trigger::Press);
    assert_eq!(home.gestures, []);
    assert_eq!(layout.recovered, []);
}

#[test]
fn recovered_bindings_move_to_the_first_free_key_or_are_refused() {
    let f = fixture();
    let mut layout = mixed(&f);
    layout.normalize_folder_controls();
    let caps = capabilities();
    // Home binds keys 1, 2 and 3: key 0 is free (Home is no folder, no
    // reserved Back).
    let moved = layout.restore_recovered(f.home, key(1), &caps).unwrap();
    assert_eq!(moved, key(0));
    let home = layout.page(f.home).unwrap();
    let restored: Vec<_> = home
        .bindings
        .iter()
        .filter(|b| b.control == key(0))
        .collect();
    assert_eq!(
        restored.iter().map(|b| b.trigger).collect::<Vec<_>>(),
        [Trigger::DoubleClick, Trigger::LongPress]
    );
    assert_eq!(home.gestures[0].control, key(0));
    assert_eq!(
        layout.recovered_of(f.home, key(1)),
        Vec::<&ControlBinding>::new()
    );
    assert_eq!(layout.recovered.len(), 1);
    // Nothing left to move.
    assert_eq!(
        layout.restore_recovered(f.home, key(1), &caps),
        Err(RecoveredError::NotFound)
    );
    // In the folder Tools, key 0 is its reserved Back: never used.
    let mut full = layout.clone();
    let moved = full.restore_recovered(f.tools, key(1), &caps).unwrap();
    assert_eq!(moved, key(2));
    // A page whose keys are all bound refuses, changing nothing.
    let tools = layout.page_mut(f.tools).unwrap();
    for index in 2..15 {
        tools
            .bindings
            .push(press(index, BindingTarget::action(f.run)));
    }
    let before = layout.clone();
    assert_eq!(
        layout.restore_recovered(f.tools, key(1), &caps),
        Err(RecoveredError::NoFreeKey)
    );
    assert_eq!(layout, before);
}

#[test]
fn recovered_bindings_can_be_discarded_and_never_be_added_by_an_edit() {
    let f = fixture();
    let mut stored = mixed(&f);
    stored.normalize_folder_controls();
    assert_eq!(stored.folder_control_edits(Some(&stored)), []);
    // Discarded: allowed.
    let mut discarded = stored.clone();
    assert_eq!(discarded.discard_recovered(f.home, key(1)), 2);
    assert_eq!(discarded.discard_recovered(f.home, key(1)), 0);
    assert_eq!(discarded.folder_control_edits(Some(&stored)), []);
    // Moved: allowed.
    let mut moved = stored.clone();
    moved
        .restore_recovered(f.home, key(1), &capabilities())
        .unwrap();
    assert_eq!(moved.folder_control_edits(Some(&stored)), []);
    // Changed or added: refused.
    let mut changed = stored.clone();
    changed.recovered[0].bindings.pop();
    assert_eq!(
        changed.folder_control_edits(Some(&stored)),
        [LayoutIssue::FolderControlRecovered]
    );
    assert_eq!(
        LayoutIssue::FolderControlRecovered.folder_control_reason(),
        Some("folderControl.recovered")
    );
    assert_eq!(
        stored.folder_control_edits(None),
        [LayoutIssue::FolderControlRecovered]
    );
}

#[test]
fn the_recovered_list_round_trips_and_follows_copies() {
    let f = fixture();
    let mut layout = mixed(&f);
    layout.normalize_folder_controls();
    let json = serde_json::to_value(&layout).unwrap();
    let entry = &json["recovered"][0];
    assert_eq!(entry["page"], f.home.to_string());
    assert_eq!(
        entry["control"],
        serde_json::json!({"kind": "key", "index": 1})
    );
    assert_eq!(entry["folder"], f.tools.to_string());
    assert_eq!(entry["reason"], "folderControl");
    assert_eq!(entry["bindings"].as_array().unwrap().len(), 2);
    assert_eq!(entry["gestures"]["doubleClickMs"], 400);
    let back: DeviceLayout = serde_json::from_value(json).unwrap();
    assert_eq!(back, layout);
    // A copy's entries point at the copy's pages.
    let copy = layout.duplicated("Copy");
    let home = copy.home_page;
    assert_eq!(copy.recovered[0].page, home);
    let tools = copy.folder_control(home, key(1)).unwrap();
    assert_eq!(copy.recovered[0].folder, tools);
    // Entries of pages that are gone are pruned.
    let mut pruned = layout.clone();
    pruned.pages.retain(|p| p.id != f.tools && p.id != f.docker);
    assert_eq!(pruned.prune_recovered(), 1);
    assert_eq!(pruned.recovered.len(), 1);
}

/// Schema 13: the folder's name is the one title of its key.
#[test]
fn folder_titles_become_the_folders_name_and_differing_ones_are_kept() {
    let f = fixture();
    let mut layout = f.layout.clone();
    let titled = |layout: &mut DeviceLayout, page: PageId, label: &str| {
        let binding = layout
            .page_mut(page)
            .unwrap()
            .bindings
            .iter_mut()
            .find(|b| b.control == key(1))
            .unwrap();
        binding.label = Some(label.into());
        binding.appearance = Some(KeyAppearance {
            show_title: Some(false),
            ..KeyAppearance::default()
        });
    };
    // Home's folder key: a title of its own. Tools' folder key: the
    // folder's name again (spaces aside).
    titled(&mut layout, f.home, "My tools");
    titled(&mut layout, f.tools, " Docker ");
    // A key opening a top-level page is no folder control: untouched.
    let other = layout
        .page_mut(f.home)
        .unwrap()
        .bindings
        .iter_mut()
        .find(|b| b.control == key(2))
        .unwrap();
    other.label = Some("Elsewhere".into());
    let report = layout.normalize_folder_labels();
    assert_eq!(
        report,
        vec![NormalizedFolderLabel {
            page: f.home,
            control: key(1),
            folder: f.tools,
            label: "My tools".into(),
        }]
    );
    let label_of = |layout: &DeviceLayout, page: PageId, index: u16| {
        layout
            .page(page)
            .unwrap()
            .bindings
            .iter()
            .find(|b| b.control == key(index))
            .unwrap()
            .label
            .clone()
    };
    assert_eq!(label_of(&layout, f.home, 1), None);
    assert_eq!(label_of(&layout, f.tools, 1), None);
    assert_eq!(label_of(&layout, f.home, 2).as_deref(), Some("Elsewhere"));
    // The look stays (a hidden title stays hidden), the names and ids too.
    let look = layout
        .page(f.home)
        .unwrap()
        .bindings
        .iter()
        .find(|b| b.control == key(1))
        .and_then(|b| b.appearance.clone());
    assert_eq!(look.and_then(|l| l.show_title), Some(false));
    assert_eq!(layout.page(f.tools).unwrap().name, "Tools");
    // The differing title is kept, not run, and can be recovered.
    assert_eq!(
        layout.recovered_folder_label(f.home, key(1)),
        Some("My tools")
    );
    assert!(
        layout.recovered_of(f.home, key(1)).is_empty(),
        "not a gesture"
    );
    // Idempotent.
    let again = layout.clone();
    assert_eq!(layout.normalize_folder_labels(), []);
    assert_eq!(layout, again);
    // Round trip through JSON (stored and exported with the layout).
    let json = serde_json::to_value(&layout).unwrap();
    assert_eq!(json["recovered"][0]["reason"], "folderLabel");
    let back: DeviceLayout = serde_json::from_value(json).unwrap();
    assert_eq!(back, layout);
    // "Use as the folder's name" and "Discard".
    let mut adopted = layout.clone();
    assert_eq!(
        adopted
            .resolve_folder_label(f.home, key(1), true)
            .as_deref(),
        Some("My tools")
    );
    assert_eq!(adopted.page(f.tools).unwrap().name, "My tools");
    assert_eq!(adopted.recovered, []);
    let mut discarded = layout.clone();
    discarded.resolve_folder_label(f.home, key(1), false);
    assert_eq!(discarded.page(f.tools).unwrap().name, "Tools");
    assert_eq!(discarded.recovered, []);
}
