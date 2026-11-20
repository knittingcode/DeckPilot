//! Conversions between the plugin crates' types and the IPC shapes of
//! protocol 10, and from stored consent to broker grants.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use base64::Engine as _;

use deckpilot_ipc::plugins as ipc;
use deckpilot_plugin_abi::capability::AuthScheme;
use deckpilot_plugin_abi::{ActionOutcome, HttpMethod, ParamValue, Params};
use deckpilot_plugin_broker::{ApprovedPermission, ApprovedPermissions};
use deckpilot_plugin_manifest::{
    IconFormat, IconTheme, MAX_RASTER_ICON_BYTES, Manifest, PermissionKind, Sha256Digest,
    ViewContext,
};
use deckpilot_plugin_store::{
    AuditRecord, GrantKey, GrantScope, GrantStatus, InstallSource, PRIVATE_NETWORK_KEY,
    PermissionDiff, PluginRecord, PluginState, StoredGrant, Tier,
};

/// Most audit records returned with a plugin's details.
const AUDIT_SHOWN: usize = 50;

pub fn state(state: PluginState) -> ipc::PluginState {
    match state {
        PluginState::Installing => ipc::PluginState::Installing,
        PluginState::Disabled => ipc::PluginState::Disabled,
        PluginState::Enabled => ipc::PluginState::Enabled,
        PluginState::Updating => ipc::PluginState::Updating,
        PluginState::RollingBack => ipc::PluginState::RollingBack,
        PluginState::Uninstalling => ipc::PluginState::Uninstalling,
        PluginState::Broken => ipc::PluginState::Broken,
        PluginState::Quarantined => ipc::PluginState::Quarantined,
    }
}

fn origin(source: &InstallSource) -> ipc::PluginOrigin {
    match source {
        InstallSource::Registry { registry } => ipc::PluginOrigin::Registry {
            registry: registry.clone(),
        },
        InstallSource::Sideload => ipc::PluginOrigin::Sideload,
        // Shipped with the app and checked against the agent's pins
        // (docs/marketplace/module-migration.md §4).
        InstallSource::Bundled => ipc::PluginOrigin::Bundled,
        InstallSource::Dev => ipc::PluginOrigin::Dev,
    }
}

fn tier(tier: Tier) -> ipc::PermissionTier {
    match tier {
        Tier::Low => ipc::PermissionTier::Low,
        Tier::Medium => ipc::PermissionTier::Medium,
        Tier::High => ipc::PermissionTier::High,
    }
}

/// The manifest's justification for the permission behind `key`.
fn justification(manifest: &Manifest, key: &GrantKey) -> Option<String> {
    manifest
        .permissions
        .iter()
        .find(|p| p.kind() == key.kind)
        .map(|p| p.justification().to_owned())
}

pub fn permission(
    manifest: &Manifest,
    key: &GrantKey,
    scope: &GrantScope,
    status: ipc::PermissionStatus,
) -> ipc::PluginPermission {
    let name = key.to_string();
    let optional = manifest.optional_permissions().contains(name.as_str());
    let actions = if optional {
        manifest
            .contributes
            .actions()
            .iter()
            .filter(|a| a.requires_permissions().contains(&name))
            .map(|a| a.id.clone())
            .collect()
    } else {
        Vec::new()
    };
    ipc::PluginPermission {
        key: name,
        kind: key.kind.as_str().to_owned(),
        tier: tier(scope.tier()),
        status,
        scope: serde_json::to_value(scope).unwrap_or(serde_json::Value::Null),
        justification: justification(manifest, key),
        optional,
        actions,
    }
}

/// Every declared permission of `manifest` with its state in `stored`.
pub fn permissions(manifest: &Manifest, stored: &[StoredGrant]) -> Vec<ipc::PluginPermission> {
    deckpilot_plugin_store::GrantSet::from_manifest(manifest)
        .iter()
        .map(|(key, scope)| {
            let status = match stored.iter().find(|g| g.key == *key) {
                Some(g) if g.status == GrantStatus::Granted && g.scope.covers(scope) => {
                    ipc::PermissionStatus::Granted
                }
                Some(g) if g.status == GrantStatus::Revoked => ipc::PermissionStatus::Revoked,
                _ => ipc::PermissionStatus::NotGranted,
            };
            permission(manifest, key, scope, status)
        })
        .collect()
}

/// Why the actions of a plugin cannot run, if they cannot.
pub fn unavailable(record: &PluginRecord) -> Option<String> {
    if let Some(problem) = super::platform_problem() {
        return Some(problem);
    }
    (record.state != PluginState::Enabled).then(|| match record.state {
        PluginState::Broken => "the plugin's files were modified; reinstall it".to_owned(),
        PluginState::Quarantined => "the registry revoked this version".to_owned(),
        other => format!("the plugin is {}", other.as_str().replace('_', " ")),
    })
}

/// Why the actions of a plugin cannot run on keys and in workflows: as
/// [`unavailable`], and also while a declared permission is not granted
/// (revoked after enabling), except an optional one (plugin API 1.9,
/// [`unmet_permissions`]). The plugin stays enabled meanwhile, and
/// `invokePluginAction` still runs it (its capability calls are denied).
pub fn unavailable_with(record: &PluginRecord, stored: &[StoredGrant]) -> Option<String> {
    unavailable(record).or_else(|| {
        let manifest = &record.active()?.manifest;
        permissions(manifest, stored)
            .into_iter()
            .find(|p| !p.optional && p.status != ipc::PermissionStatus::Granted)
            .map(|p| format!("the permission `{}` is not granted", p.key))
    })
}

/// Why each action that names an optional permission in
/// `requiresPermissions` (plugin API 1.9) cannot run while its plugin can:
/// the first of them that is not granted. The other actions run.
pub fn unmet_permissions(
    manifest: &Manifest,
    stored: &[StoredGrant],
) -> std::collections::BTreeMap<String, String> {
    let missing: Vec<String> = deckpilot_plugin_store::missing_optional_grants(
        stored,
        &deckpilot_plugin_store::GrantSet::from_manifest(manifest),
    )
    .iter()
    .map(GrantKey::to_string)
    .collect();
    if missing.is_empty() {
        return std::collections::BTreeMap::new();
    }
    manifest
        .contributes
        .actions()
        .iter()
        .filter_map(|action| {
            let key = action
                .requires_permissions()
                .iter()
                .find(|key| missing.contains(key))?;
            Some((action.id.clone(), needs_permission(key)))
        })
        .collect()
}

/// The reason of an action waiting for the optional permission `key`.
pub fn needs_permission(key: &str) -> String {
    format!("needs the permission `{key}`: allow it in Settings › Plugins")
}

fn action_info(
    m: &Manifest,
    action: &deckpilot_plugin_manifest::Action,
    reason: Option<&String>,
) -> ipc::PluginActionInfo {
    ipc::PluginActionInfo {
        action: action.id.clone(),
        action_type: format!("{}.{}", m.id, action.id),
        name: action.name.clone(),
        description: Some(action.description.clone()).filter(|d| !d.is_empty()),
        available: reason.is_none(),
        unavailable_reason: reason.cloned(),
        definition: super::engine::action_type(&m.id.to_string(), action).ok(),
    }
}

/// Every action of the active version as `getPluginActions` pages them:
/// as in [`summary`], with each one's manifest entry.
pub fn action_entries(
    record: &PluginRecord,
    stored: &[StoredGrant],
) -> Vec<ipc::PluginActionEntry> {
    let Some(m) = record.active().map(|v| &v.manifest) else {
        return Vec::new();
    };
    let reason = unavailable_with(record, stored);
    let unmet = unmet_permissions(m, stored);
    m.contributes
        .actions()
        .iter()
        .map(|action| ipc::PluginActionEntry {
            action: action_info(m, action, reason.as_ref().or(unmet.get(&action.id))),
            manifest: serde_json::to_value(action).ok(),
            too_large: false,
        })
        .collect()
}

pub fn summary(record: &PluginRecord, stored: &[StoredGrant]) -> ipc::PluginSummary {
    let active = record.active();
    let manifest = active.map(|v| &v.manifest);
    let reason = unavailable_with(record, stored);
    let actions = manifest
        .map(|m| {
            let unmet = unmet_permissions(m, stored);
            m.contributes
                .actions()
                .iter()
                .map(|action| action_info(m, action, reason.as_ref().or(unmet.get(&action.id))))
                .collect()
        })
        .unwrap_or_default();
    ipc::PluginSummary {
        id: record.id.to_string(),
        name: manifest.map(|m| m.name.clone()).unwrap_or_default(),
        publisher: manifest
            .map(|m| m.publisher.name.clone())
            .unwrap_or_default(),
        version: active.map(|v| v.version.to_string()).unwrap_or_default(),
        state: state(record.state),
        state_reason: record.state_reason.clone(),
        origin: origin(&record.source),
        previous_version: record.previous().map(|v| v.version.to_string()),
        staged_version: record.staged().map(|v| v.version.to_string()),
        actions,
        action_count: None,
        health: None,
        navigation: manifest.map(navigation).unwrap_or_default(),
        installed_for: None,
        visual_assets: manifest.map(visual_assets).unwrap_or_default(),
    }
}

pub fn changes(diff: &PermissionDiff, manifest: &Manifest) -> ipc::PermissionChanges {
    ipc::PermissionChanges {
        added: diff
            .added
            .iter()
            .map(|g| {
                permission(
                    manifest,
                    &g.key,
                    &g.scope,
                    ipc::PermissionStatus::NotGranted,
                )
            })
            .collect(),
        broadened: diff
            .broadened
            .iter()
            .map(|g| permission(manifest, &g.key, &g.to, ipc::PermissionStatus::NotGranted))
            .collect(),
        removed: diff.removed.iter().map(ToString::to_string).collect(),
        escalation: diff.is_escalation(),
    }
}

fn audit(record: &AuditRecord) -> ipc::PluginAuditEntry {
    ipc::PluginAuditEntry {
        at: record.at,
        version: record.version.clone(),
        op: record.op.clone(),
        target: record.target.clone(),
        decision: record.decision.clone(),
        reason: record.reason.clone(),
    }
}

pub fn details(
    record: &PluginRecord,
    stored: &[StoredGrant],
    audit_records: &[AuditRecord],
    pending: Option<(semver::Version, PermissionDiff)>,
) -> ipc::PluginDetails {
    let active = record.active();
    let manifest = active.map(|v| &v.manifest);
    let staged = record.staged().map(|v| &v.manifest);
    ipc::PluginDetails {
        summary: summary(record, stored),
        description: manifest.map(|m| m.description.clone()).unwrap_or_default(),
        license: manifest.map(|m| m.license.clone()).unwrap_or_default(),
        homepage: manifest.and_then(|m| m.homepage.clone()),
        permissions: manifest.map(|m| permissions(m, stored)).unwrap_or_default(),
        pending_update: pending.and_then(|(version, diff)| {
            staged.map(|m| ipc::PendingPluginUpdate {
                version: version.to_string(),
                changes: changes(&diff, m),
            })
        }),
        manifest: manifest
            .and_then(|m| serde_json::to_value(m).ok())
            .unwrap_or(serde_json::Value::Null),
        audit: audit_records
            .iter()
            .rev()
            .take(AUDIT_SHOWN)
            .map(audit)
            .collect(),
        icon: None,
        storage: None,
        dependencies: Vec::new(),
        dependents: Vec::new(),
    }
}

/// The PNG signature.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Whether `bytes` may be shown as a plugin's icon: a PNG of at most
/// [`ipc::MAX_ICON_BYTES`] whose sha256 is `expected`.
fn icon_ok(bytes: &[u8], expected: &Sha256Digest) -> bool {
    use sha2::Digest as _;
    if bytes.len() > ipc::MAX_ICON_BYTES || !bytes.starts_with(PNG) {
        return false;
    }
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    digest == *expected.as_bytes()
}

/// `bytes` as a `data:` URL for [`ipc::PluginDetails::icon`]: only a PNG
/// of at most [`ipc::MAX_ICON_BYTES`] whose sha256 is `expected`.
pub fn icon_data(bytes: &[u8], expected: &Sha256Digest) -> Option<String> {
    icon_ok(bytes, expected).then(|| {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    })
}

/// The active version's 128 px icon file (`icons.128` of its manifest),
/// read from `dir` (its install directory), and the hash its integrity
/// record has for it. Anything but a regular file of at most
/// [`ipc::MAX_ICON_BYTES`] gives none.
fn icon_file(dir: &Path, record: &PluginRecord) -> Option<(Vec<u8>, Sha256Digest)> {
    let active = record.active()?;
    let relative = &active.manifest.icons.size128;
    let expected = active.integrity.files.get(relative)?;
    let path = dir.join(relative);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > ipc::MAX_ICON_BYTES as u64 {
        return None;
    }
    Some((std::fs::read(&path).ok()?, *expected))
}

/// The active version's 128 px icon as a `data:` URL, checked against
/// the hash its integrity record has for that file. Anything else gives
/// no icon.
pub fn icon(dir: &Path, record: &PluginRecord) -> Option<String> {
    let (bytes, expected) = icon_file(dir, record)?;
    icon_data(&bytes, &expected)
}

/// The chunk of the icon [`icon`] gives that starts at `offset`
/// (`getPluginIcon`), checked the same way on every request. `None`
/// without an icon to show, or for an offset that is not a chunk's start.
pub fn icon_chunk(dir: &Path, record: &PluginRecord, offset: u64) -> Option<ipc::PluginIconChunk> {
    let (bytes, expected) = icon_file(dir, record)?;
    icon_chunk_of(&bytes, &expected, offset)
}

/// The chunk of `bytes` at `offset` when [`icon_data`] would show them.
pub fn icon_chunk_of(
    bytes: &[u8],
    expected: &Sha256Digest,
    offset: u64,
) -> Option<ipc::PluginIconChunk> {
    if !icon_ok(bytes, expected) {
        return None;
    }
    // Checked above: the file's own hash.
    chunk(bytes, "image/png", expected, offset)
}

/// The chunk of `bytes` (whose sha256 is `digest`) at `offset`; `None` for
/// an offset that is not a chunk's start.
fn chunk(
    bytes: &[u8],
    media_type: &str,
    digest: &Sha256Digest,
    offset: u64,
) -> Option<ipc::PluginIconChunk> {
    let start = usize::try_from(offset).ok()?;
    if start % ipc::ICON_CHUNK_BYTES != 0 || start >= bytes.len() {
        return None;
    }
    let end = (start + ipc::ICON_CHUNK_BYTES).min(bytes.len());
    Some(ipc::PluginIconChunk {
        media_type: media_type.to_owned(),
        size: bytes.len() as u64,
        sha256: digest.to_string(),
        offset,
        data: base64::engine::general_purpose::STANDARD.encode(&bytes[start..end]),
        next: (end < bytes.len()).then_some(end as u64),
    })
}

/// The sha256 of `bytes`.
pub fn sha256_of(bytes: &[u8]) -> Sha256Digest {
    sha256(bytes)
}

fn sha256(bytes: &[u8]) -> Sha256Digest {
    use sha2::Digest as _;
    Sha256Digest::from_bytes(sha2::Sha256::digest(bytes).into())
}

/// The toolbar entries of a manifest (`contributes.navigation`, plugin API
/// 1.3) with their views, in manifest order.
pub fn navigation(manifest: &Manifest) -> Vec<ipc::PluginNavigationEntry> {
    manifest
        .contributes
        .navigation()
        .iter()
        .filter_map(|entry| {
            let view = manifest.contributes.view_of(entry)?;
            Some(ipc::PluginNavigationEntry {
                id: entry.id.clone(),
                label: entry.label.clone(),
                view: ipc::PluginView {
                    id: view.id.clone(),
                    // Plugin API 1.9 adds `workflows` and `projects` (the
                    // app's own screens, `hostViews`); an app that does
                    // not know a kind ignores the entry.
                    kind: view.kind.as_str().to_owned(),
                    // Only the app's toolbar follows it; no data goes to the
                    // plugin for it.
                    context: view
                        .context
                        .filter(|context| *context != ViewContext::None)
                        .map(|context| context.as_str().to_owned()),
                },
            })
        })
        .collect()
}

/// What the app may show of a navigation entry's icon: the file the
/// manifest names for the theme (else for the other theme), read with
/// `read`, only when its sha256 is the one `files` (the integrity record)
/// has for it, and then as [`deckpilot_plugin_manifest::display_icon`]
/// gives it (a PNG or WebP within its limits as it is, an SVG sanitized).
/// Anything else gives none, and the app shows its fallback icon.
pub fn navigation_icon_of(
    manifest: &Manifest,
    files: &BTreeMap<String, Sha256Digest>,
    navigation: &ipc::NavigationIconRef,
    read: impl FnOnce(&str) -> Option<Vec<u8>>,
) -> Option<(IconFormat, Vec<u8>)> {
    let entry = manifest
        .contributes
        .navigation()
        .iter()
        .find(|entry| entry.id == navigation.entry)?;
    let theme = match navigation.theme {
        ipc::NavigationIconTheme::Light => IconTheme::Light,
        ipc::NavigationIconTheme::Dark => IconTheme::Dark,
    };
    let relative = entry.icon.file(theme)?;
    let expected = files.get(relative)?;
    let bytes = read(relative)?;
    if bytes.len() > MAX_RASTER_ICON_BYTES || sha256(&bytes) != *expected {
        return None;
    }
    let (format, shown) = deckpilot_plugin_manifest::display_icon(relative, &bytes).ok()?;
    Some((format, shown.into_owned()))
}

/// [`navigation_icon_of`] for the active version installed in `dir`: a
/// regular file of at most [`MAX_RASTER_ICON_BYTES`] (no link).
fn navigation_icon(
    dir: &Path,
    record: &PluginRecord,
    navigation: &ipc::NavigationIconRef,
) -> Option<(IconFormat, Vec<u8>)> {
    let active = record.active()?;
    navigation_icon_of(
        &active.manifest,
        &active.integrity.files,
        navigation,
        |relative| {
            let path = dir.join(relative);
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > MAX_RASTER_ICON_BYTES as u64 {
                return None;
            }
            std::fs::read(&path).ok()
        },
    )
}

/// A navigation icon as a `data:` URL, as the app puts it in an `<img>`
/// (`plugin_answers::navigation_icon` joins the chunks into the same).
#[cfg(test)]
fn navigation_icon_data(format: IconFormat, bytes: &[u8]) -> String {
    format!(
        "data:{};base64,{}",
        format.media_type(),
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The chunk at `offset` of a navigation entry's icon (`getPluginIcon`
/// with `navigation`), checked and sanitized on every request; its size
/// and sha256 are those of the bytes sent.
pub fn navigation_icon_chunk(
    dir: &Path,
    record: &PluginRecord,
    navigation: &ipc::NavigationIconRef,
    offset: u64,
) -> Option<ipc::PluginIconChunk> {
    let (format, bytes) = navigation_icon(dir, record, navigation)?;
    chunk(&bytes, format.media_type(), &sha256(&bytes), offset)
}

/// The icons and backgrounds of a manifest (`contributes.visualAssets`,
/// plugin API 1.8), in manifest order, named `<plugin id>.<id>`. Entries
/// whose id does not form an asset id are left out (never, for a
/// validated manifest).
pub fn visual_assets(manifest: &Manifest) -> Vec<ipc::PluginVisualAsset> {
    use deckpilot_plugin_manifest::VisualBackgroundKind;
    let plugin = manifest.id.to_string();
    let full = |id: &str| {
        deckpilot_core::AssetId::of_plugin(&plugin, id)
            .ok()
            .map(|id| id.to_string())
    };
    let assets = manifest.contributes.visual_assets();
    let icons = assets.icons().iter().filter_map(|icon| {
        Some(ipc::PluginVisualAsset {
            id: full(&icon.id)?,
            asset: icon.id.clone(),
            name: icon.name.clone(),
            kind: ipc::VisualAssetKind::Icon,
            colors: Vec::new(),
            angle: None,
            keywords: icon.keywords.clone().unwrap_or_default(),
        })
    });
    let backgrounds = assets.backgrounds().iter().filter_map(|background| {
        Some(ipc::PluginVisualAsset {
            id: full(&background.id)?,
            asset: background.id.clone(),
            name: background.name.clone(),
            kind: match background.kind {
                VisualBackgroundKind::Image => ipc::VisualAssetKind::ImageBackground,
                VisualBackgroundKind::Gradient => ipc::VisualAssetKind::GradientBackground,
            },
            colors: background
                .colors
                .iter()
                .flatten()
                .map(|c| c.to_ascii_lowercase())
                .collect(),
            angle: background.angle,
            keywords: Vec::new(),
        })
    });
    icons.chain(backgrounds).collect()
}

/// The file of the visual asset `asset` (its id in the plugin) of the
/// manifest, read with `read`, only when its sha256 is the one `files` (the
/// integrity record) has for it and its content passes
/// [`deckpilot_plugin_manifest::check_visual_asset`] again. Anything else
/// (no such asset, a gradient, a changed or oversized file) gives none:
/// keys then draw their fallback.
pub fn visual_asset_of(
    manifest: &Manifest,
    files: &BTreeMap<String, Sha256Digest>,
    asset: &str,
    read: impl FnOnce(&str) -> Option<Vec<u8>>,
) -> Option<(deckpilot_plugin_manifest::VisualFormat, Vec<u8>)> {
    use deckpilot_plugin_manifest::VisualUse;
    let assets = manifest.contributes.visual_assets();
    let (relative, used) = assets
        .icons()
        .iter()
        .find(|icon| icon.id == asset)
        .map(|icon| (icon.asset.as_str(), VisualUse::Icon))
        .or_else(|| {
            assets
                .backgrounds()
                .iter()
                .find(|b| b.id == asset)
                .and_then(|b| Some((b.asset.as_deref()?, VisualUse::Background)))
        })?;
    let expected = files.get(relative)?;
    let bytes = read(relative)?;
    if bytes.len() > ipc::MAX_VISUAL_ASSET_FILE_BYTES || sha256(&bytes) != *expected {
        return None;
    }
    let (format, ..) =
        deckpilot_plugin_manifest::check_visual_asset(relative, used, &bytes).ok()?;
    Some((format, bytes))
}

/// [`visual_asset_of`] for the active version installed in `dir`: a
/// regular file (no link) of at most [`ipc::MAX_VISUAL_ASSET_FILE_BYTES`].
pub fn visual_asset_file(
    dir: &Path,
    record: &PluginRecord,
    asset: &str,
) -> Option<(deckpilot_plugin_manifest::VisualFormat, Vec<u8>)> {
    let active = record.active()?;
    visual_asset_of(
        &active.manifest,
        &active.integrity.files,
        asset,
        |relative| {
            let path = dir.join(relative);
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > ipc::MAX_VISUAL_ASSET_FILE_BYTES as u64 {
                return None;
            }
            std::fs::read(&path).ok()
        },
    )
}

/// The chunk at `offset` of a visual asset's file (`getPluginIcon` with
/// `asset`), checked on every request.
pub fn visual_asset_chunk(
    dir: &Path,
    record: &PluginRecord,
    asset: &str,
    offset: u64,
) -> Option<ipc::PluginIconChunk> {
    let (format, bytes) = visual_asset_file(dir, record, asset)?;
    chunk(&bytes, format.media_type(), &sha256(&bytes), offset)
}

/// Grant keys from a request's consent.
pub fn keys(consent: &[String]) -> Result<BTreeSet<GrantKey>, String> {
    consent
        .iter()
        .map(|text| text.parse::<GrantKey>())
        .collect()
}

fn param(value: &serde_json::Value, depth: usize) -> Result<ParamValue, String> {
    use serde_json::Value;
    match value {
        Value::Bool(b) => Ok(ParamValue::Bool(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(ParamValue::Integer)
            .or_else(|| n.as_f64().map(ParamValue::Number))
            .ok_or_else(|| "a number is out of range".to_owned()),
        Value::String(s) => Ok(ParamValue::String(s.clone())),
        Value::Array(items) if depth < deckpilot_plugin_abi::limits::MAX_VALUE_DEPTH => items
            .iter()
            .map(|item| param(item, depth + 1))
            .collect::<Result<_, _>>()
            .map(ParamValue::List),
        Value::Array(_) => Err("lists nest too deeply".to_owned()),
        Value::Null | Value::Object(_) => {
            Err("parameters are booleans, numbers, texts or lists".to_owned())
        }
    }
}

/// Request parameters as ABI 1 values.
pub fn params(values: &BTreeMap<String, serde_json::Value>) -> Result<Params, String> {
    values
        .iter()
        .map(|(key, value)| {
            param(value, 1)
                .map(|v| (key.clone(), v))
                .map_err(|e| format!("parameter `{key}`: {e}"))
        })
        .collect()
}

fn json(value: &ParamValue) -> serde_json::Value {
    match value {
        ParamValue::Bool(b) => serde_json::Value::Bool(*b),
        ParamValue::Integer(i) => serde_json::Value::from(*i),
        ParamValue::Number(n) => serde_json::Number::from_f64(*n)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        ParamValue::String(s) => serde_json::Value::String(s.clone()),
        ParamValue::List(items) => serde_json::Value::Array(items.iter().map(json).collect()),
    }
}

/// The guest's outcome for the app: only outputs the action declares.
pub fn outcome(outcome: ActionOutcome, declared: &BTreeSet<String>) -> ipc::PluginActionOutcome {
    match outcome {
        ActionOutcome::Ok { message, data } => ipc::PluginActionOutcome::Ok {
            message,
            data: data
                .iter()
                .filter(|(key, _)| declared.contains(*key))
                .map(|(key, v)| (key.clone(), json(v)))
                .collect(),
        },
        ActionOutcome::Skipped { message } => ipc::PluginActionOutcome::Skipped { message },
        ActionOutcome::Failed {
            code,
            message,
            retryable,
        } => ipc::PluginActionOutcome::Failed {
            code,
            message,
            retryable,
        },
    }
}

fn http_method(method: deckpilot_plugin_manifest::HttpMethod) -> HttpMethod {
    use deckpilot_plugin_manifest::HttpMethod as M;
    match method {
        M::Get => HttpMethod::Get,
        M::Head => HttpMethod::Head,
        M::Post => HttpMethod::Post,
        M::Put => HttpMethod::Put,
        M::Patch => HttpMethod::Patch,
        M::Delete => HttpMethod::Delete,
    }
}

fn scheme(scheme: deckpilot_plugin_manifest::CredentialScheme) -> AuthScheme {
    use deckpilot_plugin_manifest::CredentialScheme as S;
    match scheme {
        S::Bearer => AuthScheme::Bearer,
        S::Basic => AuthScheme::Basic,
        S::Header => AuthScheme::Header,
        S::Query => AuthScheme::Query,
    }
}

/// `https://<host>` for an exact host grant; `None` for wildcards and
/// the private-network grant, which the broker never grants (exact
/// origins only).
fn origin_of(host: &str) -> Option<String> {
    (host != PRIVATE_NETWORK_KEY && !host.starts_with("*.")).then(|| format!("https://{host}"))
}

/// What the broker may allow, from the effective grants of an enabled
/// plugin (granted ∩ declared). Kinds the broker does not decide
/// (variables, background, `device:icons`, `hostViews`) are not passed;
/// their capabilities do not exist in ABI 1. Plugin API 1.9's kinds and
/// the profile read are passed; the executor carries out what has landed
/// (`executor/`) and refuses the rest as not implemented. An
/// `openUrl` grant is passed per exact host, like `network`; a wildcard
/// host is not (the broker opens exact origins only).
pub fn approved(effective: &[(GrantKey, GrantScope)]) -> (ApprovedPermissions, Vec<String>) {
    approved_with(effective, Vec::new())
}

/// [`approved`], plus the user-entered origins the user consented to
/// (G2, `origins::approved`). A credential slot with no fixed host is
/// passed when one of them binds it, so its secret goes there only.
pub fn approved_with(
    effective: &[(GrantKey, GrantScope)],
    user_origins: Vec<ApprovedPermission>,
) -> (ApprovedPermissions, Vec<String>) {
    let bound: BTreeSet<String> = user_origins
        .iter()
        .flat_map(|p| match p {
            ApprovedPermission::UserOrigin { credentials, .. } => credentials.clone(),
            _ => BTreeSet::new(),
        })
        .collect();
    let mut out = user_origins;
    let mut skipped = Vec::new();
    for (key, scope) in effective {
        match (key.kind, scope) {
            // Passed as the consented origin itself (`user_origins`), and
            // plugin API 1.9's kinds and the profile read, passed below,
            // one record each.
            (PermissionKind::Network, GrantScope::UserOriginSlot { .. })
            | (
                PermissionKind::Tools
                | PermissionKind::Resources
                | PermissionKind::ProjectContext
                | PermissionKind::Profile,
                _,
            )
            | (PermissionKind::OpenUrl, GrantScope::OpenUrlFromParams) => {}
            (PermissionKind::Network, GrantScope::Host { methods }) => match origin_of(&key.key) {
                Some(origin) => out.push(ApprovedPermission::Network {
                    origin,
                    methods: methods.iter().copied().map(http_method).collect(),
                }),
                None => skipped.push(key.to_string()),
            },
            (PermissionKind::Notifications, _) => out.push(ApprovedPermission::Notifications {}),
            // Standard dialogs (plugin API 1.5).
            (PermissionKind::Dialogs, _) => out.push(ApprovedPermission::Dialogs {}),
            // Key state (plugin API 1.2): the plugin describes what its
            // keys show. `device:icons` gives nothing yet.
            (PermissionKind::Device, _) if key.key == "state" => {
                out.push(ApprovedPermission::KeyState {});
            }
            // Passed with the consented origin itself (`user_origins`).
            (PermissionKind::OpenUrl, GrantScope::OpenUrl)
                if deckpilot_plugin_store::open_url_user_origin_setting(key).is_some() => {}
            (PermissionKind::OpenUrl, GrantScope::OpenUrl) => match origin_of(&key.key) {
                Some(origin) => out.push(ApprovedPermission::OpenUrl { origin }),
                None => skipped.push(key.to_string()),
            },
            // The larger quota; without it the default applies.
            (PermissionKind::Storage, GrantScope::Storage { quota_kb }) => {
                out.push(ApprovedPermission::Storage {
                    quota_kb: *quota_kb,
                });
            }
            (PermissionKind::Credentials, GrantScope::Credential { hosts, schemes, .. }) => {
                let origins: Option<BTreeSet<String>> =
                    hosts.iter().map(|h| origin_of(h)).collect();
                match origins {
                    Some(origins)
                        if (!origins.is_empty() || bound.contains(&key.key))
                            && !schemes.is_empty() =>
                    {
                        out.push(ApprovedPermission::Credential {
                            key: key.key.clone(),
                            origins,
                            schemes: schemes.iter().copied().map(scheme).collect(),
                        });
                    }
                    _ => skipped.push(key.to_string()),
                }
            }
            // `hostViews` places the app's own views and decides nothing
            // in the broker; nor do the kinds without an operation.
            _ => skipped.push(key.to_string()),
        }
    }
    out.extend(privileged(effective));
    (ApprovedPermissions(out), skipped)
}

/// The broker records of plugin API 1.9's kinds (first-party bundled
/// plugins only, which the manifest rules and the store guarantee) and of
/// the `profile` read: one per tool with its sub-grants, one for the
/// resources, one for the project context. A sub-grant whose tool grant
/// was revoked gives nothing. The executor carries the operations out
/// (`executor/`), answering those not landed yet as not implemented.
fn privileged(effective: &[(GrantKey, GrantScope)]) -> Vec<ApprovedPermission> {
    use deckpilot_plugin_abi::privileged::Tool;
    use deckpilot_plugin_broker::ProfileScope;
    let has = |scope: &GrantScope| effective.iter().any(|(_, s)| s == scope);
    let has_key = |text: &str| effective.iter().any(|(k, _)| k.to_string() == text);
    let mut out = Vec::new();
    for (tool, name) in [(Tool::Git, "git"), (Tool::Docker, "docker")] {
        if has_key(&format!("tools:{name}")) {
            out.push(ApprovedPermission::Tool {
                tool,
                network: tool == Tool::Git && has_key("tools:git:network"),
                modify: tool == Tool::Docker && has_key("tools:docker:modify"),
            });
        }
    }
    if has(&GrantScope::Resources) {
        out.push(ApprovedPermission::Resources {
            paths: has(&GrantScope::ResourcePaths),
            open: has(&GrantScope::ResourceOpen),
        });
    }
    if has(&GrantScope::OpenUrlFromParams) {
        out.push(ApprovedPermission::OpenUrlFromParams {});
    }
    if has(&GrantScope::ProjectRead) {
        out.push(ApprovedPermission::ProjectContext {
            select: has(&GrantScope::ProjectSelect),
        });
    }
    let read: BTreeSet<ProfileScope> = effective
        .iter()
        .filter_map(|(_, scope)| match scope {
            GrantScope::Profile { read } => Some(read),
            _ => None,
        })
        .flatten()
        .map(|scope| match scope {
            deckpilot_plugin_manifest::ProfileScope::Profile => ProfileScope::Profile,
            deckpilot_plugin_manifest::ProfileScope::Project => ProfileScope::Project,
        })
        .collect();
    if !read.is_empty() {
        out.push(ApprovedPermission::Profile { read });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_recorded_png_becomes_an_icon() {
        use sha2::Digest as _;
        let digest = |bytes: &[u8]| Sha256Digest::from_bytes(sha2::Sha256::digest(bytes).into());
        let png = [PNG, b"rest of the image"].concat();
        let url = icon_data(&png, &digest(&png)).unwrap();
        assert_eq!(
            url,
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&png)
            )
        );
        // Modified after install: not the recorded hash.
        let other = [PNG, b"another image"].concat();
        assert_eq!(icon_data(&other, &digest(&png)), None);
        // Not a PNG, even with the right hash.
        let svg = b"<svg onload=alert(1)/>";
        assert_eq!(icon_data(svg, &digest(svg)), None);
        // Too large.
        let large = [PNG, &vec![0; ipc::MAX_ICON_BYTES]].concat();
        assert_eq!(icon_data(&large, &digest(&large)), None);
    }

    #[test]
    fn icon_chunks_join_into_the_icons_data_url() {
        use sha2::Digest as _;
        let digest = |bytes: &[u8]| Sha256Digest::from_bytes(sha2::Sha256::digest(bytes).into());
        let png: Vec<u8> = [
            PNG.to_vec(),
            (0..200 * 1024_u32)
                .map(|i| u8::try_from(i % 251).unwrap())
                .collect(),
        ]
        .concat();
        let expected = digest(&png);
        let mut offset = 0;
        let mut joined = String::new();
        let mut chunks = 0;
        loop {
            let chunk = icon_chunk_of(&png, &expected, offset).unwrap();
            assert_eq!(chunk.offset, offset);
            assert_eq!(chunk.size, png.len() as u64);
            assert_eq!(chunk.sha256, expected.to_string());
            joined.push_str(&chunk.data);
            chunks += 1;
            match chunk.next {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(chunks, png.len().div_ceil(ipc::ICON_CHUNK_BYTES));
        assert_eq!(
            format!("data:image/png;base64,{joined}"),
            icon_data(&png, &expected).unwrap()
        );
        // Only chunk starts, only the recorded file.
        assert_eq!(icon_chunk_of(&png, &expected, 1), None);
        assert_eq!(icon_chunk_of(&png, &expected, png.len() as u64), None);
        assert_eq!(icon_chunk_of(&png, &digest(b"other"), 0), None);
    }

    /// A plugin API 1.3 manifest with two navigation entries: one with a
    /// light SVG and a dark PNG, one with a single WebP.
    fn navigation_manifest() -> Manifest {
        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../plugins/examples/hello-weather/manifest.json"
        ))
        .unwrap();
        value["engines"]["pluginApi"] = serde_json::json!("^1.3");
        value["contributes"]["navigation"] = serde_json::json!([
            {"id": "dashboard", "label": "Weather", "view": "live",
             "icon": {"light": "assets/nav.svg", "dark": "assets/nav-dark.png"}},
            {"id": "setup", "label": "Setup", "view": "config", "icon": "assets/setup.webp"}
        ]);
        value["contributes"]["views"] = serde_json::json!([
            {"id": "live", "kind": "variables"}, {"id": "config", "kind": "settings"}
        ]);
        let manifest: Manifest = serde_json::from_value(value).unwrap();
        manifest.validate().unwrap();
        manifest
    }

    const NAV_SVG: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><!-- c --><path d="M0 0h24v24H0z"/></svg>"#;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG.to_vec();
        bytes.extend(b"\0\0\0\x0dIHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    fn nav(entry: &str, theme: ipc::NavigationIconTheme) -> ipc::NavigationIconRef {
        ipc::NavigationIconRef {
            entry: entry.into(),
            theme,
        }
    }

    /// Plugin API 1.4: a view's context reaches the app as declared, `none`
    /// left out; it changes none of the plugin's permissions.
    #[test]
    fn navigation_views_carry_their_context() {
        let mut value = serde_json::to_value(navigation_manifest()).unwrap();
        value["engines"]["pluginApi"] = serde_json::json!("^1.4");
        let before: Manifest = serde_json::from_value(value.clone()).unwrap();
        value["contributes"]["views"] = serde_json::json!([
            {"id": "live", "kind": "variables", "context": "device+profile"},
            {"id": "config", "kind": "settings", "context": "none"}
        ]);
        let manifest: Manifest = serde_json::from_value(value).unwrap();
        manifest.validate().unwrap();
        let entries = navigation(&manifest);
        assert_eq!(entries[0].view.context.as_deref(), Some("device+profile"));
        assert_eq!(entries[1].view.context, None);
        assert_eq!(
            serde_json::to_value(&entries[1]).unwrap()["view"],
            serde_json::json!({"id": "config", "kind": "settings"})
        );
        // The declaration grants nothing: the same permissions as without it.
        assert_eq!(permissions(&manifest, &[]), permissions(&before, &[]));
        assert_eq!(manifest.permissions, before.permissions);
    }

    #[test]
    fn navigation_entries_list_their_views() {
        let manifest = navigation_manifest();
        let entries = navigation(&manifest);
        assert_eq!(
            serde_json::to_value(&entries).unwrap(),
            serde_json::json!([
                {"id": "dashboard", "label": "Weather", "view": {"id": "live", "kind": "variables"}},
                {"id": "setup", "label": "Setup", "view": {"id": "config", "kind": "settings"}}
            ])
        );
        let plain: Manifest = serde_json::from_str(include_str!(
            "../../../../plugins/examples/hello-weather/manifest.json"
        ))
        .unwrap();
        assert_eq!(navigation(&plain), Vec::new());
    }

    /// The icon the app gets as a `data:` URL, and every case that gives
    /// none (so the app shows its fallback icon).
    #[test]
    fn navigation_icons_are_checked_and_sanitized_or_absent() {
        use ipc::NavigationIconTheme::{Dark, Light};
        let manifest = navigation_manifest();
        let dark = png(32, 32);
        let mut webp = b"RIFF\x12\0\0\0WEBPVP8L\x05\0\0\0\x2f".to_vec();
        webp.extend((15u32 | (15 << 14)).to_le_bytes());
        webp.push(0);
        let disk: BTreeMap<String, Vec<u8>> = [
            ("assets/nav.svg", NAV_SVG.to_vec()),
            ("assets/nav-dark.png", dark.clone()),
            ("assets/setup.webp", webp.clone()),
        ]
        .into_iter()
        .map(|(path, bytes)| (path.to_owned(), bytes))
        .collect();
        let record: BTreeMap<String, Sha256Digest> =
            disk.iter().map(|(p, b)| (p.clone(), sha256(b))).collect();
        let show = |navigation: &ipc::NavigationIconRef,
                    files: &BTreeMap<String, Sha256Digest>,
                    disk: &BTreeMap<String, Vec<u8>>| {
            navigation_icon_of(&manifest, files, navigation, |p| disk.get(p).cloned())
                .map(|(format, bytes)| navigation_icon_data(format, &bytes))
        };

        // The light SVG, sanitized (the comment is gone).
        let light = show(&nav("dashboard", Light), &record, &disk).unwrap();
        let clean = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>"#;
        assert_eq!(
            light,
            format!(
                "data:image/svg+xml;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(clean)
            )
        );
        assert!(
            show(&nav("dashboard", Dark), &record, &disk)
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        // One file for both themes.
        for theme in [Light, Dark] {
            assert!(
                show(&nav("setup", theme), &record, &disk)
                    .unwrap()
                    .starts_with("data:image/webp;base64,")
            );
        }

        // No such entry.
        assert_eq!(show(&nav("nowhere", Light), &record, &disk), None);
        // Not in the integrity record.
        let mut unrecorded = record.clone();
        unrecorded.remove("assets/nav.svg");
        assert_eq!(show(&nav("dashboard", Light), &unrecorded, &disk), None);
        // Missing on disk.
        let mut missing = disk.clone();
        missing.remove("assets/nav-dark.png");
        assert_eq!(show(&nav("dashboard", Dark), &record, &missing), None);
        // Modified after install.
        let mut modified = disk.clone();
        modified.insert("assets/nav-dark.png".into(), png(33, 32));
        assert_eq!(show(&nav("dashboard", Dark), &record, &modified), None);
        // Unsafe or oversized, even when recorded (a store written by an
        // older agent, or a check that got stricter).
        for (path, bytes) in [
            (
                "assets/nav.svg",
                br#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>"#.to_vec(),
            ),
            (
                "assets/nav.svg",
                br#"<svg xmlns="http://www.w3.org/2000/svg"><use href="https://e.example/x#a"/></svg>"#
                    .to_vec(),
            ),
            ("assets/nav-dark.png", png(2048, 32)),
            ("assets/nav-dark.png", NAV_SVG.to_vec()),
        ] {
            let mut disk = disk.clone();
            disk.insert(path.into(), bytes);
            let record: BTreeMap<String, Sha256Digest> =
                disk.iter().map(|(p, b)| (p.clone(), sha256(b))).collect();
            let theme = if path == "assets/nav.svg" { Light } else { Dark };
            assert_eq!(show(&nav("dashboard", theme), &record, &disk), None, "{path}");
        }
    }

    #[test]
    fn navigation_icon_chunks_carry_the_media_type_and_the_sent_bytes() {
        let svg = deckpilot_plugin_manifest::sanitize_svg(NAV_SVG).unwrap();
        let first = chunk(svg.as_bytes(), "image/svg+xml", &sha256(svg.as_bytes()), 0).unwrap();
        assert_eq!(first.media_type, "image/svg+xml");
        assert_eq!(first.size, svg.len() as u64);
        assert_eq!(first.sha256, sha256(svg.as_bytes()).to_string());
        assert_eq!(first.next, None);
    }

    #[test]
    fn parameters_become_abi_values() {
        let input: BTreeMap<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({
                "name": "Ada", "count": 2, "ratio": 0.5, "on": true, "tags": ["a", ["b"]]
            }))
            .unwrap();
        let params = params(&input).unwrap();
        assert_eq!(params["count"], ParamValue::Integer(2));
        assert_eq!(params["ratio"], ParamValue::Number(0.5));
        for bad in [serde_json::json!(null), serde_json::json!({"x": 1})] {
            let input = BTreeMap::from([("p".to_owned(), bad)]);
            assert!(super::params(&input).is_err());
        }
    }

    #[test]
    fn only_exact_origins_reach_the_broker() {
        let host = |h: &str| {
            (
                GrantKey::new(PermissionKind::Network, h),
                GrantScope::Host {
                    methods: vec![deckpilot_plugin_manifest::HttpMethod::Get],
                },
            )
        };
        let (approved, skipped) = approved(&[
            host("api.example.com"),
            host("*.example.com"),
            (
                GrantKey::new(PermissionKind::Network, PRIVATE_NETWORK_KEY),
                GrantScope::PrivateNetwork,
            ),
            (
                GrantKey::new(PermissionKind::Notifications, ""),
                GrantScope::Notifications,
            ),
            (
                GrantKey::new(PermissionKind::OpenUrl, "github.com"),
                GrantScope::OpenUrl,
            ),
            (
                GrantKey::new(PermissionKind::OpenUrl, "*.github.com"),
                GrantScope::OpenUrl,
            ),
        ]);
        assert_eq!(
            approved.0,
            [
                ApprovedPermission::Network {
                    origin: "https://api.example.com".into(),
                    methods: [HttpMethod::Get].into(),
                },
                ApprovedPermission::Notifications {},
                ApprovedPermission::OpenUrl {
                    origin: "https://github.com".into(),
                },
            ]
        );
        assert_eq!(
            skipped,
            [
                "network:*.example.com",
                "network:private-network",
                "openUrl:*.github.com"
            ]
        );
    }

    /// Plugin API 1.9's grants reach the broker as one record per tool,
    /// resources and project context; a sub-grant without its tool gives
    /// nothing, and `hostViews` decides nothing there.
    #[test]
    fn privileged_grants_become_one_broker_record_each() {
        use deckpilot_plugin_abi::privileged::Tool;
        let key = |text: &str| text.parse::<GrantKey>().unwrap();
        let (approved, skipped) = approved(&[
            (
                key("tools:git"),
                GrantScope::Tool {
                    subcommands: BTreeMap::new(),
                },
            ),
            (key("tools:git:network"), GrantScope::ToolNetwork),
            // Its tool grant was revoked: the sub-grant gives nothing.
            (key("tools:docker:modify"), GrantScope::ToolModify),
            (key("resources"), GrantScope::Resources),
            (key("resources:open"), GrantScope::ResourceOpen),
            (key("openUrl:from-params"), GrantScope::OpenUrlFromParams),
            (key("projectContext"), GrantScope::ProjectRead),
            (key("hostViews"), GrantScope::HostViews),
            (
                key("profile"),
                GrantScope::Profile {
                    read: vec![deckpilot_plugin_manifest::ProfileScope::Project],
                },
            ),
        ]);
        assert_eq!(
            approved.0,
            [
                ApprovedPermission::Tool {
                    tool: Tool::Git,
                    network: true,
                    modify: false,
                },
                ApprovedPermission::Resources {
                    paths: false,
                    open: true,
                },
                ApprovedPermission::OpenUrlFromParams {},
                ApprovedPermission::ProjectContext { select: false },
                ApprovedPermission::Profile {
                    read: [deckpilot_plugin_broker::ProfileScope::Project].into(),
                },
            ]
        );
        assert_eq!(skipped, ["hostViews"]);
        deckpilot_plugin_broker::Grants::derive(&approved).unwrap();
    }
}
