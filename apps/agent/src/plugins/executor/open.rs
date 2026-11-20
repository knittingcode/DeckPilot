//! `resource.open` (X4) and `url.openParam` (X5) of plugin API 1.9
//! (`docs/marketplace/module-migration.md` §7): a bound location in a
//! bound application through the core launcher, and the literal http or
//! https URL the user stored in the invocation's own parameter, both only
//! in runs the user started.
//!
//! Not carried out yet (WP-A5): every call the broker allowed is answered
//! as not implemented, before anything opens.

use deckpilot_plugin_abi::CallResult;
use deckpilot_plugin_abi::privileged::{OpenUrlParam, ResourceOpen};
use deckpilot_plugin_broker::PluginId;

use super::{Executor, not_implemented};

impl Executor {
    /// `resource.open`, which the broker allowed (granted, a run the user
    /// started, within its rate).
    #[expect(
        clippy::unused_self,
        reason = "WP-A5 opens this executor's handles in its run context"
    )]
    pub(super) fn open_resource(&self, _plugin: &PluginId, _request: &ResourceOpen) -> CallResult {
        not_implemented("resource.open")
    }

    /// `url.openParam`, which the broker allowed.
    #[expect(
        clippy::unused_self,
        reason = "WP-A5 opens the invocation's own parameter in its run context"
    )]
    pub(super) fn open_url_param(&self, _plugin: &PluginId, _request: &OpenUrlParam) -> CallResult {
        not_implemented("url.openParam")
    }
}
