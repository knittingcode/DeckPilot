//! `time.now` and `random.bytes` (plugin API 1.1): allowed for every
//! plugin that has grants, without a permission of their own; never for a
//! plugin without grants; rate limited (the random source by bytes); and
//! audited by operation only.

use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::{NoArgs, RandomBytes};
use deckpilot_plugin_broker::*;

struct Fixture {
    broker: Broker,
    audit: Arc<MemoryAudit>,
    clock: ManualClock,
    plugin: PluginId,
}

fn fixture(limits: RateLimits) -> Fixture {
    let audit = Arc::new(MemoryAudit::new(10_000));
    let clock = ManualClock::default();
    let broker = Broker::new(audit.clone(), Arc::new(clock.clone()), limits);
    let plugin = PluginId::new("acme.timer").unwrap();
    broker
        .set_grants(&plugin, &ApprovedPermissions::from_json("[]").unwrap())
        .unwrap();
    Fixture {
        broker,
        audit,
        clock,
        plugin,
    }
}

fn now() -> CapabilityRequest {
    CapabilityRequest::TimeNow(NoArgs {})
}

fn random(length: u32) -> CapabilityRequest {
    CapabilityRequest::RandomBytes(RandomBytes { length })
}

#[test]
fn allowed_without_any_permission_while_the_plugin_has_grants() {
    let f = fixture(RateLimits::default());
    let s = f.broker.session(f.plugin.clone());
    assert_eq!(s.authorize(&now()), Decision::Allow(Allowed::TimeNow));
    assert_eq!(
        s.authorize(&random(16)),
        Decision::Allow(Allowed::RandomBytes { length: 16 })
    );
    // Whatever started the run: a schedule may read the clock too.
    assert!(s.authorize_in(&now(), None).is_allowed());

    // A plugin nobody enabled, and one that was removed, get neither.
    let stranger = f.broker.session(PluginId::new("acme.other").unwrap());
    for request in [now(), random(1)] {
        assert_eq!(
            stranger.authorize(&request).denial(),
            Some(DenyReason::UnknownPlugin)
        );
    }
    f.broker.remove_plugin(&f.plugin);
    for request in [now(), random(1)] {
        assert_eq!(
            s.authorize(&request).denial(),
            Some(DenyReason::UnknownPlugin)
        );
    }
}

#[test]
fn a_length_outside_1_to_64_is_refused_and_costs_no_bytes() {
    let f = fixture(RateLimits {
        random_bytes: Rate::new(64, Duration::from_secs(60)),
        ..RateLimits::default()
    });
    let s = f.broker.session(f.plugin.clone());
    for length in [0, 65, 4096, u32::MAX] {
        assert_eq!(
            s.authorize(&random(length)).denial(),
            Some(DenyReason::TooLarge),
            "{length}"
        );
    }
    // The whole budget is still there.
    assert!(s.authorize(&random(64)).is_allowed());
    assert_eq!(
        s.authorize(&random(1)).denial(),
        Some(DenyReason::RateLimited)
    );
}

#[test]
fn random_bytes_are_budgeted_by_bytes_per_plugin() {
    let f = fixture(RateLimits::default());
    let s = f.broker.session(f.plugin.clone());
    // 4,096 bytes: 64 calls of 64.
    for i in 0..64 {
        assert!(s.authorize(&random(64)).is_allowed(), "call {i}");
    }
    assert_eq!(
        s.authorize(&random(1)).denial(),
        Some(DenyReason::RateLimited)
    );
    // A refused call takes nothing: after a second there are 68 bytes,
    // enough for 64 and then for 4 more, not for 5.
    f.clock.advance(Duration::from_secs(1));
    assert!(s.authorize(&random(64)).is_allowed());
    assert_eq!(
        s.authorize(&random(5)).denial(),
        Some(DenyReason::RateLimited)
    );
    assert!(s.authorize(&random(4)).is_allowed());

    // Another plugin has its own budget.
    let other = PluginId::new("acme.other").unwrap();
    f.broker
        .set_grants(&other, &ApprovedPermissions::from_json("[]").unwrap())
        .unwrap();
    assert!(f.broker.session(other).authorize(&random(64)).is_allowed());

    // Never more than the burst, however long the plugin was idle.
    f.clock.advance(Duration::from_secs(24 * 3600));
    for i in 0..64 {
        assert!(s.authorize(&random(64)).is_allowed(), "call {i}");
    }
    assert!(!s.authorize(&random(1)).is_allowed());
}

#[test]
fn the_clock_is_rate_limited() {
    let f = fixture(RateLimits::default());
    let s = f.broker.session(f.plugin.clone());
    for i in 0..20 {
        assert!(s.authorize(&now()).is_allowed(), "call {i}");
    }
    assert_eq!(s.authorize(&now()).denial(), Some(DenyReason::RateLimited));
    // It does not use up the random budget, nor the other way round.
    assert!(s.authorize(&random(64)).is_allowed());
    f.clock.advance(Duration::from_millis(500));
    for i in 0..10 {
        assert!(s.authorize(&now()).is_allowed(), "call {i}");
    }
    assert!(!s.authorize(&now()).is_allowed());
}

#[test]
fn audited_by_operation_without_values() {
    let f = fixture(RateLimits::default());
    let s = f.broker.session(f.plugin.clone());
    s.authorize(&now());
    s.authorize(&random(33));
    s.authorize(&random(65));
    let records = f.audit.records();
    let seen: Vec<_> = records
        .iter()
        .map(|r| (r.op, r.target.clone(), r.decision))
        .collect();
    assert_eq!(
        seen,
        [
            ("time.now", None, AuditDecision::Allowed),
            ("random.bytes", None, AuditDecision::Allowed),
            (
                "random.bytes",
                None,
                AuditDecision::Denied(DenyReason::TooLarge)
            ),
        ]
    );
    // Nothing in a record could hold a length, a byte or a time of day.
    let text = format!("{records:?}");
    assert!(!text.contains("33") && !text.contains("65"), "{text}");
}
