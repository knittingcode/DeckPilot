//! `deckpilot.workflows`: the first-party Workflows plugin
//! (`docs/marketplace/module-migration.md` §7 X8, §11 D6, WP-B5).
//!
//! **A manifest-only plugin.** Workflows have no action ids: keys run them
//! through the core's `BindingTarget::Workflow`, and the engine, the
//! execution IPC and the run monitoring stay in the core. What the plugin
//! owns is the app's Workflows screen: its manifest declares the host view
//! of kind `workflows` and its navigation entry under the low-tier
//! `hostViews` permission, and the app shows them only while the plugin is
//! enabled. The screen is drawn by the app from core data; the plugin
//! ships no UI code and is given no data.
//!
//! The module therefore declares no actions. It exists so that the plugin
//! is installed, enabled, disabled and removed like any other.
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::PRIVILEGED_API_LEVEL;
use deckpilot_plugin_sdk::prelude::*;

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.workflows";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The plugin definition: no actions.
pub fn plugin() -> Plugin {
    Plugin::new(ID, VERSION).requires_api(PRIVILEGED_API_LEVEL)
}

deckpilot_plugin_sdk::plugin!(plugin);

#[cfg(test)]
mod tests;
