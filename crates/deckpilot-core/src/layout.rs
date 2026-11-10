//! Device layouts: pages of control bindings for one device model.
//!
//! A layout is portable configuration (part of a profile). It is specific to a
//! device *model*, not to a particular physical device, so it can be edited
//! while no device is connected.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::{
    ControlId, ControlInput, DeviceCapabilities, DeviceModelId, ModelCatalog, TouchGesture,
};

use crate::{ActionInstanceId, LayoutId, PageId, Profile, WorkflowId};

/// The input that activates a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    Press,
    Release,
    RotateClockwise,
    RotateCounterClockwise,
    Touch { gesture: TouchGesture },
}

impl Trigger {
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
            Self::Press => ControlInput::Press,
            Self::Release => ControlInput::Release,
            Self::RotateClockwise => ControlInput::Rotate { delta: 1 },
            Self::RotateCounterClockwise => ControlInput::Rotate { delta: -1 },
            Self::Touch { gesture } => ControlInput::Touch { gesture },
        }
    }
}

/// What a binding does when triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BindingTarget {
    Action {
        action: ActionInstanceId,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlBinding {
    pub control: ControlId,
    pub trigger: Trigger,
    pub target: BindingTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutPage {
    pub id: PageId,
    pub name: String,
    #[serde(default)]
    pub bindings: Vec<ControlBinding>,
}

impl LayoutPage {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: PageId::new(),
            name: name.into(),
            bindings: Vec::new(),
        }
    }

    pub fn binding_for(&self, control: ControlId, input: ControlInput) -> Option<&ControlBinding> {
        self.bindings
            .iter()
            .find(|binding| binding.control == control && binding.trigger.matches(input))
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
        }
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
            if capabilities
                .validate_input(control, binding.trigger.sample_input())
                .is_err()
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
            let target_exists = match &binding.target {
                BindingTarget::Action { action } => profile.action(*action).is_some(),
                BindingTarget::Workflow { workflow } => profile.workflow(*workflow).is_some(),
                BindingTarget::OpenPage { page } => self.page(*page).is_some(),
                BindingTarget::Back => true,
            };
            if !target_exists {
                issues.push(LayoutIssue::DanglingTarget {
                    page: name(),
                    control,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_device::{
        DeviceModel, EncoderSpec, HardwareSupport, KeyGrid, TouchSurface, models,
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
            },
            support: HardwareSupport::DescriptorOnly,
        });
        catalog
    }

    fn bind(control: ControlId, trigger: Trigger, target: BindingTarget) -> ControlBinding {
        ControlBinding {
            control,
            trigger,
            target,
            label: None,
        }
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
        assert!(layout.validate(&catalog(), &profile).is_empty());
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
            BindingTarget::Action {
                action: ActionInstanceId::new(),
            },
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
        assert!(layout.validate(&catalog(), &profile).is_empty());

        let unknown = DeviceLayout::new("X", DeviceModelId::new("vendor.unknown"));
        assert!(matches!(
            unknown.validate(&catalog(), &profile)[0],
            LayoutIssue::UnknownModel(_)
        ));
    }
}
