//! The project context of plugin API 1.9 (X6 in
//! `docs/marketplace/module-migration.md` §7: `project.current`,
//! `project.list`, `project.select`, `project.cycle`) and the profile read
//! (X9, `profile.read`).
//!
//! Not carried out yet (WP-A6): every call the broker allowed is answered
//! as not implemented, before anything is read or changed.

use deckpilot_plugin_abi::CallResult;
use deckpilot_plugin_broker::{Allowed, PluginId};

use super::{Executor, not_implemented};

impl Executor {
    /// One of the project operations or `profile.read`, which the broker
    /// allowed.
    #[expect(
        clippy::unused_self,
        reason = "WP-A6 reads and changes the project context of this executor's run"
    )]
    pub(super) fn project(&self, _plugin: &PluginId, allowed: &Allowed) -> CallResult {
        not_implemented(match allowed {
            Allowed::ProjectCurrent => "project.current",
            Allowed::ProjectList => "project.list",
            Allowed::ProjectSelect { .. } => "project.select",
            Allowed::ProjectCycle { .. } => "project.cycle",
            Allowed::ProfileRead { .. } => "profile.read",
            _ => "this operation",
        })
    }
}
