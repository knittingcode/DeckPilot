//! User-entered origins (G2, `docs/marketplace/module-migration.md` §7):
//! exactly the consented scheme, host, port and path prefix, a bound
//! credential only there, a private network only with its own consent,
//! and redirects re-checked like every other request.

use std::sync::Arc;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::HttpMethod;
use deckpilot_plugin_abi::capability::{AuthScheme, HttpAuth, HttpFetch};
use deckpilot_plugin_broker::*;

fn broker() -> (Broker, PluginId) {
    let broker = Broker::new(
        Arc::new(MemoryAudit::new(1_000)),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    (broker, PluginId::new("acme.forge").unwrap())
}

fn granted(perms: &str) -> (Broker, PluginId) {
    let (broker, plugin) = broker();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    (broker, plugin)
}

fn derive(perms: &str) -> Result<Grants, GrantError> {
    Grants::derive(&ApprovedPermissions::from_json(perms).unwrap())
}

fn fetch(method: HttpMethod, url: &str, credential: Option<&str>) -> CapabilityRequest {
    CapabilityRequest::HttpFetch(HttpFetch {
        method,
        url: url.into(),
        headers: vec![],
        body: None,
        auth: credential.map(|c| HttpAuth {
            credential: c.into(),
            scheme: AuthScheme::Bearer,
        }),
    })
}

fn get(url: &str) -> CapabilityRequest {
    fetch(HttpMethod::Get, url, None)
}

fn get_with_token(url: &str) -> CapabilityRequest {
    fetch(HttpMethod::Get, url, Some("token"))
}

fn allowed(d: &Decision) -> &AllowedFetch {
    match d {
        Decision::Allow(Allowed::Fetch(a)) => a,
        other => panic!("{other:?}"),
    }
}

/// GHE: api.github.com fixed, the user's server with a path prefix and the
/// token bound to it.
const GHE: &str = r#"[
    {"kind":"network","origin":"https://api.github.com","methods":["GET"]},
    {"kind":"userOrigin","origin":"https://ghe.example.com","pathPrefix":"/api/v3",
     "methods":["GET","POST"],"credentials":["token"]},
    {"kind":"credential","key":"token","origins":["https://api.github.com"],"schemes":["bearer"]}
]"#;

#[test]
fn requests_reach_exactly_the_user_origin_and_its_path_prefix() {
    let (broker, plugin) = granted(GHE);
    let s = broker.session(plugin);
    for url in [
        "https://ghe.example.com/api/v3",
        "https://GHE.example.com:443/api/v3/repos/a/b/pulls?state=open",
    ] {
        let d = s.authorize(&get(url));
        let a = allowed(&d);
        assert_eq!(a.origin.to_string(), "https://ghe.example.com");
        assert!(!a.private_network, "{url}");
    }
    // The slot's methods, not more.
    assert!(
        s.authorize(&fetch(
            HttpMethod::Post,
            "https://ghe.example.com/api/v3/x",
            None
        ))
        .is_allowed()
    );
    assert_eq!(
        s.authorize(&fetch(
            HttpMethod::Delete,
            "https://ghe.example.com/api/v3/x",
            None
        ))
        .denial(),
        Some(DenyReason::MethodNotGranted)
    );
    // The fixed grant keeps its own methods.
    assert_eq!(
        s.authorize(&fetch(HttpMethod::Post, "https://api.github.com/x", None))
            .denial(),
        Some(DenyReason::MethodNotGranted)
    );
    for (url, why) in [
        ("https://ghe.example.com/", DenyReason::PathNotGranted),
        (
            "https://ghe.example.com/api/v30",
            DenyReason::PathNotGranted,
        ),
        ("https://ghe.example.com/admin", DenyReason::PathNotGranted),
        (
            "https://ghe.example.com/api/v3/../../admin",
            DenyReason::PathNotGranted,
        ),
        (
            "https://ghe.example.com:8443/api/v3",
            DenyReason::OriginNotGranted,
        ),
        (
            "https://other.example.com/api/v3",
            DenyReason::OriginNotGranted,
        ),
        (
            "https://x.ghe.example.com/api/v3",
            DenyReason::OriginNotGranted,
        ),
        (
            "https://ghe.example.com.evil.net/api/v3",
            DenyReason::OriginNotGranted,
        ),
    ] {
        assert_eq!(s.authorize(&get(url)).denial(), Some(why), "{url}");
    }
    for url in [
        "http://ghe.example.com/api/v3",
        "https://user@ghe.example.com/api/v3",
    ] {
        assert!(
            matches!(s.authorize(&get(url)).denial(), Some(DenyReason::Url(_))),
            "{url}"
        );
    }
}

#[test]
fn a_bound_credential_goes_only_to_the_granted_user_origin() {
    let (broker, plugin) = granted(GHE);
    let s = broker.session(plugin);
    let d = s.authorize(&get_with_token("https://ghe.example.com/api/v3/user"));
    assert_eq!(allowed(&d).auth.as_ref().unwrap().credential, "token");
    assert!(
        s.authorize(&get_with_token("https://api.github.com/user"))
            .is_allowed()
    );
    for url in [
        "https://ghe.example.com/login",
        "https://ghe.example.com:8443/api/v3/user",
        "https://evil.example.com/api/v3/user",
    ] {
        assert!(!s.authorize(&get_with_token(url)).is_allowed(), "{url}");
    }
    // Without the binding, the user origin gets no secret.
    let unbound = GHE.replace(r#","credentials":["token"]"#, "");
    let (broker, plugin) = granted(&unbound);
    let s = broker.session(plugin);
    assert_eq!(
        s.authorize(&get_with_token("https://ghe.example.com/api/v3/user"))
            .denial(),
        Some(DenyReason::CredentialOriginNotAllowed)
    );
    assert!(
        s.authorize(&get("https://ghe.example.com/api/v3/user"))
            .is_allowed()
    );
}

#[test]
fn a_slot_only_for_the_user_origin_needs_the_binding() {
    // Jira: no fixed host at all.
    let jira = r#"[
        {"kind":"userOrigin","origin":"https://acme.atlassian.net","methods":["GET"],
         "credentials":["token"]},
        {"kind":"credential","key":"token","origins":[],"schemes":["bearer"]}
    ]"#;
    let (broker, plugin) = granted(jira);
    let s = broker.session(plugin);
    assert!(
        s.authorize(&get_with_token(
            "https://acme.atlassian.net/rest/api/3/myself"
        ))
        .is_allowed()
    );
    // Without the user origin, the slot has nowhere to go.
    let lonely = r#"[{"kind":"credential","key":"token","origins":[],"schemes":["bearer"]}]"#;
    assert_eq!(derive(lonely), Err(GrantError::EmptyCredential));
    // A user origin naming a slot that is not granted injects nothing.
    let g = derive(
        r#"[{"kind":"userOrigin","origin":"https://acme.atlassian.net","methods":["GET"],
             "credentials":["other"]}]"#,
    )
    .unwrap();
    assert_eq!(g.user_origins().next().unwrap().credentials().count(), 0);
}

#[test]
fn a_private_network_needs_its_own_consent() {
    let private = |host: &str, consent: bool| {
        format!(
            r#"[{{"kind":"userOrigin","origin":"{host}","methods":["GET"],
                 "privateNetwork":{consent}}}]"#
        )
    };
    // A private host is never granted without the consent.
    for host in [
        "https://10.0.0.5",
        "https://[fd00::5]",
        "https://ghe.internal",
        "https://gitlab",
    ] {
        assert_eq!(
            derive(&private(host, false)),
            Err(GrantError::PrivateNetworkNotGranted),
            "{host}"
        );
        let (broker, plugin) = granted(&private(host, true));
        let d = broker
            .session(plugin)
            .authorize(&get(&format!("{host}/api/v4/projects")));
        assert!(allowed(&d).private_network, "{host}");
    }
    // Never this computer or link-local, whatever the consent.
    for host in [
        "https://localhost",
        "https://127.0.0.1",
        "https://[::1]",
        "https://169.254.169.254",
    ] {
        assert!(
            matches!(derive(&private(host, true)), Err(GrantError::Origin(_))),
            "{host}"
        );
    }
    // A public name: the consent decides whether its addresses may be
    // private (enforced when connecting).
    let (broker, plugin) = granted(&private("https://ghe.example.com", false));
    let d = broker
        .session(plugin)
        .authorize(&get("https://ghe.example.com/x"));
    assert!(!allowed(&d).private_network);
    // Another private host than the granted one stays refused, as for a
    // plugin without user origins.
    let (broker, plugin) = granted(&private("https://10.0.0.5", true));
    let s = broker.session(plugin);
    for url in [
        "https://10.0.0.6/",
        "https://10.0.0.5:8443/",
        "https://other.internal/",
    ] {
        assert!(
            matches!(s.authorize(&get(url)).denial(), Some(DenyReason::Url(_))),
            "{url}"
        );
    }
}

#[test]
fn redirects_from_a_user_origin_are_rechecked() {
    let (broker, plugin) = granted(GHE);
    let s = broker.session(plugin);
    let first = s.authorize(&get_with_token("https://ghe.example.com/api/v3/repos"));
    let first = allowed(&first).clone();
    // Within the prefix: followed, with the credential.
    let next = s.authorize_redirect(&first, "/api/v3/repositories/1", 1);
    assert_eq!(allowed(&next).auth.as_ref().unwrap().credential, "token");
    // Outside the prefix, on the same origin.
    assert_eq!(
        s.authorize_redirect(&first, "/login?return=/api/v3", 1)
            .denial(),
        Some(DenyReason::PathNotGranted)
    );
    // Another origin, even a granted one: never with the credential.
    assert_eq!(
        s.authorize_redirect(&first, "https://api.github.com/repos", 1)
            .denial(),
        Some(DenyReason::CredentialOriginNotAllowed)
    );
    for location in [
        "https://evil.example.com/api/v3/repos",
        "https://10.0.0.5/api/v3/repos",
        "https://localhost/api/v3",
    ] {
        assert!(
            !s.authorize_redirect(&first, location, 1).is_allowed(),
            "{location}"
        );
    }
    assert_eq!(
        s.authorize_redirect(&first, "http://ghe.example.com/api/v3/x", 1)
            .denial(),
        Some(DenyReason::InsecureRedirect)
    );
    // A redirect from a private-network origin to a public one loses the
    // private network.
    let both = r#"[
        {"kind":"userOrigin","origin":"https://10.0.0.5","methods":["GET"],"privateNetwork":true},
        {"kind":"userOrigin","origin":"https://ghe.example.com","methods":["GET"]}
    ]"#;
    let (broker, plugin) = granted(both);
    let s = broker.session(plugin);
    let first = s.authorize(&get("https://10.0.0.5/x"));
    assert!(allowed(&first).private_network);
    let next = s.authorize_redirect(allowed(&first), "https://ghe.example.com/y", 1);
    assert!(!allowed(&next).private_network);
}

#[test]
fn user_origin_records_are_strict() {
    for (record, why) in [
        (
            r#"{"kind":"userOrigin","origin":"https://ghe.example.com","methods":[]}"#,
            GrantError::NoMethods,
        ),
        (
            r#"{"kind":"userOrigin","origin":"https://*.example.com","methods":["GET"]}"#,
            GrantError::Origin(OriginError::InvalidHost),
        ),
        (
            r#"{"kind":"userOrigin","origin":"http://ghe.example.com","methods":["GET"]}"#,
            GrantError::Origin(OriginError::SchemeNotAllowed),
        ),
        // Not normalized: not what the user consented to.
        (
            r#"{"kind":"userOrigin","origin":"https://GHE.example.com","methods":["GET"]}"#,
            GrantError::Origin(OriginError::NotAnOrigin),
        ),
        (
            r#"{"kind":"userOrigin","origin":"https://ghe.example.com","pathPrefix":"/a/",
                "methods":["GET"]}"#,
            GrantError::Origin(OriginError::NotAnOrigin),
        ),
        (
            r#"{"kind":"userOrigin","origin":"https://ghe.example.com","pathPrefix":"/a/../b",
                "methods":["GET"]}"#,
            GrantError::Origin(OriginError::InvalidPath),
        ),
    ] {
        assert_eq!(derive(&format!("[{record}]")), Err(why), "{record}");
    }
    assert_eq!(
        ApprovedPermissions::from_json(
            r#"[{"kind":"userOrigin","origin":"https://ghe.example.com","methods":["GET"],
                 "wildcard":true}]"#
        ),
        Err(GrantError::Malformed)
    );
    let many: Vec<String> = (0..=8)
        .map(|i| {
            format!(
                r#"{{"kind":"userOrigin","origin":"https://h{i}.example.com","methods":["GET"]}}"#
            )
        })
        .collect();
    assert_eq!(
        derive(&format!("[{}]", many.join(","))),
        Err(GrantError::TooManyOrigins)
    );
}

#[test]
fn new_grants_replace_a_changed_user_origin_at_once() {
    let (broker, plugin) = granted(GHE);
    let s = broker.session(plugin.clone());
    assert!(
        s.authorize(&get("https://ghe.example.com/api/v3"))
            .is_allowed()
    );
    // The user changed the setting: the agent derives again without it.
    broker
        .set_grants(
            &plugin,
            &ApprovedPermissions::from_json(
                r#"[{"kind":"network","origin":"https://api.github.com","methods":["GET"]}]"#,
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        s.authorize(&get("https://ghe.example.com/api/v3")).denial(),
        Some(DenyReason::OriginNotGranted)
    );
}
