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
//! Schema 3 adds the `bundled` install source, the journal of built-in
//! modules moving to their plugins ([`ModuleMigration`]) and the user's
//! [`Tombstone`]s (`docs/marketplace/module-migration.md` §4.2, §5.6). A
//! build that knows an older schema refuses a newer file with
//! [`StoreError::UnsupportedSchema`] before writing anything.
//!
//! Schema 5 adds the plugins installed only as another plugin's dependency
//! ([`PluginStore::dependency_installs`], [`deps`]). Before any upgrade of
//! an existing file an encrypted, verified copy is written to
//! `plugins/.backups/plugins-schema<n>-<ms>.db` (the newest
//! [`KEPT_BACKUPS`] are kept): an older build refuses the upgraded file,
//! so going back means restoring that copy as `plugins.db`.
//!
//! # Dependencies
//!
//! [`deps`] resolves what installing a plugin takes (its dependencies,
//! recursively, against the installed plugins, the built-in modules and
//! the registry), and what disabling or uninstalling one affects;
//! [`PluginManager::apply_plan`] carries out a confirmed plan all or
//! nothing, with every plugin's own consent.
//!
//! # First-party plugins
//!
//! A `deckpilot.*` id is installed only from the application bundle
//! ([`InstallSource::Bundled`]) or a registry, and a bundled plugin is
//! always first-party ([`InstallSource::allows`]); the schema checks the
//! same rule. A user's uninstall or disable of a first-party plugin
//! ([`PluginManager::uninstall_by_user`], [`PluginManager::disable_by_user`])
//! leaves a tombstone, and the store refuses to install or enable a
//! tombstoned plugin until the user clears it.
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

mod data;
mod db;
pub mod deps;
mod error;
mod failpoint;
mod grants;
mod layout;
mod manager;
mod origins;
mod plan;
mod recover;
mod state;
mod store;

pub use data::{
    CredentialStatus, KnownPlugin, MAX_SECRET_BYTES, MAX_STORAGE_KEY_CHARS,
    MAX_STORAGE_VALUE_BYTES, PluginData, Retention, Secret, StorageError, StorageQuota,
    StorageUsage,
};
pub use db::{
    BACKUP_DIR, CONFIG_DATABASE_FILE, DEFAULT_BUSY_TIMEOUT, Database, KEPT_BACKUPS, MIGRATIONS,
    StoreKey, supported_schema_version,
};
pub use error::{Result, StoreError};
pub use failpoint::{CrashOnce, Failpoint, Failpoints, NoFailpoints, Operation};
pub use grants::{
    Broadened, Consent, DeclaredGrant, GrantKey, GrantScope, GrantSet, GrantSource, GrantStatus,
    PRIVATE_NETWORK_KEY, PermissionDiff, StoredGrant, Tier, host_covers, missing_grants,
    missing_optional_grants, privileged_constructs, variable_covers,
};
pub use layout::{DATABASE_FILE, Layout, PLUGINS_DIR};
pub use manager::{
    Clock, ManagerOptions, NoHooks, PluginHooks, PluginManager, UpdateOutcome, VerdictChange,
    VerdictSource,
};
pub use origins::{
    MAX_USER_ORIGIN_LEN, StoredUserOrigin, USER_ORIGIN_KEY_PREFIX, UserOriginConsent,
    bound_credentials, declared_slot, open_url_user_origin_key, open_url_user_origin_setting,
    user_origin_key, user_origin_setting,
};
pub use plan::{AppliedPlan, PlannedStep, enabled_dependents};
pub use recover::{Inconsistency, RecoveryReport};
pub use state::{MigrationPhase, PluginState, TombstoneDecision};
pub use store::{
    AuditRecord, AuditRetention, FIRST_PARTY_PUBLISHER, InstallSource, JournalEntry,
    MAX_MIGRATION_ATTEMPTS, MAX_MIGRATION_REASON_CHARS, MigrationStep, ModuleMigration,
    PluginRecord, PluginStore, Tombstone, VersionRecord, VersionRole, is_first_party,
};

/// Seconds since the Unix epoch, from the system clock.
pub fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
