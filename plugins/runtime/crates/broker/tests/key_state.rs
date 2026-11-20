//! Key state (plugin API 1.2): the agent asks a plugin for the state of
//! its keys only while `keyState` is granted, and inside a state call the
//! broker answers reads and nothing else, whatever else is granted.

use std::sync::Arc;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_broker::*;

const EVERYTHING: &str = r#"[
    {"kind":"keyState"},
    {"kind":"network","origin":"https://api.example.com","methods":["GET","POST"]},
    {"kind":"notifications"},
    {"kind":"openUrl","origin":"https://github.com"},
    {"kind":"credential","key":"token","origins":["https://api.example.com"],"schemes":["bearer"]},
    {"kind":"storage","quotaKb":1024}
]"#;

struct Fixture {
    broker: Broker,
    audit: Arc<MemoryAudit>,
    plugin: PluginId,
}

fn fixture(perms: &str) -> Fixture {
    let audit = Arc::new(MemoryAudit::new(1_000));
    let broker = Broker::new(
        audit.clone(),
        Arc::new(ManualClock::default()),
        RateLimits::default(),
    );
    let plugin = PluginId::new("acme.board").unwrap();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    Fixture {
        broker,
        audit,
        plugin,
    }
}

fn request(json: &str) -> CapabilityRequest {
    serde_json::from_str(json).unwrap()
}

#[test]
fn state_is_asked_only_while_key_state_is_granted() {
    let f = fixture(EVERYTHING);
    let s = f.broker.session(f.plugin.clone());
    assert_eq!(s.authorize_key_state(), Ok(()));
    // Revoked: the next state call is refused; everything else stays.
    f.broker.revoke_key_state(&f.plugin);
    assert_eq!(s.authorize_key_state(), Err(DenyReason::NotGranted));
    assert!(
        s.authorize(&request(
            r#"{"op":"notify","args":{"title":"t","body":"b"}}"#
        ))
        .is_allowed()
    );
    // Every other grant together does not imply it.
    let without = EVERYTHING.replace(r#"{"kind":"keyState"},"#, "");
    let other = fixture(&without);
    assert_eq!(
        other
            .broker
            .session(other.plugin.clone())
            .authorize_key_state(),
        Err(DenyReason::NotGranted)
    );
    // A plugin without grants, or removed, is unknown.
    f.broker.remove_plugin(&f.plugin);
    assert_eq!(s.authorize_key_state(), Err(DenyReason::UnknownPlugin));

    let seen: Vec<_> = f
        .audit
        .records()
        .iter()
        .filter(|r| r.op == "key.state")
        .map(|r| (r.target.clone(), r.decision))
        .collect();
    assert_eq!(
        seen,
        [
            (None, AuditDecision::Allowed),
            (None, AuditDecision::Denied(DenyReason::NotGranted)),
            (None, AuditDecision::Denied(DenyReason::UnknownPlugin)),
        ]
    );
}

#[test]
fn key_state_is_not_a_device_grant() {
    // The typed state is the only device-facing grant there is: anything
    // raw stays never grantable.
    for kind in ["rawDevice", "brightness", "firmware", "customUi"] {
        let json = format!(r#"[{{"kind":"{kind}"}}]"#);
        assert!(
            matches!(
                ApprovedPermissions::from_json(&json),
                Err(GrantError::NeverGrantable(_))
            ),
            "{kind}"
        );
    }
    // And it takes no arguments (no key, no device, no size).
    assert!(ApprovedPermissions::from_json(r#"[{"kind":"keyState","device":"x"}]"#).is_err());
}

#[test]
fn a_state_call_reads_and_nothing_else_whatever_is_granted() {
    let f = fixture(EVERYTHING);
    let s = f.broker.session(f.plugin.clone());
    for allowed in [
        r#"{"op":"storage.get","args":{"key":"board"}}"#,
        r#"{"op":"storage.keys","args":{}}"#,
        r#"{"op":"settings.get","args":{}}"#,
        r#"{"op":"time.now","args":{}}"#,
        r#"{"op":"log","args":{"level":"debug","message":"m"}}"#,
        // Plugin API 1.9: which key this is (`keyScope`), nothing else.
        r#"{"op":"run.context","args":{}}"#,
    ] {
        assert!(
            s.authorize_state(&request(allowed)).is_allowed(),
            "{allowed}"
        );
    }
    let refused = [
        r#"{"op":"http.fetch","args":{"method":"GET","url":"https://api.example.com/"}}"#,
        r#"{"op":"http.fetch","args":{"method":"POST","url":"https://api.example.com/","body":"x","auth":{"credential":"token","scheme":"bearer"}}}"#,
        r#"{"op":"notify","args":{"title":"t","body":"b"}}"#,
        r#"{"op":"url.open","args":{"url":"https://github.com/acme"}}"#,
        r#"{"op":"storage.set","args":{"key":"k","value":1}}"#,
        r#"{"op":"storage.delete","args":{"key":"k"}}"#,
        r#"{"op":"storage.usage","args":{}}"#,
        r#"{"op":"credentials.has","args":{"credential":"token"}}"#,
        r#"{"op":"variables.publish","args":{"id":"x","value":1}}"#,
        r#"{"op":"variables.clear","args":{}}"#,
        r#"{"op":"random.bytes","args":{"length":8}}"#,
    ];
    for json in refused {
        let decision = s.authorize_state(&request(json));
        assert_eq!(
            decision.denial(),
            Some(DenyReason::NotInStateCall),
            "{json}"
        );
        assert_eq!(
            DenyReason::NotInStateCall.call_error_code(),
            deckpilot_plugin_abi::CallErrorCode::NotPermitted
        );
        // The same request in an action is allowed (but for the page,
        // which needs a run the user started): the refusal is the state
        // call's, not a missing grant.
        if !json.contains("url.open") {
            assert!(s.authorize(&request(json)).is_allowed(), "{json}");
        }
    }
    // Refused before any rate limit, and audited by operation, without a
    // target or a value.
    let records = f.audit.records();
    let denied: Vec<_> = records
        .iter()
        .filter(|r| r.decision == AuditDecision::Denied(DenyReason::NotInStateCall))
        .collect();
    assert_eq!(denied.len(), refused.len());
    assert!(denied.iter().all(|r| r.target.is_none()));
    assert_eq!(denied[0].op, "http.fetch");

    // Reads inside a state call keep their own limits and need grants.
    for _ in 0..20 {
        s.authorize_state(&request(r#"{"op":"time.now","args":{}}"#));
    }
    assert_eq!(
        s.authorize_state(&request(r#"{"op":"time.now","args":{}}"#))
            .denial(),
        Some(DenyReason::RateLimited)
    );
    f.broker.remove_plugin(&f.plugin);
    assert_eq!(
        s.authorize_state(&request(r#"{"op":"storage.get","args":{"key":"k"}}"#))
            .denial(),
        Some(DenyReason::UnknownPlugin)
    );
}
