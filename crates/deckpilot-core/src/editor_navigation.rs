//! Editor navigation: what a key does to the page or layout a device shows,
//! as the editor's interactive preview needs it
//! (`docs/editor-navigation-and-simulation.md`).
//!
//! The declaration is **data only**. An action type (built-in or a
//! plugin's, plugin API 1.7) may say that running it opens a folder, goes
//! back, turns the page or switches the layout, and which of its
//! parameters holds the target. The editor reads it from the action
//! catalog and moves its own view; nothing runs, and a plugin is never
//! called for it. The operations are an allowlist ([`NavigationOperation`]),
//! the host maps them to its own navigation code, and
//! [`EditorNavigation::check`] refuses a declaration whose target
//! parameter is not one of the action's own text parameters.
//!
//! [`control_navigation`] resolves what one control of a page does, for a
//! preview face: a binding to a page (`folder.open`), Back (`folder.back`),
//! an action whose type declares navigation (with its target resolved from
//! the instance's parameters), and the automatic Back key the runtime adds
//! to a folder page without a way back (`automatic: true`), which is not
//! part of the layout and so unknown to an editor that reads bindings only,
//! and the folder's reserved Back key (`automatic` and `system`: owned by
//! DeckPilot, see [`crate::folder`]).

use serde::{Deserialize, Serialize};

use crate::{
    ActionRegistry, BindingTarget, DeviceLayout, LayoutId, PageId, ParameterKind, ParameterSpec,
    Profile,
};
use deckpilot_device::{ControlId, DeviceCapabilities};

/// The navigation operations an action may declare. Nothing else is
/// accepted: a manifest naming another operation is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NavigationOperation {
    /// Opens a page (a folder) of the same layout; the target is a page id.
    #[serde(rename = "folder.open")]
    FolderOpen,
    /// Returns to the folder's page, or to the page that opened this one.
    #[serde(rename = "folder.back")]
    FolderBack,
    /// Shows the next page of the layout.
    #[serde(rename = "page.next")]
    PageNext,
    /// Shows the previous page of the layout.
    #[serde(rename = "page.previous")]
    PagePrevious,
    /// Shows another layout of the profile; the target is a layout id,
    /// optionally with a page of it.
    #[serde(rename = "layout.switch")]
    LayoutSwitch,
    /// Shows the layout the device showed before.
    #[serde(rename = "layout.back")]
    LayoutBack,
}

impl NavigationOperation {
    pub const ALL: [Self; 6] = [
        Self::FolderOpen,
        Self::FolderBack,
        Self::PageNext,
        Self::PagePrevious,
        Self::LayoutSwitch,
        Self::LayoutBack,
    ];

    /// The operation's name, as in the catalog and the manifest.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FolderOpen => "folder.open",
            Self::FolderBack => "folder.back",
            Self::PageNext => "page.next",
            Self::PagePrevious => "page.previous",
            Self::LayoutSwitch => "layout.switch",
            Self::LayoutBack => "layout.back",
        }
    }

    /// The operation named `name`, if it is one of the allowlist.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.as_str() == name)
    }

    /// Whether the operation needs a target parameter.
    pub fn takes_target(self) -> bool {
        matches!(self, Self::FolderOpen | Self::LayoutSwitch)
    }
}

/// An action type's declared editor navigation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorNavigation {
    pub operation: NavigationOperation,
    /// The parameter that holds the target: a page id for `folder.open`, a
    /// layout id for `layout.switch`. Required for those two, refused for
    /// the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_parameter: Option<String>,
    /// For `layout.switch` only: the parameter that holds the page of the
    /// target layout to show (its home page when empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_parameter: Option<String>,
}

impl EditorNavigation {
    /// An operation without a target.
    pub fn new(operation: NavigationOperation) -> Self {
        Self {
            operation,
            target_parameter: None,
            page_parameter: None,
        }
    }

    /// An operation whose target `parameter` holds.
    pub fn with_target(operation: NavigationOperation, parameter: &str) -> Self {
        Self {
            target_parameter: Some(parameter.to_owned()),
            ..Self::new(operation)
        }
    }

    #[must_use]
    pub fn with_page(mut self, parameter: &str) -> Self {
        self.page_parameter = Some(parameter.to_owned());
        self
    }

    /// What is wrong with the declaration for an action with `parameters`:
    /// a target where the operation takes none or none where it needs
    /// one, or a parameter that is not one of the action's text
    /// parameters.
    pub fn check(&self, parameters: &[ParameterSpec]) -> Result<(), String> {
        let op = self.operation.as_str();
        let text_parameter =
            |field: &str, key: &str| match parameters.iter().find(|spec| spec.key == key) {
                Some(spec) if matches!(spec.kind, ParameterKind::Text { .. }) => Ok(()),
                Some(_) => Err(format!("{field} `{key}` must be a text parameter")),
                None => Err(format!("{field} `{key}` is not a parameter of the action")),
            };
        match (&self.target_parameter, self.operation.takes_target()) {
            (Some(key), true) => text_parameter("targetParameter", key)?,
            (None, true) => return Err(format!("`{op}` needs a targetParameter")),
            (Some(_), false) => return Err(format!("`{op}` takes no targetParameter")),
            (None, false) => {}
        }
        match &self.page_parameter {
            Some(key) if self.operation == NavigationOperation::LayoutSwitch => {
                text_parameter("pageParameter", key)
            }
            Some(_) => Err(format!("`{op}` takes no pageParameter")),
            None => Ok(()),
        }
    }
}

/// What one control of a page does to the editor's view, resolved for a
/// preview face.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlNavigation {
    pub operation: NavigationOperation,
    /// `folder.open`, and `layout.switch` with a page: the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<PageId>,
    /// `layout.switch`: the layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutId>,
    /// The automatic Back key the runtime shows on a folder page without a
    /// way back; it is not a binding of the layout.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic: bool,
    /// The folder's reserved, protected Back key ([`crate::folder`]):
    /// owned by DeckPilot, not a binding (`automatic` is set too).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub system: bool,
}

impl ControlNavigation {
    fn of(operation: NavigationOperation) -> Self {
        Self {
            operation,
            page: None,
            layout: None,
            automatic: false,
            system: false,
        }
    }
}

/// The reserved Back key among `controls` (the model's controls with a
/// display, in order): the first key, as
/// [`crate::reserved_back_control`] picks it from the capabilities.
pub fn reserved_back_among(controls: &[ControlId]) -> Option<ControlId> {
    controls
        .iter()
        .copied()
        .find(|control| control.kind() == deckpilot_device::ControlKind::Key)
}

/// The keys the runtime may put an automatic Back on: those with a
/// display, as on a device.
pub fn display_controls(capabilities: &DeviceCapabilities) -> Vec<ControlId> {
    capabilities
        .controls()
        .into_iter()
        .filter(|control| capabilities.has_display(*control))
        .collect()
}

/// What `control` on `page` of `layout` does to the view when pressed:
/// the shown binding's navigation, else the automatic Back. `controls` are
/// the model's controls with a display ([`display_controls`]). `None`: it
/// does not navigate (or its target is not set).
pub fn control_navigation(
    registry: &ActionRegistry,
    layout: &DeviceLayout,
    page: PageId,
    profile: &Profile,
    control: ControlId,
    controls: &[ControlId],
) -> Option<ControlNavigation> {
    let shown = layout.page(page)?;
    // The reserved Back wins over anything bound to its key (a layout
    // that binds it is refused when saved).
    if layout.back_position(page) == Some(crate::BackPosition::Reserved)
        && reserved_back_among(controls) == Some(control)
    {
        return Some(ControlNavigation {
            automatic: true,
            system: true,
            ..ControlNavigation::of(NavigationOperation::FolderBack)
        });
    }
    let Some(binding) = crate::device_runtime::shown_binding(shown, control) else {
        return (layout.automatic_back(page, controls) == Some(control)).then(|| {
            ControlNavigation {
                automatic: true,
                ..ControlNavigation::of(NavigationOperation::FolderBack)
            }
        });
    };
    match &binding.target {
        BindingTarget::OpenPage { page } => Some(ControlNavigation {
            page: Some(*page),
            ..ControlNavigation::of(NavigationOperation::FolderOpen)
        }),
        BindingTarget::Back => Some(ControlNavigation::of(NavigationOperation::FolderBack)),
        BindingTarget::Action { action, overrides } => {
            let instance = profile.action(*action)?;
            let declared = registry
                .definition(&instance.action_type)?
                .editor_navigation
                .as_ref()?;
            let parameters = instance.parameters.overlay(overrides);
            let text = |key: &Option<String>| {
                key.as_deref()
                    .and_then(|key| parameters.text(key))
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
            };
            let mut resolved = ControlNavigation::of(declared.operation);
            match declared.operation {
                NavigationOperation::FolderOpen => {
                    resolved.page = Some(text(&declared.target_parameter)?.parse().ok()?);
                }
                NavigationOperation::LayoutSwitch => {
                    resolved.layout = Some(text(&declared.target_parameter)?.parse().ok()?);
                    resolved.page = text(&declared.page_parameter).and_then(|p| p.parse().ok());
                }
                _ => {}
            }
            Some(resolved)
        }
        BindingTarget::Workflow { .. } | BindingTarget::Stateful(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(key: &str) -> ParameterSpec {
        ParameterSpec {
            key: key.into(),
            label: key.into(),
            description: None,
            kind: ParameterKind::Text {
                multiline: false,
                max_length: None,
            },
            required: false,
            default: None,
        }
    }

    #[test]
    fn operations_are_an_allowlist_with_dotted_names() {
        for op in NavigationOperation::ALL {
            assert_eq!(NavigationOperation::parse(op.as_str()), Some(op));
            assert_eq!(serde_json::to_value(op).unwrap(), op.as_str());
        }
        assert_eq!(NavigationOperation::parse("url.open"), None);
        assert!(serde_json::from_value::<NavigationOperation>("profile.switch".into()).is_err());
    }

    #[test]
    fn targets_must_name_a_text_parameter_when_needed() {
        let params = [text("page"), text("layout")];
        let open = EditorNavigation::with_target(NavigationOperation::FolderOpen, "page");
        assert_eq!(open.check(&params), Ok(()));
        assert!(
            EditorNavigation::new(NavigationOperation::FolderOpen)
                .check(&params)
                .is_err()
        );
        assert!(
            EditorNavigation::with_target(NavigationOperation::FolderOpen, "missing")
                .check(&params)
                .is_err()
        );
        assert!(
            EditorNavigation::with_target(NavigationOperation::PageNext, "page")
                .check(&params)
                .is_err()
        );
        let switch = EditorNavigation::with_target(NavigationOperation::LayoutSwitch, "layout")
            .with_page("page");
        assert_eq!(switch.check(&params), Ok(()));
        assert!(
            EditorNavigation::new(NavigationOperation::LayoutBack)
                .with_page("page")
                .check(&params)
                .is_err()
        );
        let boolean = ParameterSpec {
            kind: ParameterKind::Boolean,
            ..text("flag")
        };
        assert!(
            EditorNavigation::with_target(NavigationOperation::FolderOpen, "flag")
                .check(&[boolean])
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(&switch).unwrap(),
            serde_json::json!({"operation": "layout.switch", "targetParameter": "layout",
                "pageParameter": "page"})
        );
    }
}
