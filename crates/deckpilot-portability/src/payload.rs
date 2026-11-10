//! The decrypted content of an export.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use deckpilot_core::{
    ActionParameters, ModuleRegistry, ParameterValue, Profile, ProfileId, ResourceKey,
    ResourceKind, ResourceRef,
};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore, SqliteStore};
use deckpilot_update::policy::{PREFERENCES_SETTING, UpdateMode, UpdatePreferences};

use crate::{PortabilityError, archive};

/// Version of [`Payload`].
pub const PAYLOAD_VERSION: u32 = 1;

/// Portable settings of one module, in the module's own export format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleExport {
    /// The module's [`Portability::version`](deckpilot_core::Portability).
    pub version: u32,
    pub enabled: bool,
    #[serde(default)]
    pub settings: ActionParameters,
}

/// Application settings that mean the same on every computer. Settings of
/// this computer (resource bindings, security policy, brightness) and
/// runtime state (time of the last update check, confirmations) are never
/// exported: importing a security policy could silently weaken another
/// computer's protection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdateSettings>,
}

impl AppSettings {
    pub fn is_empty(&self) -> bool {
        self.updates.is_none()
    }
}

/// How updates are checked for and applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    pub mode: UpdateMode,
    pub automatic_checks: bool,
    pub check_interval_hours: u32,
}

/// Everything an export carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    pub version: u32,
    /// Seconds since the Unix epoch.
    pub exported_at: i64,
    pub app_version: String,
    pub profiles: Vec<Profile>,
    /// Keyed by module id. Kept as text so that modules this build does not
    /// know are reported instead of failing the whole import.
    #[serde(default)]
    pub modules: BTreeMap<String, ModuleExport>,
    /// Logical resources the profiles use, with their kind where known.
    /// Machine-specific values are never exported.
    #[serde(default)]
    pub resources: BTreeMap<ResourceKey, Option<ResourceKind>>,
    /// Portable application settings; absent in exports of earlier builds.
    #[serde(default)]
    pub settings: AppSettings,
}

/// Resource keys a profile refers to: project resources and fixed resource
/// parameters (including step overrides).
pub fn resource_keys(profile: &Profile) -> BTreeSet<ResourceKey> {
    let mut keys: BTreeSet<ResourceKey> = profile
        .projects
        .iter()
        .flat_map(|project| project.resources.values().cloned())
        .collect();
    let fixed = |parameters: &ActionParameters| -> Vec<ResourceKey> {
        parameters
            .iter()
            .filter_map(|(_, value)| match value {
                ParameterValue::Resource(ResourceRef::Fixed { key }) => Some(key.clone()),
                _ => None,
            })
            .collect()
    };
    for action in &profile.actions {
        keys.extend(fixed(&action.parameters));
    }
    for step in profile.workflows.iter().flat_map(|w| &w.steps) {
        keys.extend(fixed(&step.overrides));
    }
    keys
}

/// Collects the portable configuration. `profiles`: the profiles to export,
/// all when `None`. `include_modules`: export module settings.
/// What an export contains besides the selected profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    pub module_settings: bool,
    pub app_settings: bool,
}

/// The portable application settings stored in `store`.
pub fn app_settings(store: &SqliteStore) -> Result<AppSettings, PortabilityError> {
    let updates = store
        .setting(PREFERENCES_SETTING)?
        .and_then(|text| serde_json::from_str::<UpdatePreferences>(&text).ok())
        .map(|preferences| {
            let preferences = preferences.sanitized();
            UpdateSettings {
                mode: preferences.mode,
                automatic_checks: preferences.automatic_checks,
                check_interval_hours: preferences.check_interval_hours,
            }
        });
    Ok(AppSettings { updates })
}

pub fn export(
    store: &SqliteStore,
    modules: &ModuleRegistry,
    profiles: Option<&[ProfileId]>,
    options: ExportOptions,
    exported_at: i64,
    app_version: &str,
) -> Result<Payload, PortabilityError> {
    let include_modules = options.module_settings;
    let mut selected = Vec::new();
    for summary in store.list_profiles()? {
        if profiles.is_some_and(|wanted| !wanted.contains(&summary.id)) {
            continue;
        }
        if let Some(profile) = store.load_profile(summary.id)? {
            selected.push(profile);
        }
    }
    if let Some(wanted) = profiles
        && wanted.len() != selected.len()
    {
        return Err(PortabilityError::Invalid(
            "a selected profile does not exist".into(),
        ));
    }
    let machine = store.load_machine_config()?;
    let mut resources = BTreeMap::new();
    for key in selected.iter().flat_map(resource_keys) {
        let kind = machine
            .as_ref()
            .and_then(|machine| machine.binding(&key))
            .map(|binding| binding.kind);
        resources.insert(key, kind);
    }
    let mut exported_modules = BTreeMap::new();
    if include_modules {
        for info in modules.modules() {
            if let Some((version, enabled, settings)) = modules.export_settings(&info.manifest.id) {
                exported_modules.insert(
                    info.manifest.id.to_string(),
                    ModuleExport {
                        version,
                        enabled,
                        settings,
                    },
                );
            }
        }
    }
    Ok(Payload {
        version: PAYLOAD_VERSION,
        exported_at,
        app_version: app_version.into(),
        profiles: selected,
        modules: exported_modules,
        resources,
        settings: if options.app_settings {
            app_settings(store)?
        } else {
            AppSettings::default()
        },
    })
}

/// Encrypts a payload into archive bytes.
pub fn seal(
    payload: &Payload,
    password: &Password,
    kdf: KdfParams,
) -> Result<Vec<u8>, PortabilityError> {
    let plaintext = zeroize::Zeroizing::new(serde_json::to_vec(payload)?);
    archive::seal(
        &plaintext,
        password,
        kdf,
        payload.exported_at,
        &payload.app_version,
    )
}

/// Decrypts archive bytes and parses the payload.
pub fn open(bytes: &[u8], password: &Password) -> Result<Payload, PortabilityError> {
    let (_, plaintext) = archive::open(bytes, password)?;
    let payload: Payload = serde_json::from_slice(&plaintext)?;
    if payload.version == 0 || payload.version > PAYLOAD_VERSION {
        return Err(PortabilityError::UnsupportedVersion(payload.version));
    }
    Ok(payload)
}
