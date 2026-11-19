//! Layouts as resources of their own, shared by profiles.
//!
//! A [`DeviceLayout`] (pages, folders, controls and what each control runs)
//! is stored once and *assigned* to any number of profiles; each profile
//! keeps its own projects, variables, workflows, default layout per model
//! and page state. What a layout needs from the world is derived from it,
//! never stored twice:
//!
//! - the **actions** its bindings run ([`DeviceLayout::bound_actions`]),
//!   records of the shared Action Library, which a profile that uses the
//!   layout sees in its library ([`action_closure`] adds the actions those
//!   actions name, e.g. the states of a Multi Action Switch);
//! - the **icons** its keys show ([`DeviceLayout::shown_icons`]);
//! - the **controls and inputs** it binds ([`DeviceLayout::requirements`]),
//!   checked against a device model ([`compatibility`]);
//! - what depends on the profile it runs in ([`availability`]): workflows
//!   and projects belong to profiles, so a key that runs a workflow, or
//!   names a project, works in the profiles that have it and is shown as
//!   unavailable in the others. Nothing of another profile is ever used.
//!
//! The navigation between layouts on one device (*Switch layout*, *Back to
//! previous layout*) keeps a bounded history per profile and device
//! ([`LayoutHistory`]); see `docs/data-model.md`, "Shared layouts".

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use deckpilot_device::{ControlId, ControlInput, DeviceModel, DeviceModelId, ModelCatalog};

use crate::{
    ActionInstance, ActionInstanceId, BindingTarget, DeviceLayout, IconId, LayoutId, PageId,
    Profile, Trigger, WorkflowId,
};

impl DeviceLayout {
    /// The actions the layout's bindings run, in page and binding order,
    /// each once.
    pub fn bound_actions(&self) -> Vec<ActionInstanceId> {
        let mut seen = BTreeSet::new();
        self.pages
            .iter()
            .flat_map(|page| &page.bindings)
            .filter_map(|binding| binding.target.action_id())
            .filter(|id| seen.insert(*id))
            .collect()
    }

    /// The workflows the layout's bindings start, each once.
    pub fn bound_workflows(&self) -> Vec<WorkflowId> {
        let mut seen = BTreeSet::new();
        self.pages
            .iter()
            .flat_map(|page| &page.bindings)
            .filter_map(|binding| match binding.target {
                BindingTarget::Workflow { workflow } => Some(workflow),
                _ => None,
            })
            .filter(|id| seen.insert(*id))
            .collect()
    }

    /// The icons the layout's keys show, each once.
    pub fn shown_icons(&self) -> Vec<IconId> {
        let mut seen = BTreeSet::new();
        self.pages
            .iter()
            .flat_map(|page| &page.bindings)
            .filter_map(|binding| binding.appearance.as_ref()?.icon.map(|icon| icon.id))
            .filter(|id| seen.insert(*id))
            .collect()
    }

    /// An independent copy named `name`: a new layout id and new page ids,
    /// with the links between its pages (folders, page keys, parents) and
    /// its home page following the copies. Its keys run the same Action
    /// Library actions, show the same icons and start the same workflows
    /// as the original's; navigation keys (*Switch layout*) keep pointing
    /// at the layouts they named, also when that is the original.
    #[must_use]
    pub fn duplicated(&self, name: &str) -> DeviceLayout {
        let mut copy = self.clone();
        let pages: std::collections::HashMap<PageId, PageId> = copy
            .pages
            .iter()
            .map(|page| (page.id, PageId::new()))
            .collect();
        crate::profile::remap_layout(
            &mut copy,
            LayoutId::new(),
            &pages,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        );
        name.trim().clone_into(&mut copy.name);
        copy
    }

    /// What a device needs to show and run this layout: every control it
    /// binds, with the triggers bound on it. Derived from the bindings, so
    /// it is always up to date.
    pub fn requirements(&self) -> LayoutRequirements {
        let mut controls: BTreeMap<ControlId, BTreeSet<TriggerKey>> = BTreeMap::new();
        for binding in self.pages.iter().flat_map(|page| &page.bindings) {
            controls
                .entry(binding.control)
                .or_default()
                .insert(TriggerKey(binding.trigger));
        }
        LayoutRequirements {
            model: self.model.clone(),
            controls: controls
                .into_iter()
                .map(|(control, triggers)| RequiredControl {
                    control,
                    triggers: triggers.into_iter().map(|key| key.0).collect(),
                })
                .collect(),
        }
    }
}

/// [`Trigger`] ordered by its serialized form, for sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TriggerKey(Trigger);

impl TriggerKey {
    fn rank(self) -> String {
        serde_json::to_string(&self.0).unwrap_or_default()
    }
}

impl PartialOrd for TriggerKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TriggerKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

/// What a layout binds ([`DeviceLayout::requirements`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutRequirements {
    /// The model the layout was made for.
    pub model: DeviceModelId,
    pub controls: Vec<RequiredControl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequiredControl {
    pub control: ControlId,
    pub triggers: Vec<Trigger>,
}

/// How well a layout fits a device model ([`compatibility`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ModelFit {
    /// The layout was made for this model: devices of the model can use
    /// it. The only fit the runtime and the assignment of a default or
    /// active layout accept.
    Native,
    /// Another model that has every control and input the layout binds.
    /// The layout is not used on it as it is (the model's other controls
    /// would stay empty and its key images differ); it could be copied for
    /// it.
    Fits,
    /// Another model that lacks controls or inputs the layout binds.
    Missing {
        controls: Vec<ControlId>,
        /// Bindings for an input their control has on the layout's model
        /// but not on this one.
        triggers: usize,
    },
    /// The model is not in the catalog of this build. The layout is kept
    /// unchanged and is not used.
    UnknownModel,
}

impl ModelFit {
    /// Whether devices of the model may use the layout.
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Native)
    }
}

/// The fit of `layout` on one model, with the reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutCompatibility {
    pub model: DeviceModelId,
    pub model_name: String,
    pub fit: ModelFit,
    /// Why, for the user: one sentence.
    pub explanation: String,
}

/// How `layout` fits `model`. A layout is only ever used on the model it
/// was made for: a Stream Deck MK.2 layout never runs on a Stream Deck +,
/// even when the + has the keys, because their images and the controls
/// around them differ. Other models are reported as fitting or not, so
/// the app can explain why a layout is not offered for a device.
pub fn compatibility(layout: &DeviceLayout, model: &DeviceModel) -> LayoutCompatibility {
    let explain = |fit: ModelFit, explanation: String| LayoutCompatibility {
        model: model.id.clone(),
        model_name: model.name.clone(),
        fit,
        explanation,
    };
    if layout.model == model.id {
        return explain(ModelFit::Native, format!("Made for the {}.", model.name));
    }
    let mut missing = BTreeSet::new();
    let mut triggers = 0;
    for binding in layout.pages.iter().flat_map(|page| &page.bindings) {
        if !model.capabilities.has_control(binding.control) {
            missing.insert(binding.control);
            continue;
        }
        let gesture = binding.trigger.gesture().is_some();
        let unsupported = model
            .capabilities
            .validate_input(binding.control, binding.trigger.sample_input())
            .is_err()
            || (gesture
                && model
                    .capabilities
                    .validate_input(binding.control, ControlInput::Release)
                    .is_err());
        if unsupported {
            triggers += 1;
        }
    }
    if missing.is_empty() && triggers == 0 {
        explain(
            ModelFit::Fits,
            format!(
                "The {} has every control this layout uses, but the layout is made for another \
                 model; duplicate it for the {} to use it there.",
                model.name, model.name
            ),
        )
    } else {
        let count = missing.len();
        explain(
            ModelFit::Missing {
                controls: missing.into_iter().collect(),
                triggers,
            },
            if count > 0 {
                format!(
                    "The {} lacks {count} control(s) this layout uses.",
                    model.name
                )
            } else {
                format!(
                    "{triggers} binding(s) of this layout use an input the {} does not have.",
                    model.name
                )
            },
        )
    }
}

/// The fit of `layout` on every model of `catalog`, its own model first.
/// A model the catalog does not know is reported as [`ModelFit::UnknownModel`].
pub fn compatibility_report(
    layout: &DeviceLayout,
    catalog: &ModelCatalog,
) -> Vec<LayoutCompatibility> {
    let mut report: Vec<LayoutCompatibility> = catalog
        .iter()
        .map(|model| compatibility(layout, model))
        .collect();
    if catalog.get(&layout.model).is_none() {
        report.push(LayoutCompatibility {
            model: layout.model.clone(),
            model_name: layout.model.to_string(),
            fit: ModelFit::UnknownModel,
            explanation: "This build does not know the layout's device model; the layout is \
                          kept unchanged and not used."
                .into(),
        });
    }
    report.sort_by_key(|entry| !matches!(entry.fit, ModelFit::Native | ModelFit::UnknownModel));
    report
}

/// Whether the serialized `json` names `id`. Identifiers are UUIDs, so a
/// textual match is a reference: in a binding, a workflow step or node, or
/// a text parameter such as a Multi Action Switch's `action:<id>` lines.
pub fn mentions(json: &str, id: &str) -> bool {
    json.contains(id)
}

/// `start` and every action of `library` that those actions name in their
/// parameters (the states of a Multi Action Switch, the candidates of a
/// Random Action), transitively, each once and in a stable order. Ids
/// without an action in `library` are left out.
pub fn action_closure(
    start: impl IntoIterator<Item = ActionInstanceId>,
    library: &[ActionInstance],
) -> Vec<ActionInstanceId> {
    let by_id: BTreeMap<ActionInstanceId, &ActionInstance> =
        library.iter().map(|action| (action.id, action)).collect();
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue: std::collections::VecDeque<ActionInstanceId> = start.into_iter().collect();
    while let Some(id) = queue.pop_front() {
        let Some(action) = by_id.get(&id) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        result.push(id);
        let json = serde_json::to_string(&action.parameters).unwrap_or_default();
        for other in library {
            if other.id != id && !seen.contains(&other.id) && mentions(&json, &other.id.to_string())
            {
                queue.push_back(other.id);
            }
        }
    }
    result
}

/// Something of a layout that depends on the profile it runs in and is
/// not there ([`availability`]). Shown on the key and in the app; never an
/// error, because another profile that uses the layout may have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Unavailable {
    /// The binding starts a workflow this profile does not have.
    Workflow {
        page: PageId,
        page_name: String,
        control: ControlId,
        trigger: Trigger,
        workflow: WorkflowId,
    },
    /// The binding, or the action it runs, names a project that is not one
    /// of this profile's. The run is refused; another profile's project is
    /// never used.
    Project {
        page: PageId,
        page_name: String,
        control: ControlId,
        trigger: Trigger,
    },
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Workflow {
                page_name, control, ..
            } => write!(
                f,
                "page `{page_name}`: {control} starts a workflow this profile does not have"
            ),
            Self::Project {
                page_name, control, ..
            } => write!(
                f,
                "page `{page_name}`: {control} uses a project that is not in this profile"
            ),
        }
    }
}

/// What of `layout` cannot run in `profile` because it belongs to another
/// profile (see [`Unavailable`]).
pub fn availability(layout: &DeviceLayout, profile: &Profile) -> Vec<Unavailable> {
    let mut found = Vec::new();
    for page in &layout.pages {
        for binding in &page.bindings {
            match &binding.target {
                BindingTarget::Workflow { workflow } if profile.workflow(*workflow).is_none() => {
                    found.push(Unavailable::Workflow {
                        page: page.id,
                        page_name: page.name.clone(),
                        control: binding.control,
                        trigger: binding.trigger,
                        workflow: *workflow,
                    });
                }
                BindingTarget::Action { action, overrides } => {
                    let foreign = overrides
                        .projects()
                        .chain(
                            profile
                                .action(*action)
                                .into_iter()
                                .flat_map(|instance| instance.parameters.projects()),
                        )
                        .any(|project| profile.project(project).is_none());
                    if foreign {
                        found.push(Unavailable::Project {
                            page: page.id,
                            page_name: page.name.clone(),
                            control: binding.control,
                            trigger: binding.trigger,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// Most layouts kept in one history ([`LayoutHistory`]). Older entries are
/// dropped first.
pub const MAX_LAYOUT_HISTORY: usize = 16;

/// How *Switch layout* treats the layout it leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryMode {
    /// Remember it: *Back to previous layout* returns to it.
    #[default]
    Push,
    /// Do not remember it: Back skips it and returns to what was before.
    Replace,
    /// Forget the whole history: Back has nowhere to go afterwards.
    Clear,
}

impl HistoryMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "push" => Some(Self::Push),
            "replace" => Some(Self::Replace),
            "clear" => Some(Self::Clear),
            _ => None,
        }
    }
}

/// One layout a device left, with the pages it showed (the page stack, the
/// visible page last), so that Back returns to the same page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub layout: LayoutId,
    pub pages: Vec<PageId>,
}

/// The layouts one device showed before the current one, in one profile.
/// Kept in memory by the agent for the session: per profile and device, so
/// two profiles that share a layout never share their way back, and two
/// devices go their own ways.
///
/// Rules (`docs/data-model.md`, "Switching layouts"):
///
/// - A switch to the layout already shown changes no history (at most the
///   page).
/// - A switch to a layout that is in the history *unwinds* to it: the
///   entries after it are dropped and it is not added again, so the history
///   never has a layout twice and never loops (Main → Teams → Meeting,
///   switch to Main: the history is empty again).
/// - Otherwise the layout left is added ([`HistoryMode::Push`]), not added
///   (`Replace`) or the history is emptied (`Clear`).
/// - Back takes the newest entry that is still usable (assigned to the
///   profile, made for the device's model); unusable ones are dropped on
///   the way. With none, Back does nothing.
/// - At most [`MAX_LAYOUT_HISTORY`] entries; the oldest go first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutHistory {
    entries: Vec<HistoryEntry>,
}

impl LayoutHistory {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The layouts, oldest first.
    pub fn layouts(&self) -> Vec<LayoutId> {
        self.entries.iter().map(|entry| entry.layout).collect()
    }

    /// Records a switch from `current` (showing `pages`) to `target`.
    /// Returns `false` when `target` is `current`: nothing changes.
    pub fn switch(
        &mut self,
        current: Option<(LayoutId, Vec<PageId>)>,
        target: LayoutId,
        mode: HistoryMode,
    ) -> bool {
        if current
            .as_ref()
            .is_some_and(|(layout, _)| *layout == target)
        {
            return false;
        }
        if let Some(index) = self.entries.iter().position(|e| e.layout == target) {
            self.entries.truncate(index);
            return true;
        }
        match mode {
            HistoryMode::Clear => self.entries.clear(),
            HistoryMode::Replace => {}
            HistoryMode::Push => {
                if let Some((layout, pages)) = current {
                    self.entries.push(HistoryEntry { layout, pages });
                    if self.entries.len() > MAX_LAYOUT_HISTORY {
                        let excess = self.entries.len() - MAX_LAYOUT_HISTORY;
                        self.entries.drain(..excess);
                    }
                }
            }
        }
        true
    }

    /// Takes the newest entry `usable` accepts, dropping the newer ones it
    /// refuses. `None`: nothing to go back to (the history is then empty).
    pub fn back(&mut self, usable: impl Fn(LayoutId) -> bool) -> Option<HistoryEntry> {
        while let Some(entry) = self.entries.pop() {
            if usable(entry.layout) {
                return Some(entry);
            }
        }
        None
    }

    /// Drops every entry `keep` refuses (a layout deleted, unassigned or
    /// changed to another model).
    pub fn retain(&mut self, keep: impl Fn(LayoutId) -> bool) {
        self.entries.retain(|entry| keep(entry.layout));
    }
}

#[cfg(test)]
mod tests;
