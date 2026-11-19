//! The `deck` module: actions that act on the deck itself rather than on
//! the computer. They move a device between the pages of its layout and
//! switch the active profile.
//!
//! A handler cannot reach the device runtime or the agent. Each action
//! returns a [`DeckRequest`], which the runtime applies to the device
//! whose key ran the action, or the agent applies to itself, after the
//! registry checked the module's [`Capability::DeckControl`]. Nothing here
//! touches the computer, the network or a file.
//!
//! - **Pages.** A layout's top-level pages (the pages in no folder, in
//!   layout order) are the pages *Next page* and *Previous page* move
//!   along. Inside a folder they are relative to the top-level page the
//!   folder belongs to, and leave the folder. *Go to page* is not an
//!   action: a key that opens a page is the `openPage` binding.
//! - **Page indicator.** Shows where the device is, as
//!   `{page.number}/{page.count}`; any key label can use those
//!   placeholders and `{page.name}`.
//! - **Profiles.** *Switch profile* activates one profile, or the next or
//!   previous one in the order profiles are listed. The agent has one
//!   active profile: every device follows.
//! - **Layouts.** *Switch layout* shows another layout of the active
//!   profile on the device whose key was pressed, and *Back to previous
//!   layout* returns to the one before; the profile stays. Distinct from
//!   *Next page*/*Previous page* (pages of one layout), folders and their
//!   Back key (a page inside a layout) and *Switch profile*.
//!
//! - **Random action.** Runs one of several saved actions or workflows,
//!   picked at random (see [`random`]).
//! - **Multi Action Switch.** Goes through two to eight states in turn,
//!   each running an action or workflow, and shows the current one (see
//!   [`switch`]).
//!
//! These actions run from a key or from the app. As a workflow node they
//! fail with `not-from-a-key`: nothing applies a request there.
//!
//! Error codes are listed in [`codes`].

use std::collections::BTreeMap;

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    DeckRequest, ExecutionContext, FeatureModule, HistoryMode, KeyFace, LayoutId, ModuleId,
    ModuleManifest, PageId, ParameterError, ParameterKind, ParameterSpec, ParameterValue,
    Portability, ProfileId, ProfileTarget, Project,
};

pub const MODULE_ID: &str = "deck";
pub const NEXT_PAGE: &str = "deck.next-page";
pub const PREVIOUS_PAGE: &str = "deck.previous-page";
pub const PAGE_INDICATOR: &str = "deck.page-indicator";
pub const SWITCH_PROFILE: &str = "deck.switch-profile";
pub const BRIGHTNESS: &str = "deck.brightness";
pub const RANDOM: &str = "deck.random";
pub const MULTI_SWITCH: &str = "deck.multi-switch";
/// *Switch layout*: shows another layout of the active profile on the
/// device whose key was pressed.
pub const SWITCH_LAYOUT: &str = "deck.switch-layout";
/// *Back to previous layout*: shows the layout the device showed before.
pub const PREVIOUS_LAYOUT: &str = "deck.previous-layout";

/// Parameter of *Switch layout*: the layout's id (never its name).
pub const LAYOUT: &str = "layout";
/// Parameter of *Switch layout*: the id of the page to show; the home page
/// when empty.
pub const PAGE: &str = "page";
/// Parameter of *Switch layout*: what Back does afterwards ([`HISTORIES`]).
pub const HISTORY: &str = "history";
pub const HISTORY_PUSH: &str = "push";
pub const HISTORY_REPLACE: &str = "replace";
pub const HISTORY_CLEAR: &str = "clear";
/// The values of [`HISTORY`].
pub const HISTORIES: [&str; 3] = [HISTORY_PUSH, HISTORY_REPLACE, HISTORY_CLEAR];

pub mod random;
pub mod switch;

/// Parameter of *Brightness*: what to do ([`CHANGES`]).
pub const CHANGE: &str = "change";
/// Parameter of *Brightness*: the level for [`CHANGE_SET`], in percent.
pub const LEVEL: &str = "level";
/// Parameter of *Brightness*: how far brighter and dimmer move, in
/// percentage points.
pub const STEP: &str = "step";
pub const CHANGE_SET: &str = "set";
pub const CHANGE_BRIGHTER: &str = "brighter";
pub const CHANGE_DIMMER: &str = "dimmer";
/// The values of [`CHANGE`].
pub const CHANGES: [&str; 3] = [CHANGE_SET, CHANGE_BRIGHTER, CHANGE_DIMMER];

/// Parameter of the page actions and of *Switch profile*: past either end,
/// continue at the other one.
pub const WRAP_AROUND: &str = "wrapAround";
/// Parameter of *Switch profile*: which profile ([`TARGETS`]).
pub const TARGET: &str = "target";
/// Parameter of *Switch profile*: the profile's id, for the target
/// [`TARGET_PROFILE`].
pub const PROFILE: &str = "profile";

pub const TARGET_NEXT: &str = "next";
pub const TARGET_PREVIOUS: &str = "previous";
pub const TARGET_PROFILE: &str = "profile";
/// The values of [`TARGET`].
pub const TARGETS: [&str; 3] = [TARGET_NEXT, TARGET_PREVIOUS, TARGET_PROFILE];

/// What the page indicator shows unless its label says otherwise.
pub const INDICATOR_LABEL: &str = "{page.number}/{page.count}";

/// Stable error codes of the module's actions.
pub mod codes {
    /// *Switch profile* is set to a specific profile, and none is chosen.
    pub const PROFILE_MISSING: &str = "profile-missing";
    /// The profile to switch to does not exist (any more). Reported by the
    /// agent when it applies the request.
    pub const PROFILE_NOT_FOUND: &str = "profile-not-found";
    /// There is no other profile to switch to. Reported by the agent.
    pub const NO_OTHER_PROFILE: &str = "no-other-profile";
    /// *Switch layout* has no layout, or not a layout id.
    pub const LAYOUT_MISSING: &str = "layout-missing";
    /// The layout to switch to is not assigned to the active profile (or
    /// is gone). Reported by the device runtime.
    pub const LAYOUT_UNAVAILABLE: &str = deckpilot_core::device_runtime::LAYOUT_UNAVAILABLE;
    /// The layout is made for another device model. Reported by the device
    /// runtime.
    pub const LAYOUT_INCOMPATIBLE: &str = deckpilot_core::device_runtime::LAYOUT_INCOMPATIBLE;
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

pub(crate) fn type_id(id: &str) -> ActionTypeId {
    ActionTypeId::new(id).expect("the action type id is valid")
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn next_page_id() -> ActionTypeId {
    type_id(NEXT_PAGE)
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn previous_page_id() -> ActionTypeId {
    type_id(PREVIOUS_PAGE)
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn page_indicator_id() -> ActionTypeId {
    type_id(PAGE_INDICATOR)
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn switch_profile_id() -> ActionTypeId {
    type_id(SWITCH_PROFILE)
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn brightness_id() -> ActionTypeId {
    type_id(BRIGHTNESS)
}

pub fn random_id() -> ActionTypeId {
    type_id(RANDOM)
}

pub fn multi_switch_id() -> ActionTypeId {
    type_id(MULTI_SWITCH)
}

pub fn switch_layout_id() -> ActionTypeId {
    type_id(SWITCH_LAYOUT)
}

pub fn previous_layout_id() -> ActionTypeId {
    type_id(PREVIOUS_LAYOUT)
}

#[derive(Debug)]
pub struct DeckModule {
    manifest: ModuleManifest,
}

impl DeckModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "Deck".into(),
                description:
                    "Moves between the pages and layouts of a device, shows the page a device is \
                              on and switches profiles from a key."
                        .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                // Reading the profile: the candidates of a Random Action.
                capabilities: vec![Capability::DeckControl, Capability::ProfileRead],
                config: Vec::new(),
                dependencies: Vec::new(),
                enabled_by_default: true,
                // Only the enabled state travels; there are no settings.
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::new(),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
                variables: Vec::new(),
            },
        }
    }
}

impl Default for DeckModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for DeckModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, _config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        vec![
            Box::new(TurnPage::next()),
            Box::new(TurnPage::previous()),
            Box::new(PageIndicator::new()),
            Box::new(SwitchProfile::new()),
            Box::new(Brightness::new()),
            Box::new(random::RandomAction::new()),
            Box::new(switch::MultiSwitch::new()),
            Box::new(SwitchLayout::new()),
            Box::new(PreviousLayout::new()),
        ]
    }
}

fn wrap_around(description: &str) -> ParameterSpec {
    ParameterSpec {
        key: WRAP_AROUND.into(),
        label: "Wrap around".into(),
        description: Some(description.into()),
        kind: ParameterKind::Boolean,
        required: true,
        default: Some(ParameterValue::Boolean(true)),
    }
}

/// What the deck shows changes; nothing on the computer or elsewhere does.
fn view_node(icon: &str, outputs: Vec<OutputSpec>) -> NodeSpec {
    NodeSpec {
        icon: Some(icon.into()),
        outputs,
        timeout_ms: 5_000,
        retry_safe: true,
        idempotent: true,
        effect: ActionEffect::ReadOnly,
        ..NodeSpec::default()
    }
}

pub(crate) fn output(key: &str, label: &str) -> OutputSpec {
    OutputSpec {
        key: key.into(),
        label: label.into(),
        kind: ValueKind::Text,
        always: false,
        sensitive: false,
    }
}

/// `deck.next-page` and `deck.previous-page`: show the next or previous
/// top-level page of the layout on the device whose key was pressed.
#[derive(Debug)]
pub struct TurnPage {
    definition: ActionType,
    forward: bool,
}

impl TurnPage {
    pub fn next() -> Self {
        Self::new(
            next_page_id(),
            "Next page",
            "Shows the next page of the layout on the device.",
            "chevron-right",
            "After the last page, continue with the first one.",
            true,
        )
    }

    pub fn previous() -> Self {
        Self::new(
            previous_page_id(),
            "Previous page",
            "Shows the previous page of the layout on the device.",
            "chevron-left",
            "Before the first page, continue with the last one.",
            false,
        )
    }

    fn new(
        id: ActionTypeId,
        name: &str,
        description: &str,
        icon: &str,
        wrap: &str,
        forward: bool,
    ) -> Self {
        Self {
            definition: ActionType {
                id,
                name: name.into(),
                description: description.into(),
                category: "Navigation".into(),
                parameters: vec![wrap_around(wrap)],
                requires_project: false,
                deprecated: false,
                // One page per event: a key, or a dial's detent or press.
                controls: Some(deckpilot_core::ActionControls::keys_and_steps()),
                // Set by the runtime when the device moved.
                node: view_node(
                    icon,
                    vec![output("pageId", "Page id"), output("pageName", "Page")],
                ),
            },
            forward,
        }
    }
}

impl ActionHandler for TurnPage {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Err(denied) = context.permit(Capability::DeckControl) {
            return denied.into();
        }
        let wrap_around = parameters.boolean(WRAP_AROUND).unwrap_or(true);
        let (message, request) = if self.forward {
            ("next page", DeckRequest::NextPage { wrap_around })
        } else {
            ("previous page", DeckRequest::PreviousPage { wrap_around })
        };
        ActionResult::success(message).with_request(request)
    }
}

/// `deck.page-indicator`: a key that shows where the device is among the
/// layout's pages. Pressing it does nothing.
#[derive(Debug)]
pub struct PageIndicator(ActionType);

impl PageIndicator {
    pub fn new() -> Self {
        Self(ActionType {
            id: page_indicator_id(),
            name: "Page indicator".into(),
            description: "Shows the page the device is on, such as 2/5. A label with \
                          {page.number}, {page.count} or {page.name} replaces it."
                .into(),
            category: "Navigation".into(),
            parameters: Vec::new(),
            requires_project: false,
            deprecated: false,
            controls: Some(deckpilot_core::ActionControls::keys()),
            node: view_node("layers", Vec::new()),
        })
    }
}

impl Default for PageIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for PageIndicator {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        _parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Err(denied) = context.permit(Capability::DeckControl) {
            return denied.into();
        }
        ActionResult::success("page indicator")
    }

    /// The runtime expands the page placeholders of every face; a label
    /// without one is replaced by the position.
    fn decorate_face(
        &self,
        mut face: KeyFace,
        _parameters: &ActionParameters,
        template: &str,
        _project: Option<&Project>,
    ) -> KeyFace {
        if !template.contains("{page.") {
            INDICATOR_LABEL.clone_into(&mut face.title);
        }
        face
    }
}

/// `deck.switch-profile`: activates a profile, for every device.
#[derive(Debug)]
pub struct SwitchProfile(ActionType);

impl SwitchProfile {
    pub fn new() -> Self {
        Self(ActionType {
            id: switch_profile_id(),
            name: "Switch profile".into(),
            description: "Makes another profile the active one: a specific profile, or the next \
                          or previous one. Every connected device shows it."
                .into(),
            category: "Navigation".into(),
            parameters: vec![
                ParameterSpec {
                    key: TARGET.into(),
                    label: "Switch to".into(),
                    description: Some(
                        "The next or previous profile in the order of the Profiles page, or one \
                         profile."
                            .into(),
                    ),
                    kind: ParameterKind::Choice {
                        options: TARGETS.iter().map(|&t| t.to_owned()).collect(),
                    },
                    required: true,
                    default: Some(ParameterValue::Choice(TARGET_NEXT.into())),
                },
                ParameterSpec {
                    key: PROFILE.into(),
                    label: "Profile".into(),
                    description: Some("Used when switching to one profile.".into()),
                    kind: ParameterKind::Text {
                        multiline: false,
                        max_length: Some(36),
                    },
                    required: false,
                    default: None,
                },
                wrap_around(
                    "Past the last or first profile, continue at the other end. Used for next \
                     and previous.",
                ),
            ],
            requires_project: false,
            deprecated: false,
            controls: Some(deckpilot_core::ActionControls::keys()),
            // The active profile is the agent's state. The outputs are set
            // by the agent when it switched.
            node: NodeSpec {
                icon: Some("user".into()),
                outputs: vec![
                    output("profileId", "Profile id"),
                    output("profileName", "Profile"),
                ],
                timeout_ms: 5_000,
                effect: ActionEffect::Modifies,
                ..NodeSpec::default()
            },
        })
    }

    /// The profile `parameters` name, or what is wrong with them.
    fn target(parameters: &ActionParameters) -> Result<ProfileTarget, String> {
        match parameters.text(TARGET).unwrap_or(TARGET_NEXT) {
            TARGET_PREVIOUS => Ok(ProfileTarget::Previous),
            TARGET_PROFILE => {
                let id = parameters.text(PROFILE).unwrap_or_default().trim();
                if id.is_empty() {
                    return Err("choose the profile to switch to".into());
                }
                id.parse::<ProfileId>()
                    .map(ProfileTarget::Profile)
                    .map_err(|_| "the profile is not a profile id".to_owned())
            }
            _ => Ok(ProfileTarget::Next),
        }
    }
}

impl Default for SwitchProfile {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for SwitchProfile {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn validate(&self, parameters: &ActionParameters) -> Vec<ParameterError> {
        match Self::target(parameters) {
            Ok(_) => Vec::new(),
            Err(reason) => vec![ParameterError::Invalid {
                key: PROFILE.into(),
                reason,
            }],
        }
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Err(denied) = context.permit(Capability::DeckControl) {
            return denied.into();
        }
        match Self::target(parameters) {
            Ok(target) => {
                ActionResult::success("switch profile").with_request(DeckRequest::SwitchProfile {
                    target,
                    wrap_around: parameters.boolean(WRAP_AROUND).unwrap_or(true),
                })
            }
            Err(reason) => ActionResult::error(codes::PROFILE_MISSING, reason),
        }
    }
}

/// `deck.switch-layout` (*Switch layout*): shows another layout of the
/// active profile on the device whose key was pressed, without changing
/// the profile, its projects or variables. The layout is named by its id;
/// the device runtime checks that it is assigned to the active profile and
/// made for the device's model, and keeps the way back (see
/// `deckpilot_core::LayoutHistory`). Keys only, pressed: not on encoders
/// or touch strips, which have no interaction mapped to it.
#[derive(Debug)]
pub struct SwitchLayout(ActionType);

impl SwitchLayout {
    pub fn new() -> Self {
        Self(ActionType {
            id: switch_layout_id(),
            name: "Switch layout".into(),
            description: "Shows another layout of the active profile on this device. The \
                          profile, its projects and variables stay as they are; Back to \
                          previous layout returns."
                .into(),
            category: "Navigation".into(),
            parameters: vec![
                ParameterSpec {
                    key: LAYOUT.into(),
                    label: "Layout".into(),
                    description: Some(
                        "A layout assigned to the profile and made for this device.".into(),
                    ),
                    kind: ParameterKind::Text {
                        multiline: false,
                        max_length: Some(36),
                    },
                    required: true,
                    default: None,
                },
                ParameterSpec {
                    key: PAGE.into(),
                    label: "Page".into(),
                    description: Some(
                        "The page to show; its home page when empty or no longer there.".into(),
                    ),
                    kind: ParameterKind::Text {
                        multiline: false,
                        max_length: Some(36),
                    },
                    required: false,
                    default: None,
                },
                ParameterSpec {
                    key: HISTORY.into(),
                    label: "Back".into(),
                    description: Some(
                        "push: Back to previous layout returns to the layout left. replace: \
                         Back skips it. clear: Back has nowhere to go afterwards."
                            .into(),
                    ),
                    kind: ParameterKind::Choice {
                        options: HISTORIES.iter().map(|&h| h.to_owned()).collect(),
                    },
                    required: true,
                    default: Some(ParameterValue::Choice(HISTORY_PUSH.into())),
                },
            ],
            requires_project: false,
            deprecated: false,
            controls: Some(deckpilot_core::ActionControls::keys()),
            node: NodeSpec {
                icon: Some("layout-grid".into()),
                outputs: vec![
                    output("layoutId", "Layout id"),
                    output("layoutName", "Layout"),
                ],
                timeout_ms: 5_000,
                retry_safe: true,
                idempotent: true,
                effect: ActionEffect::ReadOnly,
                ..NodeSpec::default()
            },
        })
    }

    /// The request `parameters` ask for, or what is wrong with them.
    fn request(parameters: &ActionParameters) -> Result<DeckRequest, (String, String)> {
        let layout = parameters.text(LAYOUT).unwrap_or_default().trim();
        if layout.is_empty() {
            return Err((LAYOUT.into(), "choose the layout to switch to".into()));
        }
        let layout = layout.parse::<LayoutId>().map_err(|_| {
            (
                LAYOUT.to_owned(),
                "the layout is not a layout id".to_owned(),
            )
        })?;
        let page = match parameters.text(PAGE).unwrap_or_default().trim() {
            "" => None,
            page => Some(
                page.parse::<PageId>()
                    .map_err(|_| (PAGE.to_owned(), "the page is not a page id".to_owned()))?,
            ),
        };
        let history = parameters
            .text(HISTORY)
            .and_then(HistoryMode::parse)
            .unwrap_or_default();
        Ok(DeckRequest::SwitchLayout {
            layout,
            page,
            history,
        })
    }
}

impl Default for SwitchLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for SwitchLayout {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn validate(&self, parameters: &ActionParameters) -> Vec<ParameterError> {
        match Self::request(parameters) {
            Ok(_) => Vec::new(),
            Err((key, reason)) => vec![ParameterError::Invalid { key, reason }],
        }
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Err(denied) = context.permit(Capability::DeckControl) {
            return denied.into();
        }
        match Self::request(parameters) {
            Ok(request) => ActionResult::success("switch layout").with_request(request),
            Err((_, reason)) => ActionResult::error(codes::LAYOUT_MISSING, reason),
        }
    }
}

/// `deck.previous-layout` (*Back to previous layout*): shows the layout the
/// device showed before the current one in this profile, at the page it
/// showed. With nothing to go back to it does nothing. Not the Back key of
/// a folder, which returns to the folder's page within one layout.
#[derive(Debug)]
pub struct PreviousLayout(ActionType);

impl PreviousLayout {
    pub fn new() -> Self {
        Self(ActionType {
            id: previous_layout_id(),
            name: "Back to previous layout".into(),
            description: "Shows the layout this device showed before the current one, in this \
                          profile. Does nothing when there is none. (A folder's Back key \
                          returns to a page instead.)"
                .into(),
            category: "Navigation".into(),
            parameters: Vec::new(),
            requires_project: false,
            deprecated: false,
            controls: Some(deckpilot_core::ActionControls::keys()),
            node: view_node(
                "arrow-left",
                vec![
                    output("layoutId", "Layout id"),
                    output("layoutName", "Layout"),
                ],
            ),
        })
    }
}

impl Default for PreviousLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for PreviousLayout {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        _parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Err(denied) = context.permit(Capability::DeckControl) {
            return denied.into();
        }
        ActionResult::success("previous layout").with_request(DeckRequest::PreviousLayout)
    }
}

/// `deck.brightness`: sets the display brightness of the devices, or makes
/// them brighter or dimmer by a step. It is the setting of the app's
/// Devices page; devices without a brightness are left alone.
#[derive(Debug)]
pub struct Brightness(ActionType);

impl Brightness {
    pub fn new() -> Self {
        let percent = |key: &str, label: &str, description: &str, min, default| ParameterSpec {
            key: key.into(),
            label: label.into(),
            description: Some(description.into()),
            kind: ParameterKind::Integer {
                min: Some(min),
                max: Some(100),
            },
            required: true,
            default: Some(ParameterValue::Integer(default)),
        };
        Self(ActionType {
            id: brightness_id(),
            name: "Brightness".into(),
            description: "Sets the display brightness of the connected devices to a level, or \
                          makes them brighter or dimmer by a step."
                .into(),
            category: "Device".into(),
            parameters: vec![
                ParameterSpec {
                    key: CHANGE.into(),
                    label: "Change".into(),
                    description: Some(
                        "Set a level, or step brighter or dimmer from the current one.".into(),
                    ),
                    kind: ParameterKind::Choice {
                        options: CHANGES.iter().map(|&c| c.to_owned()).collect(),
                    },
                    required: true,
                    default: Some(ParameterValue::Choice(CHANGE_SET.into())),
                },
                percent(
                    LEVEL,
                    "Level (%)",
                    "Used when setting a level. 0 is dark.",
                    0,
                    100,
                ),
                percent(
                    STEP,
                    "Step (%)",
                    "Used for brighter and dimmer; the brightness stays within 0 to 100.",
                    1,
                    10,
                ),
            ],
            requires_project: false,
            deprecated: false,
            // One step (or the set level) per event: a key, or a dial's
            // detent or press. The rotation's amount is not used.
            controls: Some(deckpilot_core::ActionControls::keys_and_steps()),
            // Changes a setting of this computer. The output is set by the
            // agent when it applied the change.
            node: NodeSpec {
                icon: Some("monitor".into()),
                outputs: vec![output("brightness", "Brightness (%)")],
                timeout_ms: 5_000,
                effect: ActionEffect::Modifies,
                ..NodeSpec::default()
            },
        })
    }
}

impl Default for Brightness {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for Brightness {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if let Err(denied) = context.permit(Capability::DeckControl) {
            return denied.into();
        }
        let percent = |key: &str, fallback: i64| {
            u8::try_from(parameters.integer(key).unwrap_or(fallback).clamp(0, 100)).unwrap_or(100)
        };
        let step = i8::try_from(percent(STEP, 10)).unwrap_or(100);
        let request = match parameters.text(CHANGE).unwrap_or(CHANGE_SET) {
            CHANGE_BRIGHTER => DeckRequest::Brightness { set: None, step },
            CHANGE_DIMMER => DeckRequest::Brightness {
                set: None,
                step: -step,
            },
            _ => DeckRequest::Brightness {
                set: Some(percent(LEVEL, 100)),
                step: 0,
            },
        };
        ActionResult::success("brightness").with_request(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use deckpilot_core::{FaceStyle, MachineConfig, Os, Profile};

    fn run(
        handler: &dyn ActionHandler,
        parameters: &ActionParameters,
        granted: &BTreeSet<Capability>,
    ) -> ActionResult {
        let profile = Profile::new("Work");
        let machine = MachineConfig::new(Os::Linux);
        let context = ExecutionContext::new(&profile, None, &machine);
        let parameters = handler.definition().normalize(parameters).unwrap();
        handler.execute(&parameters, &context.restricted(granted))
    }

    fn granted() -> BTreeSet<Capability> {
        BTreeSet::from([Capability::DeckControl])
    }

    #[test]
    fn action_types_are_namespaced_and_listed() {
        let module = DeckModule::new();
        let ids: Vec<String> = module
            .action_handlers(&ActionParameters::new())
            .iter()
            .map(|handler| handler.definition().id.to_string())
            .collect();
        assert_eq!(
            ids,
            [
                NEXT_PAGE,
                PREVIOUS_PAGE,
                PAGE_INDICATOR,
                SWITCH_PROFILE,
                BRIGHTNESS,
                RANDOM,
                MULTI_SWITCH
            ]
        );
        assert!(ids.iter().all(|id| id.starts_with("deck.")));
        assert_eq!(
            module.manifest().capabilities,
            [Capability::DeckControl, Capability::ProfileRead]
        );
    }

    #[test]
    fn page_actions_ask_the_runtime_and_wrap_by_default() {
        let next = run(&TurnPage::next(), &ActionParameters::new(), &granted());
        assert!(next.is_success());
        assert_eq!(
            next.deck_request(),
            Some(DeckRequest::NextPage { wrap_around: true })
        );
        let previous = run(
            &TurnPage::previous(),
            &ActionParameters::new().with(WRAP_AROUND, ParameterValue::Boolean(false)),
            &granted(),
        );
        assert_eq!(
            previous.deck_request(),
            Some(DeckRequest::PreviousPage { wrap_around: false })
        );
    }

    #[test]
    fn nothing_is_asked_without_the_capability() {
        let nothing = BTreeSet::new();
        for handler in DeckModule::new().action_handlers(&ActionParameters::new()) {
            // A Random Action and a switch need something to run to be run
            // at all.
            let key = match handler.definition().id.as_str() {
                RANDOM => Some(random::CANDIDATES),
                MULTI_SWITCH => Some(switch::STATES),
                _ => None,
            };
            let parameters = if let Some(key) = key {
                ActionParameters::new().with(
                    key,
                    ParameterValue::Text(format!(
                        "action:{}",
                        deckpilot_core::ActionInstanceId::new()
                    )),
                )
            } else {
                ActionParameters::new()
            };
            let result = run(handler.as_ref(), &parameters, &nothing);
            assert!(!result.is_success(), "{}", handler.definition().id);
            assert_eq!(result.code.as_deref(), Some("not-permitted"));
            assert!(result.request.is_none());
        }
    }

    #[test]
    fn the_indicator_shows_the_position_unless_the_label_does() {
        let indicator = PageIndicator::new();
        let face = || KeyFace::new(FaceStyle::Action, "Page indicator");
        let none = ActionParameters::new();
        assert_eq!(
            indicator
                .decorate_face(face(), &none, "Page indicator", None)
                .title,
            INDICATOR_LABEL
        );
        // A label of the user's own with a placeholder is kept.
        assert_eq!(
            indicator
                .decorate_face(face(), &none, "{page.name}", None)
                .title,
            "Page indicator"
        );
        let pressed = run(&indicator, &none, &granted());
        assert!(pressed.is_success());
        assert!(pressed.request.is_none());
    }

    #[test]
    fn switch_profile_names_its_target() {
        let switch = SwitchProfile::new();
        let next = run(&switch, &ActionParameters::new(), &granted());
        assert_eq!(
            next.deck_request(),
            Some(DeckRequest::SwitchProfile {
                target: ProfileTarget::Next,
                wrap_around: true,
            })
        );
        let previous = run(
            &switch,
            &ActionParameters::new()
                .with(TARGET, ParameterValue::Choice(TARGET_PREVIOUS.into()))
                .with(WRAP_AROUND, ParameterValue::Boolean(false)),
            &granted(),
        );
        assert_eq!(
            previous.deck_request(),
            Some(DeckRequest::SwitchProfile {
                target: ProfileTarget::Previous,
                wrap_around: false,
            })
        );
        let id = ProfileId::new();
        let one = run(
            &switch,
            &ActionParameters::new()
                .with(TARGET, ParameterValue::Choice(TARGET_PROFILE.into()))
                .with(PROFILE, ParameterValue::Text(id.to_string())),
            &granted(),
        );
        assert_eq!(
            one.deck_request(),
            Some(DeckRequest::SwitchProfile {
                target: ProfileTarget::Profile(id),
                wrap_around: true,
            })
        );
    }

    #[test]
    fn brightness_asks_for_a_level_or_a_step() {
        let brightness = Brightness::new();
        let change = |change: &str| {
            ActionParameters::new().with(CHANGE, ParameterValue::Choice(change.into()))
        };
        // The default: full brightness.
        assert_eq!(
            run(&brightness, &ActionParameters::new(), &granted()).deck_request(),
            Some(DeckRequest::Brightness {
                set: Some(100),
                step: 0
            })
        );
        assert_eq!(
            run(
                &brightness,
                &change(CHANGE_SET).with(LEVEL, ParameterValue::Integer(35)),
                &granted()
            )
            .deck_request(),
            Some(DeckRequest::Brightness {
                set: Some(35),
                step: 0
            })
        );
        assert_eq!(
            run(&brightness, &change(CHANGE_BRIGHTER), &granted()).deck_request(),
            Some(DeckRequest::Brightness {
                set: None,
                step: 10
            })
        );
        assert_eq!(
            run(
                &brightness,
                &change(CHANGE_DIMMER).with(STEP, ParameterValue::Integer(25)),
                &granted()
            )
            .deck_request(),
            Some(DeckRequest::Brightness {
                set: None,
                step: -25
            })
        );
        // Levels outside 0-100 are refused when the action is saved.
        assert!(
            brightness
                .definition()
                .normalize(&change(CHANGE_SET).with(LEVEL, ParameterValue::Integer(101)))
                .is_err()
        );
    }

    #[test]
    fn a_specific_profile_must_be_chosen() {
        let switch = SwitchProfile::new();
        let to_profile =
            ActionParameters::new().with(TARGET, ParameterValue::Choice(TARGET_PROFILE.into()));
        let problems = switch.validate(&switch.definition().normalize(&to_profile).unwrap());
        assert!(
            matches!(&problems[..], [ParameterError::Invalid { key, .. }] if key == PROFILE),
            "{problems:?}"
        );
        let run_anyway = run(&switch, &to_profile, &granted());
        assert_eq!(run_anyway.code.as_deref(), Some(codes::PROFILE_MISSING));
        assert!(run_anyway.request.is_none());

        let not_an_id = to_profile.with(PROFILE, ParameterValue::Text("Work".into()));
        assert_eq!(
            switch
                .validate(&switch.definition().normalize(&not_an_id).unwrap())
                .len(),
            1
        );
        // The profile is not needed for next and previous.
        let defaults = switch
            .definition()
            .normalize(&ActionParameters::new())
            .unwrap();
        assert_eq!(switch.validate(&defaults), Vec::new());
    }
}
