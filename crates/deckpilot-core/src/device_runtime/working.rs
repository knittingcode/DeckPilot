//! The editor's live preview in the runtime (IPC protocol 16,
//! `docs/editor-navigation-and-simulation.md`).
//!
//! A device may be given a **working layout**: the editor's unsaved draft
//! of one layout of the active profile. While the device shows that layout
//! ([`Runtime::layout_of`] resolves to it), the working layout is what it
//! shows and what its keys run: bindings, overrides, gestures, folder
//! keys, the protected Back and Auto Exit settings. Other devices, also
//! ones that show the same layout, keep the saved one. Nothing here is
//! saved: the pages devices remember come from the saved layout only.
//!
//! The editor also navigates the device ([`Runtime::navigate`]) without
//! running anything: a page stack of a layout, another layout as *Switch
//! layout* would show it, or *Back to previous layout*. Entering a folder
//! this way starts no Auto Exit countdown.

use deckpilot_device::DeviceId;

use super::Runtime;
use crate::{ActionResult, ActionStatus, DeviceLayout, LayoutId, PageId, ProfileId};

/// A device's working layout and the profile it belongs to.
#[derive(Debug, Clone)]
pub(crate) struct WorkingLayout {
    pub(crate) profile: ProfileId,
    pub(crate) layout: DeviceLayout,
}

/// Where the editor shows a device ([`Runtime::navigate`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorTarget {
    /// The pages of `stack` (the one shown last) of `layout`.
    Show {
        layout: LayoutId,
        stack: Vec<PageId>,
    },
    /// *Back to previous layout*.
    LayoutBack,
}

/// Why [`Runtime::navigate`] changed nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NavigationError {
    #[error("the device is not connected")]
    NotConnected,
    #[error("no profile is active")]
    NoProfile,
    #[error("the layout is not one of the profile's for this device")]
    UnknownLayout,
    #[error("the layout has no such page")]
    UnknownPage,
    #[error("there is no previous layout")]
    NoPreviousLayout,
}

/// What a device shows, as far as navigation goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationState {
    pub profile: Option<ProfileId>,
    pub layout: Option<LayoutId>,
    /// The pages it went through, the page shown last.
    pub stack: Vec<PageId>,
    pub connected: bool,
}

impl Runtime {
    /// The working layout of `device` when it is for layout `id` of the
    /// active profile.
    pub(super) fn working_for(&self, device: &DeviceId, id: LayoutId) -> Option<&DeviceLayout> {
        let profile = self.profile()?.id;
        self.working
            .get(device)
            .filter(|working| working.profile == profile && working.layout.id == id)
            .map(|working| &working.layout)
    }

    /// Layout `id` of the active profile as `device` would show it: its
    /// working layout, if it has one for it, else the saved one.
    pub(super) fn layout_by_id(&self, device: &DeviceId, id: LayoutId) -> Option<&DeviceLayout> {
        self.working_for(device, id)
            .or_else(|| self.profile()?.layout(id))
    }

    /// Gives `device` a working layout of `profile` (it replaces the one it
    /// had): shown and run while the device shows that layout of that
    /// profile. The device stays on its page, or on the nearest one that
    /// is still there; no countdown starts. The caller checks that the
    /// layout is the profile's and made for the device's model.
    pub fn set_working_layout(
        &mut self,
        device: &DeviceId,
        profile: ProfileId,
        layout: DeviceLayout,
    ) {
        self.working
            .insert(device.clone(), WorkingLayout { profile, layout });
        self.refused.clear();
        self.repair_navigation(device);
        self.follow_folders(device, true);
    }

    /// Drops the working layout of `device`: it shows the saved layout
    /// again, on its page or the nearest one that is still there. `false`
    /// when it had none.
    pub fn clear_working_layout(&mut self, device: &DeviceId) -> bool {
        if self.working.remove(device).is_none() {
            return false;
        }
        self.refused.clear();
        self.repair_navigation(device);
        self.follow_folders(device, true);
        true
    }

    /// The working layout of `device`, whatever it shows.
    pub fn working_layout(&self, device: &DeviceId) -> Option<&DeviceLayout> {
        self.working.get(device).map(|working| &working.layout)
    }

    /// What `device` shows: the active profile, its layout and page stack
    /// (the parked one while it is disconnected).
    pub fn navigation_state(&self, device: &DeviceId) -> NavigationState {
        let connected = self.devices.is_connected(device);
        let stack = if connected {
            self.navigation.get(device)
        } else {
            self.parked.get(device)
        };
        NavigationState {
            profile: self.profile().map(|profile| profile.id),
            layout: self.saved_layout_of(device).map(|layout| layout.id),
            stack: stack.cloned().unwrap_or_default(),
            connected,
        }
    }

    /// Shows `target` on `device` as the editor navigated: nothing runs,
    /// and entering a folder starts no Auto Exit countdown. Another layout
    /// is switched to as *Switch layout* would (it can be gone back to).
    ///
    /// # Errors
    ///
    /// The device is not connected, there is no profile, the layout is not
    /// the profile's or not for the device's model, a page is not in the
    /// layout, or there is no layout to go back to. Nothing changes then.
    pub fn navigate(
        &mut self,
        device: &DeviceId,
        target: &EditorTarget,
    ) -> Result<(), NavigationError> {
        if !self.devices.is_connected(device) {
            return Err(NavigationError::NotConnected);
        }
        if self.profile().is_none() {
            return Err(NavigationError::NoProfile);
        }
        match target {
            EditorTarget::Show { layout, stack } => {
                let shown = self
                    .layout_by_id(device, *layout)
                    .filter(|shown| shown.model == device.model)
                    .ok_or(NavigationError::UnknownLayout)?;
                if stack.is_empty() || stack.iter().any(|page| shown.page(*page).is_none()) {
                    return Err(NavigationError::UnknownPage);
                }
                if self.layout_of(device).map(|l| l.id) != Some(*layout) {
                    let mut result = ActionResult::success("");
                    self.switch_layout(
                        device,
                        *layout,
                        stack.last().copied(),
                        crate::HistoryMode::Push,
                        &mut result,
                    );
                    if result.status == ActionStatus::Failed {
                        return Err(NavigationError::UnknownLayout);
                    }
                }
                self.navigation.insert(device.clone(), stack.clone());
            }
            EditorTarget::LayoutBack => {
                if self.layout_history(device).is_empty() {
                    return Err(NavigationError::NoPreviousLayout);
                }
                let mut result = ActionResult::success("");
                self.previous_layout(device, &mut result);
            }
        }
        self.follow_folders(device, true);
        Ok(())
    }

    /// Keeps `device` on its pages where they still exist: the longest
    /// start of its stack whose pages are all in the layout it shows, else
    /// the page it starts on. For a working layout that dropped the page
    /// shown, a revert, or a reload.
    pub(super) fn repair_navigation(&mut self, device: &DeviceId) {
        let Some(stack) = self.navigation.get(device) else {
            return;
        };
        let Some(layout) = self.layout_of(device) else {
            if !stack.is_empty() {
                self.navigation.insert(device.clone(), Vec::new());
            }
            return;
        };
        if !stack.is_empty() && stack.iter().all(|page| layout.page(*page).is_some()) {
            return;
        }
        let kept: Vec<PageId> = stack
            .iter()
            .take_while(|page| layout.page(**page).is_some())
            .copied()
            .collect();
        if kept.is_empty() {
            self.reset_navigation(device);
        } else {
            self.navigation.insert(device.clone(), kept);
        }
    }
}
