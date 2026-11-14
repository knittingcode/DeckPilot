//! Broker decisions, with an emphasis on what must be denied.

use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::{
    Header, HttpFetch, Log, LogLevel, Notify, StorageKey, StorageSet,
};
use deckpilot_plugin_abi::{CallErrorCode, HttpMethod};
use deckpilot_plugin_broker::*;
use serde_json::json;

struct Fixture {
    broker: Broker,
    audit: Arc<MemoryAudit>,
    clock: ManualClock,
    plugin: PluginId,
}

fn fixture(perms: &str) -> Fixture {
    fixture_with(perms, RateLimits::default())
}

fn fixture_with(perms: &str, limits: RateLimits) -> Fixture {
    let audit = Arc::new(MemoryAudit::new(10_000));
    let clock = ManualClock::default();
    let broker = Broker::new(audit.clone(), Arc::new(clock.clone()), limits);
    let plugin = PluginId::new("acme.weather").unwrap();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    Fixture {
        broker,
        audit,
        clock,
        plugin,
    }
}

const API_GET: &str =
    r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]}]"#;

fn fetch(method: HttpMethod, url: &str) -> CapabilityRequest {
    CapabilityRequest::HttpFetch(HttpFetch {
        method,
        url: url.into(),
        headers: vec![],
        body: None,
    })
}

fn get(url: &str) -> CapabilityRequest {
    fetch(HttpMethod::Get, url)
}

fn log(msg: &str) -> CapabilityRequest {
    CapabilityRequest::Log(Log {
        level: LogLevel::Info,
        message: msg.into(),
    })
}

#[test]
fn allowed_fetch_is_normalized() {
    let f = fixture(API_GET);
    let s = f.broker.session(f.plugin.clone());
    let d = s.authorize(&get("https://API.example.com:443/v1/x?q=1"));
    let Decision::Allow(Allowed::Fetch(a)) = d else {
        panic!("{d:?}")
    };
    assert_eq!(a.url.as_str(), "https://api.example.com/v1/x?q=1");
    assert_eq!(a.origin.to_string(), "https://api.example.com");
    // Unicode compatibility forms map to the same ASCII host (IDNA), and
    // the normalized URL is what the network layer will use.
    let d = s.authorize(&get("https://ＡＰＩ.example.com/"));
    let Decision::Allow(Allowed::Fetch(a)) = d else {
        panic!("{d:?}")
    };
    assert_eq!(a.url.host_str(), Some("api.example.com"));
}

#[test]
fn origin_tricks_are_denied() {
    let f = fixture(API_GET);
    let s = f.broker.session(f.plugin.clone());
    let cases = [
        // userinfo: the real host is evil.net
        "https://api.example.com@evil.net/",
        "https://api.example.com:443@evil.net/",
        "https://user:pass@api.example.com/",
        "https://@api.example.com/",
        // suffix / prefix domains
        "https://api.example.com.evil.net/",
        "https://evilapi.example.com/",
        "https://example.com/",
        "https://sub.api.example.com/",
        "https://api.example.co/",
        "https://api-example.com/",
        // ports
        "https://api.example.com:8443/",
        "https://api.example.com:80/",
        "https://api.example.com:0/",
        // schemes
        "http://api.example.com/",
        "HTTP://api.example.com/",
        "ws://api.example.com/",
        "wss://api.example.com/",
        "ftp://api.example.com/",
        "file:///etc/passwd",
        "data:text/plain,api.example.com",
        "javascript:alert(1)",
        "//api.example.com/",
        "api.example.com/",
        // the granted URL hidden elsewhere
        "https://evil.net/?u=https://api.example.com/",
        "https://evil.net/#https://api.example.com/",
        "https://evil.net/https://api.example.com",
        // IDN homographs and punycode of other names
        "https://аpi.example.com/", // Cyrillic а
        "https://xn--pi-7kc.example.com/",
        "https://api.exаmple.com/",
        // IP literals, including private and metadata ranges
        "https://127.0.0.1/",
        "https://10.0.0.1/",
        "https://169.254.169.254/latest/meta-data/",
        "https://[::1]/",
        "https://[::ffff:127.0.0.1]/",
        "https://2130706433/",
        "https://0x7f.0.0.1/",
        "https://0177.0.0.1/",
        "https://127.1/",
        // local names
        "https://localhost/",
        "https://api.localhost/",
        "https://router.local/",
        "https://metadata.google.internal/",
        // parser-differential bait
        "https://api.example.com\\@evil.net/",
        "https://api.example.com\t.evil.net/",
        "https://evil.net\n.api.example.com/",
        "https://api.example.com /",
        "https://api.example.com%2F@evil.net/",
        "https://api.example.com%40evil.net/",
        "https://api.example.com./",
        "https://*.example.com/",
        "",
        "https://",
    ];
    for url in cases {
        let d = s.authorize(&get(url));
        assert!(!d.is_allowed(), "{url:?} must be denied, got {d:?}");
    }
}

#[test]
fn methods_and_headers_are_checked() {
    let f = fixture(API_GET);
    let s = f.broker.session(f.plugin.clone());
    assert_eq!(
        s.authorize(&fetch(HttpMethod::Post, "https://api.example.com/"))
            .denial(),
        Some(DenyReason::MethodNotGranted)
    );
    for (name, value) in [
        ("Host", "evil.net"),
        ("host", "api.example.com"),
        ("Authorization", "Bearer x"),
        ("Cookie", "a=b"),
        ("Proxy-Authorization", "x"),
        ("X-Forwarded-For", "127.0.0.1"),
        ("X-HTTP-Method-Override", "DELETE"),
        ("Transfer-Encoding", "chunked"),
        ("Content-Length", "1"),
        ("Sec-Fetch-Site", "none"),
        ("X-Ok", "a\r\nHost: evil.net"),
        ("X Bad", "v"),
        ("", "v"),
    ] {
        let req = CapabilityRequest::HttpFetch(HttpFetch {
            method: HttpMethod::Get,
            url: "https://api.example.com/".into(),
            headers: vec![Header {
                name: name.into(),
                value: value.into(),
            }],
            body: None,
        });
        assert_eq!(
            s.authorize(&req).denial(),
            Some(DenyReason::ForbiddenHeader),
            "{name}"
        );
    }
    let ok = CapabilityRequest::HttpFetch(HttpFetch {
        method: HttpMethod::Get,
        url: "https://api.example.com/".into(),
        headers: vec![Header {
            name: "Accept".into(),
            value: "application/json".into(),
        }],
        body: None,
    });
    assert!(s.authorize(&ok).is_allowed());
}

#[test]
fn deny_by_default() {
    let f = fixture("[]");
    let s = f.broker.session(f.plugin.clone());
    assert_eq!(
        s.authorize(&get("https://api.example.com/")).denial(),
        Some(DenyReason::OriginNotGranted)
    );
    let n = CapabilityRequest::Notify(Notify {
        title: "t".into(),
        body: "b".into(),
    });
    assert_eq!(s.authorize(&n).denial(), Some(DenyReason::NotGranted));
    // always-on capabilities
    assert!(s.authorize(&log("hello")).is_allowed());
    let unknown = f.broker.session(PluginId::new("acme.other").unwrap());
    assert_eq!(
        unknown.authorize(&log("x")).denial(),
        Some(DenyReason::UnknownPlugin)
    );
}

#[test]
fn identity_is_the_session_not_the_request() {
    let f = fixture(API_GET);
    let other = PluginId::new("evil.thief").unwrap();
    f.broker
        .set_grants(&other, &ApprovedPermissions::default())
        .unwrap();
    // A session for `evil.thief` cannot use `acme.weather`'s grants: no
    // request field could name another plugin (they do not parse).
    let thief = f.broker.session(other);
    assert!(
        !thief
            .authorize(&get("https://api.example.com/"))
            .is_allowed()
    );
    let spoof = serde_json::from_value::<CapabilityRequest>(json!({
        "op": "http.fetch",
        "args": {"method": "GET", "url": "https://api.example.com/", "plugin": "acme.weather"}
    }));
    assert!(spoof.is_err());
}

#[test]
fn revocation_applies_to_the_next_call() {
    let f = fixture(
        r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]},
            {"kind":"notifications"}]"#,
    );
    let s = f.broker.session(f.plugin.clone());
    let req = get("https://api.example.com/");
    assert!(s.authorize(&req).is_allowed());
    f.broker.revoke_origin(
        &f.plugin,
        &Origin::parse_grant("https://api.example.com").unwrap(),
    );
    assert_eq!(
        s.authorize(&req).denial(),
        Some(DenyReason::OriginNotGranted)
    );
    let n = CapabilityRequest::Notify(Notify {
        title: "t".into(),
        body: "b".into(),
    });
    assert!(s.authorize(&n).is_allowed());
    f.broker.revoke_notifications(&f.plugin);
    assert_eq!(s.authorize(&n).denial(), Some(DenyReason::NotGranted));
    f.broker.remove_plugin(&f.plugin);
    assert_eq!(
        s.authorize(&log("x")).denial(),
        Some(DenyReason::UnknownPlugin)
    );
}

#[test]
fn failed_grant_update_keeps_previous_grants() {
    let f = fixture(API_GET);
    let bad = ApprovedPermissions::from_json(
        r#"[{"kind":"network","origin":"https://*.example.com","methods":["GET"]}]"#,
    )
    .unwrap();
    assert!(f.broker.set_grants(&f.plugin, &bad).is_err());
    let s = f.broker.session(f.plugin.clone());
    assert!(s.authorize(&get("https://api.example.com/")).is_allowed());
}

#[test]
fn redirects_are_rechecked() {
    let f = fixture(
        r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]},
            {"kind":"network","origin":"https://cdn.example.com","methods":["GET"]}]"#,
    );
    let s = f.broker.session(f.plugin.clone());
    let Decision::Allow(Allowed::Fetch(first)) = s.authorize(&get("https://api.example.com/a"))
    else {
        panic!()
    };
    // relative and granted-origin redirects
    assert!(s.authorize_redirect(&first, "/b", 1).is_allowed());
    assert!(
        s.authorize_redirect(&first, "https://cdn.example.com/f", 1)
            .is_allowed()
    );
    for loc in [
        "http://api.example.com/b",
        "https://evil.net/",
        "https://127.0.0.1/",
        "https://[::1]/",
        "https://169.254.169.254/",
        "https://localhost/",
        "//evil.net/x",
        "https://api.example.com@evil.net/",
        "https://api.example.com:8443/",
        "file:///etc/passwd",
        "https://api.example.com\\@evil.net",
    ] {
        assert!(!s.authorize_redirect(&first, loc, 1).is_allowed(), "{loc}");
    }
    assert_eq!(
        s.authorize_redirect(&first, "/b", MAX_REDIRECTS + 1)
            .denial(),
        Some(DenyReason::TooManyRedirects)
    );
    assert_eq!(
        s.authorize_redirect(&first, "/b", 0).denial(),
        Some(DenyReason::TooManyRedirects)
    );
}

#[test]
fn rate_limits_apply_per_plugin_and_class() {
    let limits = RateLimits {
        http: Rate::new(3, Duration::from_secs(60)),
        log: Rate::new(2, Duration::from_secs(1)),
        ..RateLimits::default()
    };
    let f = fixture_with(API_GET, limits);
    let s = f.broker.session(f.plugin.clone());
    let req = get("https://api.example.com/");
    for _ in 0..3 {
        assert!(s.authorize(&req).is_allowed());
    }
    let d = s.authorize(&req);
    assert_eq!(d.denial(), Some(DenyReason::RateLimited));
    assert_eq!(
        d.denial().unwrap().call_error_code(),
        CallErrorCode::Throttled
    );
    // other classes are independent
    assert!(s.authorize(&log("a")).is_allowed());
    assert!(s.authorize(&log("b")).is_allowed());
    assert_eq!(
        s.authorize(&log("c")).denial(),
        Some(DenyReason::RateLimited)
    );
    // other plugins are independent
    let other = PluginId::new("acme.other").unwrap();
    f.broker
        .set_grants(&other, &ApprovedPermissions::from_json(API_GET).unwrap())
        .unwrap();
    assert!(f.broker.session(other).authorize(&req).is_allowed());
    // tokens come back with time
    f.clock.advance(Duration::from_secs(20));
    assert!(s.authorize(&req).is_allowed());
    assert!(!s.authorize(&req).is_allowed());
}

#[test]
fn flooding_is_bounded_by_the_global_bucket() {
    let limits = RateLimits {
        any: Rate::new(100, Duration::from_secs(1)),
        ..RateLimits::default()
    };
    let f = fixture_with(API_GET, limits);
    let s = f.broker.session(f.plugin.clone());
    let denied_origin = get("https://evil.net/");
    let mut rate_limited = 0;
    for _ in 0..1000 {
        if s.authorize(&denied_origin).denial() == Some(DenyReason::RateLimited) {
            rate_limited += 1;
        }
    }
    assert_eq!(rate_limited, 900);
}

#[test]
fn oversized_requests_are_denied() {
    let f = fixture(API_GET);
    let s = f.broker.session(f.plugin.clone());
    let long_url = format!("https://api.example.com/{}", "a".repeat(9000));
    assert_eq!(
        s.authorize(&get(&long_url)).denial(),
        Some(DenyReason::TooLarge)
    );
    assert_eq!(
        s.authorize(&log(&"x".repeat(5000))).denial(),
        Some(DenyReason::TooLarge)
    );
}

#[test]
fn audit_records_never_contain_values() {
    let f = fixture(
        r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET","POST"]},
            {"kind":"notifications"}]"#,
    );
    let s = f.broker.session(f.plugin.clone());
    const SECRET: &str = "s3cr3t-value";
    s.authorize(&get(&format!(
        "https://api.example.com/{SECRET}?token={SECRET}"
    )));
    s.authorize(&get(&format!("https://evil.net/{SECRET}")));
    s.authorize(&get(&format!("not a url {SECRET}")));
    s.authorize(&CapabilityRequest::HttpFetch(HttpFetch {
        method: HttpMethod::Post,
        url: "https://api.example.com/".into(),
        headers: vec![Header {
            name: "X-Key".into(),
            value: SECRET.into(),
        }],
        body: Some(SECRET.into()),
    }));
    s.authorize(&CapabilityRequest::Notify(Notify {
        title: SECRET.into(),
        body: SECRET.into(),
    }));
    s.authorize(&log(SECRET));
    s.authorize(&CapabilityRequest::StorageSet(StorageSet {
        key: SECRET.into(),
        value: json!(SECRET),
    }));
    s.authorize(&CapabilityRequest::StorageGet(StorageKey {
        key: SECRET.into(),
    }));
    let records = f.audit.records();
    assert_eq!(records.len(), 8);
    for r in &records {
        let text = format!("{r:?}");
        assert!(!text.contains(SECRET), "audit leaked a value: {text}");
        assert_eq!(r.plugin, f.plugin);
    }
    assert_eq!(
        records[0].target.as_deref(),
        Some("https://api.example.com")
    );
    assert_eq!(records[0].decision, AuditDecision::Allowed);
    assert!(matches!(records[1].decision, AuditDecision::Denied(_)));
    // A denied origin is recorded (host only), so attempts are visible.
    assert_eq!(records[1].target.as_deref(), Some("https://evil.net"));
    let seqs: Vec<u64> = records.iter().map(|r| r.seq).collect();
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1));
}
