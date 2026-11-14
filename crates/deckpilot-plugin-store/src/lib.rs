//! **Experimental.** Storage and install lifecycle of DeckPilot plugins.
//!
//! Nothing in DeckPilot uses this crate yet: no app, agent or module crate
//! depends on it, and plugins stay disabled. It implements sections 5.4
//! (grants), 5.7 (audit), 12 (local installation transaction) and 13
//! (updates and rollback) of the plugin marketplace design
//! (`docs/marketplace/architecture.md`, proposed in pull request #191).
//!
//! # Storage
//!
//! The plugin records live in **their own** SQLCipher database,
//! `<data dir>/plugins/plugins.db`, opened with a caller-supplied
//! [`StoreKey`] and migrated by this crate's own [`MIGRATIONS`]. The
//! configuration database (`deckpilot.db`, `deckpilot-storage`) and its
//! schema are never opened or changed, so installing this crate cannot
//! bump the configuration schema or break a downgrade.
//!
//! # Lifecycle
//!
//! [`PluginManager`] is the single writer. Its operations follow the state
//! machine of [`PluginState`]:
//!
//! ```text
//! install ─▶ Installing ─▶ Disabled ⇄ Enabled
//! update ──▶ Updating ───▶ (back; or the new version staged for consent)
//! rollback ▶ RollingBack ▶ Disabled | Enabled
//! uninstall ▶ Uninstalling ▶ (gone)
//! integrity failure ▶ Broken      registry `disable` ▶ Quarantined
//! ```
//!
//! Only a [`VerifiedPackage`](deckpilot_plugin_package::VerifiedPackage)
//! can be installed; raw bytes do not type-check:
//!
//! ```compile_fail
//! # use deckpilot_plugin_store::{Consent, InstallSource, PluginManager};
//! fn install_bytes(manager: &mut PluginManager, bytes: &[u8]) {
//!     let _ = manager.install(bytes, &InstallSource::Sideload, &Consent::none("user"));
//! }
//! ```
//!
//! and a `VerifiedPackage` cannot be made except by
//! `deckpilot_plugin_package::verify_package`:
//!
//! ```compile_fail
//! let forged = deckpilot_plugin_package::VerifiedPackage {
//!     manifest: todo!(),
//!     files: Default::default(),
//!     integrity: todo!(),
//! };
//! ```
//!
//! # Crash safety
//!
//! Multi-step operations write a journal entry first and delete it in their
//! final transaction; [`PluginManager::recover`] completes or undoes what a
//! crash interrupted (see the `recover` module). Crashes are simulated in
//! tests through [`Failpoints`].

mod db;
mod error;
mod failpoint;
mod grants;
mod layout;
mod manager;
mod recover;
mod state;
mod store;

pub use db::{
    CONFIG_DATABASE_FILE, DEFAULT_BUSY_TIMEOUT, Database, MIGRATIONS, StoreKey,
    supported_schema_version,
};
pub use error::{Result, StoreError};
pub use failpoint::{CrashOnce, Failpoint, Failpoints, NoFailpoints, Operation};
pub use grants::{
    Broadened, Consent, DeclaredGrant, GrantKey, GrantScope, GrantSet, GrantSource, GrantStatus,
    PRIVATE_NETWORK_KEY, PermissionDiff, StoredGrant, Tier, host_covers, missing_grants,
    variable_covers,
};
pub use layout::{DATABASE_FILE, Layout, PLUGINS_DIR};
pub use manager::{
    Clock, ManagerOptions, NoHooks, PluginHooks, PluginManager, UpdateOutcome, VerdictChange,
    VerdictSource,
};
pub use recover::{Inconsistency, RecoveryReport};
pub use state::PluginState;
pub use store::{
    AuditRecord, AuditRetention, InstallSource, JournalEntry, PluginRecord, PluginStore,
    VersionRecord, VersionRole,
};

/// Seconds since the Unix epoch, from the system clock.
pub fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
