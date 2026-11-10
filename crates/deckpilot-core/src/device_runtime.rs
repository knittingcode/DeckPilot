//! Hardware-independent runtime for connected devices.
//!
//! [`Runtime`] owns the active profile and its project context, tracks
//! connected devices and the page each one shows, dispatches control input to
//! bindings, and describes what every key should display ([`KeyFace`]). The
//! background agent drives it with [`DeviceEvent`]s; nothing in here talks to
//! hardware.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use deckpilot_device::{
    ControlId, DeviceEvent, DeviceId, DeviceModelId, DeviceRegistry, ModelCatalog,
};

use crate::{
    ActionInstanceId, ActionResult, ActiveProjectContext, BindingTarget, ControlBinding,
    CycleDirection, DeviceLayout, Engine, EngineError, FaceStyle, KeyFace, LayoutId, MachineConfig,
    PageId, Profile, Project, ProjectId, Trigger, WorkflowId, WorkflowReport, builtin,
    render_label,
};

/// Whether a profile provides a usable layout for a device model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LayoutStatus {
    NoProfile,
    UnknownModel,
    Missing,
    Compatible {
        layout: LayoutId,
        name: String,
    },
    Invalid {
        layout: LayoutId,
        name: String,
        issues: Vec<String>,
    },
}

/// Outcome of handling one device event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RuntimeOutcome {
    /// The event did not trigger anything.
    Ignored {
        reason: String,
    },
    DeviceConnected {
        device: DeviceId,
    },
    DeviceDisconnected {
        device: DeviceId,
    },
    Navigated {
        device: DeviceId,
        page: PageId,
    },
    Action {
        action: ActionInstanceId,
        result: ActionResult,
    },
    Workflow {
        report: WorkflowReport,
    },
}

#[derive(Debug)]
struct Session {
    profile: Profile,
    projects: ActiveProjectContext,
}

#[derive(Debug)]
pub struct Runtime {
    engine: Engine,
    catalog: ModelCatalog,
    machine: MachineConfig,
    session: Option<Session>,
    devices: DeviceRegistry,
    /// Page stack per connected device; the last entry is the visible page.
    navigation: BTreeMap<DeviceId, Vec<PageId>>,
}

impl Runtime {
    pub fn new(engine: Engine, catalog: ModelCatalog, machine: MachineConfig) -> Self {
        Self {
            engine,
            catalog,
            machine,
            session: None,
            devices: DeviceRegistry::default(),
            navigation: BTreeMap::new(),
        }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn catalog(&self) -> &ModelCatalog {
        &self.catalog
    }

    pub fn machine(&self) -> &MachineConfig {
        &self.machine
    }

    pub fn set_machine(&mut self, machine: MachineConfig) {
        self.machine = machine;
    }

    pub fn devices(&self) -> &DeviceRegistry {
        &self.devices
    }

    pub fn profile(&self) -> Option<&Profile> {
        self.session.as_ref().map(|session| &session.profile)
    }

    /// Activates `profile` (or none). The project selection is restored from
    /// `project` when it belongs to the profile, and every connected device
    /// returns to the home page of its layout. No action is executed.
    pub fn set_profile(&mut self, profile: Option<Profile>, project: Option<ProjectId>) {
        self.session = profile.map(|profile| Session {
            projects: ActiveProjectContext::restore(&profile, project),
            profile,
        });
        let connected: Vec<DeviceId> = self.devices.connected().cloned().collect();
        for device in connected {
            self.reset_navigation(&device);
        }
    }

    /// Replaces the active profile with an updated version of itself, keeping
    /// the project selection and the visible pages where they still exist.
    pub fn reload_profile(&mut self, profile: Profile) {
        let same = self
            .profile()
            .is_some_and(|current| current.id == profile.id);
        if !same {
            self.set_profile(Some(profile), None);
            return;
        }
        let project = self.active_project_id();
        self.session = Some(Session {
            projects: ActiveProjectContext::restore(&profile, project),
            profile,
        });
        let devices: Vec<DeviceId> = self.navigation.keys().cloned().collect();
        for device in devices {
            let valid = match self.layout_for(&device.model) {
                Some(layout) => self.navigation[&device]
                    .iter()
                    .all(|page| layout.page(*page).is_some()),
                None => self.navigation[&device].is_empty(),
            };
            if !valid {
                self.reset_navigation(&device);
            }
        }
    }

    pub fn active_project(&self) -> Option<&Project> {
        let session = self.session.as_ref()?;
        session.projects.project(&session.profile).ok().flatten()
    }

    pub fn active_project_id(&self) -> Option<ProjectId> {
        self.session
            .as_ref()
            .and_then(|session| session.projects.project_id())
    }

    pub fn select_project(&mut self, project: ProjectId) -> Result<(), EngineError> {
        let session = self.session.as_mut().ok_or(EngineError::NoActiveProfile)?;
        session
            .projects
            .select(&session.profile, project)
            .map_err(|_| EngineError::UnknownProject(project))
    }

    pub fn cycle_project(
        &mut self,
        direction: CycleDirection,
    ) -> Result<Option<ProjectId>, EngineError> {
        let session = self.session.as_mut().ok_or(EngineError::NoActiveProfile)?;
        Ok(session.projects.cycle(&session.profile, direction)?)
    }

    pub fn run_action(&mut self, action: ActionInstanceId) -> Result<ActionResult, EngineError> {
        let session = self.session.as_mut().ok_or(EngineError::NoActiveProfile)?;
        self.engine.run_action(
            &session.profile,
            &self.machine,
            &mut session.projects,
            action,
        )
    }

    pub fn run_workflow(&mut self, workflow: WorkflowId) -> Result<WorkflowReport, EngineError> {
        let session = self.session.as_mut().ok_or(EngineError::NoActiveProfile)?;
        self.engine.run_workflow(
            &session.profile,
            &self.machine,
            &mut session.projects,
            workflow,
        )
    }

    /// The layout used for devices of `model` (see [`Profile::layout_for`]).
    pub fn layout_for(&self, model: &DeviceModelId) -> Option<&DeviceLayout> {
        self.profile()?.layout_for(model)
    }

    pub fn layout_status(&self, model: &DeviceModelId) -> LayoutStatus {
        let Some(profile) = self.profile() else {
            return LayoutStatus::NoProfile;
        };
        if self.catalog.get(model).is_none() {
            return LayoutStatus::UnknownModel;
        }
        let Some(layout) = self.layout_for(model) else {
            return LayoutStatus::Missing;
        };
        let issues = layout.validate(&self.catalog, profile);
        if issues.is_empty() {
            LayoutStatus::Compatible {
                layout: layout.id,
                name: layout.name.clone(),
            }
        } else {
            LayoutStatus::Invalid {
                layout: layout.id,
                name: layout.name.clone(),
                issues: issues.iter().map(ToString::to_string).collect(),
            }
        }
    }

    /// The page currently shown on `device`, if it is connected and has a layout.
    pub fn current_page(&self, device: &DeviceId) -> Option<PageId> {
        self.navigation
            .get(device)
            .and_then(|stack| stack.last().copied())
    }

    fn reset_navigation(&mut self, device: &DeviceId) {
        let home = self
            .layout_for(&device.model)
            .map(|layout| layout.home_page);
        self.navigation
            .insert(device.clone(), home.into_iter().collect());
    }

    /// What every display-capable control of a connected device should show.
    /// Never executes actions. Returns an empty list for unknown models.
    pub fn faces(&self, device: &DeviceId) -> Vec<(ControlId, KeyFace)> {
        let Some(model) = self.catalog.get(&device.model) else {
            return Vec::new();
        };
        let controls: Vec<ControlId> = model
            .capabilities
            .controls()
            .into_iter()
            .filter(|control| model.capabilities.has_display(*control))
            .collect();

        let notice = |text: &str| {
            controls
                .iter()
                .enumerate()
                .map(|(index, control)| {
                    let face = if index == 0 {
                        KeyFace::new(FaceStyle::Notice, text)
                    } else {
                        KeyFace::empty()
                    };
                    (*control, face)
                })
                .collect()
        };

        let Some(profile) = self.profile() else {
            return notice("No profile");
        };
        let (Some(layout), Some(page)) =
            (self.layout_for(&device.model), self.current_page(device))
        else {
            return notice("No layout");
        };
        let Some(page) = layout.page(page) else {
            return notice("No layout");
        };
        page_faces(layout, page.id, profile, self.active_project(), &controls)
    }

    pub fn handle_event(&mut self, event: &DeviceEvent) -> Result<RuntimeOutcome, EngineError> {
        let ignored = |reason: &str| {
            Ok(RuntimeOutcome::Ignored {
                reason: reason.into(),
            })
        };
        match event {
            DeviceEvent::Connected { device } => {
                self.devices.apply(event);
                self.reset_navigation(device);
                Ok(RuntimeOutcome::DeviceConnected {
                    device: device.clone(),
                })
            }
            DeviceEvent::Disconnected { device } => {
                self.devices.apply(event);
                self.navigation.remove(device);
                Ok(RuntimeOutcome::DeviceDisconnected {
                    device: device.clone(),
                })
            }
            DeviceEvent::Input {
                device,
                control,
                input,
            } => {
                if !self.devices.is_connected(device) {
                    return ignored("device is not connected");
                }
                let Some(model) = self.catalog.get(&device.model) else {
                    return ignored("unknown device model");
                };
                if model.capabilities.validate_input(*control, *input).is_err() {
                    return ignored("input is not supported by the device model");
                }
                if self.session.is_none() {
                    return ignored("no active profile");
                }
                let (Some(layout), Some(page_id)) =
                    (self.layout_for(&device.model), self.current_page(device))
                else {
                    return ignored("no layout for this device model");
                };
                let target = layout
                    .page(page_id)
                    .and_then(|page| page.binding_for(*control, *input))
                    .map(|binding| binding.target);
                match target {
                    None => ignored("control is not bound"),
                    Some(target) => self.dispatch(device, target),
                }
            }
        }
    }

    fn dispatch(
        &mut self,
        device: &DeviceId,
        target: BindingTarget,
    ) -> Result<RuntimeOutcome, EngineError> {
        match target {
            BindingTarget::Action { action } => {
                let result = self.run_action(action)?;
                Ok(RuntimeOutcome::Action { action, result })
            }
            BindingTarget::Workflow { workflow } => {
                let report = self.run_workflow(workflow)?;
                Ok(RuntimeOutcome::Workflow { report })
            }
            BindingTarget::OpenPage { page } => {
                let stack = self.navigation.entry(device.clone()).or_default();
                stack.push(page);
                Ok(RuntimeOutcome::Navigated {
                    device: device.clone(),
                    page,
                })
            }
            BindingTarget::Back => {
                let stack = self.navigation.entry(device.clone()).or_default();
                if stack.len() > 1 {
                    stack.pop();
                }
                match stack.last() {
                    Some(page) => Ok(RuntimeOutcome::Navigated {
                        device: device.clone(),
                        page: *page,
                    }),
                    None => Ok(RuntimeOutcome::Ignored {
                        reason: "no page to return to".into(),
                    }),
                }
            }
        }
    }
}

/// Faces for `controls` on one page of a layout. Unbound controls are empty.
/// Used for devices and for previews in the layout designer.
pub fn page_faces(
    layout: &DeviceLayout,
    page: PageId,
    profile: &Profile,
    project: Option<&Project>,
    controls: &[ControlId],
) -> Vec<(ControlId, KeyFace)> {
    let Some(page) = layout.page(page) else {
        return controls
            .iter()
            .map(|control| (*control, KeyFace::empty()))
            .collect();
    };
    controls
        .iter()
        .map(|control| {
            let binding = page
                .bindings
                .iter()
                .filter(|binding| binding.control == *control)
                .min_by_key(|binding| binding.trigger != Trigger::Press);
            let face = binding.map_or_else(KeyFace::empty, |binding| {
                face_for(binding, layout, profile, project)
            });
            (*control, face)
        })
        .collect()
}

fn face_for(
    binding: &ControlBinding,
    layout: &DeviceLayout,
    profile: &Profile,
    project: Option<&Project>,
) -> KeyFace {
    match binding.target {
        BindingTarget::Action { action } => {
            let Some(instance) = profile.action(action) else {
                return KeyFace::new(FaceStyle::Notice, "Missing");
            };
            let template = binding
                .label
                .as_deref()
                .or(instance.label.as_deref())
                .unwrap_or(&instance.name);
            let face = KeyFace::new(
                FaceStyle::Action,
                render_label(template, &profile.name, project),
            );
            // A switcher whose label already shows the project needs no second line.
            if instance.action_type == builtin::cycle_project_id()
                && !template.contains("{project.")
            {
                face.with_detail(project.map_or("-", |p| p.key.as_str()))
            } else if instance.action_type == builtin::select_project_id() {
                let key = instance.parameters.text("project").unwrap_or_default();
                face.highlighted(project.is_some_and(|p| p.key.matches(key)))
            } else {
                face
            }
        }
        BindingTarget::Workflow { workflow } => match profile.workflow(workflow) {
            Some(definition) => KeyFace::new(
                FaceStyle::Workflow,
                render_label(
                    binding.label.as_deref().unwrap_or(&definition.name),
                    &profile.name,
                    project,
                ),
            ),
            None => KeyFace::new(FaceStyle::Notice, "Missing"),
        },
        BindingTarget::OpenPage { page } => KeyFace::new(
            FaceStyle::Folder,
            binding
                .label
                .clone()
                .or_else(|| layout.page(page).map(|p| p.name.clone()))
                .unwrap_or_else(|| "Folder".into()),
        ),
        BindingTarget::Back => KeyFace::new(
            FaceStyle::Back,
            binding.label.clone().unwrap_or_else(|| "Back".into()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionParameters, ActionRegistry, LayoutPage, Os, ParameterValue, ProjectKey};
    use deckpilot_device::{ControlInput, models};

    struct Fixture {
        profile: Profile,
        next: ActionInstanceId,
        select_cms: ActionInstanceId,
    }

    fn fixture() -> Fixture {
        let registry = ActionRegistry::with_builtins();
        let mut profile = Profile::new("Work");
        for key in ["SHOP", "CMR", "CMS"] {
            profile
                .add_project(Project::new(ProjectKey::new(key).unwrap(), key))
                .unwrap();
        }
        let next = registry
            .create_instance(
                "Next project",
                builtin::cycle_project_id(),
                &ActionParameters::new(),
            )
            .unwrap();
        let select = registry
            .create_instance(
                "CMS",
                builtin::select_project_id(),
                &ActionParameters::new().with("project", ParameterValue::Text("CMS".into())),
            )
            .unwrap();
        let (next_id, select_id) = (next.id, select.id);
        profile.actions.extend([next, select]);

        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        let folder = LayoutPage::new("Folder");
        let folder_id = folder.id;
        layout.pages.push(folder);
        let home = layout.home_page;
        let bind = |index, target| ControlBinding {
            control: ControlId::Key { index },
            trigger: Trigger::Press,
            target,
            label: None,
        };
        layout.page_mut(home).unwrap().bindings.extend([
            bind(0, BindingTarget::Action { action: next_id }),
            bind(1, BindingTarget::OpenPage { page: folder_id }),
            bind(2, BindingTarget::Action { action: select_id }),
        ]);
        layout
            .page_mut(folder_id)
            .unwrap()
            .bindings
            .push(bind(1, BindingTarget::Back));
        profile.layouts.push(layout);
        Fixture {
            profile,
            next: next_id,
            select_cms: select_id,
        }
    }

    fn runtime() -> Runtime {
        Runtime::new(
            Engine::new(ActionRegistry::with_builtins()),
            ModelCatalog::builtin(),
            MachineConfig::new(Os::current()),
        )
    }

    fn device() -> DeviceId {
        DeviceId {
            model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
            serial: "TEST".into(),
        }
    }

    fn press(index: u16) -> DeviceEvent {
        DeviceEvent::Input {
            device: device(),
            control: ControlId::Key { index },
            input: ControlInput::Press,
        }
    }

    fn release(index: u16) -> DeviceEvent {
        DeviceEvent::Input {
            device: device(),
            control: ControlId::Key { index },
            input: ControlInput::Release,
        }
    }

    fn active_key(runtime: &Runtime) -> String {
        runtime.active_project().unwrap().key.to_string()
    }

    #[test]
    fn routes_button_presses_to_actions_and_cycles_projects() {
        let f = fixture();
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile), None);
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();

        let mut visited = Vec::new();
        for _ in 0..3 {
            let outcome = runtime.handle_event(&press(0)).unwrap();
            assert!(
                matches!(outcome, RuntimeOutcome::Action { action, ref result }
                if action == f.next && result.is_success())
            );
            // Releases are not bound and must not trigger the action again.
            assert!(matches!(
                runtime.handle_event(&release(0)).unwrap(),
                RuntimeOutcome::Ignored { .. }
            ));
            visited.push(active_key(&runtime));
        }
        assert_eq!(visited, ["CMR", "CMS", "SHOP"]);

        let outcome = runtime.handle_event(&press(2)).unwrap();
        assert!(matches!(outcome, RuntimeOutcome::Action { action, .. } if action == f.select_cms));
        assert_eq!(active_key(&runtime), "CMS");
    }

    #[test]
    fn navigates_folders_and_ignores_unusable_input() {
        let f = fixture();
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile), None);
        assert!(matches!(
            runtime.handle_event(&press(0)).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();
        let home = runtime.current_page(&device()).unwrap();

        let RuntimeOutcome::Navigated { page: folder, .. } =
            runtime.handle_event(&press(1)).unwrap()
        else {
            panic!("expected navigation");
        };
        assert_ne!(folder, home);
        // Key 0 is unbound inside the folder.
        assert!(matches!(
            runtime.handle_event(&press(0)).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device()), Some(home));

        let encoder = DeviceEvent::Input {
            device: device(),
            control: ControlId::Encoder { index: 0 },
            input: ControlInput::Rotate { delta: 1 },
        };
        assert!(matches!(
            runtime.handle_event(&encoder).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));

        runtime
            .handle_event(&DeviceEvent::Disconnected { device: device() })
            .unwrap();
        assert_eq!(runtime.current_page(&device()), None);
    }

    #[test]
    fn faces_reflect_layout_and_project_context() {
        let f = fixture();
        let mut runtime = runtime();
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();

        let faces = runtime.faces(&device());
        assert_eq!(faces.len(), 15);
        assert_eq!(faces[0].1.title, "No profile");
        assert!(faces[1..].iter().all(|(_, face)| face.is_empty()));

        runtime.set_profile(Some(f.profile), None);
        let faces = runtime.faces(&device());
        assert_eq!(faces[0].1.detail.as_deref(), Some("SHOP"));
        assert_eq!(faces[1].1.style, FaceStyle::Folder);
        assert!(!faces[2].1.highlighted);
        assert!(faces[3].1.is_empty());

        runtime.handle_event(&press(2)).unwrap();
        let faces = runtime.faces(&device());
        assert_eq!(faces[0].1.detail.as_deref(), Some("CMS"));
        assert!(faces[2].1.highlighted);
    }

    #[test]
    fn label_templates_follow_the_project_cycle() {
        let mut f = fixture();
        // A project switcher whose label is the active project's key.
        f.profile.layouts[0].pages[0].bindings[0].label = Some("{project.key}".into());
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile), None);
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();

        let mut titles = vec![runtime.faces(&device())[0].1.title.clone()];
        for _ in 0..3 {
            runtime.handle_event(&press(0)).unwrap();
            titles.push(runtime.faces(&device())[0].1.title.clone());
        }
        assert_eq!(titles, ["SHOP", "CMR", "CMS", "SHOP"]);
    }

    #[test]
    fn page_faces_preview_any_project_without_a_device() {
        let f = fixture();
        let layout = &f.profile.layouts[0];
        let model = deckpilot_device::models::stream_deck_mk2();
        let controls = model.capabilities.controls();
        let cms = &f.profile.projects[2];
        let faces = page_faces(layout, layout.home_page, &f.profile, Some(cms), &controls);
        assert_eq!(faces.len(), 15);
        assert_eq!(faces[0].1.detail.as_deref(), Some("CMS"));
        assert!(
            faces[2].1.highlighted,
            "the CMS selector is highlighted for CMS"
        );
        let missing = page_faces(layout, PageId::new(), &f.profile, None, &controls);
        assert!(missing.iter().all(|(_, face)| face.is_empty()));
    }

    #[test]
    fn missing_layout_shows_fallback_and_status() {
        let mut f = fixture();
        f.profile.layouts.clear();
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile), None);
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();
        assert_eq!(runtime.faces(&device())[0].1.title, "No layout");
        assert_eq!(
            runtime.layout_status(&device().model),
            LayoutStatus::Missing
        );
        assert_eq!(
            runtime.layout_status(&DeviceModelId::new("vendor.unknown")),
            LayoutStatus::UnknownModel
        );
        assert!(matches!(
            runtime.handle_event(&press(0)).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
    }

    #[test]
    fn switching_profiles_resets_pages_without_running_actions() {
        let f = fixture();
        let other = fixture().profile;
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile.clone()), None);
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();
        runtime.handle_event(&press(1)).unwrap();
        runtime
            .handle_event(&press(0))
            .unwrap_or_else(|_| unreachable!());

        runtime.set_profile(Some(other.clone()), None);
        assert_eq!(runtime.profile().unwrap().id, other.id);
        assert_eq!(
            runtime.current_page(&device()),
            Some(other.layouts[0].home_page)
        );
        assert_eq!(active_key(&runtime), "SHOP");
        assert!(matches!(
            runtime.layout_status(&device().model),
            LayoutStatus::Compatible { .. }
        ));

        runtime.set_profile(None, None);
        assert_eq!(
            runtime.layout_status(&device().model),
            LayoutStatus::NoProfile
        );
        assert_eq!(
            runtime.run_action(f.next),
            Err(EngineError::NoActiveProfile)
        );
    }

    #[test]
    fn reload_keeps_project_and_valid_pages() {
        let f = fixture();
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile.clone()), None);
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();
        runtime.cycle_project(CycleDirection::Next).unwrap();
        runtime.handle_event(&press(1)).unwrap();
        let folder = runtime.current_page(&device());

        let mut updated = f.profile.clone();
        updated.name = "Renamed".into();
        runtime.reload_profile(updated.clone());
        assert_eq!(active_key(&runtime), "CMR");
        assert_eq!(runtime.current_page(&device()), folder);

        updated.layouts[0].pages.truncate(1);
        runtime.reload_profile(updated);
        assert_eq!(
            runtime.current_page(&device()),
            Some(f.profile.layouts[0].home_page)
        );
    }
}
