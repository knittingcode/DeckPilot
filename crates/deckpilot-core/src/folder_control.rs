//! Folder controls: the keys that open folders (`docs/data-model.md`,
//! "Folder controls").
//!
//! **The rule.** A key `K` on page `P` is a *folder control* when one of
//! its bindings opens a page `F` (`BindingTarget::OpenPage`) that is a
//! folder of `P`: `F`'s parent in the page tree, stored or inferred
//! ([`DeviceLayout::page_parents`]), is `P`. Only page and control
//! identifiers decide it, never labels, icons or names. When several
//! bindings of the key open folders of `P`, the key opens the folder of its
//! `Press` binding, else of its `Click` binding, else of its first one.
//! Dials and touch zones are never folder controls: a binding of theirs
//! that opens a page is an ordinary binding. This module is the one place
//! the rule lives; the agent's runtime, saving, migration 12, imports and
//! the editor (`src/lib/folder-control.ts` mirrors it) all use it.
//!
//! **A folder control is a core navigation control.** It has exactly one
//! binding, `Press` → `OpenPage(F)` (its label and look are free): a press
//! opens the folder at once, without gesture delays. Saving refuses any
//! other binding on it ([`LayoutIssue::FolderControlBindings`]) and any
//! other trigger for its `OpenPage` ([`LayoutIssue::FolderControlTrigger`]),
//! and an edit that turns it into another key by replacing what it opens
//! ([`DeviceLayout::folder_control_edits`]). Clearing the key or deleting
//! the folder stays possible. The runtime treats it as a raw press that
//! opens `F` whatever else is stored on it, so nothing else on it ever runs
//! ([`crate::Runtime::folder_control`]). Plugins cannot bind to it: their
//! manifests declare control kinds and inputs, never a control of a
//! layout, and no plugin host operation edits layouts.
//!
//! **Recovered bindings.** Layouts from before the rule (stored ones in
//! migration 12, imported ones when they are imported) may have folder
//! controls with more bindings. [`DeviceLayout::normalize_folder_controls`]
//! keeps the `OpenPage` (moved to `Press`) and moves the others, with the
//! key's gesture timings, to the layout's [`DeviceLayout::recovered`] list:
//! stored and exported with the layout, never run, and offered in the
//! folder key's Inspector to be moved to a free key
//! ([`DeviceLayout::restore_recovered`]) or discarded
//! ([`DeviceLayout::discard_recovered`]).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::{ControlId, ControlKind, DeviceCapabilities};

use crate::gesture::KeyGestureSettings;
use crate::{
    BindingTarget, ControlBinding, DeviceLayout, LayoutIssue, LayoutPage, PageId, Trigger,
};

/// The one trigger of a folder control's binding: the press, which opens
/// the folder at once (what a folder key did before gestures existed).
pub const FOLDER_CONTROL_TRIGGER: Trigger = Trigger::Press;

/// Why bindings were set aside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryReason {
    /// They were on a folder control besides the binding that opens its
    /// folder.
    FolderControl,
    /// The folder control's binding had a title of its own that differed
    /// from the folder's name (schema 13: the folder's name is the one
    /// title of its key). `bindings` holds that binding as it was, title
    /// included, so the title can be used as the folder's name or
    /// discarded.
    FolderLabel,
}

/// One folder control [`DeviceLayout::normalize_folder_labels`] changed:
/// its own title differed from the folder's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedFolderLabel {
    pub page: PageId,
    pub control: ControlId,
    pub folder: PageId,
    /// The title it had.
    pub label: String,
}

/// Bindings set aside from a folder control: kept with the layout, never
/// run ([`DeviceLayout::recovered`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredBindings {
    /// The page of the folder control.
    pub page: PageId,
    /// The folder control.
    pub control: ControlId,
    /// The folder it opens.
    pub folder: PageId,
    /// Its original bindings other than the one that opens the folder, as
    /// they were (controls included).
    pub bindings: Vec<ControlBinding>,
    /// Its gesture timings, if it had any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gestures: Option<KeyGestureSettings>,
    pub reason: RecoveryReason,
}

impl RecoveredBindings {
    /// Gestures set aside from the folder control `control` of `page`
    /// ([`RecoveryReason::FolderControl`]).
    fn is_gestures_of(&self, page: PageId, control: ControlId) -> bool {
        self.page == page && self.control == control && self.reason == RecoveryReason::FolderControl
    }
}

/// One folder control [`DeviceLayout::normalize_folder_controls`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedFolderControl {
    pub page: PageId,
    pub control: ControlId,
    pub folder: PageId,
    /// Bindings moved to the recovered list.
    pub recovered: usize,
}

/// Why [`DeviceLayout::restore_recovered`] changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RecoveredError {
    #[error("the key has no recovered bindings")]
    NotFound,
    #[error("every key of the page runs something; free a key first")]
    NoFreeKey,
}

/// The binding of `control` that makes it a folder control on `page`, with
/// the folder it opens: `(index in page.bindings, folder)`. `parents` is
/// the layout's page tree ([`DeviceLayout::page_parents`]).
pub(crate) fn opening_binding(
    parents: &BTreeMap<PageId, PageId>,
    page: &LayoutPage,
    control: ControlId,
) -> Option<(usize, PageId)> {
    if control.kind() != ControlKind::Key {
        return None;
    }
    let rank = |trigger: Trigger| match trigger {
        Trigger::Press => 0,
        Trigger::Click => 1,
        _ => 2,
    };
    page.bindings
        .iter()
        .enumerate()
        .filter_map(|(index, binding)| match binding.target {
            BindingTarget::OpenPage { page: folder }
                if binding.control == control
                    && folder != page.id
                    && parents.get(&folder) == Some(&page.id) =>
            {
                Some((rank(binding.trigger), index, folder))
            }
            _ => None,
        })
        .min()
        .map(|(_, index, folder)| (index, folder))
}

/// The folder controls of `page` with their folders, in binding order.
fn folder_controls_in(
    parents: &BTreeMap<PageId, PageId>,
    page: &LayoutPage,
) -> Vec<(ControlId, PageId)> {
    let mut seen = BTreeSet::new();
    page.bindings
        .iter()
        .map(|binding| binding.control)
        .filter(|control| seen.insert(*control))
        .filter_map(|control| {
            opening_binding(parents, page, control).map(|(_, folder)| (control, folder))
        })
        .collect()
}

impl DeviceLayout {
    /// The folder `control` opens when it is a folder control of `page`
    /// (see the module documentation for the rule); `None` otherwise.
    pub fn folder_control(&self, page: PageId, control: ControlId) -> Option<PageId> {
        let page = self.page(page)?;
        opening_binding(&self.page_parents(), page, control).map(|(_, folder)| folder)
    }

    /// The folder controls of `page`, with the folders they open.
    pub fn folder_controls(&self, page: PageId) -> Vec<(ControlId, PageId)> {
        self.page(page)
            .map(|p| folder_controls_in(&self.page_parents(), p))
            .unwrap_or_default()
    }

    /// The recovered bindings of the folder control `control` on `page`.
    pub fn recovered_of(&self, page: PageId, control: ControlId) -> Vec<&ControlBinding> {
        self.recovered
            .iter()
            .filter(|entry| entry.is_gestures_of(page, control))
            .flat_map(|entry| &entry.bindings)
            .collect()
    }

    /// Every recovered binding of the layout. They are stored with it
    /// (their actions and icons stay with it too), never run.
    pub fn recovered_bindings(&self) -> impl Iterator<Item = &ControlBinding> {
        self.recovered.iter().flat_map(|entry| &entry.bindings)
    }

    /// Folder controls with one binding besides their `OpenPage`, or an
    /// `OpenPage` on another trigger than the press: refused when saved.
    pub(crate) fn validate_folder_controls(
        &self,
        page: &LayoutPage,
        parents: &BTreeMap<PageId, PageId>,
        issues: &mut Vec<LayoutIssue>,
    ) {
        for (control, folder) in folder_controls_in(parents, page) {
            let folder_name = self
                .page(folder)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let bindings: Vec<&ControlBinding> = page
                .bindings
                .iter()
                .filter(|binding| binding.control == control)
                .collect();
            if bindings.len() > 1 {
                issues.push(LayoutIssue::FolderControlBindings {
                    page: page.name.clone(),
                    control,
                    folder: folder_name,
                });
            } else if let Some(binding) = bindings.first()
                && binding.trigger != FOLDER_CONTROL_TRIGGER
            {
                issues.push(LayoutIssue::FolderControlTrigger {
                    page: page.name.clone(),
                    control,
                    folder: folder_name,
                    trigger: binding.trigger,
                });
            }
        }
    }

    /// What saving this (edited) layout over `stored` (the saved version,
    /// `None` for a new layout) must refuse besides [`DeviceLayout::validate`]:
    ///
    /// - a folder control of `stored` that now runs something else while
    ///   its folder is still a folder of the page and no key of the page
    ///   opens it any more (its `OpenPage` was replaced, e.g. by an
    ///   action). Clearing the key, moving it, or deleting or moving the
    ///   folder are allowed ([`LayoutIssue::FolderControlReplaced`]);
    /// - recovered bindings that are not stored: they can only be kept,
    ///   moved to a key or discarded, never added or changed
    ///   ([`LayoutIssue::FolderControlRecovered`]).
    pub fn folder_control_edits(&self, stored: Option<&DeviceLayout>) -> Vec<LayoutIssue> {
        let mut issues = Vec::new();
        let kept = |entry: &RecoveredBindings| {
            stored.is_some_and(|stored| stored.recovered.contains(entry))
        };
        if !self.recovered.iter().all(kept) {
            issues.push(LayoutIssue::FolderControlRecovered);
        }
        let Some(stored) = stored else {
            return issues;
        };
        let parents = self.page_parents();
        let stored_parents = stored.page_parents();
        for stored_page in &stored.pages {
            let Some(page) = self.page(stored_page.id) else {
                continue;
            };
            for (control, folder) in folder_controls_in(&stored_parents, stored_page) {
                let still_folder = parents.get(&folder) == Some(&page.id);
                let opened = page
                    .bindings
                    .iter()
                    .any(|binding| binding.target == BindingTarget::OpenPage { page: folder });
                let bound = page
                    .bindings
                    .iter()
                    .any(|binding| binding.control == control);
                if still_folder && !opened && bound {
                    issues.push(LayoutIssue::FolderControlReplaced {
                        page: page.name.clone(),
                        control,
                        folder: self
                            .page(folder)
                            .map(|p| p.name.clone())
                            .unwrap_or_default(),
                    });
                }
            }
        }
        issues
    }

    /// Makes every folder control a core navigation control: its `OpenPage`
    /// on the press is its only binding. Other bindings move, with the
    /// key's gesture timings, to [`DeviceLayout::recovered`] (added to the
    /// key's entry when it has one); timings of a key with nothing else
    /// are dropped (they did nothing). When something changes in a layout
    /// whose tree is inferred, the tree is stored first
    /// ([`DeviceLayout::infer_parents`]), so removing a binding never moves
    /// a page. Nothing changes in a layout without such keys (it stays
    /// byte for byte the same), so this is idempotent. Migration 12,
    /// imports and saving run it.
    pub fn normalize_folder_controls(&mut self) -> Vec<NormalizedFolderControl> {
        let parents = self.page_parents();
        let needs = |page: &LayoutPage| {
            folder_controls_in(&parents, page)
                .into_iter()
                .any(|(control, _)| {
                    let mut own = page.bindings.iter().filter(|b| b.control == control);
                    let first = own.next();
                    own.next().is_some()
                        || first.is_some_and(|b| b.trigger != FOLDER_CONTROL_TRIGGER)
                        || page.gestures.iter().any(|g| g.control == control)
                })
        };
        if !self.pages.iter().any(needs) {
            return Vec::new();
        }
        self.infer_parents();
        let mut report = Vec::new();
        let mut recovered = Vec::new();
        for page in &mut self.pages {
            for (control, folder) in folder_controls_in(&parents, page) {
                let Some((index, _)) = opening_binding(&parents, page, control) else {
                    continue;
                };
                let mut opening = page.bindings[index].clone();
                opening.trigger = FOLDER_CONTROL_TRIGGER;
                let mut others = Vec::new();
                let mut kept = Vec::with_capacity(page.bindings.len());
                for (at, binding) in page.bindings.drain(..).enumerate() {
                    if at == index {
                        kept.push(opening.clone());
                    } else if binding.control == control {
                        others.push(binding);
                    } else {
                        kept.push(binding);
                    }
                }
                page.bindings = kept;
                let timings = page
                    .gestures
                    .iter()
                    .position(|g| g.control == control)
                    .map(|at| page.gestures.remove(at));
                page.gestures.retain(|g| g.control != control);
                if others.is_empty() {
                    continue;
                }
                report.push(NormalizedFolderControl {
                    page: page.id,
                    control,
                    folder,
                    recovered: others.len(),
                });
                recovered.push(RecoveredBindings {
                    page: page.id,
                    control,
                    folder,
                    bindings: others,
                    gestures: timings,
                    reason: RecoveryReason::FolderControl,
                });
            }
        }
        for entry in recovered {
            match self
                .recovered
                .iter_mut()
                .find(|e| e.is_gestures_of(entry.page, entry.control))
            {
                Some(existing) => {
                    existing.bindings.extend(entry.bindings);
                    if existing.gestures.is_none() {
                        existing.gestures = entry.gestures;
                    }
                }
                None => self.recovered.push(entry),
            }
        }
        report
    }

    /// Makes the folder's name the one title of every folder control
    /// (schema 13, `docs/data-model.md`, "Folder controls"): the key that
    /// opens a folder shows the folder page's name, so its binding keeps no
    /// title of its own. A title equal to the folder's name (spaces aside)
    /// is dropped; one that differs is kept, with the binding as it was,
    /// in [`DeviceLayout::recovered`] ([`RecoveryReason::FolderLabel`]) and
    /// reported, so nothing the user wrote is lost. Its look (font, colours,
    /// whether the title shows…) stays on the key. Nothing changes in a
    /// layout without such titles, so this is idempotent. Migration 13,
    /// imports and saving run it.
    pub fn normalize_folder_labels(&mut self) -> Vec<NormalizedFolderLabel> {
        let parents = self.page_parents();
        let names: BTreeMap<PageId, String> =
            self.pages.iter().map(|p| (p.id, p.name.clone())).collect();
        let mut report = Vec::new();
        let mut recovered = Vec::new();
        for page in &mut self.pages {
            for (control, folder) in folder_controls_in(&parents, page) {
                let Some((index, _)) = opening_binding(&parents, page, control) else {
                    continue;
                };
                let binding = &mut page.bindings[index];
                let Some(label) = binding.label.clone() else {
                    continue;
                };
                let name = names.get(&folder).map_or("", String::as_str);
                if label.trim() != name.trim() && !label.trim().is_empty() {
                    recovered.push(RecoveredBindings {
                        page: page.id,
                        control,
                        folder,
                        bindings: vec![binding.clone()],
                        gestures: None,
                        reason: RecoveryReason::FolderLabel,
                    });
                    report.push(NormalizedFolderLabel {
                        page: page.id,
                        control,
                        folder,
                        label: label.clone(),
                    });
                }
                binding.label = None;
            }
        }
        self.recovered.extend(recovered);
        report
    }

    /// The title a folder kept from before schema 13
    /// ([`RecoveryReason::FolderLabel`]) for its key `control` on `page`.
    pub fn recovered_folder_label(&self, page: PageId, control: ControlId) -> Option<&str> {
        self.recovered
            .iter()
            .filter(|e| {
                e.page == page && e.control == control && e.reason == RecoveryReason::FolderLabel
            })
            .flat_map(|e| &e.bindings)
            .find_map(|b| b.label.as_deref())
    }

    /// "Move to a free key": puts the recovered bindings of the folder
    /// control `control` of `page` (with their gesture timings) on the
    /// page's first free key with a display — no binding, not the folder's
    /// reserved Back — and removes them from the list. Returns that key.
    ///
    /// # Errors
    ///
    /// Nothing changes when the key has no recovered bindings or no key of
    /// the page is free.
    pub fn restore_recovered(
        &mut self,
        page: PageId,
        control: ControlId,
        capabilities: &DeviceCapabilities,
    ) -> Result<ControlId, RecoveredError> {
        if !self
            .recovered
            .iter()
            .any(|e| e.is_gestures_of(page, control))
        {
            return Err(RecoveredError::NotFound);
        }
        let reserved = self.system_back(page, capabilities);
        let stored = self.page(page).ok_or(RecoveredError::NotFound)?;
        let bound: BTreeSet<ControlId> = stored.bindings.iter().map(|b| b.control).collect();
        let free = capabilities
            .controls()
            .into_iter()
            .find(|c| {
                c.kind() == ControlKind::Key
                    && capabilities.has_display(*c)
                    && Some(*c) != reserved
                    && !bound.contains(c)
            })
            .ok_or(RecoveredError::NoFreeKey)?;
        let (moved, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.recovered)
            .into_iter()
            .partition(|e| e.is_gestures_of(page, control));
        self.recovered = rest;
        let entry = self.page_mut(page).ok_or(RecoveredError::NotFound)?;
        entry.gestures.retain(|g| g.control != free);
        for recovered in moved {
            for mut binding in recovered.bindings {
                binding.control = free;
                entry.bindings.push(binding);
            }
            if let Some(mut timings) = recovered.gestures
                && !entry.gestures.iter().any(|g| g.control == free)
            {
                timings.control = free;
                entry.gestures.push(timings);
            }
        }
        Ok(free)
    }

    /// "Discard": forgets the recovered bindings of the folder control
    /// `control` of `page`. Returns how many there were.
    pub fn discard_recovered(&mut self, page: PageId, control: ControlId) -> usize {
        let mut discarded = 0;
        self.recovered.retain(|entry| {
            let matches = entry.is_gestures_of(page, control);
            if matches {
                discarded += entry.bindings.len();
            }
            !matches
        });
        discarded
    }

    /// Forgets the title the folder control `control` of `page` kept from
    /// before schema 13 ([`RecoveryReason::FolderLabel`]); with `adopt`,
    /// that title becomes the folder's name first. Returns the title.
    pub fn resolve_folder_label(
        &mut self,
        page: PageId,
        control: ControlId,
        adopt: bool,
    ) -> Option<String> {
        let label = self.recovered_folder_label(page, control)?.to_owned();
        let folder = self.folder_control(page, control);
        self.recovered.retain(|e| {
            !(e.page == page && e.control == control && e.reason == RecoveryReason::FolderLabel)
        });
        if adopt && let Some(folder) = folder.and_then(|f| self.page_mut(f)) {
            folder.name.clone_from(&label);
        }
        Some(label)
    }

    /// Forgets recovered bindings whose page no longer exists (it was
    /// deleted, or moved to another layout). Returns how many entries.
    pub fn prune_recovered(&mut self) -> usize {
        let pages: BTreeSet<PageId> = self.pages.iter().map(|p| p.id).collect();
        let before = self.recovered.len();
        self.recovered.retain(|entry| pages.contains(&entry.page));
        before - self.recovered.len()
    }
}

#[cfg(test)]
mod tests;
