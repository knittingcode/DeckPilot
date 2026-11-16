//! Broker decisions, with an emphasis on what must be denied.

use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::{
    AuthScheme, CredentialRef, Header, HttpAuth, HttpFetch, Log, LogLevel, NoArgs, Notify,
    StorageKey, StoragePrefix, StorageSet,
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
        auth: None,
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
            auth: None,
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
        auth: None,
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
        severity: None,
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
        severity: None,
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

/// The denials a built-in module's transport names too carry that name
/// (the kind) for the guest; every other denial carries none.
#[test]
fn refused_redirects_name_their_network_kind() {
    use deckpilot_plugin_abi::NetworkErrorKind;
    let f = fixture(API_GET);
    let s = f.broker.session(f.plugin.clone());
    let Decision::Allow(Allowed::Fetch(first)) = s.authorize(&get("https://api.example.com/a"))
    else {
        panic!()
    };
    let denied = |loc: &str, hop| s.authorize_redirect(&first, loc, hop).denial().unwrap();
    for loc in [
        "http://api.example.com/b",
        "HTTP://evil.net/",
        "http://10.0.0.1/",
    ] {
        let reason = denied(loc, 1);
        assert_eq!(reason, DenyReason::InsecureRedirect, "{loc}");
        assert_eq!(
            reason.network_kind(),
            Some(NetworkErrorKind::InsecureRedirect)
        );
        // Nothing of the target in what the guest is told.
        assert_eq!(reason.to_string(), "redirect from https to http");
    }
    let too_many = denied("/b", MAX_REDIRECTS + 1);
    assert_eq!(
        too_many.network_kind(),
        Some(NetworkErrorKind::TooManyRedirects)
    );
    for loc in [
        "https://evil.net/",
        "https://127.0.0.1/",
        "file:///etc/passwd",
    ] {
        assert_eq!(denied(loc, 1).network_kind(), None, "{loc}");
    }
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
        auth: None,
    }));
    s.authorize(&CapabilityRequest::Notify(Notify {
        title: SECRET.into(),
        body: SECRET.into(),
        severity: None,
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

const WITH_TOKEN: &str = r#"[
    {"kind":"network","origin":"https://api.example.com","methods":["GET"]},
    {"kind":"network","origin":"https://cdn.example.com","methods":["GET"]},
    {"kind":"credential","key":"token","origins":["https://api.example.com"],"schemes":["bearer"]}]"#;

fn get_with(url: &str, credential: &str, scheme: AuthScheme) -> CapabilityRequest {
    CapabilityRequest::HttpFetch(HttpFetch {
        method: HttpMethod::Get,
        url: url.into(),
        headers: vec![],
        body: None,
        auth: Some(HttpAuth {
            credential: credential.into(),
            scheme,
        }),
    })
}

#[test]
fn credential_handles_go_only_to_their_origins_and_schemes() {
    let f = fixture(WITH_TOKEN);
    let s = f.broker.session(f.plugin.clone());
    let d = s.authorize(&get_with(
        "https://api.example.com/me",
        "token",
        AuthScheme::Bearer,
    ));
    let Decision::Allow(Allowed::Fetch(first)) = &d else {
        panic!("{d:?}")
    };
    assert_eq!(
        first.auth.as_ref().map(|x| x.credential.as_str()),
        Some("token")
    );
    // Same request without a handle: allowed, nothing to inject.
    let Decision::Allow(Allowed::Fetch(plain)) = s.authorize(&get("https://api.example.com/me"))
    else {
        panic!()
    };
    assert_eq!(plain.auth, None);

    for (req, reason) in [
        (
            get_with("https://cdn.example.com/", "token", AuthScheme::Bearer),
            DenyReason::CredentialOriginNotAllowed,
        ),
        (
            get_with("https://api.example.com/", "token", AuthScheme::Basic),
            DenyReason::CredentialSchemeNotAllowed,
        ),
        (
            get_with("https://api.example.com/", "other", AuthScheme::Bearer),
            DenyReason::CredentialNotGranted,
        ),
        // The origin check comes first: an ungranted origin stays an
        // ungranted origin, whatever handle is attached.
        (
            get_with("https://evil.net/", "token", AuthScheme::Bearer),
            DenyReason::OriginNotGranted,
        ),
    ] {
        assert_eq!(s.authorize(&req).denial(), Some(reason), "{req:?}");
        assert_eq!(reason.call_error_code(), CallErrorCode::NotPermitted);
    }

    // A redirect keeps the handle only on the original origin.
    assert!(s.authorize_redirect(first, "/v2/me", 1).is_allowed());
    assert_eq!(
        s.authorize_redirect(first, "https://cdn.example.com/x", 1)
            .denial(),
        Some(DenyReason::CredentialOriginNotAllowed)
    );

    // Revocation applies to the next call, redirects included.
    f.broker.revoke_credential(&f.plugin, "token");
    assert_eq!(
        s.authorize(&get_with(
            "https://api.example.com/",
            "token",
            AuthScheme::Bearer
        ))
        .denial(),
        Some(DenyReason::CredentialNotGranted)
    );
    assert_eq!(
        s.authorize_redirect(first, "/v2/me", 1).denial(),
        Some(DenyReason::CredentialNotGranted)
    );
}

#[test]
fn credential_grants_are_strict() {
    let derive = |json: &str| Grants::derive(&ApprovedPermissions::from_json(json)?);
    let net = r#"{"kind":"network","origin":"https://api.example.com","methods":["GET"]}"#;
    let cred = |body: &str| format!(r#"[{net},{{"kind":"credential",{body}}}]"#);
    assert!(
        derive(&cred(
            r#""key":"token","origins":["https://api.example.com"],"schemes":["bearer"]"#
        ))
        .is_ok()
    );
    for (body, err) in [
        (
            r#""key":"Token","origins":["https://api.example.com"],"schemes":["bearer"]"#,
            GrantError::CredentialKey,
        ),
        (
            r#""key":"token","origins":[],"schemes":["bearer"]"#,
            GrantError::EmptyCredential,
        ),
        (
            r#""key":"token","origins":["https://api.example.com"],"schemes":[]"#,
            GrantError::EmptyCredential,
        ),
        (
            r#""key":"token","origins":["https://api.example.com"],"schemes":["bearer"],"value":"s3cret""#,
            GrantError::Malformed,
        ),
        (
            r#""key":"token","origins":["https://api.example.com"],"schemes":["digest"]"#,
            GrantError::Malformed,
        ),
    ] {
        assert_eq!(derive(&cred(body)), Err(err), "{body}");
    }
    let twice = format!(
        r#"[{net},{{"kind":"credential","key":"t","origins":["https://api.example.com"],"schemes":["bearer"]}},
            {{"kind":"credential","key":"t","origins":["https://api.example.com"],"schemes":["basic"]}}]"#
    );
    assert_eq!(derive(&twice), Err(GrantError::DuplicateCredential));
    // A slot towards a host without a network grant is not an invalid
    // record (the user may revoke the host at any time): the slot is
    // unusable, nothing else is affected.
    let unreachable = derive(&cred(
        r#""key":"token","origins":["https://other.example.com"],"schemes":["bearer"]"#,
    ))
    .unwrap();
    assert!(unreachable.credential("token").is_none());
    assert_eq!(unreachable.origins().count(), 1);
    assert_eq!(
        unreachable.suspended_credentials()["token"]
            .iter()
            .map(Origin::to_string)
            .collect::<Vec<_>>(),
        ["https://other.example.com"]
    );
}

/// The permissions of a plugin with two hosts, a credential slot towards
/// the first, notifications and a larger storage quota.
fn slot_and_more(network: &[&str]) -> String {
    let mut items: Vec<String> = network
        .iter()
        .map(|o| format!(r#"{{"kind":"network","origin":"{o}","methods":["GET"]}}"#))
        .collect();
    items.push(
        r#"{"kind":"credential","key":"token","origins":["https://api.example.com"],"schemes":["bearer"]}"#
            .into(),
    );
    items.push(r#"{"kind":"notifications"}"#.into());
    items.push(r#"{"kind":"storage","quotaKb":512}"#.into());
    format!("[{}]", items.join(","))
}

fn notify() -> CapabilityRequest {
    CapabilityRequest::Notify(Notify {
        title: "t".into(),
        body: "b".into(),
        severity: None,
    })
}

fn has(key: &str) -> CapabilityRequest {
    CapabilityRequest::CredentialHas(CredentialRef {
        credential: key.into(),
    })
}

/// Everything but the slot whose host lost its network grant keeps
/// working: settings, storage (with its granted quota), the other host,
/// notifications. Asserted the same way whether the grants were set
/// without the host (the agent re-deriving after a revoke) or the host was
/// revoked on a running session.
fn assert_only_the_slot_is_gone(f: &Fixture) {
    let s = f.broker.session(f.plugin.clone());
    // The slot: unusable for every call, never injected.
    assert_eq!(
        s.authorize(&has("token")).denial(),
        Some(DenyReason::CredentialNotGranted)
    );
    for url in ["https://api.example.com/me", "https://cdn.example.com/me"] {
        let d = s.authorize(&get_with(url, "token", AuthScheme::Bearer));
        assert!(!d.is_allowed(), "{url}: {d:?}");
        assert_eq!(
            d.denial().map(DenyReason::call_error_code),
            Some(CallErrorCode::NotPermitted),
            "{url}"
        );
    }
    // The host itself, without a credential: no network grant.
    assert_eq!(
        s.authorize(&get("https://api.example.com/")).denial(),
        Some(DenyReason::OriginNotGranted)
    );
    // Everything else.
    assert!(s.authorize(&get("https://cdn.example.com/")).is_allowed());
    assert!(s.authorize(&notify()).is_allowed());
    assert_eq!(
        s.authorize(&CapabilityRequest::SettingsGet(NoArgs {})),
        Decision::Allow(Allowed::SettingsGet)
    );
    let d = s.authorize(&storage_set("k"));
    assert_eq!(quota_of(&d).bytes, 512 * 1024);
    assert!(s.authorize(&log("still here")).is_allowed());
}

#[test]
fn a_revoked_host_takes_only_its_credential_slot_with_it() {
    let both = ["https://api.example.com", "https://cdn.example.com"];
    // Re-derived without the host, as the agent does after a revoke.
    let f = fixture(&slot_and_more(&both));
    let s = f.broker.session(f.plugin.clone());
    assert!(
        s.authorize(&get_with(
            "https://api.example.com/me",
            "token",
            AuthScheme::Bearer
        ))
        .is_allowed()
    );
    f.broker
        .set_grants(
            &f.plugin,
            &ApprovedPermissions::from_json(&slot_and_more(&["https://cdn.example.com"])).unwrap(),
        )
        .unwrap();
    assert_only_the_slot_is_gone(&f);
    // Granted again: the slot is back on the next call.
    f.broker
        .set_grants(
            &f.plugin,
            &ApprovedPermissions::from_json(&slot_and_more(&both)).unwrap(),
        )
        .unwrap();
    assert!(
        s.authorize(&get_with(
            "https://api.example.com/me",
            "token",
            AuthScheme::Bearer
        ))
        .is_allowed()
    );
    assert!(s.authorize(&has("token")).is_allowed());

    // Revoked on the running grants: the same.
    let f = fixture(&slot_and_more(&both));
    f.broker.revoke_origin(
        &f.plugin,
        &Origin::parse_grant("https://api.example.com").unwrap(),
    );
    assert_only_the_slot_is_gone(&f);
}

#[test]
fn a_slot_keeps_only_the_origins_that_still_have_a_network_grant() {
    let f = fixture(
        r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]},
            {"kind":"network","origin":"https://uploads.example.com","methods":["GET"]},
            {"kind":"credential","key":"token","schemes":["bearer"],
             "origins":["https://api.example.com","https://uploads.example.com"]}]"#,
    );
    let s = f.broker.session(f.plugin.clone());
    f.broker.revoke_origin(
        &f.plugin,
        &Origin::parse_grant("https://uploads.example.com").unwrap(),
    );
    // Never towards the revoked host...
    assert!(
        !s.authorize(&get_with(
            "https://uploads.example.com/x",
            "token",
            AuthScheme::Bearer
        ))
        .is_allowed()
    );
    // ...still towards the one the user kept.
    assert!(s.authorize(&has("token")).is_allowed());
    assert!(
        s.authorize(&get_with(
            "https://api.example.com/x",
            "token",
            AuthScheme::Bearer
        ))
        .is_allowed()
    );
    // A redirect from the kept host to the revoked one carries nothing.
    let Decision::Allow(Allowed::Fetch(first)) = s.authorize(&get_with(
        "https://api.example.com/x",
        "token",
        AuthScheme::Bearer,
    )) else {
        panic!("allowed above");
    };
    assert!(
        !s.authorize_redirect(&first, "https://uploads.example.com/x", 1)
            .is_allowed()
    );
}

fn storage_set(key: &str) -> CapabilityRequest {
    CapabilityRequest::StorageSet(StorageSet {
        key: key.into(),
        value: json!(1),
    })
}

fn quota_of(d: &Decision) -> StorageQuota {
    match d {
        Decision::Allow(Allowed::StorageSet { quota, .. } | Allowed::StorageUsage { quota }) => {
            *quota
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn storage_is_always_on_with_a_quota_a_grant_can_raise() {
    let f = fixture("[]");
    let s = f.broker.session(f.plugin.clone());
    let d = s.authorize(&storage_set("k"));
    assert_eq!(quota_of(&d), StorageQuota::default());
    assert_eq!(quota_of(&d).bytes, 256 * 1024);
    assert_eq!(quota_of(&d).keys, 1_000);
    let usage = s.authorize(&CapabilityRequest::StorageUsage(NoArgs {}));
    assert_eq!(quota_of(&usage), StorageQuota::default());
    let keys = s.authorize(&CapabilityRequest::StorageKeys(StoragePrefix {
        prefix: Some("a.".into()),
    }));
    assert_eq!(
        keys,
        Decision::Allow(Allowed::StorageKeys {
            prefix: Some("a.".into())
        })
    );

    // A `storage` grant raises the quota; the next call sees it.
    f.broker
        .set_grants(
            &f.plugin,
            &ApprovedPermissions::from_json(r#"[{"kind":"storage","quotaKb":1024}]"#).unwrap(),
        )
        .unwrap();
    assert_eq!(quota_of(&s.authorize(&storage_set("k"))).bytes, 1024 * 1024);
    // Revoked (grants replaced without it): the very next call is back to
    // the default quota.
    f.broker
        .set_grants(&f.plugin, &ApprovedPermissions::default())
        .unwrap();
    assert_eq!(quota_of(&s.authorize(&storage_set("k"))).bytes, 256 * 1024);
    // Disabled or uninstalled: storage is denied too.
    f.broker.remove_plugin(&f.plugin);
    assert_eq!(
        s.authorize(&storage_set("k")).denial(),
        Some(DenyReason::UnknownPlugin)
    );
}

#[test]
fn storage_quota_grants_are_bounded() {
    for (kb, ok) in [
        (256, false),
        (257, true),
        (10_240, true),
        (10_241, false),
        (0, false),
    ] {
        let json = format!(r#"[{{"kind":"storage","quotaKb":{kb}}}]"#);
        let derived = Grants::derive(&ApprovedPermissions::from_json(&json).unwrap());
        assert_eq!(derived.is_ok(), ok, "{kb}");
        if !ok {
            assert_eq!(derived, Err(GrantError::StorageQuota));
        }
    }
    assert!(
        ApprovedPermissions::from_json(r#"[{"kind":"storage","quotaKb":512,"keys":5000}]"#)
            .is_err()
    );
}

#[test]
fn storage_is_scoped_to_the_session_and_rate_limited() {
    let f = fixture_with(
        "[]",
        RateLimits {
            storage: Rate::new(2, Duration::from_secs(1)),
            ..RateLimits::default()
        },
    );
    // A plugin the broker has no grants for gets nothing, not even
    // storage; no request can name another plugin's storage.
    let other = f.broker.session(PluginId::new("evil.reader").unwrap());
    assert_eq!(
        other
            .authorize(&CapabilityRequest::StorageGet(StorageKey {
                key: "k".into()
            }))
            .denial(),
        Some(DenyReason::UnknownPlugin)
    );
    let s = f.broker.session(f.plugin.clone());
    assert!(s.authorize(&storage_set("a")).is_allowed());
    assert!(s.authorize(&storage_set("b")).is_allowed());
    assert_eq!(
        s.authorize(&storage_set("c")).denial(),
        Some(DenyReason::RateLimited)
    );
    f.clock.advance(Duration::from_secs(1));
    assert!(s.authorize(&storage_set("c")).is_allowed());
    assert_eq!(s.plugin(), &f.plugin);
}

#[test]
fn credential_presence_needs_the_slot_grant_and_is_revoked_at_once() {
    let f = fixture(WITH_TOKEN);
    let s = f.broker.session(f.plugin.clone());
    let has = |key: &str| {
        CapabilityRequest::CredentialHas(CredentialRef {
            credential: key.into(),
        })
    };
    assert_eq!(
        s.authorize(&has("token")),
        Decision::Allow(Allowed::CredentialHas {
            key: "token".into()
        })
    );
    assert_eq!(
        s.authorize(&has("other")).denial(),
        Some(DenyReason::CredentialNotGranted)
    );
    f.broker.revoke_credential(&f.plugin, "token");
    assert_eq!(
        s.authorize(&has("token")).denial(),
        Some(DenyReason::CredentialNotGranted)
    );
    assert_eq!(
        s.authorize(&get_with(
            "https://api.example.com/me",
            "token",
            AuthScheme::Bearer
        ))
        .denial(),
        Some(DenyReason::CredentialNotGranted)
    );
}

#[test]
fn settings_are_the_plugins_own_while_it_runs() {
    let f = fixture("[]");
    let s = f.broker.session(f.plugin.clone());
    assert_eq!(
        s.authorize(&CapabilityRequest::SettingsGet(NoArgs {})),
        Decision::Allow(Allowed::SettingsGet)
    );
    f.broker.remove_plugin(&f.plugin);
    assert_eq!(
        s.authorize(&CapabilityRequest::SettingsGet(NoArgs {}))
            .denial(),
        Some(DenyReason::UnknownPlugin)
    );
}

#[test]
fn own_variables_are_always_allowed_but_rate_limited() {
    use deckpilot_plugin_abi::capability::{VariableClear, VariablePublish};
    let publish = |id: &str| {
        CapabilityRequest::VariablesPublish(VariablePublish {
            id: id.into(),
            value: json!(21),
        })
    };
    // No permission is needed: the own namespace is always granted.
    let f = fixture("[]");
    let s = f.broker.session(f.plugin.clone());
    let d = s.authorize(&publish("temperature"));
    assert_eq!(
        d,
        Decision::Allow(Allowed::VariablePublish {
            id: "temperature".into(),
            value: json!(21),
        })
    );
    assert_eq!(
        s.authorize(&CapabilityRequest::VariablesClear(VariableClear {
            id: None
        })),
        Decision::Allow(Allowed::VariableClear { id: None })
    );
    // Absolute ids (another plugin's) are malformed, never allowed.
    assert_eq!(
        s.authorize(&publish("plugins.other.prs.x")).denial(),
        Some(DenyReason::TooLarge)
    );
    // 10 per second (two already taken above).
    let mut allowed = 2;
    while s.authorize(&publish("temperature")).is_allowed() {
        allowed += 1;
        assert!(allowed <= 10, "no rate limit");
    }
    assert_eq!(allowed, 10);
    f.clock.advance(Duration::from_secs(1));
    assert!(s.authorize(&publish("temperature")).is_allowed());
    // A plugin without grants (disabled, removed) publishes nothing.
    f.broker.remove_plugin(&f.plugin);
    assert_eq!(
        s.authorize(&publish("temperature")).denial(),
        Some(DenyReason::UnknownPlugin)
    );
}
