//! User-entered origins (G2, `docs/marketplace/module-migration.md` §7) in
//! the agent: what an origin setting holds, the consent the user gives to
//! exactly that origin, and which consents reach the broker.
//!
//! A consent reaches the broker only while all of this holds, checked each
//! time the grants are derived:
//!
//! - the active manifest still declares the origin slot, and its grant
//!   (`network:user-origin:<setting>`) is granted;
//! - the setting (saved, or its default) still holds exactly the consented
//!   origin and path prefix: a changed setting revokes the consent
//!   (`Service`), and a stale one never matches;
//! - methods, the private network and the credential slots are each the
//!   intersection of what the user consented to and what the slot (and,
//!   for a slot, its own grant) allows now.
//!
//! The broker parses the origin strictly again ([`UserOrigin::parse`]) and
//! refuses a private host without the private-network consent.

use std::collections::{BTreeMap, BTreeSet};

use deckpilot_ipc::plugins as ipc;
use deckpilot_plugin_broker::{ApprovedPermission, OriginError, UserOrigin};
use deckpilot_plugin_manifest::{HttpMethod, Manifest, Permission, PermissionKind, UserOriginSlot};
use deckpilot_plugin_store::{
    GrantKey, GrantScope, GrantStatus, StoredGrant, StoredUserOrigin, UserOriginConsent,
    bound_credentials, declared_slot, user_origin_key,
};
use serde_json::Value;

/// What an origin setting holds now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Current {
    Unset,
    Invalid(String),
    Origin(UserOrigin),
}

impl Current {
    fn shown(&self) -> Option<String> {
        match self {
            Self::Origin(origin) => Some(origin.to_string()),
            Self::Unset | Self::Invalid(_) => None,
        }
    }
}

/// The origin slots of `manifest`, with the methods each allows.
pub fn slots(manifest: &Manifest) -> Vec<(&UserOriginSlot, Vec<HttpMethod>)> {
    manifest
        .permissions
        .iter()
        .flat_map(Permission::user_origins)
        .filter_map(|slot| declared_slot(manifest, &slot.setting))
        .collect()
}

fn problem(error: OriginError) -> String {
    match error {
        OriginError::SchemeNotAllowed => "must start with https://".into(),
        OriginError::UserinfoNotAllowed => "must not contain a user name or password".into(),
        OriginError::NotAnOrigin => "must not have a query or fragment".into(),
        OriginError::IpLiteralNotAllowed => {
            "is an address on this computer or a link-local, reserved or special network, which \
             plugins never reach"
                .into()
        }
        OriginError::InvalidHost => "does not name a server plugins can reach".into(),
        OriginError::InvalidPath => {
            "has a path with `.` or `..`, empty segments or encoded slashes".into()
        }
        OriginError::TooLong => "is too long".into(),
        OriginError::IllegalCharacters => "contains spaces, control characters or `\\`".into(),
        OriginError::Unparsable => "is not a URL".into(),
    }
}

/// Parses what the user entered into the setting of `slot`: `None` when
/// empty; an error for anything that is not an origin this slot takes.
pub fn parse(slot: &UserOriginSlot, text: &str) -> Result<Option<UserOrigin>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let origin = UserOrigin::parse(text).map_err(problem)?;
    if origin.path_prefix().is_some() && !slot.path_prefix() {
        return Err("must be an origin without a path (https://server[:port])".into());
    }
    if origin.private_host() && !slot.private_network() {
        return Err("is on a private network, which this setting does not allow".into());
    }
    Ok(Some(origin))
}

/// What each origin setting holds now: the saved value, or the default.
pub fn current(manifest: &Manifest, saved: &BTreeMap<String, Value>) -> BTreeMap<String, Current> {
    let (effective, _) = super::data::effective_settings(manifest, saved);
    slots(manifest)
        .into_iter()
        .map(|(slot, _)| {
            let value = effective
                .get(&slot.setting)
                .and_then(Value::as_str)
                .unwrap_or_default();
            let current = match parse(slot, value) {
                Ok(None) => Current::Unset,
                Ok(Some(origin)) => Current::Origin(origin),
                Err(problem) => Current::Invalid(problem),
            };
            (slot.setting.clone(), current)
        })
        .collect()
}

/// Whether `origin` is exactly a host the network permission declares
/// (granted at install): it then needs no consent of its own.
fn declared(manifest: &Manifest, origin: &UserOrigin) -> bool {
    if origin.private_host() {
        return false;
    }
    let host = match origin.origin().port() {
        443 => origin.origin().host().to_owned(),
        port => format!("{}:{port}", origin.origin().host()),
    };
    manifest.permissions.iter().any(|p| match p {
        Permission::Network { hosts, .. } => hosts.contains(&host),
        _ => false,
    })
}

/// Normalizes the origin settings among `values` (what the user saves):
/// each becomes the origin as the broker sees it. Anything that is not an
/// origin its slot takes is refused.
pub fn normalize(manifest: &Manifest, values: &mut BTreeMap<String, Value>) -> Result<(), String> {
    for (slot, _) in slots(manifest) {
        let Some(Value::String(text)) = values.get(&slot.setting) else {
            continue;
        };
        let label = manifest
            .contributes
            .settings()
            .iter()
            .find(|s| s.key == slot.setting)
            .map_or(slot.setting.as_str(), |s| s.label.as_str());
        match parse(slot, text) {
            Ok(Some(origin)) => {
                values.insert(slot.setting.clone(), Value::String(origin.to_string()));
            }
            Ok(None) => {}
            Err(problem) => return Err(format!("{label}: `{text}` {problem}")),
        }
    }
    Ok(())
}

/// The origin settings whose origin is not what it was.
pub fn changed(
    before: &BTreeMap<String, Current>,
    after: &BTreeMap<String, Current>,
) -> Vec<String> {
    before
        .iter()
        .filter(|(setting, was)| after.get(*setting).and_then(Current::shown) != was.shown())
        .map(|(setting, _)| setting.clone())
        .collect()
}

fn abi_method(method: HttpMethod) -> deckpilot_plugin_abi::HttpMethod {
    use deckpilot_plugin_abi::HttpMethod as A;
    match method {
        HttpMethod::Get => A::Get,
        HttpMethod::Head => A::Head,
        HttpMethod::Post => A::Post,
        HttpMethod::Put => A::Put,
        HttpMethod::Patch => A::Patch,
        HttpMethod::Delete => A::Delete,
    }
}

/// What the broker receives of the consents in `rows`, given the plugin's
/// effective grants (granted ∩ declared) and settings. A consent that no
/// longer matches its setting, or whose slot is gone or not granted, gives
/// nothing.
pub fn approved(
    manifest: &Manifest,
    saved: &BTreeMap<String, Value>,
    effective: &[(GrantKey, GrantScope)],
    rows: &[StoredUserOrigin],
) -> Vec<ApprovedPermission> {
    let current = current(manifest, saved);
    let granted_credentials: BTreeSet<&str> = effective
        .iter()
        .filter(|(key, _)| key.kind == PermissionKind::Credentials)
        .map(|(key, _)| key.key.as_str())
        .collect();
    rows.iter()
        .filter_map(|row| {
            let consent = &row.consent;
            let key = user_origin_key(&consent.setting);
            let Some((
                _,
                GrantScope::UserOriginSlot {
                    methods: allowed,
                    private_network: slot_private,
                    ..
                },
            )) = effective.iter().find(|(k, _)| *k == key)
            else {
                return None;
            };
            let Some(Current::Origin(origin)) = current.get(&consent.setting) else {
                return None;
            };
            if origin.to_string() != consent.display() {
                return None;
            }
            let private_network = consent.private_network && *slot_private;
            if origin.private_host() && !private_network {
                return None;
            }
            let methods: BTreeSet<_> = consent
                .methods
                .iter()
                .filter(|m| allowed.contains(m))
                .copied()
                .map(abi_method)
                .collect();
            if methods.is_empty() {
                return None;
            }
            let bound = bound_credentials(manifest, &consent.setting);
            let credentials = consent
                .credentials
                .iter()
                .filter(|k| bound.contains(k) && granted_credentials.contains(k.as_str()))
                .cloned()
                .collect();
            Some(ApprovedPermission::UserOrigin {
                origin: origin.origin().to_string(),
                path_prefix: origin.path_prefix().map(str::to_owned),
                methods,
                private_network,
                credentials,
            })
        })
        .collect()
}

/// The consent the user gives to `origin` in `setting`: exactly what the
/// setting holds now, all the slot's methods and its bound credential
/// slots, and a private network only when asked for (required for a
/// private host).
pub fn consent(
    manifest: &Manifest,
    saved: &BTreeMap<String, Value>,
    setting: &str,
    origin: &str,
    private_network: bool,
) -> Result<UserOriginConsent, String> {
    let Some((slot, methods)) = declared_slot(manifest, setting) else {
        return Err(format!(
            "`{setting}` is not an origin setting of {}",
            manifest.id
        ));
    };
    let current = match current(manifest, saved).remove(setting) {
        Some(Current::Origin(current)) if current.to_string() == origin => current,
        Some(Current::Invalid(problem)) => {
            return Err(format!("the setting `{setting}` {problem}"));
        }
        _ => {
            return Err(format!(
                "the setting `{setting}` does not hold `{origin}`; save the setting first"
            ));
        }
    };
    if declared(manifest, &current) {
        return Err(format!(
            "`{origin}` is a host the plugin declares; it needs no consent of its own"
        ));
    }
    if private_network && !slot.private_network() {
        return Err(format!(
            "the setting `{setting}` does not allow a server on a private network"
        ));
    }
    if current.private_host() && !private_network {
        return Err(format!(
            "`{origin}` is on a private network: allowing that needs its own consent"
        ));
    }
    Ok(UserOriginConsent {
        setting: setting.to_owned(),
        origin: current.origin().to_string(),
        path_prefix: current.path_prefix().map(str::to_owned),
        methods,
        private_network,
        credentials: bound_credentials(manifest, setting),
    })
}

fn method_name(method: HttpMethod) -> String {
    serde_json::to_value(method)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The origin settings of `manifest` for the app: what each holds and
/// where its consent stands.
pub fn infos(
    manifest: &Manifest,
    saved: &BTreeMap<String, Value>,
    grants: &[StoredGrant],
    rows: &[StoredUserOrigin],
) -> Vec<ipc::PluginUserOrigin> {
    let declared_grants = deckpilot_plugin_store::GrantSet::from_manifest(manifest);
    let mut current = current(manifest, saved);
    slots(manifest)
        .into_iter()
        .map(|(slot, methods)| {
            let key = user_origin_key(&slot.setting);
            let slot_granted = declared_grants.get(&key).is_some_and(|scope| {
                grants.iter().any(|g| {
                    g.key == key && g.status == GrantStatus::Granted && g.scope.covers(scope)
                })
            });
            let row = rows.iter().find(|r| r.consent.setting == slot.setting);
            let mut info = ipc::PluginUserOrigin {
                setting: slot.setting.clone(),
                label: manifest
                    .contributes
                    .settings()
                    .iter()
                    .find(|s| s.key == slot.setting)
                    .map_or_else(|| slot.setting.clone(), |s| s.label.clone()),
                permission: key.to_string(),
                status: ipc::UserOriginStatus::Unset,
                origin: None,
                problem: None,
                private_host: false,
                private_network_allowed: slot.private_network(),
                private_network_granted: false,
                path_prefix_allowed: slot.path_prefix(),
                methods: methods.into_iter().map(method_name).collect(),
                credentials: bound_credentials(manifest, &slot.setting),
                granted_at: None,
            };
            match current.remove(&slot.setting).unwrap_or(Current::Unset) {
                Current::Unset => {}
                Current::Invalid(problem) => {
                    info.status = ipc::UserOriginStatus::Invalid;
                    info.problem = Some(problem);
                }
                Current::Origin(origin) => {
                    info.private_host = origin.private_host();
                    info.origin = Some(origin.to_string());
                    let consented = row.filter(|r| {
                        r.consent.display() == origin.to_string()
                            && (r.consent.private_network || !origin.private_host())
                    });
                    info.status = if declared(manifest, &origin) {
                        ipc::UserOriginStatus::Declared
                    } else if !slot_granted {
                        ipc::UserOriginStatus::SlotNotGranted
                    } else if let Some(row) = consented {
                        info.private_network_granted =
                            row.consent.private_network && slot.private_network();
                        info.granted_at = Some(row.decided_at);
                        ipc::UserOriginStatus::Granted
                    } else {
                        ipc::UserOriginStatus::NeedsConsent
                    };
                }
            }
            info
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the tests build the slot inline"
    )]
    fn manifest(slot: Value) -> Manifest {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/examples/hello-weather/manifest.json"
        ))
        .unwrap();
        let mut m: Value = serde_json::from_str(&text).unwrap();
        m["permissions"] = serde_json::json!([
            {"kind": "network", "hosts": ["api.github.com"], "methods": ["GET", "POST"],
             "userOrigins": [slot], "justification": "Talks to GitHub or your server."},
            {"kind": "credentials", "refs": ["token"],
             "justification": "Signs in to GitHub or your server."}
        ]);
        m["contributes"]["settings"] = serde_json::json!({"fields": [
            {"key": "apiUrl", "label": "API URL", "required": false,
             "kind": {"type": "text", "maxLength": 2048},
             "default": {"type": "text", "value": "https://api.github.com"}}
        ]});
        m["contributes"]["credentials"] = serde_json::json!([
            {"key": "token", "label": "Token", "hosts": ["api.github.com"],
             "userOrigin": "apiUrl", "schemes": ["bearer"]}
        ]);
        let manifest: Manifest = serde_json::from_value(m).unwrap();
        manifest.validate().unwrap();
        manifest
    }

    fn ghe() -> Manifest {
        manifest(serde_json::json!({"setting": "apiUrl", "pathPrefix": true,
            "privateNetwork": true}))
    }

    fn saved(value: &str) -> BTreeMap<String, Value> {
        BTreeMap::from([("apiUrl".to_owned(), Value::from(value))])
    }

    fn effective(m: &Manifest) -> Vec<(GrantKey, GrantScope)> {
        deckpilot_plugin_store::GrantSet::from_manifest(m)
            .iter()
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect()
    }

    fn row(consent: UserOriginConsent) -> StoredUserOrigin {
        StoredUserOrigin {
            consent,
            decided_by: "user".into(),
            decided_at: 7,
            granted_for_version: "1.0.0".into(),
        }
    }

    #[test]
    fn values_are_normalized_and_checked_against_the_slot() {
        let m = ghe();
        let mut values = saved("https://GHE.example.com:443/api/v3/");
        normalize(&m, &mut values).unwrap();
        assert_eq!(values["apiUrl"], "https://ghe.example.com/api/v3");
        for bad in [
            "http://ghe.example.com",
            "https://u:p@ghe.example.com",
            "https://ghe.example.com/?x",
            "https://*.example.com",
            "https://localhost",
            "https://127.0.0.1",
            "ghe.example.com",
        ] {
            assert!(normalize(&m, &mut saved(bad)).is_err(), "{bad}");
        }
        // A slot without a path or a private network.
        let plain = manifest(serde_json::json!({"setting": "apiUrl"}));
        for bad in [
            "https://ghe.example.com/api/v3",
            "https://10.0.0.5",
            "https://ghe.internal",
        ] {
            let error = normalize(&plain, &mut saved(bad)).unwrap_err();
            assert!(error.starts_with("API URL: "), "{error}");
        }
        normalize(&plain, &mut saved("https://ghe.example.com")).unwrap();
    }

    #[test]
    fn a_consent_is_for_exactly_what_the_setting_holds() {
        let m = ghe();
        let values = saved("https://ghe.example.com/api/v3");
        let c = consent(
            &m,
            &values,
            "apiUrl",
            "https://ghe.example.com/api/v3",
            false,
        )
        .unwrap();
        assert_eq!(c.origin, "https://ghe.example.com");
        assert_eq!(c.path_prefix.as_deref(), Some("/api/v3"));
        assert_eq!(c.methods, [HttpMethod::Get, HttpMethod::Post]);
        assert_eq!(c.credentials, ["token"]);
        assert!(!c.private_network);
        // Another origin than the setting holds.
        assert!(consent(&m, &values, "apiUrl", "https://evil.example.com", false).is_err());
        assert!(consent(&m, &values, "apiUrl", "https://ghe.example.com", false).is_err());
        // The default is a declared host: nothing to consent to.
        assert!(
            consent(
                &m,
                &BTreeMap::new(),
                "apiUrl",
                "https://api.github.com",
                false
            )
            .is_err()
        );
        // A private host needs the private-network consent.
        let lan = saved("https://10.0.0.5");
        assert!(consent(&m, &lan, "apiUrl", "https://10.0.0.5", false).is_err());
        assert!(
            consent(&m, &lan, "apiUrl", "https://10.0.0.5", true)
                .unwrap()
                .private_network
        );
        // And a slot that does not allow it never gets one.
        let plain = manifest(serde_json::json!({"setting": "apiUrl"}));
        let values = saved("https://ghe.example.com");
        assert!(consent(&plain, &values, "apiUrl", "https://ghe.example.com", true).is_err());
    }

    #[test]
    fn only_a_consent_matching_the_current_setting_reaches_the_broker() {
        let m = ghe();
        let values = saved("https://ghe.example.com/api/v3");
        let c = consent(
            &m,
            &values,
            "apiUrl",
            "https://ghe.example.com/api/v3",
            false,
        )
        .unwrap();
        let rows = [row(c)];
        let approved_now = approved(&m, &values, &effective(&m), &rows);
        assert_eq!(
            approved_now,
            [ApprovedPermission::UserOrigin {
                origin: "https://ghe.example.com".into(),
                path_prefix: Some("/api/v3".into()),
                methods: [
                    deckpilot_plugin_abi::HttpMethod::Get,
                    deckpilot_plugin_abi::HttpMethod::Post
                ]
                .into(),
                private_network: false,
                credentials: ["token".to_owned()].into(),
            }]
        );
        // The setting changed: nothing.
        let other = saved("https://ghe2.example.com/api/v3");
        assert_eq!(approved(&m, &other, &effective(&m), &rows).len(), 0);
        // The slot grant is not effective: nothing.
        let without_slot: Vec<_> = effective(&m)
            .into_iter()
            .filter(|(k, _)| *k != user_origin_key("apiUrl"))
            .collect();
        assert_eq!(approved(&m, &values, &without_slot, &rows).len(), 0);
        // The credential grant is not effective: no secret goes there.
        let without_token: Vec<_> = effective(&m)
            .into_iter()
            .filter(|(k, _)| k.kind != PermissionKind::Credentials)
            .collect();
        assert!(matches!(
            &approved(&m, &values, &without_token, &rows)[0],
            ApprovedPermission::UserOrigin { credentials, .. } if credentials.is_empty()
        ));
        // A later version without a private network drops it, and a
        // private host with it.
        let lan = saved("https://10.0.0.5");
        let c = consent(&m, &lan, "apiUrl", "https://10.0.0.5", true).unwrap();
        let rows = [row(c)];
        assert_eq!(approved(&m, &lan, &effective(&m), &rows).len(), 1);
        let narrower = manifest(serde_json::json!({"setting": "apiUrl", "pathPrefix": true}));
        let mut effective_narrow = effective(&narrower);
        effective_narrow.retain(|(k, _)| k.kind != PermissionKind::Credentials);
        assert_eq!(approved(&narrower, &lan, &effective_narrow, &rows).len(), 0);
    }

    #[test]
    fn infos_tell_where_each_consent_stands() {
        let m = ghe();
        let grants: Vec<StoredGrant> = effective(&m)
            .into_iter()
            .map(|(key, scope)| StoredGrant {
                key,
                scope,
                status: GrantStatus::Granted,
                source: deckpilot_plugin_store::GrantSource::Install,
                decided_by: "user".into(),
                decided_at: 1,
                granted_for_version: "1.0.0".into(),
            })
            .collect();
        let status = |values: &BTreeMap<String, Value>, rows: &[StoredUserOrigin]| {
            infos(&m, values, &grants, rows)[0].clone()
        };
        let default = status(&BTreeMap::new(), &[]);
        assert_eq!(default.status, ipc::UserOriginStatus::Declared);
        assert_eq!(default.origin.as_deref(), Some("https://api.github.com"));
        assert_eq!(status(&saved(""), &[]).status, ipc::UserOriginStatus::Unset);
        let invalid = status(&saved("http://x.example.com"), &[]);
        assert_eq!(invalid.status, ipc::UserOriginStatus::Invalid);
        assert!(invalid.problem.is_some());
        let values = saved("https://ghe.example.com/api/v3");
        let pending = status(&values, &[]);
        assert_eq!(pending.status, ipc::UserOriginStatus::NeedsConsent);
        assert_eq!(pending.methods, ["GET", "POST"]);
        assert_eq!(pending.credentials, ["token"]);
        assert!(pending.path_prefix_allowed && pending.private_network_allowed);
        let c = consent(
            &m,
            &values,
            "apiUrl",
            "https://ghe.example.com/api/v3",
            false,
        )
        .unwrap();
        let granted = status(&values, &[row(c.clone())]);
        assert_eq!(granted.status, ipc::UserOriginStatus::Granted);
        assert_eq!(granted.granted_at, Some(7));
        // Another origin in the setting than the one consented to.
        assert_eq!(
            status(&saved("https://ghe.example.com/api/v4"), &[row(c)]).status,
            ipc::UserOriginStatus::NeedsConsent
        );
        let lan = status(&saved("https://10.0.0.5"), &[]);
        assert!(lan.private_host);
        // Without the slot grant.
        let none = infos(&m, &values, &[], &[])[0].clone();
        assert_eq!(none.status, ipc::UserOriginStatus::SlotNotGranted);
    }

    /// The origin settings in a `pluginSettings` answer are bounded by the
    /// manifest (at most four slots, each with a URL of bounded length, a
    /// short label, six methods and eight credential keys) and never carry
    /// what the user typed when it is not an origin, so they need no paging
    /// (`deckpilot_ipc::plugin_answers`): the largest is a small part of
    /// one message.
    #[test]
    fn the_origin_settings_of_an_answer_are_small() {
        let m = ghe();
        let longest = format!("https://ghe.example.com/{}", "a".repeat(480));
        let one = serde_json::to_vec(&infos(&m, &saved(&longest), &[], &[])).unwrap();
        assert!(one.len() < 2 * 1024, "{}", one.len());
        // Text that is not an origin is described, never echoed.
        let junk = "x".repeat(2_000);
        let invalid = infos(&m, &saved(&junk), &[], &[]);
        assert_eq!(invalid[0].status, ipc::UserOriginStatus::Invalid);
        let invalid = serde_json::to_vec(&invalid).unwrap();
        assert!(invalid.len() < 1024, "{}", invalid.len());
        // A path prefix beyond the broker's bound is not an origin.
        let long = format!("https://ghe.example.com/{}", "a".repeat(600));
        assert!(normalize(&m, &mut saved(&long)).is_err());
    }

    #[test]
    fn changed_settings_are_found() {
        let m = ghe();
        let before = current(&m, &saved("https://ghe.example.com"));
        assert_eq!(
            changed(&before, &current(&m, &saved("https://GHE.example.com/"))).len(),
            0
        );
        assert_eq!(
            changed(&before, &current(&m, &saved("https://ghe.example.com/api"))),
            ["apiUrl"]
        );
        assert_eq!(changed(&before, &current(&m, &saved(""))), ["apiUrl"]);
    }
}
