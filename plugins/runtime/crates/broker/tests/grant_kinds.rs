//! A grant of one kind never implies another (ported from the retired
//! runtime spike, `grant_of_one_kind_does_not_imply_another`): a network
//! grant gives no notifications, a notifications grant gives no network,
//! and a credential needs its own grant even for a granted origin.

use std::sync::Arc;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::HttpMethod;
use deckpilot_plugin_abi::capability::{AuthScheme, HttpAuth, HttpFetch, Notify};
use deckpilot_plugin_broker::*;

fn session(perms: &str) -> Session {
    let broker = Broker::new(
        Arc::new(MemoryAudit::new(100)),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    let plugin = PluginId::new("acme.weather").unwrap();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    broker.session(plugin)
}

fn get(url: &str, auth: Option<HttpAuth>) -> CapabilityRequest {
    CapabilityRequest::HttpFetch(HttpFetch {
        method: HttpMethod::Get,
        url: url.into(),
        headers: vec![],
        body: None,
        auth,
    })
}

fn notify() -> CapabilityRequest {
    CapabilityRequest::Notify(Notify {
        title: "t".into(),
        body: "b".into(),
    })
}

#[test]
fn network_grant_does_not_imply_notifications() {
    let s = session(r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]}]"#);
    assert!(
        s.authorize(&get("https://api.example.com/", None))
            .is_allowed()
    );
    assert_eq!(
        s.authorize(&notify()).denial(),
        Some(DenyReason::NotGranted)
    );
}

#[test]
fn notifications_grant_does_not_imply_network() {
    let s = session(r#"[{"kind":"notifications"}]"#);
    assert!(s.authorize(&notify()).is_allowed());
    for url in ["https://api.example.com/", "https://example.com/"] {
        assert_eq!(
            s.authorize(&get(url, None)).denial(),
            Some(DenyReason::OriginNotGranted),
            "{url}"
        );
    }
}

#[test]
fn network_grant_does_not_imply_a_credential() {
    let s = session(r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]}]"#);
    let auth = HttpAuth {
        credential: "token".into(),
        scheme: AuthScheme::Bearer,
    };
    assert_eq!(
        s.authorize(&get("https://api.example.com/", Some(auth)))
            .denial(),
        Some(DenyReason::CredentialNotGranted)
    );
}
