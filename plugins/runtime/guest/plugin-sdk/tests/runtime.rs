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
    assert_eq!(deckpilot_plugin_sdk::LATEST_API_LEVEL.to_string(), "1.1");
    assert_eq!(deckpilot_plugin_sdk::SDK_VERSION, env!("CARGO_PKG_VERSION"));
}

#[test]
fn manifest_mismatches_name_every_difference() {
    use deckpilot_plugin_sdk::testing::manifest_mismatches;
    let p = Plugin::new("acme.check", "1.0.0").action(
        Action::new("a", ok)
            .param(Param::text("notes"))
            .param(Param::integer("n").min(1.0).max(5.0).default(2))
            .param(Param::number("x")),
    );
    let manifest = json!({
        "id": "acme.check", "version": "1.0.1",
        "engines": {"pluginApi": "^1.0", "sdk": "9.9.9"},
        "contributes": {"actions": [{"id": "a", "parameters": [
            {"key": "notes", "required": false, "kind": {"type": "lines"}},
            {"key": "n", "required": false, "kind": {"type": "integer", "min": 1, "max": 6},
             "default": {"type": "integer", "value": 2}},
            {"key": "x", "required": false, "kind": {"type": "text"}}
        ]}, {"id": "b", "parameters": []}]}
    });
    let found = manifest_mismatches(&p, &manifest);
    let joined = found.join("\n");
    assert_eq!(found.len(), 5, "{joined}");
    for needle in [
        "version is \"1.0.1\" in the manifest, \"1.0.0\" in the plugin",
        "engines.sdk is \"9.9.9\"",
        "action ids is [\"a\",\"b\"]",
        "a.n: kind.max is 6 in the manifest, 5 in the plugin",
        "a.x: kind.type is \"text\" in the manifest, \"number (no manifest kind in version 1)\"",
    ] {
        assert!(joined.contains(needle), "{needle}\n{joined}");
    }
}

/// A plugin needs API 1.0 unless it says more; what it says is what
/// `dp_describe` reports and what its manifest must declare.
#[test]
fn a_plugin_declares_the_api_level_it_needs() {
    use deckpilot_plugin_sdk::testing::manifest_mismatches;
    use deckpilot_plugin_sdk::{API_LEVEL, ApiLevel, LATEST_API_LEVEL};
    let plugin = || Plugin::new("acme.check", "1.0.0");
    let manifest = |api: &str| {
        json!({"id": "acme.check", "version": "1.0.0", "engines": {"pluginApi": api},
               "contributes": {"actions": []}})
    };
    assert_eq!(plugin().api_level(), API_LEVEL);
    assert_eq!(plugin().descriptor().api_level, "1.0");
    assert!(manifest_mismatches(&plugin(), &manifest("^1.0")).is_empty());

    let timed = plugin().requires_api(ApiLevel { major: 1, minor: 1 });
    assert_eq!(timed.api_level(), LATEST_API_LEVEL);
    assert_eq!(timed.descriptor().api_level, "1.1");
    assert!(manifest_mismatches(&timed, &manifest("^1.1")).is_empty());
    // A manifest that still says 1.0 would install on an agent the plugin
    // cannot work with: named.
    let found = manifest_mismatches(&timed, &manifest("^1.0")).join("\n");
    assert!(
        found.contains("engines.pluginApi is \"^1.0\" in the manifest, \"^1.1\" in the plugin"),
        "{found}"
    );

    // Never below the base level or above what the SDK implements.
    let level = |major, minor| plugin().requires_api(ApiLevel { major, minor }).api_level();
    assert_eq!(level(0, 9), API_LEVEL);
    assert_eq!(level(1, 0), API_LEVEL);
    assert_eq!(level(1, 99), LATEST_API_LEVEL);
    assert_eq!(level(2, 0), LATEST_API_LEVEL);
}

/// `time.now` and `random.bytes` through the SDK: typed answers, bounds
/// checked before the host is asked, and an older agent's refusal as
/// `Unsupported`.
#[test]
fn the_clock_and_random_bytes_are_typed_and_bounded() {
    use deckpilot_plugin_sdk::CapabilityErrorKind;
    let mut host = MockHost::new(|req| match req {
        CapabilityRequest::TimeNow(_) => CallResult::Ok(json!({
            "unixMs": 1_794_871_799_999_u64, "utcOffsetMinutes": -480
        })),
        CapabilityRequest::RandomBytes(r) => {
            CallResult::Ok(json!({"bytes": b64(r.length as usize)}))
        }
        _ => CallResult::Ok(serde_json::Value::Null),
    });
    let mut ctx = host.context();
    // 2026-11-16 23:29:59.999 UTC is 15:29 on the same Monday eight hours
    // west.
    let now = ctx.now().unwrap();
    assert_eq!(now.unix_seconds(), 1_794_871_799);
    assert_eq!(now.local_minutes_of_day(), 15 * 60 + 29);
    assert_eq!(now.local_weekday(), 0);
    assert_eq!(now.millis_since(now.unix_ms - 250), 250);
    assert_eq!(now.millis_since(now.unix_ms + 1), 0);

    assert_eq!(ctx.random_bytes(3).unwrap(), [0xAB; 3]);
    assert_eq!(ctx.random_bytes(64).unwrap().len(), 64);
    assert_eq!(ctx.random_u64().unwrap(), 0xABAB_ABAB_ABAB_ABAB);
    let calls = ctx.calls();
    for length in [0, 65, usize::MAX] {
        let e = ctx.random_bytes(length).unwrap_err();
        assert_eq!(e.kind(), CapabilityErrorKind::BadRequest, "{length}");
    }
    assert_eq!(ctx.random_below(0).unwrap(), 0);
    assert_eq!(ctx.random_below(1).unwrap(), 0);
    assert_eq!(ctx.calls(), calls, "refused or answered without the host");
    assert!(ctx.random_below(6).unwrap() < 6);

    // A reply of another length, or not base64, is malformed, not bytes.
    for reply in [json!({"bytes": "AAAA"}), json!({"bytes": "?"}), json!(null)] {
        let mut wrong = MockHost::new(move |_| CallResult::Ok(reply.clone()));
        let e = wrong.context().random_bytes(16).unwrap_err();
        assert_eq!(e.kind(), CapabilityErrorKind::MalformedReply);
    }

    // An agent that predates plugin API 1.1.
    let mut old =
        MockHost::new(|_| CallResult::error(CallErrorCode::UnsupportedOp, "unsupported operation"));
    let mut ctx = old.context();
    assert_eq!(
        ctx.now().unwrap_err().kind(),
        CapabilityErrorKind::Unsupported
    );
    assert_eq!(
        ctx.random_u64().unwrap_err().kind(),
        CapabilityErrorKind::Unsupported
    );
}

/// `length` bytes of 0xAB as padded base64, without a base64 crate.
fn b64(length: usize) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = vec![0xAB_u8; length];
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
