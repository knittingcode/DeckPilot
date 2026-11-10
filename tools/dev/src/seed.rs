//! Demonstration data for the development database.

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionRegistry, BindingTarget,
    ControlBinding, DeviceLayout, LayoutPage, MachineConfig, MachineResourceBinding, OnFailure, Os,
    ParameterValue, Profile, Project, ProjectKey, ResourceKey, ResourceKind, ResourceRef,
    StepCondition, Trigger, Workflow, WorkflowId, builtin,
};
use deckpilot_device::{ControlId, TouchGesture, models};
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore};

use crate::session::DevPaths;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Builder<'a> {
    registry: &'a ActionRegistry,
    profile: Profile,
}

impl Builder<'_> {
    fn project(&mut self, key: &str, name: &str, resources: &[(&str, &str)]) -> Result<()> {
        let mut project = Project::new(ProjectKey::new(key)?, name);
        for (role, resource) in resources {
            project
                .resources
                .insert((*role).to_owned(), ResourceKey::new(*resource)?);
        }
        self.profile.add_project(project)?;
        Ok(())
    }

    fn action(
        &mut self,
        name: &str,
        action_type: deckpilot_core::ActionTypeId,
        parameters: &ActionParameters,
        icon: &str,
        label: Option<&str>,
    ) -> Result<deckpilot_core::ActionInstanceId> {
        let mut instance: ActionInstance =
            self.registry
                .create_instance(name, action_type, parameters)?;
        instance.icon = Some(icon.to_owned());
        instance.label = label.map(str::to_owned);
        let id = instance.id;
        self.profile.actions.push(instance);
        Ok(id)
    }
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.to_owned())
}

fn press(control: ControlId, target: BindingTarget) -> ControlBinding {
    ControlBinding {
        control,
        trigger: Trigger::Press,
        target,
        label: None,
    }
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn work_profile(registry: &ActionRegistry) -> Result<Profile> {
    let mut b = Builder {
        registry,
        profile: Profile::new("Work"),
    };
    b.profile.description = "Client projects with a shared project switcher".into();
    b.project(
        "SHOP",
        "Online Shop",
        &[("repository", "shop.repository"), ("docs", "shop.docs")],
    )?;
    b.project(
        "CMR",
        "Customer Relations",
        &[("repository", "cmr.repository"), ("docs", "cmr.docs")],
    )?;
    b.project(
        "CMS",
        "Content Management",
        &[("repository", "cms.repository")],
    )?;
    b.profile.default_project = Some(b.profile.projects[0].id);

    let ids = work_actions(&mut b)?;
    let overview = DeviceLayout::generate("Overview", &models::stream_deck_xl(), &b.profile);
    for layout in [mk2_layout(&ids), overview, plus_layout(&ids)] {
        let id = layout.id;
        b.profile.layouts.push(layout);
        b.profile.activate_layout(id)?;
    }
    Ok(b.profile)
}

struct WorkIds {
    next: ActionInstanceId,
    previous: ActionInstanceId,
    selectors: Vec<ActionInstanceId>,
    repository: ActionInstanceId,
    docs: ActionInstanceId,
    announce: ActionInstanceId,
    shop_repository: ActionInstanceId,
    start: WorkflowId,
}

fn project_role(role: &str) -> ActionParameters {
    ActionParameters::new().with(
        "resource",
        ParameterValue::Resource(ResourceRef::ProjectRole { role: role.into() }),
    )
}

/// Actions and the "Start Work" workflow of the Work profile.
fn work_actions(b: &mut Builder<'_>) -> Result<WorkIds> {
    let cycle = |direction: &str| {
        ActionParameters::new().with("direction", ParameterValue::Choice(direction.into()))
    };
    let next = b.action(
        "Next project",
        builtin::cycle_project_id(),
        &cycle("next"),
        "chevron-right",
        Some("{project.key}"),
    )?;
    let previous = b.action(
        "Previous project",
        builtin::cycle_project_id(),
        &cycle("previous"),
        "chevron-left",
        Some("Previous"),
    )?;
    let mut selectors = Vec::new();
    for project in ["SHOP", "CMR", "CMS"] {
        let parameters = ActionParameters::new().with("project", text(project));
        let name = format!("Select {project}");
        selectors.push(b.action(
            &name,
            builtin::select_project_id(),
            &parameters,
            "folder",
            Some(project),
        )?);
    }
    let resolve = builtin::resolve_resource_id;
    let repository = b.action(
        "Repository — active project",
        resolve(),
        &project_role("repository"),
        "git-branch",
        Some("Repo {project.key}"),
    )?;
    let fixed = ActionParameters::new().with(
        "resource",
        ParameterValue::Resource(ResourceRef::Fixed {
            key: ResourceKey::new("shop.repository")?,
        }),
    );
    let shop_repository = b.action(
        "Repository — SHOP",
        resolve(),
        &fixed,
        "git-branch",
        Some("Repo SHOP"),
    )?;
    let docs = b.action(
        "Documentation — active project",
        resolve(),
        &project_role("docs"),
        "book-open",
        Some("Docs"),
    )?;
    let message = ActionParameters::new().with("message", text("Now working on {project.name}"));
    let announce = b.action(
        "Announce project",
        builtin::log_message_id(),
        &message,
        "megaphone",
        Some("{project.name}"),
    )?;

    let mut start = Workflow::new("Start Work")
        .step(announce, OnFailure::Continue)
        .step(repository, OnFailure::Continue)
        .step(docs, OnFailure::Continue)
        .step(shop_repository, OnFailure::Stop);
    start.description = "Announce the project and resolve its resources".into();
    start.steps[3].condition = StepCondition::ActiveProject {
        project: b.profile.projects[0].id,
    };
    let start_id = start.id;
    b.profile.workflows.push(start);
    Ok(WorkIds {
        next,
        previous,
        selectors,
        repository,
        docs,
        announce,
        shop_repository,
        start: start_id,
    })
}

/// MK.2: project switcher, selectors, workflow and a folder.
fn mk2_layout(ids: &WorkIds) -> DeviceLayout {
    let mut main = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let mut tools = LayoutPage::new("Tools");
    tools.bindings = vec![
        press(key(0), BindingTarget::Back),
        press(
            key(1),
            BindingTarget::Action {
                action: ids.announce,
            },
        ),
        press(
            key(2),
            BindingTarget::Action {
                action: ids.shop_repository,
            },
        ),
    ];
    let folder = ControlBinding {
        label: Some("Tools".into()),
        ..press(key(14), BindingTarget::OpenPage { page: tools.id })
    };
    main.pages[0].bindings = vec![
        press(key(0), BindingTarget::Action { action: ids.next }),
        press(
            key(1),
            BindingTarget::Action {
                action: ids.previous,
            },
        ),
        press(
            key(2),
            BindingTarget::Action {
                action: ids.selectors[0],
            },
        ),
        press(
            key(3),
            BindingTarget::Action {
                action: ids.selectors[1],
            },
        ),
        press(
            key(4),
            BindingTarget::Action {
                action: ids.selectors[2],
            },
        ),
        press(
            key(5),
            BindingTarget::Workflow {
                workflow: ids.start,
            },
        ),
        press(
            key(6),
            BindingTarget::Action {
                action: ids.repository,
            },
        ),
        press(key(7), BindingTarget::Action { action: ids.docs }),
        folder,
    ];
    main.pages.push(tools);
    main
}

/// Stream Deck +: dials switch projects, the touch strip announces.
fn plus_layout(ids: &WorkIds) -> DeviceLayout {
    let dial = ControlId::Encoder { index: 0 };
    let bind = |control, trigger, target, label: Option<&str>| ControlBinding {
        control,
        trigger,
        target,
        label: label.map(str::to_owned),
    };
    let mut dials = DeviceLayout::new("Dials", models::stream_deck_plus().id);
    dials.pages[0].bindings = vec![
        press(
            key(0),
            BindingTarget::Action {
                action: ids.selectors[0],
            },
        ),
        press(
            key(1),
            BindingTarget::Action {
                action: ids.selectors[1],
            },
        ),
        press(
            key(2),
            BindingTarget::Action {
                action: ids.selectors[2],
            },
        ),
        press(
            key(3),
            BindingTarget::Workflow {
                workflow: ids.start,
            },
        ),
        bind(
            dial,
            Trigger::RotateClockwise,
            BindingTarget::Action { action: ids.next },
            Some("{project.key}"),
        ),
        bind(
            dial,
            Trigger::RotateCounterClockwise,
            BindingTarget::Action {
                action: ids.previous,
            },
            None,
        ),
        press(
            ControlId::Encoder { index: 1 },
            BindingTarget::Workflow {
                workflow: ids.start,
            },
        ),
        bind(
            ControlId::TouchZone { index: 0 },
            Trigger::Touch {
                gesture: TouchGesture::Tap,
            },
            BindingTarget::Action {
                action: ids.announce,
            },
            None,
        ),
    ];
    dials
}

fn personal_profile(registry: &ActionRegistry) -> Result<Profile> {
    let mut b = Builder {
        registry,
        profile: Profile::new("Personal"),
    };
    b.profile.description = "Side projects".into();
    b.project("BLOG", "Blog", &[("repository", "blog.repository")])?;
    b.project(
        "DOTFILES",
        "Dotfiles",
        &[("repository", "dotfiles.repository")],
    )?;
    b.action(
        "Next project",
        builtin::cycle_project_id(),
        &ActionParameters::new(),
        "chevron-right",
        Some("{project.key}"),
    )?;
    let announce = b.action(
        "Announce project",
        builtin::log_message_id(),
        &ActionParameters::new().with("message", text("Personal time: {project.name}")),
        "coffee",
        None,
    )?;
    let repository = b.action(
        "Repository — active project",
        builtin::resolve_resource_id(),
        &ActionParameters::new().with(
            "resource",
            ParameterValue::Resource(ResourceRef::ProjectRole {
                role: "repository".into(),
            }),
        ),
        "git-branch",
        None,
    )?;
    // Same name as in Work, different steps and identifiers.
    b.profile.workflows.push(
        Workflow::new("Start Work")
            .step(announce, OnFailure::Continue)
            .step(repository, OnFailure::Stop),
    );
    let layout = DeviceLayout::generate("Main", &models::stream_deck_mk2(), &b.profile);
    b.profile.layouts.push(layout);
    Ok(b.profile)
}

fn machine(paths: &DevPaths) -> Result<MachineConfig> {
    let mut config = MachineConfig::new(Os::current());
    let mut bind = |key: &str, kind, value: String| -> Result<()> {
        config.set_binding(MachineResourceBinding {
            key: ResourceKey::new(key)?,
            kind,
            value,
        });
        Ok(())
    };
    for name in ["shop", "cmr", "blog"] {
        let dir = paths.workspace.join(name);
        std::fs::create_dir_all(&dir)?;
        bind(
            &format!("{name}.repository"),
            ResourceKind::Directory,
            dir.display().to_string(),
        )?;
    }
    // A binding that points to a missing folder, to show validation.
    bind(
        "dotfiles.repository",
        ResourceKind::Directory,
        paths
            .workspace
            .join("dotfiles-missing")
            .display()
            .to_string(),
    )?;
    bind(
        "shop.docs",
        ResourceKind::Url,
        "https://example.com/docs/shop".into(),
    )?;
    bind(
        "cmr.docs",
        ResourceKind::Url,
        "https://example.com/docs/cmr".into(),
    )?;
    // cms.repository is intentionally left unbound.
    Ok(config)
}

/// Seeds the development database if it has no profiles. Returns whether
/// data was written.
pub fn seed(paths: &DevPaths) -> Result<bool> {
    paths.create()?;
    let store = SqliteStore::open(paths.data.join("deckpilot.db"), &Encryption::None)?;
    if !store.list_profiles()?.is_empty() {
        return Ok(false);
    }
    let registry = ActionRegistry::with_builtins();
    let work = work_profile(&registry)?;
    let personal = personal_profile(&registry)?;
    let catalog = deckpilot_device::ModelCatalog::builtin();
    for profile in [&work, &personal] {
        let issues = profile.validate(&catalog);
        if !issues.is_empty() {
            return Err(format!("seed profile `{}` is invalid: {issues:?}", profile.name).into());
        }
        store.save_profile(profile)?;
    }
    store.set_active_profile(Some(work.id))?;
    store.save_machine_config(&machine(paths)?)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demonstration_profiles_are_valid_and_independent() {
        let registry = ActionRegistry::with_builtins();
        let catalog = deckpilot_device::ModelCatalog::builtin();
        let work = work_profile(&registry).unwrap();
        let personal = personal_profile(&registry).unwrap();
        assert!(
            work.validate(&catalog).is_empty(),
            "{:?}",
            work.validate(&catalog)
        );
        assert_eq!(personal.validate(&catalog), Vec::new());
        assert_eq!(work.layouts.len(), 3);
        assert_eq!(work.workflows[0].name, personal.workflows[0].name);
        assert_ne!(work.workflows[0].id, personal.workflows[0].id);
        assert_ne!(work.workflows[0].steps, personal.workflows[0].steps);
    }
}
