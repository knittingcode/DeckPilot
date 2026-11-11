use deckpilot_device::{DeviceModelId, models};

use super::*;
use crate::{ControlBinding, Trigger};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn open(index: u16, page: PageId) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, BindingTarget::OpenPage { page })
}

fn back(index: u16) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, BindingTarget::Back)
}

fn mk2() -> DeviceModelId {
    DeviceModelId::new(models::STREAM_DECK_MK2_ID)
}

fn catalog() -> ModelCatalog {
    ModelCatalog::builtin()
}

/// Home opens Tools (key 1); Tools opens Git (key 1) and has Back;
/// Git has Back. Stored as an old layout: no parents.
struct Tree {
    layout: DeviceLayout,
    tools: PageId,
    git: PageId,
}

fn tree() -> Tree {
    let mut layout = DeviceLayout::new("Main", mk2());
    let tools = LayoutPage::new("Tools");
    let git = LayoutPage::new("Git");
    let (tools_id, git_id) = (tools.id, git.id);
    // Children listed before their folders, to show order does not matter.
    layout.pages.insert(0, git);
    layout.pages.push(tools);
    let home = layout.home_page;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(open(1, tools_id));
    let page = layout.page_mut(tools_id).unwrap();
    page.bindings.extend([back(0), open(1, git_id)]);
    layout.page_mut(git_id).unwrap().bindings.push(back(0));
    Tree {
        layout,
        tools: tools_id,
        git: git_id,
    }
}

#[test]
fn trees_of_old_layouts_are_inferred_losslessly_and_idempotently() {
    let Tree {
        mut layout,
        tools,
        git,
    } = tree();
    let home = layout.home_page;
    let stored = serde_json::to_value(&layout).unwrap();
    assert!(!stored.to_string().contains("parent"));
    assert_eq!(layout.parent_of(tools), Some(home));
    assert_eq!(layout.parent_of(git), Some(tools));
    assert_eq!(layout.parent_of(home), None);
    assert_eq!(layout.ancestors(git), [tools, home]);
    assert_eq!(layout.subtree(tools), [tools, git]);
    // Reading changes nothing: the layout serializes as before.
    assert_eq!(serde_json::to_value(&layout).unwrap(), stored);

    let parents = layout.page_parents();
    let bindings: Vec<_> = layout.pages.iter().map(|p| p.bindings.clone()).collect();
    assert_eq!(layout.infer_parents(), 2);
    assert_eq!(layout.page_parents(), parents, "the same tree");
    assert_eq!(
        layout
            .pages
            .iter()
            .map(|p| p.bindings.clone())
            .collect::<Vec<_>>(),
        bindings
    );
    assert_eq!(layout.infer_parents(), 0, "idempotent");
    assert_eq!(layout.validate(&catalog(), &Profile::new("W")), []);
}

#[test]
fn explicit_parents_win_and_inference_never_creates_cycles() {
    let Tree {
        mut layout,
        tools,
        git,
    } = tree();
    let home = layout.home_page;
    // Git is filed under Home explicitly, although Tools opens it.
    layout.page_mut(git).unwrap().parent = Some(home);
    assert_eq!(layout.parent_of(git), Some(home));
    // Tools is explicitly inside Git, and Git opens Tools: no cycle is
    // inferred, Git stays where it is filed.
    layout.page_mut(git).unwrap().parent = None;
    layout.page_mut(tools).unwrap().parent = Some(git);
    layout.page_mut(git).unwrap().bindings.push(open(2, tools));
    let parents = layout.page_parents();
    assert_eq!(parents.get(&tools), Some(&git));
    assert_ne!(parents.get(&git), Some(&tools));
    // Unreachable pages without a parent are top-level.
    let orphan = LayoutPage::new("Orphan");
    let orphan_id = orphan.id;
    layout.pages.push(orphan);
    assert_eq!(layout.parent_of(orphan_id), None);
}

#[test]
fn parents_are_validated() {
    let Tree {
        mut layout,
        tools,
        git,
    } = tree();
    let home = layout.home_page;
    let profile = Profile::new("W");
    layout.page_mut(tools).unwrap().parent = Some(git);
    layout.page_mut(git).unwrap().parent = Some(tools);
    let issues = layout.validate(&catalog(), &profile);
    assert_eq!(
        issues
            .iter()
            .filter(|i| matches!(i, LayoutIssue::ParentCycle { .. }))
            .count(),
        2
    );
    layout.page_mut(git).unwrap().parent = Some(PageId::new());
    layout.page_mut(tools).unwrap().parent = None;
    assert!(matches!(
        layout.validate(&catalog(), &profile)[..],
        [LayoutIssue::UnknownParent { .. }]
    ));
    layout.page_mut(git).unwrap().parent = None;
    layout.page_mut(home).unwrap().parent = Some(tools);
    assert!(
        layout
            .validate(&catalog(), &profile)
            .contains(&LayoutIssue::HomeHasParent)
    );
}

#[test]
fn folders_need_a_way_back_or_get_an_automatic_one() {
    let Tree {
        mut layout,
        tools,
        git,
    } = tree();
    let controls = models::stream_deck_mk2().capabilities.controls();
    assert_eq!(layout.warnings(), []);
    assert_eq!(layout.automatic_back(git, &controls), None);
    // Git loses its Back key: a warning, not an error, and the runtime
    // uses the first free key.
    layout.page_mut(git).unwrap().bindings.clear();
    assert!(matches!(
        layout.warnings()[..],
        [LayoutIssue::MissingBack { .. }]
    ));
    assert_eq!(layout.validate(&catalog(), &Profile::new("W")), []);
    assert_eq!(layout.automatic_back(git, &controls), Some(key(0)));
    let home = layout.home_page;
    layout.page_mut(git).unwrap().bindings.push(open(0, home));
    // Opening a folder above counts as a way back.
    assert_eq!(layout.automatic_back(git, &controls), None);
    layout.page_mut(git).unwrap().bindings[0] = open(0, PageId::new());
    assert_eq!(layout.automatic_back(git, &controls), Some(key(1)));
    // Pages outside folders need none.
    assert_eq!(layout.automatic_back(layout.home_page, &controls), None);
    assert!(layout.has_way_back(tools));
}

#[test]
fn pages_are_reordered_and_refiled_without_cycles() {
    let Tree {
        mut layout,
        tools,
        git,
    } = tree();
    let home = layout.home_page;
    assert_eq!(
        layout.reorder_pages(&[home, tools]),
        Err(ProfileIssue::PageOrder)
    );
    assert_eq!(
        layout.reorder_pages(&[home, tools, tools]),
        Err(ProfileIssue::PageOrder)
    );
    let tree_before = layout.page_parents();
    layout.reorder_pages(&[home, tools, git]).unwrap();
    assert_eq!(
        layout.pages.iter().map(|p| p.id).collect::<Vec<_>>(),
        [home, tools, git]
    );
    assert_eq!(layout.page_parents(), tree_before);

    assert_eq!(
        layout.set_parent(tools, Some(git)),
        Err(ProfileIssue::PageCycle)
    );
    assert_eq!(
        layout.set_parent(tools, Some(tools)),
        Err(ProfileIssue::PageCycle)
    );
    assert_eq!(
        layout.set_parent(home, Some(tools)),
        Err(ProfileIssue::HomePage)
    );
    layout.set_parent(git, Some(home)).unwrap();
    assert_eq!(layout.parent_of(git), Some(home));
    layout.set_parent(git, None).unwrap();
    assert_eq!(layout.parent_of(git), None);
}

fn profile_with(layouts: Vec<DeviceLayout>) -> Profile {
    let mut profile = Profile::new("Work");
    profile.layouts = layouts;
    profile
}

#[test]
fn pages_are_copied_with_their_folders() {
    let Tree { layout, tools, git } = tree();
    let source = layout.id;
    let mut other = DeviceLayout::new("Second", mk2());
    other.pages[0].bindings.push(back(14));
    let target = other.id;
    let mut profile = profile_with(vec![layout, other]);

    let copy = profile
        .copy_page(
            source,
            tools,
            PageDestination {
                layout: target,
                parent: None,
            },
            &catalog(),
        )
        .unwrap();
    let other = profile.layout(target).unwrap();
    assert_eq!(other.pages.len(), 3);
    assert_eq!(other.parent_of(copy), Some(other.home_page));
    let copied_git = other.subtree(copy)[1];
    assert_ne!(copied_git, git);
    assert_eq!(other.parent_of(copied_git), Some(copy));
    // The link to Git points to the copy.
    assert!(
        other
            .page(copy)
            .unwrap()
            .bindings
            .iter()
            .any(|b| b.target == BindingTarget::OpenPage { page: copied_git })
    );
    assert_eq!(profile.validate(&catalog()), []);
    // The source is unchanged.
    assert_eq!(profile.layout(source).unwrap().pages.len(), 3);

    // Within the same layout: a sibling copy with new ids.
    let twin = profile
        .copy_page(
            source,
            git,
            PageDestination {
                layout: source,
                parent: None,
            },
            &catalog(),
        )
        .unwrap();
    let layout = profile.layout(source).unwrap();
    assert_eq!(layout.parent_of(twin), Some(tools));
    assert_eq!(layout.pages.len(), 4);
}

#[test]
fn copies_that_would_break_are_refused() {
    let Tree {
        mut layout, tools, ..
    } = tree();
    let home = layout.home_page;
    // Tools links back to Home with an openPage: fine in its layout, not
    // in another one.
    layout.page_mut(tools).unwrap().bindings.push(open(2, home));
    let source = layout.id;
    let other = DeviceLayout::new("Second", mk2());
    let target = other.id;
    // A model without key 14.
    let mut small = DeviceLayout::new("Mini", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    small.model = DeviceModelId::new("elgato.streamdeck.plus");
    let small_id = small.id;
    let mut profile = profile_with(vec![layout, other, small]);
    let to = |layout| PageDestination {
        layout,
        parent: None,
    };
    assert_eq!(
        profile.copy_page(source, tools, to(target), &catalog()),
        Err(ProfileIssue::OpensOutside)
    );
    profile.layouts[0]
        .page_mut(tools)
        .unwrap()
        .bindings
        .retain(|b| b.control != key(2));
    profile.layouts[0]
        .page_mut(tools)
        .unwrap()
        .bindings
        .push(back(14));
    assert!(matches!(
        profile.copy_page(source, tools, to(small_id), &catalog()),
        Err(ProfileIssue::IncompatiblePage(_))
    ));
    assert_eq!(profile.layouts[2].pages.len(), 1, "nothing was added");
    assert_eq!(
        profile.copy_page(source, PageId::new(), to(target), &catalog()),
        Err(ProfileIssue::NotFound("page"))
    );
}

#[test]
fn pages_are_moved_with_their_folders() {
    let Tree { layout, tools, git } = tree();
    let source = layout.id;
    let home = layout.home_page;
    let other = DeviceLayout::new("Second", mk2());
    let target = other.id;
    let mut profile = profile_with(vec![layout, other]);
    let to = |layout, parent| PageDestination { layout, parent };

    assert_eq!(
        profile.move_page(source, home, to(target, None), &catalog()),
        Err(ProfileIssue::HomePage)
    );
    // Within a layout: refiled, same ids.
    profile
        .move_page(source, git, to(source, Some(home)), &catalog())
        .unwrap();
    assert_eq!(profile.layouts[0].parent_of(git), Some(home));
    profile
        .move_page(source, git, to(source, Some(tools)), &catalog())
        .unwrap();

    let removed = profile
        .move_page(source, tools, to(target, None), &catalog())
        .unwrap();
    // Home's key that opened Tools is gone with it.
    assert_eq!(removed, 1);
    let old = &profile.layouts[0];
    assert_eq!(old.pages.len(), 1);
    assert_eq!(old.pages[0].bindings, []);
    let new = &profile.layouts[1];
    assert_eq!(new.subtree(tools), [tools, git]);
    assert_eq!(new.parent_of(tools), Some(new.home_page));
    assert_eq!(profile.validate(&catalog()), []);
}

#[test]
fn copied_layouts_and_profiles_keep_their_tree() {
    let Tree {
        mut layout, tools, ..
    } = tree();
    layout.infer_parents();
    assert!(layout.explicit_tree);
    let id = layout.id;
    let mut profile = profile_with(vec![layout]);
    let copy = profile.duplicate_layout(id, "Copy").unwrap();
    let copy = profile.layout(copy).unwrap();
    let copied_tools = copy.pages.iter().find(|p| p.name == "Tools").unwrap().id;
    assert_ne!(copied_tools, tools);
    assert_eq!(copy.parent_of(copied_tools), Some(copy.home_page));
    assert_eq!(profile.validate(&catalog()), []);
    let other = profile.duplicate("Other");
    assert_eq!(other.validate(&catalog()), []);
    assert_eq!(
        other.layouts[0].subtree(other.layouts[0].home_page).len(),
        3
    );
}
