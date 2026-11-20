//! Hardware-independent runtime for connected devices.
//!
//! [`Runtime`] owns the active profile and its project context, tracks
//! connected devices and the page each one shows, dispatches control input to
//! bindings, and describes what every key should display ([`KeyFace`]). The
//! background agent drives it with [`DeviceEvent`]s; nothing in here talks to
//! hardware.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use deckpilot_device::{
    ControlId, DeviceCapabilities, DeviceEvent, DeviceId, DeviceModelId, DeviceRegistry,
    ModelCatalog,
};

use crate::gesture::{BoundGestures, Gesture, GestureTimings};
use crate::i18n::SystemText;
use crate::stateful::StatefulBinding;
use crate::{
    ActionInstanceId, ActionParameters, ActionRegistry, ActionResult, ActiveProjectContext,
    BindingTarget, ControlBinding, CycleDirection, DeviceLayout, Engine, EngineError, FaceStyle,
    FolderExit, FolderTimers, KeyFace, LayoutId, MachineConfig, PageId, PageLabel, Profile,
    ProfileId, Project, ProjectId, Trigger, WorkflowId, WorkflowReport, render_label,
    render_page_label,
};

/// The clock Auto Exit countdowns ([`crate::FolderTimers`]) are measured
/// with: [`Instant::now`], replaced in tests ([`Runtime::set_clock`]).
#[derive(Clone)]
struct Clock(Arc<dyn Fn() -> Instant + Send + Sync>);

impl Default for Clock {
    fn default() -> Self {
        Self(Arc::new(Instant::now))
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Clock")
    }
}

/// The error code of an action that changes a device's page and was run
/// without a device, e.g. with "Run" in the app.
pub const NO_DEVICE: &str = "no-device";

/// *Switch layout*: the layout is not assigned to the active profile (or
/// no longer exists).
pub const LAYOUT_UNAVAILABLE: &str = "layout-unavailable";

/// The detail of a *Back to previous layout* key with nowhere to go back
/// to.
pub const NO_PREVIOUS_LAYOUT: &str = "No previous layout";

/// *Switch layout*: the layout is made for another device model.
pub const LAYOUT_INCOMPATIBLE: &str = "layout-incompatible";

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
    /// A control is bound to an action whose handler runs detached
    /// ([`crate::ActionHandler::runs_detached`], e.g. a plugin's action).
    /// The runtime does not run it in place: the caller runs
    /// [`Runtime::detached_action`] on a thread of its own and reports the
    /// result when it is there.
    ActionRequested {
        action: ActionInstanceId,
        /// The binding's parameter overrides.
        #[serde(default, skip_serializing_if = "ActionParameters::is_empty")]
        overrides: ActionParameters,
        /// The device whose control asked.
        device: DeviceId,
    },
    /// A control is bound to this workflow. Workflows can run for a long
    /// time, so the runtime does not run them itself: the caller starts
    /// the run, e.g. in the background (see [`Runtime::run_workflow`] for
    /// running one in place).
    WorkflowRequested {
        workflow: WorkflowId,
    },
    /// A gesture of a stateful key. The runtime does not know the
    /// machine's state: the caller picks the operation from the state at
    /// the press (see [`StatefulBinding`]) and lets the machine's owner
    /// apply it.
    StatefulRequested {
        binding: StatefulBinding,
        gesture: Gesture,
    },
    /// The control is bound to an action that cannot run from it (an
    /// encoder rotation bound to a key-only action, a gesture the action
    /// does not support; see [`crate::compatibility`]). Nothing ran. The
    /// binding is kept and works again once the action or device it needs
    /// is back.
    Unavailable {
        action: ActionInstanceId,
        control: ControlId,
        trigger: Trigger,
        reason: crate::IncompatibilityReason,
        /// The first refusal of this binding since the profile or the
        /// action types were loaded: the caller tells the user once.
        first: bool,
    },
}

/// One run of a detached action ([`Runtime::detached_action`]), to be
/// executed on a thread of the caller's choice with
/// [`DetachedAction::run`]. It works on copies taken when it was created,
/// so the runtime may change meanwhile.
#[derive(Debug)]
pub struct DetachedAction {
    engine: Arc<Engine>,
    profile: Profile,
    machine: MachineConfig,
    projects: ActiveProjectContext,
    action: ActionInstanceId,
    overrides: Option<ActionParameters>,
    invocation: crate::resolution::Invocation,
}

impl DetachedAction {
    pub fn action(&self) -> ActionInstanceId {
        self.action
    }

    /// The instance's name in the profile it was taken from.
    pub fn name(&self) -> String {
        self.profile
            .action(self.action)
            .map_or_else(|| self.action.to_string(), |a| a.name.clone())
    }

    /// Runs the action (blocking). It works on a copy of the project
    /// context, so a result that asks to change the context is turned
    /// into a failure rather than reported as applied.
    pub fn run(mut self) -> Result<ActionResult, EngineError> {
        let result = self.engine.run_action_with(
            &self.profile,
            &self.machine,
            &mut self.projects,
            self.action,
            self.overrides.as_ref(),
            &self.invocation,
        )?;
        if result.is_success() && !result.effects.is_empty() {
            return Ok(ActionResult::failure(
                "an action that runs detached cannot change the project context",
            ));
        }
        if result.request.is_some() {
            return Ok(ActionResult::failure(
                "an action that runs detached cannot change pages or profiles",
            ));
        }
        Ok(result)
    }
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
    /// The top-level page each device showed in a profile when the profile
    /// was left (or when the agent last ran), shown again when the device
    /// next shows that profile. Folders are not reopened. A device on the
    /// home page has no entry: that is where it starts anyway.
    remembered: BTreeMap<(ProfileId, DeviceId), PageId>,
    /// Incompatible bindings refused since the profile or the engine was
    /// last set ([`RuntimeOutcome::Unavailable`]), so each is reported
    /// once.
    refused: HashSet<(LayoutId, PageId, ControlId, Trigger)>,
    /// The layout each device shows in a profile, where *Switch layout* or
    /// *Back to previous layout* chose one (see [`Runtime::layout_of`]).
    /// Saved by the agent ([`Runtime::device_layouts`]) and restored after
    /// a restart; entries for layouts that are gone are ignored.
    device_layouts: BTreeMap<(ProfileId, DeviceId), LayoutId>,
    /// The way back of each device in each profile: in memory only, empty
    /// after a restart.
    histories: BTreeMap<(ProfileId, DeviceId), crate::LayoutHistory>,
    /// [`Runtime::device_layouts`] changed since it was last taken.
    device_layouts_changed: bool,
    /// The folders each device is in and their Auto Exit countdowns: in
    /// memory only, never saved.
    folders: FolderTimers,
    /// The editor's unsaved working layout a device shows and runs
    /// instead of the saved one, while a live preview session lasts
    /// (`working.rs`): in memory only, never saved.
    working: BTreeMap<DeviceId, working::WorkingLayout>,
    clock: Clock,
}

/// Most pages kept in [`Runtime::remembered_pages`]: profiles times
/// devices, far beyond real use. Beyond it nothing more is remembered.
const MAX_REMEMBERED_PAGES: usize = 512;

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
            remembered: BTreeMap::new(),
            refused: HashSet::new(),
            device_layouts: BTreeMap::new(),
            histories: BTreeMap::new(),
            device_layouts_changed: false,
            folders: FolderTimers::default(),
            working: BTreeMap::new(),
            clock: Clock::default(),
        }
    }

    /// Measures Auto Exit countdowns with `clock` instead of
    /// [`Instant::now`] (tests).
    pub fn set_clock(&mut self, clock: impl Fn() -> Instant + Send + Sync + 'static) {
        self.clock = Clock(Arc::new(clock));
    }

    /// The folders devices are in and their Auto Exit countdowns.
    pub fn folder_timers(&self) -> &FolderTimers {
        &self.folders
    }

    /// When the next Auto Exit is due.
    pub fn next_folder_exit(&self) -> Option<Instant> {
        self.folders.next_due()
    }

    /// The Auto Exits due at `now`, to pass to [`Runtime::exit_folder`].
    pub fn due_folder_exits(&self, now: Instant) -> Vec<FolderExit> {
        self.folders.due(now)
    }

    /// Leaves the folder of a due Auto Exit: the device shows the folder's
    /// parent, exactly as Back from the folder page goes (one folder
    /// level; folders opened inside it close with it). Nothing happens,
    /// and `None` is returned, when the device left the folder or entered
    /// it again since (`exit`'s entry is no longer the current one), is
    /// disconnected, or shows another layout.
    pub fn exit_folder(&mut self, exit: &FolderExit) -> Option<RuntimeOutcome> {
        if !self.devices.is_connected(&exit.device) || !self.folders.is_current(exit) {
            return None;
        }
        let layout = self.layout_of(&exit.device)?;
        if layout.id != exit.layout {
            return None;
        }
        let path = page_path(layout, Some(exit.page))?;
        let stack = self.navigation.entry(exit.device.clone()).or_default();
        match stack.iter().rposition(|page| *page == exit.page) {
            Some(index) => stack.truncate(index + 1),
            None => *stack = path,
        }
        let page = self.go_back(&exit.device)?;
        self.follow_folders(&exit.device, false);
        Some(RuntimeOutcome::Navigated {
            device: exit.device.clone(),
            page,
        })
    }

    /// Follows the folders `device` shows ([`FolderTimers`]): entering one
    /// starts its countdown, unless `quiet`.
    fn follow_folders(&mut self, device: &DeviceId, quiet: bool) {
        let now = (self.clock.0)();
        let mut folders = std::mem::take(&mut self.folders);
        let shown = self
            .layout_of(device)
            .zip(self.current_page(device))
            .filter(|(layout, page)| layout.page(*page).is_some());
        folders.follow(device, shown, now, quiet);
        self.folders = folders;
    }

    /// The protected system Back of the page `device` shows, if it has one
    /// ([`DeviceLayout::system_back`]).
    pub fn system_back(&self, device: &DeviceId) -> Option<ControlId> {
        let model = self.catalog.get(&device.model)?;
        self.layout_of(device)?
            .system_back(self.current_page(device)?, &model.capabilities)
    }

    /// The folder `control` opens when it is a folder control
    /// ([`crate::folder_control`]) of the page `device` shows: a raw press
    /// opens it at once, and nothing else stored on the key ever runs.
    pub fn folder_control(&self, device: &DeviceId, control: ControlId) -> Option<PageId> {
        self.layout_of(device)?
            .folder_control(self.current_page(device)?, control)
    }

    /// The top-level page each device shows, or showed last, per profile,
    /// where that is not the home page: what [`Runtime::restore_pages`]
    /// takes after a restart. Empty while every device is on its home page,
    /// so nothing needs to be saved for a deck that never left it.
    pub fn remembered_pages(&self) -> Vec<(ProfileId, DeviceId, PageId)> {
        let mut pages = self.remembered.clone();
        if let Some(profile) = self.profile() {
            for (device, stack) in self.navigation.iter().chain(&self.parked) {
                let key = (profile.id, device.clone());
                match self.top_level_shown(device, stack) {
                    Some(page) => pages.insert(key, page),
                    None => pages.remove(&key),
                };
            }
        }
        pages
            .into_iter()
            .map(|((profile, device), page)| (profile, device, page))
            .collect()
    }

    /// Makes devices start on the pages of an earlier run. A page that no
    /// longer exists, or is inside a folder now, is ignored when its
    /// device connects.
    pub fn restore_pages(
        &mut self,
        pages: impl IntoIterator<Item = (ProfileId, DeviceId, PageId)>,
    ) {
        for (profile, device, page) in pages.into_iter().take(MAX_REMEMBERED_PAGES) {
            self.remembered.insert((profile, device), page);
        }
    }

    /// The top-level page of the page on top of `stack`, in the layout
    /// for `model`; `None` for the home page, which needs no remembering.
    fn top_level_shown(&self, device: &DeviceId, stack: &[PageId]) -> Option<PageId> {
        // Saved pages only: a page that exists only in an unsaved working
        // layout is never remembered.
        let layout = self.saved_layout_of(device)?;
        let page = *stack.last()?;
        layout.page(page)?;
        Some(layout.top_level_of(page)).filter(|top| *top != layout.home_page)
    }

    /// Notes the page every device shows in the active profile, before the
    /// profile is left.
    fn remember_pages(&mut self) {
        let Some(profile) = self.profile().map(|profile| profile.id) else {
            return;
        };
        let shown: Vec<(DeviceId, Option<PageId>)> = self
            .navigation
            .iter()
            .chain(&self.parked)
            .map(|(device, stack)| (device.clone(), self.top_level_shown(device, stack)))
            .collect();
        for (device, page) in shown {
            let key = (profile, device);
            match page {
                Some(page)
                    if self.remembered.len() < MAX_REMEMBERED_PAGES
                        || self.remembered.contains_key(&key) =>
                {
                    self.remembered.insert(key, page);
                }
                _ => {
                    self.remembered.remove(&key);
                }
            }
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
        if self.system_back(device) == Some(control)
            || self.folder_control(device, control).is_some()
        {
            // The reserved Back and folder keys react to the press at once.
            return None;
        }
        let layout = self.layout_of(device)?;
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
        self.dispatch_gesture_as(device, control, gesture, crate::RunOrigin::Gesture)
    }

    /// [`Runtime::dispatch_gesture`], with the binding run as started by
    /// `origin`.
    pub fn dispatch_gesture_as(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        gesture: Gesture,
        origin: crate::RunOrigin,
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
        if self.system_back(device) == Some(control) {
            return ignored("the folder's Back key reacts to a press");
        }
        if self.folder_control(device, control).is_some() {
            return ignored("a folder key reacts to a press");
        }
        let target = self
            .layout_of(device)
            .zip(self.current_page(device))
            .and_then(|(layout, page)| layout.page(page))
            .and_then(|page| page.gesture_binding(control, gesture))
            .map(|binding| binding.target.clone());
        if let Some(refused) = self.refuse_incompatible(device, control, Trigger::from(gesture)) {
            return Ok(refused);
        }
        match target {
            None => ignored("gesture is not bound"),
            Some(BindingTarget::Stateful(binding)) => {
                Ok(RuntimeOutcome::StatefulRequested { binding, gesture })
            }
            Some(target) => self.dispatch(device, target, origin),
        }
    }

    /// The stateful binding of `control` on the page `device` shows, if the
    /// control has one; its state is captured when a press starts a
    /// gesture.
    pub fn stateful_binding(
        &self,
        device: &DeviceId,
        control: ControlId,
    ) -> Option<&StatefulBinding> {
        if self.system_back(device) == Some(control)
            || self.folder_control(device, control).is_some()
        {
            return None;
        }
        let layout = self.layout_of(device)?;
        let page = layout.page(self.current_page(device)?)?;
        page.bindings
            .iter()
            .find_map(|binding| match &binding.target {
                BindingTarget::Stateful(stateful) if binding.control == control => Some(stateful),
                _ => None,
            })
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
        self.refused.clear();
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
    /// shows the top-level page it showed in that profile before (the home
    /// page of its layout the first time); folders are closed. No action is
    /// executed.
    pub fn set_profile(&mut self, profile: Option<Profile>, project: Option<ProjectId>) {
        self.remember_pages();
        self.parked.clear();
        self.refused.clear();
        self.session = profile.map(|profile| Session {
            projects: ActiveProjectContext::restore(&profile, project),
            profile,
        });
        self.prune_layouts();
        // Another profile (or none, when locked): every countdown ends.
        self.folders.clear();
        let connected: Vec<DeviceId> = self.devices.connected().cloned().collect();
        for device in connected {
            self.reset_navigation(&device);
            self.follow_folders(&device, true);
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
        self.refused.clear();
        self.session = Some(Session {
            projects: ActiveProjectContext::restore(&profile, project),
            profile,
        });
        self.prune_layouts();
        let devices: Vec<DeviceId> = self.navigation.keys().cloned().collect();
        for device in devices {
            // A device with no pages (connected before its model had a
            // layout) is not valid once a layout exists: it goes home.
            let stack = &self.navigation[&device];
            let valid = match self.layout_of(&device) {
                Some(_) => self.pages_exist(&device, stack),
                None => stack.is_empty(),
            };
            if !valid {
                // The nearest page that is still there, else the start.
                self.repair_navigation(&device);
            }
            // A device still in its folder keeps its countdown (cancelled
            // if Auto Exit was switched off); a reload enters nothing.
            self.follow_folders(&device, true);
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
        // Run from the app (`runAction`).
        let result = self.engine.run_action_from(
            &session.profile,
            &self.machine,
            &mut session.projects,
            action,
            &crate::resolution::Invocation::default().with_origin(crate::RunOrigin::App),
        )?;
        // No device asked: its pages cannot be meant.
        if result
            .deck_request()
            .is_some_and(crate::DeckRequest::needs_device)
        {
            return Ok(ActionResult::error(
                NO_DEVICE,
                "this action changes the page of the device whose key runs it; run it from a key",
            ));
        }
        Ok(result)
    }

    /// Whether the active profile's `action` runs detached
    /// ([`crate::ActionHandler::runs_detached`]): it must be run with
    /// [`Runtime::detached_action`] on a thread of its own, not with
    /// [`Runtime::run_action`].
    pub fn runs_detached(&self, action: ActionInstanceId) -> bool {
        self.profile()
            .and_then(|profile| profile.action(action))
            .is_some_and(|instance| self.engine.registry().runs_detached(&instance.action_type))
    }

    /// Everything needed to run `action` of the active profile on another
    /// thread: the engine, and copies of the profile, the machine and the
    /// project context as they are now. `overrides` and `device` are a
    /// key binding's, as for a press; `origin` says what started it (a
    /// key, a gesture, the app).
    pub fn detached_action(
        &self,
        action: ActionInstanceId,
        overrides: Option<ActionParameters>,
        device: Option<DeviceId>,
        origin: crate::RunOrigin,
    ) -> Result<DetachedAction, EngineError> {
        let session = self.session.as_ref().ok_or(EngineError::NoActiveProfile)?;
        if session.profile.action(action).is_none() {
            return Err(EngineError::UnknownAction(action));
        }
        Ok(DetachedAction {
            engine: self.engine.clone(),
            profile: session.profile.clone(),
            machine: self.machine.clone(),
            projects: session.projects.clone(),
            action,
            overrides: overrides.filter(|o| !o.is_empty()),
            invocation: device
                .map_or_else(
                    crate::resolution::Invocation::default,
                    crate::resolution::Invocation::from_device,
                )
                .with_origin(origin),
        })
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

    /// The layout the active profile uses for devices of `model` that did
    /// not switch to another one (see [`Profile::layout_for`]).
    pub fn layout_for(&self, model: &DeviceModelId) -> Option<&DeviceLayout> {
        self.profile()?.layout_for(model)
    }

    /// The layout `device` shows: the one it switched to in the active
    /// profile, while that is still assigned to the profile and made for
    /// the device's model; otherwise the profile's layout for the model.
    /// Resolution: device → profile → that profile's layout → page →
    /// binding.
    ///
    /// While a live preview session gives the device a working layout for
    /// that layout ([`Runtime::set_working_layout`]), the working layout:
    /// what it shows and what its keys run.
    pub fn layout_of(&self, device: &DeviceId) -> Option<&DeviceLayout> {
        let saved = self.saved_layout_of(device)?;
        Some(self.working_for(device, saved.id).unwrap_or(saved))
    }

    /// The saved layout `device` shows, whatever a live preview shows on
    /// it.
    pub fn saved_layout_of(&self, device: &DeviceId) -> Option<&DeviceLayout> {
        let profile = self.profile()?;
        self.device_layouts
            .get(&(profile.id, device.clone()))
            .and_then(|id| profile.layout(*id))
            .filter(|layout| layout.model == device.model)
            .or_else(|| profile.layout_for(&device.model))
    }

    /// The layouts devices switched to, per profile, for saving.
    pub fn device_layouts(&self) -> Vec<(ProfileId, DeviceId, LayoutId)> {
        self.device_layouts
            .iter()
            .map(|((profile, device), layout)| (*profile, device.clone(), *layout))
            .collect()
    }

    /// Whether [`Runtime::device_layouts`] changed since the last call.
    pub fn take_device_layouts_changed(&mut self) -> bool {
        std::mem::take(&mut self.device_layouts_changed)
    }

    /// Replaces the layouts devices switched to with saved ones (after a
    /// start or a reload: the database is the truth). Devices whose layout
    /// changed show its home page. Entries the active profile does not
    /// have, or for another model, are dropped.
    pub fn restore_device_layouts(
        &mut self,
        layouts: impl IntoIterator<Item = (ProfileId, DeviceId, LayoutId)>,
    ) {
        let before: Vec<(DeviceId, Option<LayoutId>)> = self
            .navigation
            .keys()
            .map(|device| (device.clone(), self.layout_of(device).map(|l| l.id)))
            .collect();
        self.device_layouts = layouts
            .into_iter()
            .take(MAX_REMEMBERED_PAGES)
            .filter(|(_, device, _)| self.catalog.get(&device.model).is_some())
            .map(|(profile, device, layout)| ((profile, device), layout))
            .collect();
        self.prune_layouts();
        for (device, layout) in before {
            if self.layout_of(&device).map(|l| l.id) != layout {
                self.reset_navigation(&device);
            }
            self.follow_folders(&device, true);
        }
    }

    /// Forgets switched-to layouts and history entries of the active
    /// profile that are no longer assigned to it or not made for the
    /// device's model, so nothing refers to a layout that is gone.
    fn prune_layouts(&mut self) {
        let Some(profile) = self.session.as_ref().map(|s| &s.profile) else {
            return;
        };
        let usable = |device: &DeviceId, id: LayoutId| {
            profile
                .layout(id)
                .is_some_and(|layout| layout.model == device.model)
        };
        let before = self.device_layouts.len();
        self.device_layouts
            .retain(|(owner, device), id| *owner != profile.id || usable(device, *id));
        if self.device_layouts.len() != before {
            self.device_layouts_changed = true;
        }
        for ((owner, device), history) in &mut self.histories {
            if *owner == profile.id {
                history.retain(|id| usable(device, id));
            }
        }
    }

    /// The layouts `device` can go back to in the active profile, oldest
    /// first.
    pub fn layout_history(&self, device: &DeviceId) -> Vec<LayoutId> {
        self.profile()
            .and_then(|profile| self.histories.get(&(profile.id, device.clone())))
            .map(crate::LayoutHistory::layouts)
            .unwrap_or_default()
    }

    /// The status of the layout `device` shows (see
    /// [`Runtime::layout_status`]).
    pub fn device_layout_status(&self, device: &DeviceId) -> LayoutStatus {
        let Some(profile) = self.profile() else {
            return LayoutStatus::NoProfile;
        };
        if self.catalog.get(&device.model).is_none() {
            return LayoutStatus::UnknownModel;
        }
        match self.layout_of(device) {
            Some(layout) => self.status_of(layout, profile),
            None => LayoutStatus::Missing,
        }
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
        self.status_of(layout, profile)
    }

    fn status_of(&self, layout: &DeviceLayout, profile: &Profile) -> LayoutStatus {
        let mut issues = layout.validate(&self.catalog, profile);
        issues.extend(layout.check_overrides(self.engine.registry(), profile));
        issues.extend(layout.check_compatibility(&self.catalog, self.engine.registry(), profile));
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
        self.layout_of(device).is_some_and(|layout| {
            !stack.is_empty() && stack.iter().all(|page| layout.page(*page).is_some())
        })
    }

    /// Sends `device` to the page it showed last in the active profile, if
    /// that is still a top-level page of its layout, else to the home page.
    fn reset_navigation(&mut self, device: &DeviceId) {
        let start = self.layout_of(device).map(|layout| {
            self.profile()
                .and_then(|profile| self.remembered.get(&(profile.id, device.clone())))
                .copied()
                .filter(|page| layout.top_level_pages().contains(page))
                .unwrap_or(layout.home_page)
        });
        self.navigation
            .insert(device.clone(), start.into_iter().collect());
    }

    /// Back on `device`: to the folder of the page it shows; for a page
    /// outside folders, to the page that opened it. `None`: there is no
    /// page to return to.
    fn go_back(&mut self, device: &DeviceId) -> Option<PageId> {
        let current = self.current_page(device);
        let folder = current.and_then(|page| {
            let layout = self.layout_of(device)?;
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
        stack.last().copied()
    }

    /// Whether the page `device` shows is a folder that closes after one of
    /// its keys ran something ([`crate::LayoutPage::auto_return`]).
    fn closes_after_use(&self, device: &DeviceId) -> bool {
        self.layout_of(device)
            .zip(self.current_page(device))
            .is_some_and(|(layout, page)| {
                layout.page(page).is_some_and(|p| p.auto_return) && layout.parent_of(page).is_some()
            })
    }

    /// Applies the page request of an action's `result` to `device` and
    /// takes it out; a request for the agent (a profile switch) stays in
    /// the result for the caller.
    fn apply_page_request(&mut self, device: &DeviceId, result: &mut ActionResult) {
        let (forward, wrap_around) = match result.deck_request() {
            Some(crate::DeckRequest::NextPage { wrap_around }) => (true, wrap_around),
            Some(crate::DeckRequest::PreviousPage { wrap_around }) => (false, wrap_around),
            Some(crate::DeckRequest::SwitchLayout {
                layout,
                page,
                history,
            }) => {
                result.request = None;
                self.switch_layout(device, layout, page, history, result);
                return;
            }
            Some(crate::DeckRequest::PreviousLayout) => {
                result.request = None;
                self.previous_layout(device, result);
                return;
            }
            Some(
                crate::DeckRequest::SwitchProfile { .. }
                | crate::DeckRequest::Brightness { .. }
                | crate::DeckRequest::RunAction { .. }
                | crate::DeckRequest::RunWorkflow { .. }
                | crate::DeckRequest::RunSwitch { .. },
            )
            | None => return,
        };
        result.request = None;
        let target = self
            .layout_of(device)
            .zip(self.current_page(device))
            .and_then(|(layout, page)| {
                let target = layout.adjacent_page(page, forward, wrap_around)?;
                Some((target, layout.page(target)?.name.clone()))
            });
        match target {
            Some((page, name)) => {
                self.navigation.insert(device.clone(), vec![page]);
                result.message = format!("page {name}");
                result.data.insert("pageId".into(), page.to_string());
                result.data.insert("pageName".into(), name);
            }
            None => {
                result.message = if forward {
                    "there is no next page".into()
                } else {
                    "there is no previous page".into()
                };
            }
        }
    }

    /// *Switch layout* on `device`: shows `target` of the active profile,
    /// at `page` (the home page when not given or gone), and records the
    /// layout left in the device's history per `mode`. Refused, changing
    /// nothing, when the layout is not assigned to the profile or not made
    /// for the device's model; the profile never changes.
    fn switch_layout(
        &mut self,
        device: &DeviceId,
        target: LayoutId,
        page: Option<PageId>,
        mode: crate::HistoryMode,
        result: &mut ActionResult,
    ) {
        let Some(profile) = self.profile() else {
            *result = ActionResult::error(LAYOUT_UNAVAILABLE, "no profile is active");
            return;
        };
        let profile_id = profile.id;
        let Some(layout) = self.layout_by_id(device, target) else {
            *result = ActionResult::error(
                LAYOUT_UNAVAILABLE,
                format!("the layout is not assigned to the profile {}", profile.name),
            );
            return;
        };
        if layout.model != device.model {
            let model = self
                .catalog
                .get(&layout.model)
                .map_or_else(|| layout.model.to_string(), |m| m.name.clone());
            *result = ActionResult::error(
                LAYOUT_INCOMPATIBLE,
                format!(
                    "the layout {} is made for the {model}, not for this device",
                    layout.name
                ),
            );
            return;
        }
        let name = layout.name.clone();
        let stack = page_path(layout, page).unwrap_or_else(|| vec![layout.home_page]);
        let current = self.layout_of(device).map(|layout| layout.id).zip(Some(
            self.navigation.get(device).cloned().unwrap_or_default(),
        ));
        let key = (profile_id, device.clone());
        let switched = self
            .histories
            .entry(key.clone())
            .or_default()
            .switch(current, target, mode);
        if switched || page.is_some() {
            self.navigation.insert(device.clone(), stack);
        }
        if self.device_layouts.get(&key) != Some(&target) {
            self.device_layouts.insert(key, target);
            self.device_layouts_changed = true;
        }
        result.message = if switched {
            format!("layout {name}")
        } else {
            format!("layout {name} is shown")
        };
        result.data.insert("layoutId".into(), target.to_string());
        result.data.insert("layoutName".into(), name);
    }

    /// *Back to previous layout* on `device`: the newest layout of its
    /// history that is still usable, at the page it showed (the home page
    /// if that is gone). With an empty history nothing changes and the
    /// result says so.
    fn previous_layout(&mut self, device: &DeviceId, result: &mut ActionResult) {
        let Some(profile) = self.session.as_ref().map(|s| &s.profile) else {
            *result = ActionResult::error(LAYOUT_UNAVAILABLE, "no profile is active");
            return;
        };
        let key = (profile.id, device.clone());
        let entry = self.histories.get_mut(&key).and_then(|history| {
            history.back(|id| {
                profile
                    .layout(id)
                    .is_some_and(|layout| layout.model == device.model)
            })
        });
        let Some(entry) = entry else {
            "there is no previous layout".clone_into(&mut result.message);
            return;
        };
        let Some(layout) = self
            .working
            .get(device)
            .filter(|w| w.profile == profile.id && w.layout.id == entry.layout)
            .map(|w| &w.layout)
            .or_else(|| profile.layout(entry.layout))
        else {
            return;
        };
        let name = layout.name.clone();
        let stack = if !entry.pages.is_empty()
            && entry.pages.iter().all(|page| layout.page(*page).is_some())
        {
            entry.pages
        } else {
            vec![layout.home_page]
        };
        self.navigation.insert(device.clone(), stack);
        if self.device_layouts.get(&key) != Some(&entry.layout) {
            self.device_layouts.insert(key, entry.layout);
            self.device_layouts_changed = true;
        }
        result.message = format!("layout {name}");
        result
            .data
            .insert("layoutId".into(), entry.layout.to_string());
        result.data.insert("layoutName".into(), name);
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

        let notice = |text: Result<&str, SystemText>| {
            controls
                .iter()
                .enumerate()
                .map(|(index, control)| {
                    let face = if index == 0 {
                        match text {
                            Ok(text) => KeyFace::new(FaceStyle::Notice, text),
                            Err(system) => KeyFace::system(FaceStyle::Notice, system),
                        }
                    } else {
                        KeyFace::empty()
                    };
                    (*control, face)
                })
                .collect()
        };

        let Some(profile) = self.profile() else {
            return notice(self.placeholder.as_deref().ok_or(SystemText::NoProfile));
        };
        let (Some(layout), Some(page)) = (self.layout_of(device), self.current_page(device)) else {
            return notice(Err(SystemText::NoLayout));
        };
        let Some(page) = layout.page(page) else {
            return notice(Err(SystemText::NoLayout));
        };
        let mut faces = page_faces(
            self.engine.registry(),
            layout,
            page.id,
            profile,
            self.active_project(),
            &controls,
            Some(&model.capabilities),
        );
        // *Back to previous layout* with nowhere to go back to.
        let system_back = self.system_back(device);
        if self.layout_history(device).is_empty() {
            for (control, face) in faces
                .iter_mut()
                .filter(|(control, _)| Some(*control) != system_back)
            {
                let back = shown_binding(page, *control)
                    .and_then(|binding| binding.target.action_id())
                    .and_then(|action| profile.action(action))
                    .and_then(|instance| {
                        self.engine
                            .registry()
                            .layout_navigation(&instance.action_type, &instance.parameters)
                    })
                    == Some(crate::LayoutNavigation::Back);
                if back && face.style != FaceStyle::Notice {
                    face.style = FaceStyle::Notice;
                    *face = std::mem::replace(face, KeyFace::empty())
                        .with_system_detail(SystemText::NoPreviousLayout);
                }
            }
        }
        faces
    }

    /// The bindings of the active profile's layouts that cannot run their
    /// action on their model's controls, by model. Computed from the
    /// loaded action types whenever it is asked, so a binding is
    /// available again as soon as what it needs is back. Layouts of
    /// unknown models are left out.
    pub fn unavailable_bindings(&self) -> Vec<(DeviceModelId, crate::IncompatibleBinding)> {
        let Some(profile) = self.profile() else {
            return Vec::new();
        };
        profile
            .layouts
            .iter()
            .filter_map(|layout| Some((layout, self.catalog.get(&layout.model)?)))
            .flat_map(|(layout, model)| {
                layout
                    .incompatible_bindings(&model.capabilities, self.engine.registry(), profile)
                    .into_iter()
                    .map(|found| (layout.model.clone(), found))
            })
            .collect()
    }

    /// [`RuntimeOutcome::Unavailable`] when the binding of `control` for
    /// `trigger` on the page `device` shows runs an action that cannot
    /// run from it; `None` otherwise.
    fn refuse_incompatible(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        trigger: Trigger,
    ) -> Option<RuntimeOutcome> {
        let profile = self.profile()?;
        let layout = self.layout_of(device)?;
        let page = layout.page(self.current_page(device)?)?;
        let binding = page
            .bindings
            .iter()
            .find(|binding| binding.control == control && binding.trigger == trigger)?;
        let model = self.catalog.get(&device.model)?;
        let reason = crate::binding_incompatibility(
            binding,
            &model.capabilities,
            self.engine.registry(),
            profile,
        )?;
        let action = binding.target.action_id()?;
        let key = (layout.id, page.id, control, trigger);
        let first = self.refused.insert(key);
        Some(RuntimeOutcome::Unavailable {
            action,
            control,
            trigger,
            reason,
            first,
        })
    }

    /// The binding target behind each face of [`Runtime::faces`] that has
    /// one, for decorating faces with live state. Empty without a profile
    /// or layout.
    pub fn shown_targets(&self, device: &DeviceId) -> Vec<(ControlId, BindingTarget)> {
        let (Some(layout), Some(page)) = (self.layout_of(device), self.current_page(device)) else {
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
            // The reserved Back key shows Back, whatever is bound to it.
            .filter(|control| layout.system_back(page.id, &model.capabilities) != Some(*control))
            .filter_map(|control| shown_binding(page, control).map(|binding| (control, binding)))
            // A binding that cannot run its action shows as unavailable,
            // without live state.
            .filter(|(_, binding)| {
                self.profile().is_none_or(|profile| {
                    crate::binding_incompatibility(
                        binding,
                        &model.capabilities,
                        self.engine.registry(),
                        profile,
                    )
                    .is_none()
                })
            })
            .map(|(control, binding)| (control, binding.target.clone()))
            .collect()
    }

    pub fn handle_event(&mut self, event: &DeviceEvent) -> Result<RuntimeOutcome, EngineError> {
        self.handle_event_as(event, crate::RunOrigin::Key)
    }

    /// [`Runtime::handle_event`], with an input's binding run as started
    /// by `origin` (a simulated press runs as
    /// [`crate::RunOrigin::Simulated`]).
    pub fn handle_event_as(
        &mut self,
        event: &DeviceEvent,
        origin: crate::RunOrigin,
    ) -> Result<RuntimeOutcome, EngineError> {
        let ignored = |reason: &str| {
            Ok(RuntimeOutcome::Ignored {
                reason: reason.into(),
            })
        };
        match event {
            DeviceEvent::Connected { device } => {
                self.devices.apply(event);
                match self.parked.remove(device) {
                    Some(stack) => {
                        self.navigation.insert(device.clone(), stack);
                        self.repair_navigation(device);
                    }
                    None => self.reset_navigation(device),
                }
                // A device that comes back starts no countdown: only
                // entering a folder does.
                self.follow_folders(device, true);
                Ok(RuntimeOutcome::DeviceConnected {
                    device: device.clone(),
                })
            }
            DeviceEvent::Disconnected { device } => {
                self.devices.apply(event);
                if let Some(stack) = self.navigation.remove(device) {
                    self.parked.insert(device.clone(), stack);
                }
                self.folders.forget(device);
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
                if let Some(outcome) = self.core_control(device, *control, *input, origin) {
                    return outcome;
                }
                // A binding whose action cannot run from this control is
                // refused, not run.
                let trigger = self
                    .layout_of(device)
                    .zip(self.current_page(device))
                    .and_then(|(layout, page)| layout.page(page))
                    .and_then(|page| page.binding_for(*control, *input))
                    .map(|binding| binding.trigger);
                if let Some(refused) =
                    trigger.and_then(|trigger| self.refuse_incompatible(device, *control, trigger))
                {
                    return Ok(refused);
                }
                let Some(model) = self.catalog.get(&device.model) else {
                    return ignored("unknown device model");
                };
                let (Some(layout), Some(page_id)) =
                    (self.layout_of(device), self.current_page(device))
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
                    Some(target) => self.dispatch(device, target, origin),
                }
            }
        }
    }

    /// The input of a control DeckPilot owns on the page `device` shows,
    /// whatever is bound to it; `None` for every other control.
    ///
    /// - The folder's reserved Back key goes back on the press (a layout
    ///   that binds it is refused when saved).
    /// - A folder key ([`crate::folder_control`]) opens its folder on the
    ///   press; what else is stored on it (recovered, or not yet migrated)
    ///   never runs.
    fn core_control(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        input: deckpilot_device::ControlInput,
        origin: crate::RunOrigin,
    ) -> Option<Result<RuntimeOutcome, EngineError>> {
        let (target, refusal) = if self.system_back(device) == Some(control) {
            (
                BindingTarget::Back,
                "the folder's Back key reacts to a press",
            )
        } else {
            let folder = self.folder_control(device, control)?;
            (
                BindingTarget::OpenPage { page: folder },
                "a folder key reacts to a press",
            )
        };
        Some(match input {
            deckpilot_device::ControlInput::Press => self.dispatch(device, target, origin),
            _ => Ok(RuntimeOutcome::Ignored {
                reason: refusal.into(),
            }),
        })
    }

    fn dispatch(
        &mut self,
        device: &DeviceId,
        target: BindingTarget,
        origin: crate::RunOrigin,
    ) -> Result<RuntimeOutcome, EngineError> {
        // A folder that closes by itself returns once one of its keys ran
        // an action or started a workflow, whatever came of it.
        let closes = self.closes_after_use(device);
        let shown = self.current_page(device);
        let outcome = self.dispatch_target(device, target, origin)?;
        if closes
            && self.current_page(device) == shown
            && matches!(
                outcome,
                RuntimeOutcome::Action { .. }
                    | RuntimeOutcome::ActionRequested { .. }
                    | RuntimeOutcome::WorkflowRequested { .. }
            )
        {
            self.go_back(device);
        }
        self.follow_folders(device, false);
        Ok(outcome)
    }

    /// Runs `target` for `device` as a key bound to it would, without
    /// closing a folder after it: what a Random Action picked. The pick ran
    /// from a key of the page, which already closed it if it closes.
    ///
    /// # Errors
    ///
    /// As a key's binding: no active profile, an unknown action or
    /// workflow.
    pub fn run_picked(
        &mut self,
        device: &DeviceId,
        target: BindingTarget,
        origin: crate::RunOrigin,
    ) -> Result<RuntimeOutcome, EngineError> {
        let outcome = self.dispatch_target(device, target, origin);
        self.follow_folders(device, false);
        outcome
    }

    fn dispatch_target(
        &mut self,
        device: &DeviceId,
        target: BindingTarget,
        origin: crate::RunOrigin,
    ) -> Result<RuntimeOutcome, EngineError> {
        match target {
            BindingTarget::Action { action, overrides } => {
                if self.runs_detached(action) {
                    return Ok(RuntimeOutcome::ActionRequested {
                        action,
                        overrides,
                        device: device.clone(),
                    });
                }
                // The key's device is part of the context its references
                // are resolved in (`system.device.current.id`). The
                // binding's overrides are resolved in the same pipeline.
                let session = self.session.as_mut().ok_or(EngineError::NoActiveProfile)?;
                let mut result = self.engine.run_action_with(
                    &session.profile,
                    &self.machine,
                    &mut session.projects,
                    action,
                    Some(&overrides),
                    &crate::resolution::Invocation::from_device(device.clone()).with_origin(origin),
                )?;
                self.apply_page_request(device, &mut result);
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
            // Back returns to the page's folder; for a page outside
            // folders, to the page that opened it.
            BindingTarget::Back => match self.go_back(device) {
                Some(page) => Ok(RuntimeOutcome::Navigated {
                    device: device.clone(),
                    page,
                }),
                None => Ok(RuntimeOutcome::Ignored {
                    reason: "no page to return to".into(),
                }),
            },
            // Refused when a layout is saved: the state at the press
            // belongs to a gesture.
            BindingTarget::Stateful(_) => Ok(RuntimeOutcome::Ignored {
                reason: "a stateful key reacts to gestures only".into(),
            }),
        }
    }
}

/// The page stack that shows `page` of `layout`: its folders, then the
/// page. `None` when no page is given or it is not in the layout.
fn page_path(layout: &DeviceLayout, page: Option<PageId>) -> Option<Vec<PageId>> {
    let page = page.filter(|page| layout.page(*page).is_some())?;
    let mut path = layout.ancestors(page);
    path.reverse();
    path.push(page);
    Some(path)
}

/// Faces for `controls` on one page of a layout. Unbound controls are empty.
/// Used for devices and for previews in the layout designer. With the
/// model's `capabilities`, a control whose shown binding runs an action
/// that cannot run from it shows as unavailable, as an action whose type
/// cannot run here does.
pub fn page_faces(
    registry: &ActionRegistry,
    layout: &DeviceLayout,
    page: PageId,
    profile: &Profile,
    project: Option<&Project>,
    controls: &[ControlId],
    capabilities: Option<&DeviceCapabilities>,
) -> Vec<(ControlId, KeyFace)> {
    let Some(page) = layout.page(page) else {
        return controls
            .iter()
            .map(|control| (*control, KeyFace::empty()))
            .collect();
    };
    let automatic_back = layout.automatic_back(page.id, controls);
    // The folder's reserved Back key shows Back, whatever is bound to it.
    let system_back = match capabilities {
        Some(capabilities) => layout.system_back(page.id, capabilities),
        None => (layout.back_position(page.id) == Some(crate::BackPosition::Reserved))
            .then(|| crate::editor_navigation::reserved_back_among(controls))
            .flatten(),
    };
    let (index, count) = layout.page_position(page.id).unwrap_or((0, 1));
    let label = PageLabel {
        number: index + 1,
        count,
        name: &page.name,
    };
    controls
        .iter()
        .map(|control| {
            let binding = shown_binding(page, *control).filter(|_| system_back != Some(*control));
            let mut face = if system_back == Some(*control) {
                system_back_face()
            } else {
                binding.map_or_else(
                    || {
                        if automatic_back == Some(*control) {
                            KeyFace::system(FaceStyle::Back, SystemText::Back)
                                .with_symbol(Some(symbols::BACK))
                        } else {
                            KeyFace::empty()
                        }
                    },
                    |binding| {
                        let incompatible = capabilities.is_some_and(|capabilities| {
                            crate::binding_incompatibility(binding, capabilities, registry, profile)
                                .is_some()
                        });
                        if incompatible {
                            unavailable_face(binding, profile, project)
                        } else {
                            face_for(registry, binding, layout, profile, project)
                        }
                        .with_appearance(binding.appearance.as_ref())
                        .with_title_suggestions(title_suggestions(registry, binding, profile))
                        .with_user_title(user_titled(binding, profile))
                    },
                )
            };
            face.title = render_page_label(&face.title, label);
            face.detail = face.detail.map(|detail| render_page_label(&detail, label));
            (*control, face)
        })
        .collect()
}

/// A plugin's suggested title for `instance`, while the action keeps its
/// type's name (a name the user changed wins).
fn suggested_title<'a>(
    registry: &'a ActionRegistry,
    instance: &crate::ActionInstance,
) -> Option<&'a str> {
    registry
        .definition(&instance.action_type)
        .filter(|definition| definition.name == instance.name)
        .and_then(|definition| definition.title.as_ref())
        .and_then(|title| title.default.as_deref())
}

/// Whether the user titled a key that runs a plugin's action (the key's or
/// the action's label): that title is kept over the plugin's dynamic title
/// (`docs/key-text.md`, "Title precedence").
fn user_titled(binding: &ControlBinding, profile: &Profile) -> bool {
    let BindingTarget::Action { action, .. } = &binding.target else {
        return false;
    };
    profile.action(*action).is_some_and(|instance| {
        (binding.label.is_some() || instance.label.is_some())
            && crate::ModuleId::owner_of(&instance.action_type)
                .is_some_and(|owner| owner.is_plugin())
    })
}

/// The title suggestions of the action type a binding runs (a plugin's,
/// plugin API 1.8), if it has any.
fn title_suggestions<'a>(
    registry: &'a ActionRegistry,
    binding: &ControlBinding,
    profile: &Profile,
) -> Option<&'a crate::ActionTitle> {
    let BindingTarget::Action { action, .. } = &binding.target else {
        return None;
    };
    let instance = profile.action(*action)?;
    registry.definition(&instance.action_type)?.title.as_ref()
}

/// The face of a folder's protected Back key: its own icon only, no title
/// (the app names it "Back" for assistive technology).
pub fn system_back_face() -> KeyFace {
    KeyFace::new(FaceStyle::Back, String::new()).with_symbol(Some(symbols::FOLDER_BACK))
}

/// The binding a control's face shows: its press binding, else its
/// click binding, else the first.
pub fn shown_binding(page: &crate::LayoutPage, control: ControlId) -> Option<&ControlBinding> {
    page.bindings
        .iter()
        .filter(|binding| binding.control == control)
        .min_by_key(|binding| match binding.trigger {
            Trigger::Press => 0,
            Trigger::Click => 1,
            _ => 2,
        })
}

/// The face of an action binding that cannot run from its control: its
/// label, marked unavailable (the look of an action whose type cannot run
/// here).
fn unavailable_face(
    binding: &ControlBinding,
    profile: &Profile,
    project: Option<&Project>,
) -> KeyFace {
    let instance = binding
        .target
        .action_id()
        .and_then(|action| profile.action(action));
    let template = instance
        .map(|instance| {
            binding
                .label
                .as_deref()
                .or(instance.label.as_deref())
                .unwrap_or(&instance.name)
                .to_owned()
        })
        .unwrap_or_default();
    let face = KeyFace::new(
        FaceStyle::Notice,
        render_label(&template, &profile.name, project),
    )
    .with_system_detail(SystemText::Unavailable);
    match instance {
        Some(instance) if binding.label.is_none() && instance.label.is_none() => {
            face.with_action_name(instance.action_type.to_string())
        }
        _ => face,
    }
}

/// The title of a key that opens `page`: a folder's name when `page` is a
/// folder, its one title (schema 13, `normalize_folder_labels`); else the
/// key's own title, else the page's name.
fn open_page_title(
    layout: &DeviceLayout,
    binding: &ControlBinding,
    page: PageId,
) -> Option<String> {
    let name = || layout.page(page).map(|p| p.name.clone());
    if layout.page_parents().contains_key(&page) {
        return name().or_else(|| binding.label.clone());
    }
    binding.label.clone().or_else(name)
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
                return KeyFace::system(FaceStyle::Notice, SystemText::Missing);
            };
            let parameters = instance.parameters.overlay(overrides);
            // *Switch layout* without a label of its own is titled with
            // the layout it shows; a layout the profile does not use is
            // unavailable here.
            let target = match registry.layout_navigation(&instance.action_type, &parameters) {
                Some(crate::LayoutNavigation::SwitchTo(id)) => Some(profile.layout(id)),
                _ => None,
            };
            let template = binding
                .label
                .as_deref()
                .or(instance.label.as_deref())
                .or_else(|| target.flatten().map(|layout| layout.name.as_str()))
                .or_else(|| suggested_title(registry, instance))
                .unwrap_or(&instance.name);
            let label = render_label(template, &profile.name, project);
            // The instance's name, which it took from its type: a default
            // the agent may show in another language ([`crate::i18n`]).
            let default_name = binding.label.is_none()
                && instance.label.is_none()
                && target.flatten().is_none()
                && suggested_title(registry, instance).is_none();
            let named = |face: KeyFace| {
                if default_name {
                    face.with_action_name(instance.action_type.to_string())
                } else {
                    face
                }
            };
            if target == Some(None) {
                return named(
                    KeyFace::new(FaceStyle::Notice, label)
                        .with_system_detail(SystemText::NotInThisProfile),
                );
            }
            // The action's own icon, else its type's.
            let symbol = instance.icon.clone().or_else(|| {
                registry
                    .definition(&instance.action_type)
                    .and_then(|definition| definition.node.icon.clone())
            });
            // The owning module may add context such as the active project.
            named(
                registry
                    .decorate_face(
                        &instance.action_type,
                        KeyFace::new(FaceStyle::Action, label.clone()).with_symbol(symbol),
                        &parameters,
                        template,
                        project,
                    )
                    .unwrap_or_else(|| {
                        KeyFace::new(FaceStyle::Notice, label)
                            .with_system_detail(SystemText::Unavailable)
                    }),
            )
        }
        BindingTarget::Workflow { workflow } => match profile.workflow(*workflow) {
            Some(definition) => KeyFace::new(
                FaceStyle::Workflow,
                render_label(
                    binding.label.as_deref().unwrap_or(&definition.name),
                    &profile.name,
                    project,
                ),
            )
            .with_symbol(Some(symbols::WORKFLOW)),
            // A layout shared with a profile that has the workflow.
            None => match &binding.label {
                Some(label) => KeyFace::new(FaceStyle::Notice, label.clone()),
                None => KeyFace::system(FaceStyle::Notice, SystemText::Workflow),
            }
            .with_system_detail(SystemText::NotInThisProfile),
        },
        BindingTarget::OpenPage { page } => open_page_title(layout, binding, *page)
            .map_or_else(
                || KeyFace::system(FaceStyle::Folder, SystemText::Folder),
                |title| KeyFace::new(FaceStyle::Folder, title),
            )
            .with_symbol(Some(symbols::FOLDER)),
        BindingTarget::Back => binding
            .label
            .clone()
            .map_or_else(
                || KeyFace::system(FaceStyle::Back, SystemText::Back),
                |title| KeyFace::new(FaceStyle::Back, title),
            )
            .with_symbol(Some(symbols::BACK)),
        // The machine's own face (its state) is the agent's; this is the
        // face without it, e.g. in the layout designer's preview.
        BindingTarget::Stateful(stateful) => KeyFace::new(
            FaceStyle::Action,
            render_label(
                binding.label.as_deref().unwrap_or(stateful.name()),
                &profile.name,
                project,
            ),
        )
        .with_symbol(Some(match stateful {
            StatefulBinding::WorkSession { .. } => symbols::WORK_SESSION,
        })),
    }
}

/// The icon names of keys that are not actions (see [`KeyFace::symbol`]),
/// the ones the app shows for them.
pub mod symbols {
    pub const FOLDER: &str = "folder";
    pub const BACK: &str = "arrow-left";
    /// The folder's protected Back key: not a font icon but the vector
    /// `crates/deckpilot-render/assets/icons/folder-back.svg`, drawn
    /// centred and without a title (the renderer and the app read it).
    pub const FOLDER_BACK: &str = "folder-back";
    pub const WORKFLOW: &str = "list-ordered";
    pub const WORK_SESSION: &str = "timer";
}

mod working;
pub use working::{EditorTarget, NavigationError, NavigationState};

#[cfg(test)]
mod compat_tests;

#[cfg(test)]
mod working_tests;

#[cfg(test)]
mod deck_tests;

#[cfg(test)]
mod layout_tests;

#[cfg(test)]
mod folder_tests;

#[cfg(test)]
mod title_tests;

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

    /// `test.later`: [`crate::testing::Echo`], but it runs detached.
    struct Later(crate::testing::Echo);

    impl crate::ActionHandler for Later {
        fn definition(&self) -> &crate::ActionType {
            self.0.definition()
        }

        fn execute(
            &self,
            parameters: &ActionParameters,
            context: &crate::ExecutionContext<'_>,
        ) -> ActionResult {
            self.0.execute(parameters, context)
        }

        fn runs_detached(&self) -> bool {
            true
        }
    }

    #[test]
    fn detached_actions_are_requested_not_run_in_place() {
        let mut registry = registry();
        registry
            .register(Box::new(Later(crate::testing::Echo::with_id("test.later"))))
            .unwrap();
        let later_type = crate::ActionTypeId::new("test.later").unwrap();
        assert!(registry.runs_detached(&later_type));
        assert!(!registry.runs_detached(&select_type()));
        let mut f = fixture();
        let later = registry
            .create_instance(
                "Later",
                later_type,
                &ActionParameters::new().with("message", ParameterValue::Text("later".into())),
            )
            .unwrap();
        let later_id = later.id;
        f.profile.actions.push(later);
        let home = f.profile.layouts[0].home_page;
        f.profile.layouts[0]
            .page_mut(home)
            .unwrap()
            .bindings
            .push(ControlBinding {
                control: ControlId::Key { index: 3 },
                trigger: Trigger::Press,
                target: BindingTarget::action(later_id),
                label: None,
                appearance: None,
            });
        let mut runtime = Runtime::new(
            Engine::new(registry),
            ModelCatalog::builtin(),
            MachineConfig::new(Os::current()),
        );
        runtime.set_profile(Some(f.profile), None);
        runtime
            .handle_event(&DeviceEvent::Connected { device: device() })
            .unwrap();
        assert!(runtime.runs_detached(later_id));
        assert!(!runtime.runs_detached(f.next));

        let outcome = runtime.handle_event(&press(3)).unwrap();
        let RuntimeOutcome::ActionRequested {
            action,
            overrides,
            device: from,
        } = outcome
        else {
            panic!("expected a request, got {outcome:?}");
        };
        assert_eq!((action, from.clone()), (later_id, device()));
        let run = runtime
            .detached_action(action, Some(overrides), Some(from), crate::RunOrigin::Key)
            .unwrap();
        assert_eq!(run.name(), "Later");
        // Runs on another thread, from copies.
        let result = std::thread::spawn(move || run.run())
            .join()
            .unwrap()
            .unwrap();
        assert!(result.is_success(), "{result:?}");
        assert_eq!(result.message, "later");
        assert!(matches!(
            runtime.detached_action(ActionInstanceId::new(), None, None, crate::RunOrigin::App),
            Err(EngineError::UnknownAction(_))
        ));
        // Other keys still run in place.
        assert!(matches!(
            runtime.handle_event(&press(0)).unwrap(),
            RuntimeOutcome::Action { .. }
        ));
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
        assert_eq!(faces[0].1.symbol, None, "notices have no icon");

        runtime.set_profile(Some(f.profile), None);
        let faces = runtime.faces(&device());
        assert_eq!(faces[0].1.detail.as_deref(), Some("SHOP"));
        assert_eq!(faces[1].1.style, FaceStyle::Folder);
        assert_eq!(faces[1].1.symbol.as_deref(), Some(symbols::FOLDER));
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

    /// The editor's preview learns what a key does to its view without
    /// running it, the automatic Back key included.
    #[test]
    fn control_navigation_names_folders_back_and_the_automatic_back() {
        use crate::{ControlNavigation, NavigationOperation, control_navigation};
        let mut f = fixture();
        let layout = &mut f.profile.layouts[0];
        let home = layout.home_page;
        let folder = layout.pages[1].id;
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
        let layout = f.profile.layouts[0].clone();
        let registry = registry();
        let controls: Vec<ControlId> = (0..6).map(|index| ControlId::Key { index }).collect();
        let at = |page, index| {
            control_navigation(
                &registry,
                &layout,
                page,
                &f.profile,
                ControlId::Key { index },
                &controls,
            )
        };
        assert_eq!(
            at(home, 3),
            Some(ControlNavigation {
                operation: NavigationOperation::FolderOpen,
                page: Some(sub_id),
                layout: None,
                automatic: false,
                system: false,
            })
        );
        // The folder's own Back key (key 1).
        let back = at(folder, 1).unwrap();
        assert_eq!(back.operation, NavigationOperation::FolderBack);
        assert!(!back.automatic);
        // The first free key of a folder page without a way back.
        let automatic = at(sub_id, 0).unwrap();
        assert_eq!(automatic.operation, NavigationOperation::FolderBack);
        assert!(automatic.automatic);
        assert_eq!(
            serde_json::to_value(&automatic).unwrap(),
            serde_json::json!({"operation": "folder.back", "automatic": true})
        );
        assert_eq!(at(sub_id, 1), None);
        // An action key without declared navigation does not navigate.
        assert_eq!(at(home, 0), None);
    }

    #[test]
    fn faces_carry_the_binding_appearance() {
        let mut f = fixture();
        let look = crate::KeyAppearance {
            background: Some(crate::Color([10, 20, 30]).into()),
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
            None,
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
            None,
        );
        assert_eq!(faces.len(), 15);
        assert_eq!(faces[0].1.detail.as_deref(), Some("CMS"));
        assert!(
            faces[2].1.highlighted,
            "the CMS selector is highlighted for CMS"
        );
        // A key shows its action's icon, else its type's.
        let action = layout
            .page(layout.home_page)
            .and_then(|page| page.bindings.iter().find(|b| b.control == faces[2].0))
            .and_then(|binding| binding.target.action_id())
            .unwrap();
        let instance = f.profile.action(action).unwrap();
        let type_icon = registry
            .definition(&instance.action_type)
            .and_then(|definition| definition.node.icon.clone());
        assert_eq!(faces[2].1.symbol, instance.icon.clone().or(type_icon));
        let mut iconed = f.profile.clone();
        iconed
            .actions
            .iter_mut()
            .find(|a| a.id == action)
            .unwrap()
            .icon = Some("rocket".into());
        let faces = page_faces(
            &registry,
            layout,
            layout.home_page,
            &iconed,
            Some(cms),
            &controls,
            None,
        );
        assert_eq!(faces[2].1.symbol.as_deref(), Some("rocket"));
        let missing = page_faces(
            &registry,
            layout,
            PageId::new(),
            &f.profile,
            None,
            &controls,
            None,
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

    #[test]
    fn reload_sends_devices_without_pages_to_a_new_layout_home() {
        // A device connected while the profile had no layout has no pages.
        let f = fixture();
        let layout = f.profile.layouts[0].clone();
        let home = layout.home_page;
        let mut bare = f.profile.clone();
        bare.layouts.clear();
        let other = DeviceId {
            model: device().model,
            serial: "OTHER".into(),
        };
        let mut runtime = runtime();
        connect(&mut runtime, &device());
        runtime.set_profile(Some(bare.clone()), None);
        connect(&mut runtime, &other);
        assert_eq!(runtime.current_page(&device()), None);
        assert_eq!(runtime.current_page(&other), None);
        assert!(matches!(
            runtime.handle_event(&press(0)).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));

        // The layout is added to the active profile: every device goes home
        // and its keys work.
        let mut with_layout = bare.clone();
        with_layout.layouts.push(layout.clone());
        with_layout.activate_layout(layout.id).unwrap();
        runtime.reload_profile(with_layout.clone());
        assert_eq!(runtime.current_page(&device()), Some(home));
        assert_eq!(runtime.current_page(&other), Some(home));
        assert!(matches!(
            runtime.handle_event(&press(0)).unwrap(),
            RuntimeOutcome::Action { action, .. } if action == f.next
        ));

        // A valid stack is still kept by a reload.
        runtime.handle_event(&press(1)).unwrap();
        let folder = runtime.current_page(&device());
        assert_ne!(folder, Some(home));
        runtime.reload_profile(with_layout.clone());
        assert_eq!(runtime.current_page(&device()), folder);
        assert_eq!(runtime.current_page(&other), Some(home));

        // Removed: no pages. Added again: home again.
        runtime.reload_profile(bare);
        assert_eq!(runtime.current_page(&device()), None);
        assert_eq!(runtime.current_page(&other), None);
        runtime.reload_profile(with_layout);
        assert_eq!(runtime.current_page(&device()), Some(home));
        assert_eq!(runtime.current_page(&other), Some(home));
    }

    /// Key 3 of the home page: click runs the CMS selector as saved, double
    /// click runs the same instance with SHOP instead, long press opens the
    /// folder. Its long press takes 1.2 s.
    fn gesture_fixture() -> (Fixture, ProjectId) {
        let mut f = fixture();
        let shop = f.profile.projects[0].id;
        let layout = &mut f.profile.layouts[0];
        // A page inside the folder: a key of Home that opens it is an
        // ordinary page key with gestures, not a folder key (those open on
        // the press only, `crate::folder_control`).
        let inside = LayoutPage {
            parent: Some(layout.pages[1].id),
            ..LayoutPage::new("Inside")
        };
        let folder = inside.id;
        layout.pages.push(inside);
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
        let folder = f.profile.layouts[0].pages.last().unwrap().id;
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
    fn stateful_gestures_are_handed_to_the_caller() {
        let mut f = fixture();
        let home = f.profile.layouts[0].home_page;
        let key = ControlId::Key { index: 4 };
        let stateful = StatefulBinding::work_session();
        let page = f.profile.layouts[0].page_mut(home).unwrap();
        page.bindings.extend([
            ControlBinding::new(
                key,
                Trigger::Click,
                BindingTarget::Stateful(stateful.clone()),
            ),
            ControlBinding::new(
                key,
                Trigger::LongPress,
                BindingTarget::Stateful(stateful.clone()),
            ),
            // Refused when saved; does nothing if it gets through.
            ControlBinding::new(
                ControlId::Key { index: 5 },
                Trigger::Press,
                BindingTarget::Stateful(stateful.clone()),
            ),
        ]);
        let mut runtime = runtime();
        runtime.set_profile(Some(f.profile.clone()), None);
        connect(&mut runtime, &device());

        assert_eq!(runtime.stateful_binding(&device(), key), Some(&stateful));
        assert_eq!(
            runtime.stateful_binding(&device(), ControlId::Key { index: 0 }),
            None
        );
        for gesture in [Gesture::Click, Gesture::LongPress] {
            assert_eq!(
                runtime.dispatch_gesture(&device(), key, gesture).unwrap(),
                RuntimeOutcome::StatefulRequested {
                    binding: stateful.clone(),
                    gesture
                }
            );
        }
        assert!(matches!(
            runtime.handle_event(&press(5)).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
        // Without the machine's state: its name.
        let faces = runtime.faces(&device());
        assert_eq!(
            faces[4].1,
            KeyFace::new(FaceStyle::Action, "Work Session")
                .with_symbol(Some(symbols::WORK_SESSION))
        );
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
