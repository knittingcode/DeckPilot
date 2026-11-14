//! The SDK's invocation path, natively, through `testing::MockHost`:
//! lifecycle, dispatch, result checks and capability errors.

use deckpilot_plugin_sdk::prelude::*;
use deckpilot_plugin_sdk::testing::MockHost;
use deckpilot_plugin_sdk::{CallErrorCode, CallResult, CapabilityRequest, ParamValue};
use serde_json::json;

fn failure(o: &ActionOutcome) -> (&str, &str) {
    match o {
        ActionOutcome::Failed { code, message, .. } => (code, message.as_deref().unwrap_or("")),
        other => panic!("expected failed, got {other:?}"),
    }
}

fn ok(_: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    Ok(Output::message("fine"))
}

#[test]
fn failing_init_fails_every_invocation_with_its_reason() {
    let p = Plugin::new("acme.broken", "1.0.0")
        .on_init(|| Err(PluginError::new("no-config", "the table is empty")))
        .action(Action::new("a", ok));
    let mut host = MockHost::allow_all();
    for _ in 0..2 {
        let o = host.invoke(&p, "a", json!({}));
        let (code, message) = failure(&o);
        assert_eq!(code, codes::PLUGIN_ERROR);
        assert!(
            message.contains("the table is empty (no-config)"),
            "{message}"
        );
    }
}

#[test]
fn invalid_definitions_never_run_handlers() {
    let p = Plugin::new("acme.broken", "1.0.0")
        .action(Action::new("a", ok))
        .action(Action::new("a", ok));
    let o = MockHost::allow_all().invoke(&p, "a", json!({}));
    let (code, message) = failure(&o);
    assert_eq!(code, codes::PLUGIN_ERROR);
    assert!(message.contains("declared twice"), "{message}");
}

#[test]
fn results_are_checked_before_they_leave_the_plugin() {
    let p = Plugin::new("acme.results", "1.0.0")
        .action(Action::new("bad-code", |_, _| {
            Err(PluginError::new("Bad", "x"))
        }))
        .action(Action::new("bad-key", |_, _| {
            Ok(Output::ok().with("not a key", 1))
        }))
        .action(Action::new("nan", |_, _| {
            Ok(Output::ok().with("x", f64::NAN))
        }))
        .action(Action::new("skip", |_, _| {
            Ok(Output::skipped("nothing to do"))
        }));
    let mut host = MockHost::allow_all();
    for a in ["bad-code", "bad-key", "nan"] {
        let o = host.invoke(&p, a, json!({}));
        assert_eq!(failure(&o).0, codes::INVALID_RESULT, "{a}");
    }
    assert_eq!(
        host.invoke(&p, "skip", json!({})),
        ActionOutcome::Skipped {
            message: Some("nothing to do".into())
        }
    );
}

#[test]
fn capability_errors_map_to_shared_codes() {
    fn fetch(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
        let r = ctx.http_get("https://api.example.com/x?token=s3cret")?;
        Ok(Output::ok().with("status", r.status))
    }
    let p = Plugin::new("acme.net", "1.0.0").action(Action::new("fetch", fetch));
    for (code, expected, retryable) in [
        (CallErrorCode::NotPermitted, codes::NOT_PERMITTED, false),
        (CallErrorCode::Throttled, codes::THROTTLED, true),
        (CallErrorCode::Failed, codes::REQUEST_FAILED, true),
        (CallErrorCode::Unavailable, codes::REQUEST_FAILED, true),
        (CallErrorCode::BadRequest, codes::REQUEST_FAILED, false),
        (CallErrorCode::UnsupportedOp, codes::UNSUPPORTED, false),
    ] {
        let mut host = MockHost::new(move |_| CallResult::error(code, "because"));
        let o = host.invoke(&p, "fetch", json!({}));
        let ActionOutcome::Failed {
            code: c,
            message,
            retryable: r,
        } = &o
        else {
            panic!("{o:?}")
        };
        assert_eq!((c.as_str(), *r), (expected, retryable), "{code:?}");
        let message = message.as_deref().unwrap();
        assert!(message.contains("https://api.example.com"), "{message}");
        assert!(!message.contains("s3cret"), "no path or query: {message}");
    }
    // A reply that is not an HTTP response.
    let mut host = MockHost::new(|_| CallResult::Ok(json!("nope")));
    let o = host.invoke(&p, "fetch", json!({}));
    assert_eq!(failure(&o).0, codes::REQUEST_FAILED);
    // A non-2xx status is a response, not a capability error.
    let mut host = MockHost::new(|req| match req {
        CapabilityRequest::HttpFetch(_) => CallResult::Ok(json!({"status": 418, "body": "teapot"})),
        _ => CallResult::Ok(serde_json::Value::Null),
    });
    let o = host.invoke(&p, "fetch", json!({}));
    let ActionOutcome::Ok { data, .. } = o else {
        panic!()
    };
    assert_eq!(data["status"], ParamValue::Integer(418));
}

#[test]
fn logging_is_best_effort_and_bounded() {
    fn chatty(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
        ctx.info("x".repeat(10_000));
        ctx.warn("short");
        Ok(Output::ok().with("calls", ctx.calls()))
    }
    let p = Plugin::new("acme.logs", "1.0.0").action(Action::new("chatty", chatty));
    // Denied logs do not fail the action.
    let mut host = MockHost::deny_all();
    let o = host.invoke(&p, "chatty", json!({}));
    assert!(matches!(o, ActionOutcome::Ok { .. }), "{o:?}");
    let CapabilityRequest::Log(first) = &host.requests()[0] else {
        panic!()
    };
    assert_eq!(first.message.len(), deckpilot_plugin_sdk::MAX_LOG_LEN);
}

#[test]
fn versions_are_exposed() {
    assert_eq!(deckpilot_plugin_sdk::ABI_VERSION, 1);
    assert_eq!(deckpilot_plugin_sdk::API_LEVEL.to_string(), "1.0");
    assert_eq!(deckpilot_plugin_sdk::SDK_VERSION, env!("CARGO_PKG_VERSION"));
}
