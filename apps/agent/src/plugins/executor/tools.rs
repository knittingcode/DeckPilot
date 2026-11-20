//! `tool.run` (plugin API 1.9, X3 in `docs/marketplace/module-migration.md`
//! §7): the user's own `git` or `docker`, with the host's policy, fixed
//! prefix, environment and limits, in the directory of a resource handle
//! of this invocation ([`Executor::bound_inside`]).
//!
//! Not carried out yet (WP-A4): every call the broker allowed is answered
//! as not implemented, before anything runs.

use deckpilot_plugin_abi::CallResult;
use deckpilot_plugin_abi::privileged::{ToolAccess, ToolRun};
use deckpilot_plugin_broker::PluginId;
use deckpilot_plugin_supervisor::CallInterrupt;

use super::{Executor, not_implemented};

impl Executor {
    /// `tool.run`, which the broker allowed for its tool and `access`.
    #[expect(
        clippy::unused_self,
        reason = "WP-A4 runs the tool with this executor's handles and run context"
    )]
    pub(super) fn run_tool(
        &self,
        _plugin: &PluginId,
        _request: &ToolRun,
        _access: ToolAccess,
        _interrupt: Option<&CallInterrupt>,
    ) -> CallResult {
        not_implemented("tool.run")
    }
}
