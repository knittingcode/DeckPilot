//! First-party plugins shipped inside the application bundle
//! (`docs/marketplace/module-migration.md` §4): seeding them into the
//! plugin store, offline, every time the store opens (at start-up or at
//! unlock), and checking them against the pins compiled into the agent
//! every time one is loaded.
//!
//! Seeding uses the normal lifecycle (install, update, enable) with the
//! install source `bundled`, and it is conservative:
//!
//! * **Pins first.** The files in `bundled/<id>/` must have exactly the
//!   sizes and digests compiled into this agent
//!   ([`deckpilot_plugin_bundled::EMBEDDED_PINS`]); the `pins.json` next to
//!   them is never trusted. On macOS the app bundle's code seal must also
//!   hold ([`deckpilot_plugin_bundled::seal`]).
//! * **Tombstones win.** A plugin the user uninstalled or disabled is never
//!   installed, updated or enabled again; only the user undoes that.
//! * **No silent broad permissions.** Only the low-tier permissions the
//!   manifest declares are granted (`derived`, decided by
//!   `bundled:<app version>`). A plugin that declares anything else stays
//!   disabled until the user consents in the normal dialog; one with
//!   low-tier permissions only is enabled once, right after it is first
//!   installed, and never again by seeding.
//! * **Never backwards.** A copy from another source (a registry update) or
//!   a newer version is left alone; only an older `bundled` copy is updated.
//! * **`default` class only.** `migration`-class plugins are installed by
//!   the migration engine, never here.

use std::path::{Path, PathBuf};

use deckpilot_plugin_bundled::{BundleClass, Pin, Pins, seal};
use deckpilot_plugin_manifest::{Manifest, PluginId};
use deckpilot_plugin_store::{
    Consent, GrantKey, GrantSet, InstallSource, PluginManager, PluginRecord, Tier, UpdateOutcome,
};
use tracing::{info, warn};

use super::PluginSettings;

/// Test only (feature `test-hooks`): pins read from this file instead of
/// the ones compiled in, so the end-to-end test can bundle a plugin it
/// built after the agent. Never read by a release build.
#[cfg(feature = "test-hooks")]
pub const TEST_PINS_ENV: &str = "DECKPILOT_TEST_BUNDLED_PINS";

/// The application's bundled plugins as this agent sees them.
#[derive(Debug, Clone)]
pub(super) struct Bundle {
    /// `bundled/` in the application's resources, if there is one.
    pub dir: Option<PathBuf>,
    /// The pins compiled into the agent; empty if they do not parse.
    pub pins: Pins,
    /// Whether files may be seeded from `dir` (the code seal on macOS);
    /// `Err` says why not. Installed copies are checked either way.
    pub trust: Result<(), String>,
}

impl Bundle {
    /// The bundle of the running agent: the resources next to it (or the
    /// development directory `--bundled-plugins`, honoured only in debug
    /// and test builds), the compiled pins, and the seal check.
    pub fn locate(settings: &PluginSettings) -> Self {
        let exe = std::env::current_exe().ok();
        let dir = match &settings.bundled {
            Some(dir) if cfg!(any(debug_assertions, feature = "test-hooks")) => Some(dir.clone()),
            Some(dir) => {
                warn!(dir = %dir.display(), "--bundled-plugins is for development builds; ignored");
                exe.as_deref().and_then(deckpilot_plugin_bundled::locate)
            }
            None => exe.as_deref().and_then(deckpilot_plugin_bundled::locate),
        };
        let pins = pins();
        let trust = match (&dir, exe.as_deref()) {
            (None, _) => Ok(()),
            // An explicit development directory is outside any app bundle.
            (Some(_), _) if settings.bundled.is_some() && cfg!(debug_assertions) => Ok(()),
            (Some(_), Some(exe)) => {
                let checked = seal::check(exe);
                let decision = seal::decide(&checked, cfg!(debug_assertions));
                match &decision {
                    Ok(()) => info!(seal = %checked, "bundled plugins: code seal"),
                    Err(why) => warn!(%why, "bundled plugins are not seeded"),
                }
                decision
            }
            (Some(_), None) => Err("the agent's own path is unknown".to_owned()),
        };
        Self { dir, pins, trust }
    }

    /// The pin of the installed copy `record`, if it is a bundled copy.
    pub fn pin_of(&self, record: &PluginRecord) -> Option<&Pin> {
        let active = record.active()?;
        self.pins.pinned(&record.id, &active.version)
    }
}

/// The pins compiled into this agent (or, in test builds, the file named
/// by [`TEST_PINS_ENV`]). Pins that do not parse pin nothing.
fn pins() -> Pins {
    #[cfg(feature = "test-hooks")]
    if let Some(path) = std::env::var_os(TEST_PINS_ENV).filter(|p| !p.is_empty()) {
        return match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| Pins::parse(&bytes).map_err(|e| e.to_string()))
        {
            Ok(pins) => pins,
            Err(e) => {
                warn!(error = %e, "test pins refused; nothing is bundled");
                Pins::empty()
            }
        };
    }
    Pins::embedded().unwrap_or_else(|e| {
        warn!(error = %e, "the compiled bundled-plugin pins are invalid; nothing is bundled");
        Pins::empty()
    })
}

/// What seeding did with one pinned plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Seeded {
    /// Installed and enabled (low-tier permissions only).
    Enabled,
    /// Installed, disabled until the user consents to the rest.
    AwaitingConsent(Vec<GrantKey>),
    /// An older bundled copy was updated to the pinned version.
    Updated,
    /// The update is staged until the user approves its new permissions.
    UpdateAwaitingConsent,
    /// The installed copy is the pinned one and its files still match.
    Current,
    /// The installed copy's files differ from the pins: marked broken.
    Broken(String),
    /// Left alone, with the reason.
    Skipped(String),
    /// Refused or failed, with the reason; nothing was changed.
    Failed(String),
}

/// `bundled:<app version>`: who decided the grants of a seeded plugin.
pub(super) fn decided_by() -> String {
    format!("bundled:{}", crate::service::VERSION)
}

/// The low-tier permissions `manifest` declares (what seeding may grant),
/// and the others (what needs the user's consent).
fn split_by_tier(manifest: &Manifest) -> (Vec<GrantKey>, Vec<GrantKey>) {
    let declared = GrantSet::from_manifest(manifest);
    let (low, other): (Vec<_>, Vec<_>) = declared
        .iter()
        .partition(|(_, scope)| scope.tier() == Tier::Low);
    (
        low.into_iter().map(|(key, _)| key.clone()).collect(),
        other.into_iter().map(|(key, _)| key.clone()).collect(),
    )
}

/// Seeds every pinned plugin of `bundle` into `manager`.
pub(super) fn seed(manager: &mut PluginManager, bundle: &Bundle) -> Vec<(PluginId, Seeded)> {
    let mut report = Vec::new();
    if let Some(dir) = &bundle.dir {
        for extra in deckpilot_plugin_bundled::unpinned_entries(dir, &bundle.pins) {
            warn!(%extra, "bundled/ has an entry no pin names; ignored");
        }
    }
    for pin in &bundle.pins.plugins {
        let outcome = seed_one(manager, bundle, pin);
        match &outcome {
            Seeded::Failed(why) | Seeded::Broken(why) => {
                warn!(plugin = %pin.id, version = %pin.version, %why, "bundled plugin");
            }
            other => info!(plugin = %pin.id, version = %pin.version, ?other, "bundled plugin"),
        }
        report.push((pin.id.clone(), outcome));
    }
    report
}

fn seed_one(manager: &mut PluginManager, bundle: &Bundle, pin: &Pin) -> Seeded {
    if pin.class != BundleClass::Default {
        return Seeded::Skipped(format!(
            "{} class: installed by the migration engine only",
            pin.class
        ));
    }
    match manager.store().tombstone(&pin.id) {
        Ok(Some(tombstone)) => {
            return Seeded::Skipped(format!("{} by the user", tombstone.decision));
        }
        Ok(None) => {}
        Err(e) => return Seeded::Failed(e.to_string()),
    }
    let record = match manager.store().plugin(&pin.id) {
        Ok(record) => record,
        Err(e) => return Seeded::Failed(e.to_string()),
    };
    match record {
        None => install(manager, bundle, pin),
        Some(record) => refresh(manager, bundle, pin, &record),
    }
}

/// The plugin's files in `bundled/`, checked against `pin`.
fn verified(
    bundle: &Bundle,
    pin: &Pin,
) -> Result<deckpilot_plugin_package::VerifiedPackage, String> {
    bundle.trust.clone()?;
    let dir = bundle
        .dir
        .as_ref()
        .ok_or("the application has no bundled plugins directory")?;
    deckpilot_plugin_bundled::verify_bundled(&dir.join(pin.id.as_str()), pin)
        .map_err(|e| e.to_string())
}

fn install(manager: &mut PluginManager, bundle: &Bundle, pin: &Pin) -> Seeded {
    let package = match verified(bundle, pin) {
        Ok(package) => package,
        Err(why) => return Seeded::Failed(why),
    };
    let (low, other) = split_by_tier(package.manifest());
    if let Err(e) = manager.install(
        &package,
        &InstallSource::Bundled,
        &Consent::of(low, decided_by()),
    ) {
        return Seeded::Failed(e.to_string());
    }
    if !other.is_empty() {
        return Seeded::AwaitingConsent(other);
    }
    if !super::execution_supported() {
        return Seeded::Skipped("installed; plugins cannot run on this platform".into());
    }
    match manager.enable(&pin.id) {
        Ok(()) => Seeded::Enabled,
        Err(e) => Seeded::Failed(format!("installed, but not enabled: {e}")),
    }
}

fn refresh(
    manager: &mut PluginManager,
    bundle: &Bundle,
    pin: &Pin,
    record: &PluginRecord,
) -> Seeded {
    if record.source != InstallSource::Bundled {
        return Seeded::Skipped(format!(
            "installed from {}; the bundled copy never replaces it",
            record.source.as_str()
        ));
    }
    let Some(active) = record.active() else {
        return Seeded::Skipped(format!("{} (no active version)", record.state));
    };
    if active.version > pin.version {
        return Seeded::Skipped(format!(
            "version {} is newer than the bundled {}",
            active.version, pin.version
        ));
    }
    if active.version == pin.version {
        let dir = manager.layout().resolve(&active.install_dir);
        return match deckpilot_plugin_bundled::verify_installed(&dir, pin) {
            Ok(()) => Seeded::Current,
            Err(e) => {
                let why = e.to_string();
                let _ = manager.mark_broken(&pin.id, &why);
                Seeded::Broken(why)
            }
        };
    }
    if record.staged().is_some() {
        return Seeded::Skipped("an update waits for the user's approval".into());
    }
    let package = match verified(bundle, pin) {
        Ok(package) => package,
        Err(why) => return Seeded::Failed(why),
    };
    match manager.update(&package, &InstallSource::Bundled) {
        Ok(UpdateOutcome::Applied { .. }) => Seeded::Updated,
        Ok(UpdateOutcome::AwaitingConsent { diff, .. }) => {
            // New low-tier permissions are what seeding may grant; anything
            // else stays staged for the user.
            let low = diff.added.iter().all(|g| g.tier == Tier::Low)
                && diff.broadened.iter().all(|b| b.tier == Tier::Low);
            if !low {
                return Seeded::UpdateAwaitingConsent;
            }
            let keys = diff.escalations();
            match manager.approve_update(&pin.id, &Consent::of(keys, decided_by())) {
                Ok(_) => Seeded::Updated,
                Err(e) => Seeded::Failed(format!("the update was staged, not approved: {e}")),
            }
        }
        Err(e) => Seeded::Failed(e.to_string()),
    }
}

/// At every load of `record`: a bundled copy must be a pinned version
/// whose installed files (in `dir`) still match the pins. A mismatch marks
/// the plugin broken. Other sources are not pinned (`Ok(None)`).
pub(super) fn check_installed<'a>(
    manager: &mut PluginManager,
    bundle: &'a Bundle,
    record: &PluginRecord,
    dir: &Path,
) -> Result<Option<&'a Pin>, String> {
    if record.source != InstallSource::Bundled {
        return Ok(None);
    }
    let problem = match bundle.pin_of(record) {
        None => format!(
            "this agent pins no bundled {} {}",
            record.id,
            record
                .active()
                .map(|v| v.version.to_string())
                .unwrap_or_default()
        ),
        Some(pin) => match deckpilot_plugin_bundled::verify_installed(dir, pin) {
            Ok(()) => return Ok(Some(pin)),
            Err(e) => e.to_string(),
        },
    };
    let _ = manager.mark_broken(&record.id, &problem);
    Err(problem)
}

/// The module bytes about to go to the host are the pinned ones.
pub(super) fn check_module(
    manager: &mut PluginManager,
    record: &PluginRecord,
    pin: Option<&Pin>,
    module: &[u8],
) -> Result<(), String> {
    let Some(pin) = pin else {
        return Ok(());
    };
    deckpilot_plugin_bundled::check_module(pin, module).map_err(|e| {
        let why = e.to_string();
        let _ = manager.mark_broken(&record.id, &why);
        why
    })
}

#[cfg(test)]
mod tests;
