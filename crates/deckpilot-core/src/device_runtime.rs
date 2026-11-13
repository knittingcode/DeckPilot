//! Hardware-independent runtime for connected devices.
//!
//! [`Runtime`] owns the active profile and its project context, tracks
//! connected devices and the page each one shows, dispatches control input to
//! bindings, and describes what every key should display ([`KeyFace`]). The
//! background agent drives it with [`DeviceEvent`]s; nothing in here talks to
//! hardware.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use deckpilot_device::{
    ControlId, DeviceEvent, DeviceId, DeviceModelId, DeviceRegistry, ModelCatalog,
};

use crate::gesture::{BoundGestures, Gesture, GestureTimings};
use crate::{
    ActionInstanceId, ActionRegistry, ActionResult, ActiveProjectContext, BindingTarget,
    ControlBinding, CycleDirection, DeviceLayout, Engine, EngineError, FaceStyle, KeyFace,
    LayoutId, MachineConfig, PageId, Profile, Project, ProjectId, Trigger, WorkflowId,
    WorkflowReport, render_label,
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
    /// A control is bound to this workflow. Workflows can run for a long
    /// time, so the runtime does not run them itself: the caller starts
    /// the run, e.g. in the background (see [`Runtime::run_workflow`] for
    /// running one in place).
    WorkflowRequested {
        workflow: WorkflowId,
    },
}

#[derive(Debug)]
struct Session {
    profile: Profile,
    projects: ActiveProjectContext,
}

#[derive(Debug)]
pub struct Runtime {
    engine: Arc<Engine>,
    catalog: ModelCatalog,
    machine: MachineConfig,
    session: Option<Session>,
    devices: DeviceRegistry,
    /// Page stack per connected device; the last entry is the visible page.
    navigation: BTreeMap<DeviceId, Vec<PageId>>,
    /// Page stacks of devices that were disconnected, so that a device that
    /// comes back (a loose cable, a hub reset) shows the page it showed.
    /// Forgotten when the profile changes.
    parked: BTreeMap<DeviceId, Vec<PageId>>,
    /// Shown instead of "No profile" while there is no profile, e.g.
    /// "Locked" while the configuration is encrypted and not unlocked.
    placeholder: Option<String>,
    /// Gesture timings of controls whose page sets none (a machine
    /// setting).
    gesture_defaults: GestureTimings,
}

impl Runtime {
    pub fn new(engine: Engine, catalog: ModelCatalog, machine: MachineConfig) -> Self {
        Self {
            engine: Arc::new(engine),
            catalog,
            machine,
            session: None,
            devices: DeviceRegistry::default(),
            navigation: BTreeMap::new(),
            parked: BTreeMap::new(),
            placeholder: None,
            gesture_defaults: GestureTimings::default(),
        }
    }

    /// Sets the gesture timings of controls whose page sets none. Values
    /// outside the allowed ranges are ignored.
    pub fn set_gesture_defaults(&mut self, defaults: GestureTimings) {
        self.gesture_defaults = defaults.validate().unwrap_or_default();
    }

    pub fn gesture_defaults(&self) -> GestureTimings {
        self.gesture_defaults
    }

    /// The gestures bound to `control` on the page `device` shows, with
    /// the control's timings. `None`: raw mode, or nothing is shown.
    pub fn key_gestures(
        &self,
        device: &DeviceId,
        control: ControlId,
    ) -> Option<(BoundGestures, GestureTimings)> {
        let layout = self.layout_for(&device.model)?;
        let page = layout.page(self.current_page(device)?)?;
        let bound = page.bound_gestures(control);
        bound
            .any()
            .then(|| (bound, page.gesture_timings(control, self.gesture_defaults)))
    }

    /// Runs the binding of `control` for a recognized `gesture` on the
    /// page `device` shows, like a raw input in [`Runtime::handle_event`].
    /// The caller checks that the gesture was recognized on this page.
    pub fn dispatch_gesture(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        gesture: Gesture,
    ) -> Result<RuntimeOutcome, EngineError> {
        let ignored = |reason: &str| {
            Ok(RuntimeOutcome::Ignored {
                reason: reason.into(),
            })
        };
        if !self.devices.is_connected(device) {
            return ignored("device is not connected");
        }
        if self.session.is_none() {
            return ignored("no active profile");
        }
        let target = self
            .layout_for(&device.model)
            .zip(self.current_page(device))
            .and_then(|(layout, page)| layout.page(page))
            .and_then(|page| page.gesture_binding(control, gesture))
            .map(|binding| binding.target.clone());
        match target {
            None => ignored("gesture is not bound"),
            Some(target) => self.dispatch(device, target),
        }
    }

    /// Sets the text devices show while no profile is loaded.
    pub fn set_placeholder(&mut self, text: Option<&str>) {
        self.placeholder = text.map(str::to_owned);
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Replaces the engine, e.g. after feature modules were enabled or
    /// disabled. The profile, project context and pages are kept.
    pub fn set_engine(&mut self, engine: Engine) {
        self.engine = Arc::new(engine);
    }

    /// The engine, to run work on another thread.
    pub fn shared_engine(&self) -> Arc<Engine> {
        self.engine.clone()
    }

    /// The project selection of the active profile.
    pub fn project_context(&self) -> Option<&ActiveProjectContext> {
        self.session.as_ref().map(|session| &session.projects)
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
        self.parked.clear();
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
        let mut issues = layout.validate(&self.catalog, profile);
        issues.extend(layout.check_overrides(self.engine.registry(), profile));
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

    /// Whether every page of `stack` exists in the layout for `device`.
    fn pages_exist(&self, device: &DeviceId, stack: &[PageId]) -> bool {
        self.layout_for(&device.model).is_some_and(|layout| {
            !stack.is_empty() && stack.iter().all(|page| layout.page(*page).is_some())
        })
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
            return notice(self.placeholder.as_deref().unwrap_or("No profile"));
        };
        let (Some(layout), Some(page)) =
            (self.layout_for(&device.model), self.current_page(device))
        else {
            return notice("No layout");
        };
        let Some(page) = layout.page(page) else {
            return notice("No layout");
        };
        page_faces(
            self.engine.registry(),
            layout,
            page.id,
            profile,
            self.active_project(),
            &controls,
        )
    }

    /// The binding target behind each face of [`Runtime::faces`] that has
    /// one, for decorating faces with live state. Empty without a profile
    /// or layout.
    pub fn shown_targets(&self, device: &DeviceId) -> Vec<(ControlId, BindingTarget)> {
        let (Some(layout), Some(page)) =
            (self.layout_for(&device.model), self.current_page(device))
        else {
            return Vec::new();
        };
        let Some(page) = layout.page(page) else {
            return Vec::new();
        };
        let Some(model) = self.catalog.get(&device.model) else {
            return Vec::new();
        };
        model
            .capabilities
            .controls()
            .into_iter()
            .filter(|control| model.capabilities.has_display(*control))
            .filter_map(|control| {
                shown_binding(page, control).map(|binding| (control, binding.target.clone()))
            })
            .collect()
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
                match self.parked.remove(device) {
                    Some(stack) if self.pages_exist(device, &stack) => {
                        self.navigation.insert(device.clone(), stack);
                    }
                    _ => self.reset_navigation(device),
                }
                Ok(RuntimeOutcome::DeviceConnected {
                    device: device.clone(),
                })
            }
            DeviceEvent::Disconnected { device } => {
                self.devices.apply(event);
                if let Some(stack) = self.navigation.remove(device) {
                    self.parked.insert(device.clone(), stack);
                }
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
                let mut target = layout
                    .page(page_id)
                    .and_then(|page| page.binding_for(*control, *input))
                    .map(|binding| binding.target.clone());
                // A folder page without a way back gets one on its first
                // free key.
                if target.is_none() && matches!(input, deckpilot_device::ControlInput::Press) {
                    let controls: Vec<ControlId> = model
                        .capabilities
                        .controls()
                        .into_iter()
                        .filter(|c| model.capabilities.has_display(*c))
                        .collect();
                    if layout.automatic_back(page_id, &controls) == Some(*control) {
                        target = Some(BindingTarget::Back);
                    }
                }
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
            BindingTarget::Action { action, overrides } => {
                // The key's device is part of the context its references
                // are resolved in (`system.device.current.id`). The
                // binding's overrides are resolved in the same pipeline.
                let session = self.session.as_mut().ok_or(EngineError::NoActiveProfile)?;
                let result = self.engine.run_action_with(
                    &session.profile,
                    &self.machine,
                    &mut session.projects,
                    action,
                    Some(&overrides),
                    &crate::resolution::Invocation::from_device(device.clone()),
                )?;
                Ok(RuntimeOutcome::Action { action, result })
            }
            BindingTarget::Workflow { workflow } => {
                if self.profile().and_then(|p| p.workflow(workflow)).is_none() {
                    return Err(EngineError::UnknownWorkflow(workflow));
                }
                Ok(RuntimeOutcome::WorkflowRequested { workflow })
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
                // Back returns to the page's folder; for a page outside
                // folders, to the page that opened it.
                let current = self.current_page(device);
                let folder = current.and_then(|page| {
                    let layout = self.layout_for(&device.model)?;
                    let parent = layout.parent_of(page)?;
                    let mut path = layout.ancestors(parent);
                    path.reverse();
                    path.push(parent);
                    Some((parent, path))
                });
                let stack = self.navigation.entry(device.clone()).or_default();
                match folder {
                    Some((parent, path)) => match stack.iter().position(|p| *p == parent) {
                        Some(index) => stack.truncate(index + 1),
                        None => *stack = path,
                    },
                    None if stack.len() > 1 => {
                        stack.pop();
                    }
                    None => {}
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
    registry: &ActionRegistry,
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
    let automatic_back = layout.automatic_back(page.id, controls);
    controls
        .iter()
        .map(|control| {
            let binding = shown_binding(page, *control);
            let face = binding.map_or_else(
                || {
                    if automatic_back == Some(*control) {
                        KeyFace::new(FaceStyle::Back, "Back")
                    } else {
                        KeyFace::empty()
                    }
                },
                |binding| {
                    face_for(registry, binding, layout, profile, project)
                        .with_appearance(binding.appearance.as_ref())
                },
            );
            (*control, face)
        })
        .collect()
}

/// The binding a control's face shows: its press binding, else its
/// click binding, else the first.
fn shown_binding(page: &crate::LayoutPage, control: ControlId) -> Option<&ControlBinding> {
    page.bindings
        .iter()
        .filter(|binding| binding.control == control)
        .min_by_key(|binding| match binding.trigger {
            Trigger::Press => 0,
            Trigger::Click => 1,
            _ => 2,
        })
}

fn face_for(
    registry: &ActionRegistry,
    binding: &ControlBinding,
    layout: &DeviceLayout,
    profile: &Profile,
    project: Option<&Project>,
) -> KeyFace {
    match &binding.target {
        BindingTarget::Action { action, overrides } => {
            let Some(instance) = profile.action(*action) else {
                return KeyFace::new(FaceStyle::Notice, "Missing");
            };
            let template = binding
                .label
                .as_deref()
                .or(instance.label.as_deref())
                .unwrap_or(&instance.name);
            let label = render_label(template, &profile.name, project);
            // The owning module may add context such as the active project.
            registry
                .decorate_face(
                    &instance.action_type,
                    KeyFace::new(FaceStyle::Action, label.clone()),
                    &instance.parameters.overlay(overrides),
                    template,
                    project,
                )
                .unwrap_or_else(|| {
                    KeyFace::new(FaceStyle::Notice, label).with_detail("Unavailable")
                })
        }
        BindingTarget::Workflow { workflow } => match profile.workflow(*workflow) {
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
                .or_else(|| layout.page(*page).map(|p| p.name.clone()))
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
    use crate::testing::{registry, select_type, with_projects};
    use crate::{ActionParameters, LayoutPage, Os, ParameterValue};
    use deckpilot_device::{ControlInput, models};

    struct Fixture {
        profile: Profile,
        next: ActionInstanceId,
        select_cms: ActionInstanceId,
    }

    fn fixture() -> Fixture {
        let registry = registry();
        let mut profile = with_projects("Work", &["SHOP", "CMR", "CMS"]);
        let next = registry
            .create_instance("Next project", select_type(), &ActionParameters::new())
            .unwrap();
        let cms = profile.projects[2].id;
        let select = registry
            .create_instance(
                "CMS",
                select_type(),
                &ActionParameters::new().with("project", ParameterValue::Project(cms)),
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
            appearance: None,
        };
        layout.page_mut(home).unwrap().bindings.extend([
            bind(0, BindingTarget::action(next_id)),
            bind(1, BindingTarget::OpenPage { page: folder_id }),
            bind(2, BindingTarget::action(select_id)),
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
            Engine::new(registry()),
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

    fn connect(runtime: &mut Runtime, device: &DeviceId) {
        runtime
            .handle_event(&DeviceEvent::Connected {
                device: device.clone(),
            })
            .unwrap();
    }

    fn disconnect(runtime: &mut Runtime, device: &DeviceId) {
        runtime
            .handle_event(&DeviceEvent::Disconnected {
                device: device.clone(),
            })
            .unwrap();
    }

    #[test]
    fn a_reconnected_device_returns_to_its_page() {
        let f = fixture();
        let home = f.profile.layouts[0].home_page;
        let folder = f.profile.layouts[0].pages[1].id;
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile.clone()), None);
        connect(&mut runtime, &device());
        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device()), Some(folder));

        disconnect(&mut runtime, &device());
        assert_eq!(runtime.current_page(&device()), None);
        connect(&mut runtime, &device());
        assert_eq!(runtime.current_page(&device()), Some(folder));
        // The restored stack still knows the way back.
        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device()), Some(home));

        // A page deleted while the device was away: back to home.
        runtime.handle_event(&press(1)).unwrap();
        disconnect(&mut runtime, &device());
        let mut edited = f.profile.clone();
        edited.layouts[0].pages.retain(|page| page.id != folder);
        edited.layouts[0].pages[0]
            .bindings
            .retain(|binding| binding.target != BindingTarget::OpenPage { page: folder });
        runtime.reload_profile(edited);
        connect(&mut runtime, &device());
        assert_eq!(runtime.current_page(&device()), Some(home));

        // Another profile while the device was away: its home page.
        runtime.set_profile(Some(f.profile.clone()), None);
        runtime.handle_event(&press(1)).unwrap();
        disconnect(&mut runtime, &device());
        runtime.set_profile(Some(f.profile), None);
        connect(&mut runtime, &device());
        assert_eq!(runtime.current_page(&device()), Some(home));
    }

    #[test]
    fn devices_navigate_independently() {
        let f = fixture();
        let home = f.profile.layouts[0].home_page;
        let folder = f.profile.layouts[0].pages[1].id;
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile), None);
        let other = DeviceId {
            model: device().model,
            serial: "OTHER".into(),
        };
        connect(&mut runtime, &device());
        connect(&mut runtime, &other);

        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device()), Some(folder));
        assert_eq!(runtime.current_page(&other), Some(home));
        assert_eq!(runtime.faces(&other)[1].1.style, FaceStyle::Folder);

        disconnect(&mut runtime, &device());
        assert_eq!(runtime.current_page(&other), Some(home));
        let input = DeviceEvent::Input {
            device: device(),
            control: ControlId::Key { index: 1 },
            input: ControlInput::Press,
        };
        assert!(matches!(
            runtime.handle_event(&input).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
        assert_eq!(runtime.current_page(&other), Some(home));
    }

    #[test]
    fn back_returns_to_the_folder_and_folders_without_back_get_one() {
        let mut f = fixture();
        let layout = &mut f.profile.layouts[0];
        let home = layout.home_page;
        let folder = layout.pages[1].id;
        // "Sub" is inside the folder, has no Back key, and Home opens it
        // directly (key 3).
        let mut sub = LayoutPage::new("Sub");
        sub.parent = Some(folder);
        let sub_id = sub.id;
        layout.pages.push(sub);
        layout
            .page_mut(home)
            .unwrap()
            .bindings
            .push(ControlBinding::new(
                ControlId::Key { index: 3 },
                Trigger::Press,
                BindingTarget::OpenPage { page: sub_id },
            ));
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile), None);
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();
        runtime.handle_event(&press(3)).unwrap();
        assert_eq!(runtime.current_page(&device()), Some(sub_id));
        // The automatic Back sits on the first free key.
        let faces = runtime.faces(&device());
        assert_eq!(faces[0].1.style, FaceStyle::Back);
        assert!(faces[1].1.is_empty());
        // Back goes to the page's folder, not to where it was opened from.
        assert!(matches!(
            runtime.handle_event(&press(0)).unwrap(),
            RuntimeOutcome::Navigated { page, .. } if page == folder
        ));
        // The folder's own Back key (key 1) goes home.
        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device()), Some(home));
        // Opened the usual way, the folder goes back home as before.
        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device()), Some(folder));
        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device()), Some(home));
    }

    #[test]
    fn faces_carry_the_binding_appearance() {
        let mut f = fixture();
        let look = crate::KeyAppearance {
            background: Some(crate::Color([10, 20, 30])),
            ..crate::KeyAppearance::default()
        };
        f.profile.layouts[0].pages[0].bindings[1].appearance = Some(look.clone());
        // A default appearance is the same as none.
        f.profile.layouts[0].pages[0].bindings[2].appearance =
            Some(crate::KeyAppearance::default());
        let layout = &f.profile.layouts[0];
        let controls = deckpilot_device::models::stream_deck_mk2()
            .capabilities
            .controls();
        let faces = page_faces(
            &registry(),
            layout,
            layout.home_page,
            &f.profile,
            None,
            &controls,
        );
        assert_eq!(faces[1].1.appearance, Some(look));
        assert_eq!(faces[2].1.appearance, None);
        assert_eq!(faces[0].1.appearance, None);
    }

    #[test]
    fn shown_targets_follow_the_visible_page() {
        let f = fixture();
        let mut runtime = runtime();
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();
        assert_eq!(runtime.shown_targets(&device()), []);
        runtime.set_profile(Some(f.profile), None);
        let targets = runtime.shown_targets(&device());
        assert_eq!(targets.len(), 3);
        assert_eq!(
            targets[0],
            (ControlId::Key { index: 0 }, BindingTarget::action(f.next))
        );
        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(
            runtime.shown_targets(&device()),
            [(ControlId::Key { index: 1 }, BindingTarget::Back)]
        );
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
        let registry = registry();
        let faces = page_faces(
            &registry,
            layout,
            layout.home_page,
            &f.profile,
            Some(cms),
            &controls,
        );
        assert_eq!(faces.len(), 15);
        assert_eq!(faces[0].1.detail.as_deref(), Some("CMS"));
        assert!(
            faces[2].1.highlighted,
            "the CMS selector is highlighted for CMS"
        );
        let missing = page_faces(
            &registry,
            layout,
            PageId::new(),
            &f.profile,
            None,
            &controls,
        );
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

    /// Key 3 of the home page: click runs the CMS selector as saved, double
    /// click runs the same instance with SHOP instead, long press opens the
    /// folder. Its long press takes 1.2 s.
    fn gesture_fixture() -> (Fixture, ProjectId) {
        let mut f = fixture();
        let shop = f.profile.projects[0].id;
        let layout = &mut f.profile.layouts[0];
        let folder = layout.pages[1].id;
        let home = layout.home_page;
        let page = layout.page_mut(home).unwrap();
        let key = ControlId::Key { index: 3 };
        page.bindings.extend([
            ControlBinding::new(key, Trigger::Click, BindingTarget::action(f.select_cms)),
            ControlBinding::new(
                key,
                Trigger::DoubleClick,
                BindingTarget::Action {
                    action: f.select_cms,
                    overrides: ActionParameters::new()
                        .with("project", ParameterValue::Project(shop)),
                },
            ),
            ControlBinding::new(
                key,
                Trigger::LongPress,
                BindingTarget::OpenPage { page: folder },
            ),
        ]);
        page.gestures.push(crate::gesture::KeyGestureSettings {
            control: key,
            double_click_ms: None,
            long_press_ms: Some(1200),
        });
        (f, shop)
    }

    #[test]
    fn gestures_run_their_own_bindings() {
        let (f, _) = gesture_fixture();
        let saved = f.profile.action(f.select_cms).unwrap().parameters.clone();
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile.clone()), None);
        runtime.set_gesture_defaults(GestureTimings {
            double_click_ms: 400,
            long_press_ms: 700,
        });
        connect(&mut runtime, &device());
        let key = ControlId::Key { index: 3 };

        // Keys bound with press stay raw; key 3 is in gesture mode.
        assert_eq!(
            runtime.key_gestures(&device(), ControlId::Key { index: 0 }),
            None
        );
        assert_eq!(
            runtime.key_gestures(&device(), key),
            Some((
                BoundGestures {
                    click: true,
                    double_click: true,
                    long_press: true
                },
                GestureTimings {
                    double_click_ms: 400,
                    long_press_ms: 1200
                }
            ))
        );
        // A raw edge of a gesture key runs nothing.
        assert!(matches!(
            runtime.handle_event(&press(3)).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
        assert_eq!(active_key(&runtime), "SHOP");

        let outcome = runtime
            .dispatch_gesture(&device(), key, Gesture::Click)
            .unwrap();
        assert!(matches!(outcome, RuntimeOutcome::Action { action, .. } if action == f.select_cms));
        assert_eq!(active_key(&runtime), "CMS");
        // The same instance with the double click's override; the saved
        // instance is unchanged.
        runtime
            .dispatch_gesture(&device(), key, Gesture::DoubleClick)
            .unwrap();
        assert_eq!(active_key(&runtime), "SHOP");
        assert_eq!(
            runtime
                .profile()
                .unwrap()
                .action(f.select_cms)
                .unwrap()
                .parameters,
            saved
        );
        let folder = f.profile.layouts[0].pages[1].id;
        assert_eq!(
            runtime
                .dispatch_gesture(&device(), key, Gesture::LongPress)
                .unwrap(),
            RuntimeOutcome::Navigated {
                device: device(),
                page: folder
            }
        );
        // Nothing is bound to key 3 in the folder.
        assert!(matches!(
            runtime
                .dispatch_gesture(&device(), key, Gesture::Click)
                .unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
        assert_eq!(runtime.key_gestures(&device(), key), None);
    }

    #[test]
    fn faces_show_the_click_binding_with_its_overrides() {
        let (mut f, shop) = gesture_fixture();
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile.clone()), None);
        connect(&mut runtime, &device());
        // Click: the CMS selector, not highlighted while SHOP is active.
        let faces = runtime.faces(&device());
        assert_eq!(faces[3].1.style, FaceStyle::Action);
        assert!(!faces[3].1.highlighted);
        assert_eq!(
            runtime.shown_targets(&device())[3],
            (
                ControlId::Key { index: 3 },
                BindingTarget::action(f.select_cms)
            )
        );

        // The face of a binding with overrides is drawn with them.
        let home = f.profile.layouts[0].home_page;
        let page = f.profile.layouts[0].page_mut(home).unwrap();
        page.bindings.retain(|b| b.trigger != Trigger::Click);
        page.bindings.push(ControlBinding::new(
            ControlId::Key { index: 3 },
            Trigger::Click,
            BindingTarget::Action {
                action: f.select_cms,
                overrides: ActionParameters::new().with("project", ParameterValue::Project(shop)),
            },
        ));
        runtime.reload_profile(f.profile);
        assert!(runtime.faces(&device())[3].1.highlighted);
    }

    #[test]
    fn layout_status_reports_invalid_overrides() {
        let (mut f, _) = gesture_fixture();
        let home = f.profile.layouts[0].home_page;
        let layout_id = f.profile.layouts[0].id;
        let page = f.profile.layouts[0].page_mut(home).unwrap();
        for binding in &mut page.bindings {
            if let BindingTarget::Action { overrides, .. } = &mut binding.target
                && !overrides.is_empty()
            {
                *overrides = ActionParameters::new().with("nope", ParameterValue::Boolean(true));
            }
        }
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile), None);
        let status = runtime.layout_status(&device().model);
        assert!(
            matches!(&status, LayoutStatus::Invalid { layout, issues, .. }
                if *layout == layout_id && issues.len() == 1 && issues[0].contains("nope")),
            "{status:?}"
        );
    }
}
