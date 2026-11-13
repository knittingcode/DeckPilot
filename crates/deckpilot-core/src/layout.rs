//! Device layouts: pages of control bindings for one device model.
//!
//! A layout is portable configuration (part of a profile). It is specific to a
//! device *model*, not to a particular physical device, so it can be edited
//! while no device is connected.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::{
    ControlId, ControlInput, ControlKind, DeviceCapabilities, DeviceModel, DeviceModelId,
    ModelCatalog, TouchGesture,
};

use crate::gesture::{
    BoundGestures, Gesture, GestureTimingError, GestureTimings, KeyGestureSettings,
};
use crate::stateful::{StatefulBinding, StatefulIssue};
use crate::variables::VariableRef;
use crate::{
    ActionInstanceId, ActionParameters, ActionRegistry, LayoutId, PageId, ParameterError, Profile,
    WorkflowId,
};

/// The input that activates a binding.
///
/// `Press` and `Release` are the raw edges: they fire at once, as every
/// binding did before gestures. `Click`, `DoubleClick` and `LongPress` are
/// gestures, recognized by the agent's gesture engine (see
/// `docs/gestures-and-work-session.md`). A control uses one kind or the
/// other on a page, never both ([`LayoutIssue::MixedTriggers`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    Press,
    Release,
    RotateClockwise,
    RotateCounterClockwise,
    Touch {
        gesture: TouchGesture,
    },
    /// A single click: on the press when nothing else is bound to the
    /// control, on the release when a long press is, after the
    /// double-click window when a double click is.
    Click,
    DoubleClick,
    LongPress,
}

impl From<Gesture> for Trigger {
    fn from(gesture: Gesture) -> Self {
        match gesture {
            Gesture::Click => Self::Click,
            Gesture::DoubleClick => Self::DoubleClick,
            Gesture::LongPress => Self::LongPress,
        }
    }
}

impl Trigger {
    /// The gesture this trigger reacts to; `None` for raw triggers.
    pub fn gesture(self) -> Option<Gesture> {
        match self {
            Self::Click => Some(Gesture::Click),
            Self::DoubleClick => Some(Gesture::DoubleClick),
            Self::LongPress => Some(Gesture::LongPress),
            Self::Press
            | Self::Release
            | Self::RotateClockwise
            | Self::RotateCounterClockwise
            | Self::Touch { .. } => None,
        }
    }

    /// Whether a raw input fires this trigger. Gesture triggers never
    /// match a raw input: the gesture engine reports them.
    pub fn matches(self, input: ControlInput) -> bool {
        match (self, input) {
            (Self::Press, ControlInput::Press) | (Self::Release, ControlInput::Release) => true,
            (Self::RotateClockwise, ControlInput::Rotate { delta }) => delta > 0,
            (Self::RotateCounterClockwise, ControlInput::Rotate { delta }) => delta < 0,
            (Self::Touch { gesture }, ControlInput::Touch { gesture: actual }) => gesture == actual,
            _ => false,
        }
    }

    /// A representative input, used to check whether a control supports the trigger.
    fn sample_input(self) -> ControlInput {
        match self {
            // Gestures need the release too (checked separately).
            Self::Press | Self::Click | Self::DoubleClick | Self::LongPress => ControlInput::Press,
            Self::Release => ControlInput::Release,
            Self::RotateClockwise => ControlInput::Rotate { delta: 1 },
            Self::RotateCounterClockwise => ControlInput::Rotate { delta: -1 },
            Self::Touch { gesture } => ControlInput::Touch { gesture },
        }
    }
}

/// What a binding does when triggered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BindingTarget {
    /// Runs a saved action instance.
    Action {
        action: ActionInstanceId,
        /// Parameter values that replace the instance's for this binding
        /// only: literals, references or templates, resolved when the
        /// binding fires (the rules of a workflow node's overrides). One
        /// instance can serve several keys or gestures with different
        /// values, without copies. Empty, and not stored, by default.
        #[serde(default, skip_serializing_if = "ActionParameters::is_empty")]
        overrides: ActionParameters,
    },
    Workflow {
        workflow: WorkflowId,
    },
    /// Navigates into another page; used for both pages and folders.
    OpenPage {
        page: PageId,
    },
    /// Returns to the previously open page.
    Back,
    /// Asks a built-in state machine for an operation, chosen by the
    /// machine's state at the press and the gesture (see
    /// [`crate::stateful`]). Only on gesture triggers; every stateful
    /// binding of a key carries the same machine and rules.
    Stateful(StatefulBinding),
}

impl BindingTarget {
    /// Runs `action` with its own parameters.
    pub fn action(action: ActionInstanceId) -> Self {
        Self::Action {
            action,
            overrides: ActionParameters::new(),
        }
    }

    /// The action instance this target runs, if it runs one.
    pub fn action_id(&self) -> Option<ActionInstanceId> {
        match self {
            Self::Action { action, .. } => Some(*action),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlBinding {
    pub control: ControlId,
    pub trigger: Trigger,
    pub target: BindingTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// How the key looks; `None`: the default look of its style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<crate::KeyAppearance>,
}

impl ControlBinding {
    /// A binding with the default label and look.
    pub fn new(control: ControlId, trigger: Trigger, target: BindingTarget) -> Self {
        Self {
            control,
            trigger,
            target,
            label: None,
            appearance: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutPage {
    pub id: PageId,
    pub name: String,
    /// The folder this page belongs to: Back returns there. `None`: a
    /// top-level page, or (in layouts from before parents were stored) a
    /// parent inferred from the bindings that open the page (see
    /// [`DeviceLayout::page_parents`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<PageId>,
    #[serde(default)]
    pub bindings: Vec<ControlBinding>,
    /// Gesture timings of controls that differ from the machine's
    /// defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gestures: Vec<KeyGestureSettings>,
}

impl LayoutPage {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: PageId::new(),
            name: name.into(),
            parent: None,
            bindings: Vec::new(),
            gestures: Vec::new(),
        }
    }

    pub fn binding_for(&self, control: ControlId, input: ControlInput) -> Option<&ControlBinding> {
        self.bindings
            .iter()
            .find(|binding| binding.control == control && binding.trigger.matches(input))
    }

    /// The binding of `control` for `gesture`.
    pub fn gesture_binding(&self, control: ControlId, gesture: Gesture) -> Option<&ControlBinding> {
        let trigger = Trigger::from(gesture);
        self.bindings
            .iter()
            .find(|binding| binding.control == control && binding.trigger == trigger)
    }

    /// The gestures bound to `control`. None: the control is in raw mode,
    /// and its press and release fire at once.
    pub fn bound_gestures(&self, control: ControlId) -> BoundGestures {
        let mut bound = BoundGestures::default();
        for binding in self.bindings.iter().filter(|b| b.control == control) {
            match binding.trigger.gesture() {
                Some(Gesture::Click) => bound.click = true,
                Some(Gesture::DoubleClick) => bound.double_click = true,
                Some(Gesture::LongPress) => bound.long_press = true,
                None => {}
            }
        }
        bound
    }

    /// The timings of `control`: the page's settings for it over
    /// `defaults`. Settings outside the allowed ranges (refused when a
    /// layout is saved) fall back to `defaults`.
    pub fn gesture_timings(&self, control: ControlId, defaults: GestureTimings) -> GestureTimings {
        self.gestures
            .iter()
            .find(|settings| settings.control == control)
            .and_then(|settings| settings.timings(defaults).ok())
            .unwrap_or(defaults)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLayout {
    pub id: LayoutId,
    pub name: String,
    pub model: DeviceModelId,
    pub home_page: PageId,
    pub pages: Vec<LayoutPage>,
    /// The page tree is stored: [`LayoutPage::parent`] is the whole truth
    /// and `None` means top level. Without it (layouts from before page
    /// parents), parents that are not stored are inferred.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit_tree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LayoutIssue {
    #[error("device model `{0}` is unknown")]
    UnknownModel(DeviceModelId),
    #[error("home page does not exist")]
    MissingHomePage,
    #[error("page `{page}`: the device has no {control}")]
    UnknownControl { page: String, control: ControlId },
    #[error("page `{page}`: {control} does not support {trigger:?}")]
    UnsupportedTrigger {
        page: String,
        control: ControlId,
        trigger: Trigger,
    },
    #[error("page `{page}`: {control} has more than one {trigger:?} binding")]
    DuplicateBinding {
        page: String,
        control: ControlId,
        trigger: Trigger,
    },
    #[error("page `{page}`: binding references a missing action, workflow or page")]
    DanglingTarget { page: String, control: ControlId },
    #[error("page `{page}`: {control} shows an icon that is not in the profile")]
    MissingIcon { page: String, control: ControlId },
    #[error("page `{page}` belongs to a folder that does not exist")]
    UnknownParent { page: String },
    #[error("page `{page}` is inside itself (its folders form a cycle)")]
    ParentCycle { page: String },
    #[error("the home page cannot be inside a folder")]
    HomeHasParent,
    #[error(
        "page `{page}`: {control} mixes press or release with click, double click or long press"
    )]
    MixedTriggers { page: String, control: ControlId },
    #[error("page `{page}`: {control}: {reason}")]
    InvalidGestureTiming {
        page: String,
        control: ControlId,
        reason: GestureTimingError,
    },
    #[error("page `{page}`: the gesture timings of {control} are set more than once")]
    DuplicateGestureSettings { page: String, control: ControlId },
    #[error("page `{page}`: {control} ({trigger:?}): {reason}")]
    InvalidOverrides {
        page: String,
        control: ControlId,
        trigger: Trigger,
        reason: String,
    },
    #[error(
        "page `{page}`: {control} ({trigger:?}): a stateful key reacts to click, double click or long press"
    )]
    StatefulNeedsGesture {
        page: String,
        control: ControlId,
        trigger: Trigger,
    },
    #[error("page `{page}`: the stateful bindings of {control} follow different machines or rules")]
    InconsistentStateful { page: String, control: ControlId },
    #[error("page `{page}`: {control}: {reason}")]
    InvalidStateful {
        page: String,
        control: ControlId,
        reason: StatefulIssue,
    },
    /// A warning ([`DeviceLayout::warnings`]): the runtime shows an
    /// automatic Back key on the page's first free key.
    #[error("page `{page}` has no Back key; an automatic one is shown on its first free key")]
    MissingBack { page: String },
}

impl DeviceLayout {
    /// Creates a layout with an empty home page.
    pub fn new(name: impl Into<String>, model: DeviceModelId) -> Self {
        let home = LayoutPage::new("Home");
        Self {
            id: LayoutId::new(),
            name: name.into(),
            model,
            home_page: home.id,
            pages: vec![home],
            explicit_tree: false,
        }
    }

    /// Generates a layout that places the profile's actions and then its
    /// workflows on the model's keys, in order. When they do not fit, the last
    /// key of a page opens the next page and the first key of every following
    /// page goes back. Only the model's capabilities are used, so the result
    /// fits any key grid.
    pub fn generate(name: impl Into<String>, model: &DeviceModel, profile: &Profile) -> Self {
        let mut layout = Self::new(name, model.id.clone());
        let keys: Vec<ControlId> = model
            .capabilities
            .controls()
            .into_iter()
            .filter(|control| control.kind() == ControlKind::Key)
            .collect();
        let items: Vec<(BindingTarget, String)> = profile
            .actions
            .iter()
            .map(|a| (BindingTarget::action(a.id), a.name.clone()))
            .chain(
                profile
                    .workflows
                    .iter()
                    .map(|w| (BindingTarget::Workflow { workflow: w.id }, w.name.clone())),
            )
            .collect();

        let press = |control, target, label: Option<String>| ControlBinding {
            label,
            ..ControlBinding::new(control, Trigger::Press, target)
        };

        let mut next = 0;
        let mut page_index = 0;
        let mut page_id = layout.home_page;
        loop {
            let mut slots = keys.clone();
            let mut bindings = Vec::new();
            if page_index > 0 && !slots.is_empty() {
                bindings.push(press(slots.remove(0), BindingTarget::Back, None));
            }
            let remaining = items.len() - next;
            let overflow = remaining > slots.len();
            let capacity = if overflow {
                slots.len().saturating_sub(1)
            } else {
                slots.len()
            };
            if overflow && capacity == 0 {
                // Too few keys to paginate; stop rather than loop forever.
                break;
            }
            for (slot, (target, _)) in slots
                .iter()
                .zip(&items[next..next + capacity.min(remaining)])
            {
                bindings.push(press(*slot, target.clone(), None));
            }
            next += capacity.min(remaining);

            let next_page = overflow.then(|| LayoutPage::new(format!("Page {}", page_index + 2)));
            if let (Some(page), Some(slot)) = (&next_page, slots.last()) {
                bindings.push(press(
                    *slot,
                    BindingTarget::OpenPage { page: page.id },
                    Some("More".into()),
                ));
            }
            if let Some(page) = layout.page_mut(page_id) {
                page.bindings = bindings;
            }
            match next_page {
                Some(page) => {
                    page_id = page.id;
                    layout.pages.push(page);
                    page_index += 1;
                }
                None => break,
            }
        }
        layout
    }

    pub fn page(&self, id: PageId) -> Option<&LayoutPage> {
        self.pages.iter().find(|page| page.id == id)
    }

    pub fn page_mut(&mut self, id: PageId) -> Option<&mut LayoutPage> {
        self.pages.iter_mut().find(|page| page.id == id)
    }

    /// Validates the layout against the model's capabilities and the
    /// profile's actions and workflows.
    pub fn validate(&self, catalog: &ModelCatalog, profile: &Profile) -> Vec<LayoutIssue> {
        let Some(model) = catalog.get(&self.model) else {
            return vec![LayoutIssue::UnknownModel(self.model.clone())];
        };
        let mut issues = Vec::new();
        if self.page(self.home_page).is_none() {
            issues.push(LayoutIssue::MissingHomePage);
        }
        for page in &self.pages {
            self.validate_page(page, &model.capabilities, profile, &mut issues);
        }
        self.validate_parents(&mut issues);
        issues
    }

    fn validate_page(
        &self,
        page: &LayoutPage,
        capabilities: &DeviceCapabilities,
        profile: &Profile,
        issues: &mut Vec<LayoutIssue>,
    ) {
        let name = || page.name.clone();
        for (index, binding) in page.bindings.iter().enumerate() {
            let control = binding.control;
            if !capabilities.has_control(control) {
                issues.push(LayoutIssue::UnknownControl {
                    page: name(),
                    control,
                });
                continue;
            }
            let gesture = binding.trigger.gesture().is_some();
            if capabilities
                .validate_input(control, binding.trigger.sample_input())
                .is_err()
                || (gesture
                    && capabilities
                        .validate_input(control, ControlInput::Release)
                        .is_err())
            {
                issues.push(LayoutIssue::UnsupportedTrigger {
                    page: name(),
                    control,
                    trigger: binding.trigger,
                });
            }
            let duplicate = page.bindings[..index]
                .iter()
                .any(|other| other.control == control && other.trigger == binding.trigger);
            if duplicate {
                issues.push(LayoutIssue::DuplicateBinding {
                    page: name(),
                    control,
                    trigger: binding.trigger,
                });
            }
            if gesture && mixes_triggers(page, index) {
                issues.push(LayoutIssue::MixedTriggers {
                    page: name(),
                    control,
                });
            }
            let target_exists = match &binding.target {
                BindingTarget::Action { action, .. } => profile.action(*action).is_some(),
                BindingTarget::Workflow { workflow } => profile.workflow(*workflow).is_some(),
                BindingTarget::OpenPage { page } => self.page(*page).is_some(),
                BindingTarget::Back | BindingTarget::Stateful(_) => true,
            };
            if !target_exists {
                issues.push(LayoutIssue::DanglingTarget {
                    page: name(),
                    control,
                });
            }
            if let BindingTarget::Action { overrides, .. } = &binding.target
                && overrides
                    .projects()
                    .any(|project| profile.project(project).is_none())
            {
                issues.push(LayoutIssue::InvalidOverrides {
                    page: name(),
                    control,
                    trigger: binding.trigger,
                    reason: "a project that is not in this profile".into(),
                });
            }
            if let Some(icon) = binding.appearance.as_ref().and_then(|a| a.icon)
                && profile.icon(icon.id).is_none()
            {
                issues.push(LayoutIssue::MissingIcon {
                    page: name(),
                    control,
                });
            }
            if let BindingTarget::Stateful(stateful) = &binding.target {
                validate_stateful(page, index, stateful, issues);
            }
        }
        validate_gesture_settings(page, capabilities, issues);
    }

    /// Checks the parameter overrides of action bindings against the
    /// action types in `registry`, as a workflow node's overrides are
    /// checked: every key is a parameter of the type, the merged values
    /// are valid, and references are ones a key press can resolve.
    /// Shared variables are resolved, and their values checked, when the
    /// binding fires.
    pub fn check_overrides(
        &self,
        registry: &ActionRegistry,
        profile: &Profile,
    ) -> Vec<LayoutIssue> {
        let mut issues = Vec::new();
        for page in &self.pages {
            for binding in &page.bindings {
                let BindingTarget::Action { action, overrides } = &binding.target else {
                    continue;
                };
                if overrides.is_empty() {
                    continue;
                }
                let mut report = |reason: String| {
                    issues.push(LayoutIssue::InvalidOverrides {
                        page: page.name.clone(),
                        control: binding.control,
                        trigger: binding.trigger,
                        reason,
                    });
                };
                // A missing action is a dangling target (see `validate`).
                let Some(instance) = profile.action(*action) else {
                    continue;
                };
                let Some(definition) = registry.definition(&instance.action_type) else {
                    report(format!(
                        "{} cannot run here, so its parameters cannot be checked",
                        instance.name
                    ));
                    continue;
                };
                for (key, value) in overrides.iter() {
                    if value
                        .references()
                        .iter()
                        .any(|reference| matches!(reference, VariableRef::Step(_)))
                    {
                        report(format!(
                            "`{key}`: node outputs exist only inside a workflow"
                        ));
                    }
                }
                if let Err(errors) = definition.normalize(&instance.parameters.overlay(overrides)) {
                    // Problems of the instance's own values are the
                    // instance's, reported where it is edited.
                    for error in errors {
                        let (ParameterError::Unknown(key)
                        | ParameterError::Missing(key)
                        | ParameterError::WrongType { key, .. }
                        | ParameterError::Invalid { key, .. }) = &error;
                        if overrides.get(key).is_some() {
                            report(error.to_string());
                        }
                    }
                }
            }
        }
        issues
    }
}

/// Whether the gesture binding at `index` is the first of its control on
/// `page`, and the control also has a press or release binding: the mix is
/// reported once per control.
fn mixes_triggers(page: &LayoutPage, index: usize) -> bool {
    let control = page.bindings[index].control;
    let first = !page.bindings[..index]
        .iter()
        .any(|other| other.control == control && other.trigger.gesture().is_some());
    first
        && page.bindings.iter().any(|other| {
            other.control == control && matches!(other.trigger, Trigger::Press | Trigger::Release)
        })
}

/// A stateful binding: on a gesture trigger, with the same machine and
/// rules as the key's other stateful bindings, and rules the machine
/// allows for the gestures the key binds to it. The key's checks are
/// reported once, at its first stateful binding.
fn validate_stateful(
    page: &LayoutPage,
    index: usize,
    stateful: &StatefulBinding,
    issues: &mut Vec<LayoutIssue>,
) {
    let binding = &page.bindings[index];
    let control = binding.control;
    if binding.trigger.gesture().is_none() {
        issues.push(LayoutIssue::StatefulNeedsGesture {
            page: page.name.clone(),
            control,
            trigger: binding.trigger,
        });
    }
    let others = page
        .bindings
        .iter()
        .enumerate()
        .filter_map(|(at, other)| match &other.target {
            BindingTarget::Stateful(target) if other.control == control => {
                Some((at, other.trigger, target))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if others.first().map(|(at, _, _)| *at) != Some(index) {
        return;
    }
    if others.iter().any(|(_, _, other)| *other != stateful) {
        issues.push(LayoutIssue::InconsistentStateful {
            page: page.name.clone(),
            control,
        });
        return;
    }
    let mut bound = BoundGestures::default();
    for (_, trigger, _) in &others {
        match trigger.gesture() {
            Some(Gesture::Click) => bound.click = true,
            Some(Gesture::DoubleClick) => bound.double_click = true,
            Some(Gesture::LongPress) => bound.long_press = true,
            None => {}
        }
    }
    for reason in stateful.issues(bound) {
        issues.push(LayoutIssue::InvalidStateful {
            page: page.name.clone(),
            control,
            reason,
        });
    }
}

/// Timings set for controls the device has, once each, in range.
fn validate_gesture_settings(
    page: &LayoutPage,
    capabilities: &DeviceCapabilities,
    issues: &mut Vec<LayoutIssue>,
) {
    let name = || page.name.clone();
    for (index, settings) in page.gestures.iter().enumerate() {
        let control = settings.control;
        if !capabilities.has_control(control) {
            issues.push(LayoutIssue::UnknownControl {
                page: name(),
                control,
            });
            continue;
        }
        if page.gestures[..index].iter().any(|s| s.control == control) {
            issues.push(LayoutIssue::DuplicateGestureSettings {
                page: name(),
                control,
            });
        }
        // Each value set must be in its range; the defaults are.
        if let Err(reason) = settings.timings(GestureTimings::default()) {
            issues.push(LayoutIssue::InvalidGestureTiming {
                page: name(),
                control,
                reason,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_device::{
        DeviceModel, EncoderSpec, HardwareSupport, KeyGrid, ModelVerification, TouchSurface, models,
    };

    fn catalog() -> ModelCatalog {
        let mut catalog = ModelCatalog::builtin();
        catalog.register(DeviceModel {
            id: DeviceModelId::new("test.encoders"),
            vendor: "Test".into(),
            name: "Encoder test device".into(),
            usb: None,
            capabilities: DeviceCapabilities {
                keys: Some(KeyGrid {
                    rows: 2,
                    columns: 4,
                    image: None,
                }),
                encoders: Some(EncoderSpec {
                    count: 4,
                    pressable: true,
                }),
                touch: Some(TouchSurface {
                    zones: 4,
                    image: None,
                    gestures: vec![TouchGesture::Tap],
                }),
                brightness: false,
            },
            support: HardwareSupport::DescriptorOnly,
            verification: ModelVerification::default(),
        });
        catalog
    }

    fn bind(control: ControlId, trigger: Trigger, target: BindingTarget) -> ControlBinding {
        ControlBinding {
            control,
            trigger,
            target,
            label: None,
            appearance: None,
        }
    }

    fn profile_with_actions(count: usize) -> Profile {
        let mut profile = Profile::new("Work");
        for index in 0..count {
            profile.actions.push(crate::ActionInstance {
                id: ActionInstanceId::new(),
                name: format!("A{index}"),
                action_type: crate::testing::echo_type(),
                parameters: crate::ActionParameters::new(),
                description: String::new(),
                icon: None,
                label: None,
            });
        }
        profile
    }

    #[test]
    fn generated_layout_fits_on_one_page_when_possible() {
        let profile = profile_with_actions(15);
        let layout = DeviceLayout::generate("Auto", &models::stream_deck_mk2(), &profile);
        assert_eq!(layout.pages.len(), 1);
        assert_eq!(layout.pages[0].bindings.len(), 15);
        assert_eq!(
            layout.validate(&catalog(), &profile),
            Vec::<LayoutIssue>::new()
        );
    }

    #[test]
    fn generated_layout_paginates_by_capabilities() {
        // 15 keys: 14 on home + More, 13 + Back + More, then the rest + Back.
        let profile = profile_with_actions(30);
        let layout = DeviceLayout::generate("Auto", &models::stream_deck_mk2(), &profile);
        assert_eq!(layout.pages.len(), 3);
        let actions = |page: &LayoutPage| {
            page.bindings
                .iter()
                .filter(|b| matches!(b.target, BindingTarget::Action { .. }))
                .count()
        };
        assert_eq!(actions(&layout.pages[0]), 14);
        assert_eq!(actions(&layout.pages[1]), 13);
        assert_eq!(actions(&layout.pages[2]), 3);
        assert_eq!(layout.pages[1].bindings[0].target, BindingTarget::Back);
        assert_eq!(
            layout.pages[0].bindings[14].target,
            BindingTarget::OpenPage {
                page: layout.pages[1].id
            }
        );
        assert_eq!(
            layout.validate(&catalog(), &profile),
            Vec::<LayoutIssue>::new()
        );

        // A smaller grid from the test catalog gets more pages.
        let small = catalog()
            .get(&DeviceModelId::new("test.encoders"))
            .unwrap()
            .clone();
        let layout = DeviceLayout::generate("Auto", &small, &profile);
        assert!(layout.pages.len() > 3);
        assert_eq!(
            layout.validate(&catalog(), &profile),
            Vec::<LayoutIssue>::new()
        );
    }

    #[test]
    fn generated_layout_handles_models_without_keys() {
        let model = DeviceModel {
            capabilities: DeviceCapabilities::default(),
            ..models::stream_deck_mk2()
        };
        let layout = DeviceLayout::generate("Auto", &model, &profile_with_actions(3));
        assert_eq!(layout.pages.len(), 1);
        assert_eq!(layout.pages[0].bindings, Vec::<ControlBinding>::new());
    }

    #[test]
    fn triggers_match_inputs() {
        assert!(Trigger::Press.matches(ControlInput::Press));
        assert!(!Trigger::Press.matches(ControlInput::Release));
        assert!(Trigger::RotateClockwise.matches(ControlInput::Rotate { delta: 3 }));
        assert!(!Trigger::RotateClockwise.matches(ControlInput::Rotate { delta: -1 }));
        assert!(Trigger::RotateCounterClockwise.matches(ControlInput::Rotate { delta: -1 }));
        let tap = Trigger::Touch {
            gesture: TouchGesture::Tap,
        };
        assert!(tap.matches(ControlInput::Touch {
            gesture: TouchGesture::Tap
        }));
        assert!(!tap.matches(ControlInput::Touch {
            gesture: TouchGesture::Swipe
        }));
    }

    #[test]
    fn valid_folder_layout_for_mk2() {
        let profile = Profile::new("Work");
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        let folder = LayoutPage::new("Folder");
        let folder_id = folder.id;
        layout.pages.push(folder);
        let home = layout.home_page;
        layout.page_mut(home).unwrap().bindings.push(bind(
            ControlId::Key { index: 14 },
            Trigger::Press,
            BindingTarget::OpenPage { page: folder_id },
        ));
        layout.page_mut(folder_id).unwrap().bindings.push(bind(
            ControlId::Key { index: 0 },
            Trigger::Press,
            BindingTarget::Back,
        ));
        assert_eq!(
            layout.validate(&catalog(), &profile),
            Vec::<LayoutIssue>::new()
        );
    }

    #[test]
    fn appearances_are_optional_and_icons_must_exist() {
        // A binding stored before appearances existed.
        let old = serde_json::json!({
            "control": ControlId::Key { index: 3 },
            "trigger": {"type": "press"},
            "target": {"type": "back"},
            "label": "Up",
        });
        let binding: ControlBinding = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(binding.appearance, None);
        assert_eq!(serde_json::to_value(&binding).unwrap(), old);

        let mut profile = Profile::new("Work");
        let icon = crate::Icon {
            id: crate::IconId::new(),
            name: "Logo".into(),
            width: 8,
            height: 8,
            png: crate::IconData::new(crate::appearance::tests::png_header(8, 8)),
        };
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        let home = layout.home_page;
        let mut back = bind(
            ControlId::Key { index: 0 },
            Trigger::Press,
            BindingTarget::Back,
        );
        back.appearance = Some(crate::KeyAppearance {
            icon: Some(crate::IconRef {
                id: icon.id,
                fit: crate::IconFit::Fit,
            }),
            ..crate::KeyAppearance::default()
        });
        layout.page_mut(home).unwrap().bindings.push(back);
        assert!(matches!(
            layout.validate(&catalog(), &profile)[..],
            [LayoutIssue::MissingIcon { .. }]
        ));
        profile.add_icon(icon).unwrap();
        assert_eq!(layout.validate(&catalog(), &profile), []);
    }

    #[test]
    fn rejects_controls_and_triggers_the_model_lacks() {
        let profile = Profile::new("Work");
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        let home = layout.home_page;
        let bindings = &mut layout.page_mut(home).unwrap().bindings;
        bindings.push(bind(
            ControlId::Key { index: 15 },
            Trigger::Press,
            BindingTarget::Back,
        ));
        bindings.push(bind(
            ControlId::Encoder { index: 0 },
            Trigger::Press,
            BindingTarget::Back,
        ));
        bindings.push(bind(
            ControlId::Key { index: 0 },
            Trigger::RotateClockwise,
            BindingTarget::Back,
        ));
        bindings.push(bind(
            ControlId::Key { index: 1 },
            Trigger::Press,
            BindingTarget::action(ActionInstanceId::new()),
        ));
        bindings.push(bind(
            ControlId::Key { index: 2 },
            Trigger::Press,
            BindingTarget::Back,
        ));
        bindings.push(bind(
            ControlId::Key { index: 2 },
            Trigger::Press,
            BindingTarget::Back,
        ));

        let issues = layout.validate(&catalog(), &profile);
        assert_eq!(issues.len(), 5, "{issues:#?}");
        assert!(matches!(issues[0], LayoutIssue::UnknownControl { .. }));
        assert!(matches!(issues[1], LayoutIssue::UnknownControl { .. }));
        assert!(matches!(issues[2], LayoutIssue::UnsupportedTrigger { .. }));
        assert!(matches!(issues[3], LayoutIssue::DanglingTarget { .. }));
        assert!(matches!(issues[4], LayoutIssue::DuplicateBinding { .. }));
    }

    #[test]
    fn supports_encoders_and_touch_on_capable_models() {
        let profile = Profile::new("Work");
        let mut layout = DeviceLayout::new("Dials", DeviceModelId::new("test.encoders"));
        let home = layout.home_page;
        let bindings = &mut layout.page_mut(home).unwrap().bindings;
        bindings.push(bind(
            ControlId::Encoder { index: 3 },
            Trigger::RotateClockwise,
            BindingTarget::Back,
        ));
        bindings.push(bind(
            ControlId::Encoder { index: 3 },
            Trigger::Press,
            BindingTarget::Back,
        ));
        bindings.push(bind(
            ControlId::TouchZone { index: 0 },
            Trigger::Touch {
                gesture: TouchGesture::Tap,
            },
            BindingTarget::Back,
        ));
        assert_eq!(
            layout.validate(&catalog(), &profile),
            Vec::<LayoutIssue>::new()
        );

        let unknown = DeviceLayout::new("X", DeviceModelId::new("vendor.unknown"));
        assert!(matches!(
            unknown.validate(&catalog(), &profile)[0],
            LayoutIssue::UnknownModel(_)
        ));
    }

    fn key(index: u16) -> ControlId {
        ControlId::Key { index }
    }

    #[test]
    fn bindings_from_before_gestures_decode_and_encode_unchanged() {
        // A page as 0.6.0 stored it: press bindings, no timings.
        let action = ActionInstanceId::new();
        let old = serde_json::json!({
            "id": PageId::new(),
            "name": "Home",
            "bindings": [
                {
                    "control": {"kind": "key", "index": 0},
                    "trigger": {"type": "press"},
                    "target": {"type": "action", "action": action},
                },
                {
                    "control": {"kind": "key", "index": 1},
                    "trigger": {"type": "release"},
                    "target": {"type": "back"},
                },
            ],
        });
        let page: LayoutPage = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(page.bindings[0].target, BindingTarget::action(action));
        assert_eq!(page.gestures, []);
        // Raw mode: no gesture is bound, so the edges fire at once.
        assert!(!page.bound_gestures(key(0)).any());
        assert!(!page.bound_gestures(key(1)).any());
        assert_eq!(serde_json::to_value(&page).unwrap(), old);
    }

    #[test]
    fn gesture_bindings_and_timings_serialize_as_documented() {
        let action = ActionInstanceId::new();
        let mut page = LayoutPage::new("Home");
        page.bindings = vec![
            bind(key(3), Trigger::Click, BindingTarget::action(action)),
            bind(
                key(3),
                Trigger::DoubleClick,
                BindingTarget::Action {
                    action,
                    overrides: crate::ActionParameters::new().with(
                        "message",
                        crate::ParameterValue::Reference(
                            VariableRef::parse("shared.team.name").unwrap(),
                        ),
                    ),
                },
            ),
            bind(key(3), Trigger::LongPress, BindingTarget::Back),
        ];
        page.gestures.push(KeyGestureSettings {
            control: key(3),
            double_click_ms: None,
            long_press_ms: Some(1200),
        });
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(
            json["bindings"][0]["trigger"],
            serde_json::json!({"type": "click"})
        );
        assert_eq!(json["bindings"][0]["target"].get("overrides"), None);
        assert_eq!(
            json["bindings"][1]["trigger"],
            serde_json::json!({"type": "doubleClick"})
        );
        assert_eq!(
            json["bindings"][1]["target"]["overrides"],
            serde_json::json!({"message": {"type": "reference", "value": "shared.team.name"}})
        );
        assert_eq!(
            json["bindings"][2]["trigger"],
            serde_json::json!({"type": "longPress"})
        );
        assert_eq!(
            json["gestures"],
            serde_json::json!([{"control": {"kind": "key", "index": 3}, "longPressMs": 1200}])
        );
        let back: LayoutPage = serde_json::from_value(json).unwrap();
        assert_eq!(back, page);

        assert_eq!(
            page.bound_gestures(key(3)),
            BoundGestures {
                click: true,
                double_click: true,
                long_press: true
            }
        );
        let defaults = GestureTimings {
            double_click_ms: 400,
            long_press_ms: 700,
        };
        assert_eq!(
            page.gesture_timings(key(3), defaults),
            GestureTimings {
                double_click_ms: 400,
                long_press_ms: 1200
            }
        );
        assert_eq!(page.gesture_timings(key(4), defaults), defaults);
        assert_eq!(
            page.gesture_binding(key(3), Gesture::LongPress)
                .map(|b| &b.target),
            Some(&BindingTarget::Back)
        );
        assert_eq!(page.gesture_binding(key(4), Gesture::Click), None);
        for gesture in Gesture::ALL {
            assert_eq!(Trigger::from(gesture).gesture(), Some(gesture));
            // The engine reports gestures; raw edges never match them.
            assert!(!Trigger::from(gesture).matches(ControlInput::Press));
            assert!(!Trigger::from(gesture).matches(ControlInput::Release));
        }
        assert_eq!(Trigger::Press.gesture(), None);
    }

    #[test]
    fn gesture_layouts_are_validated() {
        let profile = Profile::new("Work");
        let mut layout = DeviceLayout::new("Dials", DeviceModelId::new("test.encoders"));
        let home = layout.home_page;
        let page = layout.page_mut(home).unwrap();
        page.bindings = vec![
            // Valid: three gestures on a key, a click on a pressable dial.
            bind(key(0), Trigger::Click, BindingTarget::Back),
            bind(key(0), Trigger::DoubleClick, BindingTarget::Back),
            bind(key(0), Trigger::LongPress, BindingTarget::Back),
            bind(
                ControlId::Encoder { index: 0 },
                Trigger::Click,
                BindingTarget::Back,
            ),
            // Mixed raw and gesture triggers on one key: one issue.
            bind(key(1), Trigger::Press, BindingTarget::Back),
            bind(key(1), Trigger::Click, BindingTarget::Back),
            bind(key(1), Trigger::LongPress, BindingTarget::Back),
            // A touch zone cannot be held or released.
            bind(
                ControlId::TouchZone { index: 0 },
                Trigger::LongPress,
                BindingTarget::Back,
            ),
            // Twice the same gesture.
            bind(key(2), Trigger::Click, BindingTarget::Back),
            bind(key(2), Trigger::Click, BindingTarget::Back),
        ];
        page.gestures = vec![
            KeyGestureSettings {
                control: key(0),
                double_click_ms: Some(150),
                long_press_ms: Some(5000),
            },
            KeyGestureSettings {
                control: key(2),
                double_click_ms: Some(100),
                long_press_ms: None,
            },
            KeyGestureSettings {
                control: key(2),
                double_click_ms: None,
                long_press_ms: Some(6000),
            },
            KeyGestureSettings {
                control: key(99),
                double_click_ms: None,
                long_press_ms: None,
            },
        ];
        let issues = layout.validate(&catalog(), &profile);
        assert!(
            matches!(
                &issues[..],
                [
                    LayoutIssue::MixedTriggers { control: c1, .. },
                    LayoutIssue::UnsupportedTrigger {
                        trigger: Trigger::LongPress,
                        ..
                    },
                    LayoutIssue::DuplicateBinding {
                        trigger: Trigger::Click,
                        ..
                    },
                    LayoutIssue::InvalidGestureTiming {
                        reason: GestureTimingError::DoubleClick { value: 100, .. },
                        ..
                    },
                    LayoutIssue::DuplicateGestureSettings { .. },
                    LayoutIssue::InvalidGestureTiming {
                        reason: GestureTimingError::LongPress { value: 6000, .. },
                        ..
                    },
                    LayoutIssue::UnknownControl { .. },
                ] if *c1 == key(1)
            ),
            "{issues:#?}"
        );
    }

    #[test]
    fn overrides_are_checked_like_node_overrides() {
        let registry = crate::testing::registry();
        let mut profile = profile_with_actions(1);
        let echo = profile.actions[0].id;
        profile.actions[0].parameters = crate::ActionParameters::new()
            .with("message", crate::ParameterValue::Text("saved".into()));
        let select = registry
            .create_instance(
                "Select",
                crate::testing::select_type(),
                &crate::ActionParameters::new(),
            )
            .unwrap();
        let select_id = select.id;
        profile.actions.push(select);
        let reference =
            |text: &str| crate::ParameterValue::Reference(VariableRef::parse(text).unwrap());
        let with = |action, key: &str, value| BindingTarget::Action {
            action,
            overrides: crate::ActionParameters::new().with(key, value),
        };
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        let home = layout.home_page;
        layout.page_mut(home).unwrap().bindings = vec![
            // Valid: a shared variable, a template, a literal.
            bind(
                key(0),
                Trigger::Click,
                with(echo, "message", reference("shared.team.name")),
            ),
            bind(
                key(0),
                Trigger::LongPress,
                with(
                    echo,
                    "message",
                    crate::ParameterValue::Template("hi {{ system.project.current.key }}".into()),
                ),
            ),
            bind(
                key(1),
                Trigger::Press,
                with(echo, "message", crate::ParameterValue::Text("x".into())),
            ),
            // Invalid.
            bind(
                key(2),
                Trigger::Click,
                with(echo, "nope", crate::ParameterValue::Text("x".into())),
            ),
            bind(
                key(3),
                Trigger::Click,
                with(echo, "message", crate::ParameterValue::Integer(3)),
            ),
            bind(
                key(4),
                Trigger::Click,
                with(echo, "message", reference("steps.git.output.branch")),
            ),
            bind(
                key(5),
                Trigger::Click,
                with(
                    select_id,
                    "project",
                    crate::ParameterValue::Template("{{ shared.x }}".into()),
                ),
            ),
        ];
        assert_eq!(layout.validate(&catalog(), &profile), []);
        let issues = layout.check_overrides(&registry, &profile);
        let controls: Vec<ControlId> = issues
            .iter()
            .map(|issue| match issue {
                LayoutIssue::InvalidOverrides { control, .. } => *control,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(controls, [key(2), key(3), key(4), key(5)], "{issues:#?}");

        // A project of another profile.
        layout.page_mut(home).unwrap().bindings = vec![bind(
            key(0),
            Trigger::Click,
            with(
                select_id,
                "project",
                crate::ParameterValue::Project(crate::ProjectId::new()),
            ),
        )];
        assert!(matches!(
            layout.validate(&catalog(), &profile)[..],
            [LayoutIssue::InvalidOverrides { .. }]
        ));
    }

    fn work_session(rules: &[(WorkSessionState, Gesture, WorkSessionOperation)]) -> BindingTarget {
        BindingTarget::Stateful(StatefulBinding::WorkSession {
            rules: rules
                .iter()
                .map(|(s, g, o)| StatefulRule::new(*s, *g, *o))
                .collect(),
        })
    }

    use crate::stateful::{StatefulRule, WorkSessionOperation};
    use crate::work_session::WorkSessionState;

    #[test]
    fn stateful_bindings_serialize_as_documented() {
        let binding = bind(
            key(4),
            Trigger::Click,
            work_session(&[(
                WorkSessionState::Idle,
                Gesture::Click,
                WorkSessionOperation::Start,
            )]),
        );
        let json = serde_json::to_value(&binding).unwrap();
        assert_eq!(
            json["target"],
            serde_json::json!({
                "type": "stateful",
                "machine": "workSession",
                "rules": [{"state": "idle", "gesture": "click", "operation": "start"}]
            })
        );
        assert_eq!(
            serde_json::from_value::<ControlBinding>(json).unwrap(),
            binding
        );
        let defaults = BindingTarget::Stateful(StatefulBinding::work_session());
        let json = serde_json::to_value(&defaults).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "stateful", "machine": "workSession"})
        );
        assert_eq!(
            serde_json::from_value::<BindingTarget>(json).unwrap(),
            defaults
        );
        assert!(
            serde_json::from_value::<BindingTarget>(
                serde_json::json!({"type": "stateful", "machine": "script", "code": "1"})
            )
            .is_err()
        );
        assert_eq!(defaults.action_id(), None);
    }

    #[test]
    fn stateful_layouts_are_validated() {
        use Gesture as G;
        use WorkSessionOperation as O;
        use WorkSessionState as S;

        let profile = Profile::new("Work");
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        let home = layout.home_page;
        let defaults = || BindingTarget::Stateful(StatefulBinding::work_session());
        let page = layout.page_mut(home).unwrap();
        page.bindings = vec![
            // Valid: the default rules on click and long press, and own
            // rules on a key that binds only a double click.
            bind(key(0), Trigger::Click, defaults()),
            bind(key(0), Trigger::LongPress, defaults()),
            bind(
                key(1),
                Trigger::DoubleClick,
                work_session(&[(S::Idle, G::DoubleClick, O::Start)]),
            ),
        ];
        assert_eq!(layout.validate(&catalog(), &profile), []);

        let page = layout.page_mut(home).unwrap();
        page.bindings = vec![
            // A raw trigger cannot carry the state of a press.
            bind(key(2), Trigger::Press, defaults()),
            // Two bindings of one key with different rules.
            bind(key(3), Trigger::Click, defaults()),
            bind(
                key(3),
                Trigger::LongPress,
                work_session(&[(S::Working, G::LongPress, O::Verify)]),
            ),
            // Finishing by a long press; a rule for a gesture that goes
            // elsewhere; an operation the state does not allow.
            bind(
                key(4),
                Trigger::Click,
                work_session(&[
                    (S::ReadyToFinish, G::LongPress, O::Finish),
                    (S::Working, G::Click, O::Start),
                ]),
            ),
            bind(
                key(4),
                Trigger::LongPress,
                work_session(&[
                    (S::ReadyToFinish, G::LongPress, O::Finish),
                    (S::Working, G::Click, O::Start),
                ]),
            ),
            bind(
                key(5),
                Trigger::Click,
                work_session(&[(S::Idle, G::LongPress, O::Start)]),
            ),
            bind(key(5), Trigger::LongPress, BindingTarget::Back),
        ];
        let issues = layout.validate(&catalog(), &profile);
        assert!(
            matches!(
                &issues[..],
                [
                    LayoutIssue::StatefulNeedsGesture {
                        trigger: Trigger::Press,
                        ..
                    },
                    LayoutIssue::InconsistentStateful { control: c3, .. },
                    LayoutIssue::InvalidStateful {
                        reason: StatefulIssue::GestureNotAllowed {
                            gesture: G::LongPress,
                            ..
                        },
                        control: c4,
                        ..
                    },
                    LayoutIssue::InvalidStateful {
                        reason: StatefulIssue::NotAllowed { .. },
                        ..
                    },
                    LayoutIssue::InvalidStateful {
                        reason: StatefulIssue::GestureNotBound {
                            gesture: G::LongPress,
                            ..
                        },
                        control: c5,
                        ..
                    },
                ] if *c3 == key(3) && *c4 == key(4) && *c5 == key(5)
            ),
            "{issues:#?}"
        );
        assert_eq!(
            issues[2].to_string(),
            "page `Home`: key 4: `finish` cannot be asked for by LongPress"
        );
    }
}
