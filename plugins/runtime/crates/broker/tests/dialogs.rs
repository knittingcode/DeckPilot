//! Plugin dialogs (plugin API 1.5): `dialog.open` needs the `dialogs`
//! grant and a run the user started (checked before the rate limit), is
//! limited to 6 per minute per plugin, never inside a state call, and its
//! request passes every bound before the executor sees it.

use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_abi::CallErrorCode;
use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::{RunCause, RunTrigger};
use deckpilot_plugin_broker::*;

struct Fixture {
    broker: Broker,
    audit: Arc<MemoryAudit>,
    clock: ManualClock,
    plugin: PluginId,
}

fn fixture(perms: &str) -> Fixture {
    let audit = Arc::new(MemoryAudit::new(1_000));
    let clock = ManualClock::default();
    let broker = Broker::new(
        audit.clone(),
        Arc::new(clock.clone()),
        RateLimits::default(),
    );
    let plugin = PluginId::new("acme.repo").unwrap();
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

fn confirm() -> CapabilityRequest {
    serde_json::from_str(r#"{"op":"dialog.open","args":{"type":"confirm","title":"Pull now?"}}"#)
        .unwrap()
}

fn key() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

#[test]
fn dialogs_need_the_grant() {
    let f = fixture(r#"[{"kind":"notifications"}]"#);
    let s = f.broker.session(f.plugin.clone());
    let d = s.authorize_in(&confirm(), Some(&key()));
    assert_eq!(d.denial(), Some(DenyReason::NotGranted));
    assert_eq!(
        DenyReason::NotGranted.call_error_code(),
        CallErrorCode::NotPermitted
    );

    let f = fixture(r#"[{"kind":"dialogs"}]"#);
    let s = f.broker.session(f.plugin.clone());
    let d = s.authorize_in(&confirm(), Some(&key()));
    assert!(
        matches!(&d, Decision::Allow(Allowed::DialogOpen { request }) if request.title() == "Pull now?"),
        "{d:?}"
    );
    // Revoked: the next call is denied.
    f.broker.revoke_dialogs(&f.plugin);
    assert_eq!(
        s.authorize_in(&confirm(), Some(&key())).denial(),
        Some(DenyReason::NotGranted)
    );
    // A plugin without grants is unknown.
    let other = f.broker.session(PluginId::new("acme.other").unwrap());
    assert_eq!(
        other.authorize_in(&confirm(), Some(&key())).denial(),
        Some(DenyReason::UnknownPlugin)
    );
    // The grant is strict: no members.
    assert!(ApprovedPermissions::from_json(r#"[{"kind":"dialogs","secret":true}]"#).is_err());
    for kind in ["ui", "customUi", "UI"] {
        assert!(
            matches!(
                ApprovedPermissions::from_json(&format!(r#"[{{"kind":"{kind}"}}]"#)),
                Err(GrantError::NeverGrantable(_))
            ),
            "{kind}"
        );
    }
}

#[test]
fn only_runs_the_user_started_open_dialogs_before_any_rate_limit() {
    let f = fixture(r#"[{"kind":"dialogs"}]"#);
    let s = f.broker.session(f.plugin.clone());
    for context in [
        Some(RunContext::workflow(RunCause::Schedule, None)),
        Some(RunContext::workflow(RunCause::Event, None)),
        Some(RunContext::workflow(RunCause::Startup, None)),
        Some(RunContext::workflow(RunCause::WorkSession, None)),
        Some(RunContext::workflow(RunCause::Other, None)),
        Some(RunContext::unknown()),
        None,
    ] {
        // Many more than the rate allows: none of them uses a token.
        for _ in 0..10 {
            let d = s.authorize_in(&confirm(), context.as_ref());
            assert_eq!(
                d.denial(),
                Some(DenyReason::DialogNotUserInitiated),
                "{context:?}"
            );
        }
    }
    assert_eq!(
        DenyReason::DialogNotUserInitiated.call_error_code(),
        CallErrorCode::NotPermitted
    );
    assert!(
        DenyReason::DialogNotUserInitiated
            .to_string()
            .contains("the user started")
    );
    // Inside a state call: refused before anything else.
    assert_eq!(
        s.authorize_state(&confirm()).denial(),
        Some(DenyReason::NotInStateCall)
    );
    // The user's own runs: all six tokens are still there.
    for context in [
        RunContext::direct(RunTrigger::Key),
        RunContext::direct(RunTrigger::Gesture),
        RunContext::direct(RunTrigger::App),
        RunContext::direct(RunTrigger::Test),
        RunContext::workflow(RunCause::User, None),
        RunContext::direct(RunTrigger::Key),
    ] {
        assert!(
            s.authorize_in(&confirm(), Some(&context)).is_allowed(),
            "{context:?}"
        );
    }
    let records = f.audit.records();
    assert!(
        records
            .iter()
            .all(|r| r.op == "dialog.open" && r.target.is_none())
    );
}

#[test]
fn six_dialogs_a_minute_per_plugin() {
    let f = fixture(r#"[{"kind":"dialogs"}]"#);
    let s = f.broker.session(f.plugin.clone());
    for _ in 0..6 {
        assert!(s.authorize_in(&confirm(), Some(&key())).is_allowed());
    }
    let d = s.authorize_in(&confirm(), Some(&key()));
    assert_eq!(d.denial(), Some(DenyReason::RateLimited));
    assert_eq!(
        DenyReason::RateLimited.call_error_code(),
        CallErrorCode::Throttled
    );
    // Another plugin has its own budget.
    let other = PluginId::new("acme.other").unwrap();
    f.broker
        .set_grants(
            &other,
            &ApprovedPermissions::from_json(r#"[{"kind":"dialogs"}]"#).unwrap(),
        )
        .unwrap();
    assert!(
        f.broker
            .session(other)
            .authorize_in(&confirm(), Some(&key()))
            .is_allowed()
    );
    // Refilled continuously: one more after ten seconds.
    f.clock.advance(Duration::from_secs(10));
    assert!(s.authorize_in(&confirm(), Some(&key())).is_allowed());
    assert_eq!(
        s.authorize_in(&confirm(), Some(&key())).denial(),
        Some(DenyReason::RateLimited)
    );
}

#[test]
fn a_request_that_breaks_a_bound_never_reaches_the_executor() {
    let f = fixture(r#"[{"kind":"dialogs"}]"#);
    let s = f.broker.session(f.plugin.clone());
    let long: CapabilityRequest = serde_json::from_str(&format!(
        r#"{{"op":"dialog.open","args":{{"type":"alert","title":"{}"}}}}"#,
        "t".repeat(81)
    ))
    .unwrap();
    let d = s.authorize_in(&long, Some(&key()));
    assert_eq!(d.denial(), Some(DenyReason::TooLarge));
    assert_eq!(
        DenyReason::TooLarge.call_error_code(),
        CallErrorCode::BadRequest
    );
}
