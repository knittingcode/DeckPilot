//! Conversions between the plugin crates' types and the IPC shapes of
//! protocol 10, and from stored consent to broker grants.

use std::collections::{BTreeMap, BTreeSet};

use deckpilot_ipc::plugins as ipc;
use deckpilot_plugin_abi::capability::AuthScheme;
use deckpilot_plugin_abi::{ActionOutcome, HttpMethod, ParamValue, Params};
use deckpilot_plugin_broker::{ApprovedPermission, ApprovedPermissions};
use deckpilot_plugin_manifest::{Manifest, PermissionKind};
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

pub fn summary(record: &PluginRecord) -> ipc::PluginSummary {
    let active = record.active();
    let manifest = active.map(|v| &v.manifest);
    let reason = unavailable(record);
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
        summary: summary(record),
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
    }
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
/// (profile, variables, storage quota, open URL, background, device) are
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
