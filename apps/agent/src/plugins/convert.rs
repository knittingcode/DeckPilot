//! Conversions between the plugin crates' types and the IPC shapes of
//! protocol 10, and from stored consent to broker grants.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use base64::Engine as _;

use deckpilot_ipc::plugins as ipc;
use deckpilot_plugin_abi::capability::AuthScheme;
use deckpilot_plugin_abi::{ActionOutcome, HttpMethod, ParamValue, Params};
use deckpilot_plugin_broker::{ApprovedPermission, ApprovedPermissions};
use deckpilot_plugin_manifest::{Manifest, PermissionKind, Sha256Digest};
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
    ipc::PluginPermission {
        key: key.to_string(),
        kind: key.kind.as_str().to_owned(),
        tier: tier(scope.tier()),
        status,
        scope: serde_json::to_value(scope).unwrap_or(serde_json::Value::Null),
        justification: justification(manifest, key),
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
/// (revoked after enabling). The plugin stays enabled meanwhile, and
/// `invokePluginAction` still runs it (its capability calls are denied).
pub fn unavailable_with(record: &PluginRecord, stored: &[StoredGrant]) -> Option<String> {
    unavailable(record).or_else(|| {
        let manifest = &record.active()?.manifest;
        permissions(manifest, stored)
            .into_iter()
            .find(|p| p.status != ipc::PermissionStatus::Granted)
            .map(|p| format!("the permission `{}` is not granted", p.key))
    })
}

pub fn summary(record: &PluginRecord, stored: &[StoredGrant]) -> ipc::PluginSummary {
    let active = record.active();
    let manifest = active.map(|v| &v.manifest);
    let reason = unavailable_with(record, stored);
    let actions = manifest
        .map(|m| {
            m.contributes
                .actions()
                .iter()
                .map(|action| ipc::PluginActionInfo {
                    action: action.id.clone(),
                    action_type: format!("{}.{}", m.id, action.id),
                    name: action.name.clone(),
                    description: Some(action.description.clone()).filter(|d| !d.is_empty()),
                    available: reason.is_none(),
                    unavailable_reason: reason.clone(),
                    definition: super::engine::action_type(&m.id.to_string(), action).ok(),
                })
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
        health: None,
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
    }
}

/// The PNG signature.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// `bytes` as a `data:` URL for [`ipc::PluginDetails::icon`]: only a PNG
/// of at most [`ipc::MAX_ICON_BYTES`] whose sha256 is `expected`.
pub fn icon_data(bytes: &[u8], expected: &Sha256Digest) -> Option<String> {
    use sha2::Digest as _;
    if bytes.len() > ipc::MAX_ICON_BYTES || !bytes.starts_with(PNG) {
        return None;
    }
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    (digest == *expected.as_bytes()).then(|| {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    })
}

/// The active version's 128 px icon (`icons.128` of its manifest), read
/// from `dir` (its install directory) and checked against the hash its
/// integrity record has for that file. Anything else gives no icon.
pub fn icon(dir: &Path, record: &PluginRecord) -> Option<String> {
    let active = record.active()?;
    let relative = &active.manifest.icons.size128;
    let expected = active.integrity.files.get(relative)?;
    let path = dir.join(relative);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > ipc::MAX_ICON_BYTES as u64 {
        return None;
    }
    icon_data(&std::fs::read(&path).ok()?, expected)
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
/// (profile, variables, open URL, background, device) are
/// not passed; their capabilities do not exist in ABI 1 or are refused by
/// the executor.
pub fn approved(effective: &[(GrantKey, GrantScope)]) -> (ApprovedPermissions, Vec<String>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for (key, scope) in effective {
        match (key.kind, scope) {
            (PermissionKind::Network, GrantScope::Host { methods }) => match origin_of(&key.key) {
                Some(origin) => out.push(ApprovedPermission::Network {
                    origin,
                    methods: methods.iter().copied().map(http_method).collect(),
                }),
                None => skipped.push(key.to_string()),
            },
            (PermissionKind::Notifications, _) => out.push(ApprovedPermission::Notifications {}),
            // The larger quota; without it the default applies.
            (PermissionKind::Storage, GrantScope::Storage { quota_kb }) => {
                out.push(ApprovedPermission::Storage {
                    quota_kb: *quota_kb,
                });
            }
            (PermissionKind::Credentials, GrantScope::Credential { hosts, schemes }) => {
                let origins: Option<BTreeSet<String>> =
                    hosts.iter().map(|h| origin_of(h)).collect();
                match origins {
                    Some(origins) if !origins.is_empty() && !schemes.is_empty() => {
                        out.push(ApprovedPermission::Credential {
                            key: key.key.clone(),
                            origins,
                            schemes: schemes.iter().copied().map(scheme).collect(),
                        });
                    }
                    _ => skipped.push(key.to_string()),
                }
            }
            _ => skipped.push(key.to_string()),
        }
    }
    (ApprovedPermissions(out), skipped)
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
        ]);
        assert_eq!(
            approved.0,
            [
                ApprovedPermission::Network {
                    origin: "https://api.example.com".into(),
                    methods: [HttpMethod::Get].into(),
                },
                ApprovedPermission::Notifications {},
            ]
        );
        assert_eq!(
            skipped,
            ["network:*.example.com", "network:private-network"]
        );
    }
}
