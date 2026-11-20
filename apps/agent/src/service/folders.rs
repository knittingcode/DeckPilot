//! Auto Exit of folders on the service task (`docs/data-model.md`, "Pages
//! and folders").
//!
//! The countdowns live in the core's runtime ([`deckpilot_core::FolderTimers`]),
//! per device, in memory only: the agent runs them with the desktop app
//! closed, and a restart starts none. The run loop sleeps until the next
//! one ends, like gesture deadlines: nothing is spawned and input is never
//! held up. An exit applies only while its entry is still the device's
//! current one (its generation), so a countdown of a folder that was left,
//! or entered again, does nothing. A device that leaves its folder this way
//! is shown and published as after a key that went back.

use std::time::Instant;

use deckpilot_core::RuntimeOutcome;
use tracing::debug;

use super::Agent;

impl Agent {
    /// When the next Auto Exit is due, if a countdown runs.
    pub(super) fn next_folder_exit(&self) -> Option<Instant> {
        self.runtime.next_folder_exit()
    }

    /// Countdowns ended: their devices leave their folders.
    pub(super) fn on_folder_exit_due(&mut self) {
        self.exit_folders_at(Instant::now());
    }

    /// Applies the exits due at `now`; returns how many devices left a
    /// folder.
    pub(super) fn exit_folders_at(&mut self, now: Instant) -> usize {
        let mut left = 0;
        for exit in self.runtime.due_folder_exits(now) {
            if let Some(RuntimeOutcome::Navigated { device, page }) =
                self.runtime.exit_folder(&exit)
            {
                debug!(%device, %page, "auto exit");
                self.gestures.page_changed(&device);
                left += 1;
            }
        }
        if left > 0 {
            self.after_change();
        }
        left
    }
}
