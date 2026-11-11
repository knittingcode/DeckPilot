//! Demonstration data for the development database.
//!
//! This is the only place that creates example projects and actions. A
//! production database starts empty. The seed includes an empty profile so
//! that empty states stay visible during development.

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionRegistry, BindingTarget,
    ControlBinding, DeviceLayout, LayoutPage, MachineConfig, MachineResourceBinding, OnFailure, Os,
    ParameterValue, Profile, Project, ProjectKey, ResourceKey, ResourceKind, StepCondition,
    Trigger, Workflow, WorkflowId,
};
use deckpilot_device::{ControlId, TouchGesture, models};
use deckpilot_modules::general;
use deckpilot_modules::projects::{ProjectCatalog, actions};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::{DatabaseState, Vault};
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore};

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

/// Actions and the "Start Work" workflow of the Work profile.
fn work_actions(b: &mut Builder<'_>) -> Result<WorkIds> {
    let next = b.action(
        "Next project",
        actions::next_id(),
        &ActionParameters::new(),
        "chevron-right",
        Some("{project.key}"),
    )?;
    let previous = b.action(
        "Previous project",
        actions::previous_id(),
        &ActionParameters::new(),
        "chevron-left",
        Some("Previous"),
    )?;
    // Instances the user could have created: one Select action without a
    // project (the current one) and two that pick a project explicitly.
    let mut selectors = vec![b.action(
        "Current project",
        actions::select_id(),
        &ActionParameters::new(),
        "folder",
        Some("Now"),
    )?];
    for index in [1, 2] {
        let project = &b.profile.projects[index];
        let (id, key) = (project.id, project.key.to_string());
        selectors.push(b.action(
            &format!("Select {key}"),
            actions::select_id(),
            &ActionParameters::new().with("project_id", ParameterValue::Project(id)),
            "folder",
            Some(&key),
        )?);
    }
    // Generic types: the current project unless a project is chosen.
    let repository = b.action(
        "Repository — current project",
        actions::repository_id(),
        &ActionParameters::new(),
        "git-branch",
        Some("Repo {project.key}"),
    )?;
    let shop = b.profile.projects[0].id;
    let shop_repository = b.action(
        "Repository — SHOP",
        actions::repository_id(),
        &ActionParameters::new().with("project_id", ParameterValue::Project(shop)),
        "git-branch",
        Some("Repo SHOP"),
    )?;
    let docs = b.action(
        "Documentation — current project",
        actions::documentation_id(),
        &ActionParameters::new(),
        "book-open",
        Some("Docs"),
    )?;
    let message = ActionParameters::new().with("message", text("Now working on {project.name}"));
    let announce = b.action(
        "Announce project",
        general::log_id(),
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
    b.profile
        .workflows
        .push(check_workflow(announce, repository, docs));
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

/// A graph workflow: announce, resolve repository and documentation in
/// parallel, then wait a few seconds, long enough to watch it run.
fn check_workflow(
    announce: ActionInstanceId,
    repository: ActionInstanceId,
    docs: ActionInstanceId,
) -> Workflow {
    let nodes = serde_json::json!([
        {"id": "start", "type": "trigger", "position": {"x": 0, "y": 140}},
        {"id": "announce", "type": "action", "action": announce, "position": {"x": 260, "y": 140}},
        {"id": "fork", "type": "parallel", "position": {"x": 520, "y": 140}},
        {"id": "repository", "type": "action", "action": repository, "position": {"x": 780, "y": 40},
         "policy": {"onError": "continue"}},
        {"id": "docs", "type": "action", "action": docs, "position": {"x": 780, "y": 240},
         "policy": {"onError": "continue"}},
        {"id": "both", "type": "join", "position": {"x": 1040, "y": 140}},
        {"id": "settle", "type": "delay", "ms": 8000, "label": "Settle", "position": {"x": 1300, "y": 140}},
    ]);
    let edges = serde_json::json!([
        {"from": "start", "to": "announce"},
        {"from": "announce", "to": "fork"},
        {"from": "fork", "to": "repository"},
        {"from": "fork", "to": "docs"},
        {"from": "repository", "to": "both"},
        {"from": "docs", "to": "both"},
        {"from": "both", "to": "settle"},
    ]);
    let mut workflow = Workflow::new("Project check");
    workflow.description = "Announce, resolve resources in parallel, then wait".into();
    workflow.graph = Some(
        serde_json::from_value(serde_json::json!({"version": 1, "nodes": nodes, "edges": edges}))
            .expect("the demonstration graph is valid"),
    );
    workflow
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
        actions::next_id(),
        &ActionParameters::new(),
        "chevron-right",
        Some("{project.key}"),
    )?;
    let announce = b.action(
        "Announce project",
        general::log_id(),
        &ActionParameters::new().with("message", text("Personal time: {project.name}")),
        "coffee",
        None,
    )?;
    let repository = b.action(
        "Repository — current project",
        actions::repository_id(),
        &ActionParameters::new(),
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

/// Development-only setting that records that the seed ran.
const SEEDED: &str = "dev.seeded";

/// Seeds the development database if it has no profiles. Returns whether
/// data was written.
/// Password of the development database. A fixture for `make up` only: it
/// exists in this development tool, never in the application, and protects
/// nothing but demonstration data in `.dev/data`.
pub const DEV_PASSWORD: &str = "deckpilot-dev";

fn vault(paths: &DevPaths) -> Vault {
    Vault::new(paths.data.join("deckpilot.db"))
}

/// The key of the development database, if it is encrypted with the
/// development password.
pub fn dev_key(paths: &DevPaths) -> Option<DatabaseKey> {
    let vault = vault(paths);
    match vault.state() {
        Ok(DatabaseState::Encrypted) => vault.unlock(&Password::new(DEV_PASSWORD)).ok(),
        _ => None,
    }
}

/// Encrypts an unencrypted development database from an earlier version
/// with [`DEV_PASSWORD`], like the app's migration does.
pub fn encrypt_legacy(paths: &DevPaths) -> Result<()> {
    let vault = vault(paths);
    if vault.state()? == DatabaseState::Plaintext {
        vault.encrypt_plaintext(&Password::new(DEV_PASSWORD))?;
    }
    Ok(())
}

/// Seeds a password-protected development database once per data
/// directory. A missing database is left to the agent, which creates it
/// with a system-managed key like a fresh installation; it is seeded through
/// the agent then ([`seed_with_key`]). `password`: create a missing database
/// protected by [`DEV_PASSWORD`] instead. An unencrypted database from an
/// earlier version is left alone so that the migration can be tried.
/// Returns whether data was written.
pub fn seed(paths: &DevPaths, password: bool) -> Result<bool> {
    paths.create()?;
    let vault = vault(paths);
    let key = match vault.state()? {
        DatabaseState::Missing if password => vault.initialize(&Password::new(DEV_PASSWORD))?,
        DatabaseState::Missing => return Ok(false),
        DatabaseState::Plaintext => {
            println!(
                "the development database is unencrypted (created by an earlier version); \
                 the agent encrypts it with a system-managed key when it starts"
            );
            return Ok(false);
        }
        DatabaseState::Encrypted => {
            if !matches!(
                vault.protection()?,
                Some(deckpilot_storage::vault::Protection::Password { .. })
            ) {
                // System-managed: only the agent reads the key.
                return Ok(false);
            }
            let Ok(key) = vault.unlock(&Password::new(DEV_PASSWORD)) else {
                println!("the development database uses a different password; not seeding");
                return Ok(false);
            };
            key
        }
    };
    seed_with_key(paths, key)
}

/// Seeds the development database opened with `key` (from the agent for a
/// system-managed key), once per data directory.
pub fn seed_with_key(paths: &DevPaths, key: DatabaseKey) -> Result<bool> {
    let store = SqliteStore::open(vault(paths).database(), &Encryption::Key(key))?;
    // Seed once per data directory: deleting the demo data must not bring
    // it back on the next `make up`. `seed --reset` starts a new directory.
    if store.setting(SEEDED)?.is_some() {
        return Ok(false);
    }
    // The app asks for the password once per version; not in development.
    store.set_setting(CONFIRMED_VERSION, env!("CARGO_PKG_VERSION"))?;
    if !store.list_profiles()?.is_empty() {
        store.set_setting(SEEDED, "existing data")?;
        return Ok(false);
    }
    let registry = deckpilot_modules::builtin(Os::current()).action_registry();
    let work = work_profile(&registry)?;
    let personal = personal_profile(&registry)?;
    let mut empty = Profile::new("Empty");
    empty.description = "No projects, actions or layouts yet".into();
    let catalog = deckpilot_device::ModelCatalog::builtin();
    for profile in [&work, &personal, &empty] {
        let issues = profile.validate(&catalog);
        if !issues.is_empty() {
            return Err(format!("seed profile `{}` is invalid: {issues:?}", profile.name).into());
        }
        store.save_profile(profile)?;
    }
    store.set_active_profile(Some(work.id))?;
    store.set_setting(SEEDED, "demo")?;
    store.save_machine_config(&machine(paths)?)?;
    Ok(true)
}

/// Setting the configuration app uses to ask for the password once after
/// an update.
const CONFIRMED_VERSION: &str = "security.confirmedVersion";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demonstration_profiles_are_valid_and_independent() {
        let registry = deckpilot_modules::builtin(Os::current()).action_registry();
        let catalog = deckpilot_device::ModelCatalog::builtin();
        let work = work_profile(&registry).unwrap();
        let personal = personal_profile(&registry).unwrap();
        assert!(
            work.validate(&catalog).is_empty(),
            "{:?}",
            work.validate(&catalog)
        );
        assert_eq!(personal.validate(&catalog), Vec::new());
        // The graph workflow runs: no validation errors.
        let check = work
            .workflows
            .iter()
            .find(|w| w.graph.is_some())
            .expect("a graph workflow");
        let engine = deckpilot_core::Engine::new(registry);
        assert_eq!(engine.check_runnable(&work, check.id), Ok(()));
        let registry = engine.registry();
        assert_eq!(work.layouts.len(), 3);
        assert_eq!(work.workflows[0].name, personal.workflows[0].name);
        assert_ne!(work.workflows[0].id, personal.workflows[0].id);
        assert_ne!(work.workflows[0].steps, personal.workflows[0].steps);
        // Every seeded action is valid in its own profile.
        for profile in [&work, &personal] {
            for action in &profile.actions {
                assert_eq!(
                    registry.check_in_profile(action, profile),
                    Vec::<String>::new()
                );
            }
        }
    }
}
