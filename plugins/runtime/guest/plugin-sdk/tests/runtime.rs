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
    assert_eq!(deckpilot_plugin_sdk::LATEST_API_LEVEL.to_string(), "1.9");
    assert_eq!(
        deckpilot_plugin_sdk::PRIVILEGED_API_LEVEL.to_string(),
        "1.9"
    );
    assert_eq!(
        deckpilot_plugin_sdk::LOCALIZATION_API_LEVEL.to_string(),
        "1.6"
    );
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

/// An action's controls are compared with the manifest entry's as sets
/// per kind; absent on both sides is equal.
#[test]
fn manifest_mismatches_compare_controls() {
    use deckpilot_plugin_sdk::testing::manifest_mismatches;
    use deckpilot_plugin_sdk::{ApiLevel, Controls, Input};
    let plugin = |controls: Option<Controls>| {
        let action = Action::new("next", ok);
        Plugin::new("acme.pager", "1.0.0")
            .requires_api(ApiLevel { major: 1, minor: 3 })
            .action(match controls {
                Some(c) => action.controls(c),
                None => action,
            })
    };
    let manifest = |controls: Option<serde_json::Value>| {
        let mut action = json!({"id": "next", "parameters": []});
        if let Some(c) = controls {
            action["controls"] = c;
        }
        json!({"id": "acme.pager", "version": "1.0.0", "engines": {"pluginApi": "^1.3"},
               "contributes": {"actions": [action]}})
    };
    let dial = Controls::new()
        .key([Input::Press, Input::Click])
        .encoder([Input::RotateClockwise, Input::RotateCounterClockwise]);
    assert!(manifest_mismatches(&plugin(None), &manifest(None)).is_empty());
    // Another order in the manifest is the same declaration.
    let reordered = json!({"encoder": ["rotate-counter-clockwise", "rotate-clockwise"],
                           "key": ["click", "press"]});
    assert!(
        manifest_mismatches(&plugin(Some(dial.clone())), &manifest(Some(reordered))).is_empty()
    );

    let missing = manifest_mismatches(&plugin(Some(dial.clone())), &manifest(None)).join("\n");
    assert!(
        missing.contains("next: controls is null in the manifest"),
        "{missing}"
    );
    let extra =
        manifest_mismatches(&plugin(None), &manifest(Some(json!({"key": ["press"]})))).join("\n");
    assert!(
        extra.contains("next: controls is {\"key\":[\"press\"]} in the manifest, null"),
        "{extra}"
    );
    let other = manifest_mismatches(
        &plugin(Some(dial)),
        &manifest(Some(json!({"key": ["press"]}))),
    );
    assert_eq!(other.len(), 1, "{other:?}");
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
    assert_eq!(level(1, 3).to_string(), "1.3");
    assert_eq!(level(1, 5).to_string(), "1.5");
    assert_eq!(level(1, 6).to_string(), "1.6");
    assert_eq!(level(1, 9), LATEST_API_LEVEL);
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

/// Key state through the SDK: the handler's description, checked before
/// it leaves the plugin; actions without a handler and unknown actions
/// report none; a failure keeps its code.
#[test]
fn key_state_is_described_checked_and_optional() {
    use deckpilot_plugin_sdk::CapabilityErrorKind;
    fn shown(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<KeyState, PluginError> {
        let cell: i64 = args.get("cell")?;
        match cell {
            0 => Ok(KeyState::new()),
            1 => {
                let count = ctx.storage_get_as::<i64>("count")?.unwrap_or(0);
                Ok(KeyState::new()
                    .label(format!("{count}"))
                    .detail("clicks")
                    .indicator(Indicator::Ok)
                    .progress(40)
                    .toggle(true)
                    .accent(Accent::Purple)
                    .glyph(Glyph::Star)
                    .badge("new")
                    .series([1, 50, 100]))
            }
            2 => Ok(KeyState::new().label("x".repeat(33))),
            3 => Ok(KeyState::new().series(vec![0; 33])),
            4 => Err(PluginError::new("not-configured", "Choose a city")),
            // What the agent refuses inside a state call.
            _ => match ctx.storage_set("count", 1) {
                Err(e) if e.kind() == CapabilityErrorKind::NotPermitted => {
                    Ok(KeyState::new().label("read only"))
                }
                other => Err(PluginError::new("unexpected", format!("{other:?}"))),
            },
        }
    }
    let plugin = Plugin::new("acme.cells", "1.0.0")
        .action(
            Action::new("cell", ok)
                .param(Param::integer("cell").min(0.0).max(9.0).default(0))
                .state(shown),
        )
        .action(Action::new("plain", ok));
    assert!(plugin.find("cell").unwrap().has_state());
    assert!(!plugin.find("plain").unwrap().has_state());
    // A state handler changes neither the descriptor nor the level a
    // plugin needs: on an older agent its keys are just static.
    assert_eq!(plugin.api_level().to_string(), "1.0");
    let descriptor = serde_json::to_string(&plugin.descriptor()).unwrap();
    assert!(!descriptor.contains("state"), "{descriptor}");

    let mut host = MockHost::new(|req| match req {
        CapabilityRequest::StorageGet(_) => CallResult::Ok(json!(7)),
        CapabilityRequest::StorageSet(_) => CallResult::error(
            CallErrorCode::NotPermitted,
            "not available while a key's state is read",
        ),
        _ => CallResult::Ok(serde_json::Value::Null),
    });
    let mut state =
        |action: &str, cell: i64| host.key_state(&plugin, action, json!({"cell": cell}));
    let failed = |outcome: StateOutcome| match outcome {
        StateOutcome::Error(failure) => (failure.code, failure.message),
        StateOutcome::Ok(state) => panic!("{state:?}"),
    };
    assert_eq!(state("cell", 0), StateOutcome::Ok(KeyState::new()));
    let StateOutcome::Ok(full) = state("cell", 1) else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(&full).unwrap(),
        json!({"label": "7", "detail": "clicks", "indicator": "ok", "progress": 40,
               "toggle": true, "accent": "purple", "glyph": "star", "badge": "new",
               "series": [1, 50, 100]})
    );
    assert_eq!(
        failed(state("cell", 2)),
        ("invalid-state".into(), "label is out of bounds".into())
    );
    assert_eq!(failed(state("cell", 3)).1, "series is out of bounds");
    assert_eq!(
        failed(state("cell", 4)),
        ("not-configured".into(), "Choose a city".into())
    );
    assert_eq!(
        state("cell", 5),
        StateOutcome::Ok(KeyState::new().label("read only"))
    );
    // Parameters are checked like an action's.
    assert_eq!(failed(state("cell", 99)).0, "invalid-parameters");
    assert_eq!(failed(state("plain", 0)).0, "unsupported");
    assert_eq!(failed(state("missing", 0)).0, "unsupported");
}

/// `locale.get` and `locale.translate` (plugin API 1.6) through the SDK:
/// the locale tag, the plugin's own keys with parameters and plurals, the
/// request checked before the host is asked, and an older agent's refusal.
#[test]
fn the_locale_and_translations_reach_the_host_as_plain_requests() {
    use deckpilot_plugin_sdk::testing::MockHost;
    use deckpilot_plugin_sdk::{CapabilityErrorKind, CapabilityRequest, TranslateArg};

    let mut host = MockHost::localized(
        "pl-PL",
        &[
            ("greeting", "Cześć, {{name}}!"),
            ("prs_one", "{{count}} PR"),
            ("prs_few", "{{count}} PR-y"),
            ("prs_many", "{{count}} PR-ów"),
        ],
    );
    let mut ctx = host.context();
    assert_eq!(ctx.locale().unwrap(), "pl-PL");
    assert_eq!(
        ctx.t("greeting", &[("name", "Ala")]).unwrap(),
        "Cześć, Ala!"
    );
    assert_eq!(ctx.t("prs", &[("count", 1)]).unwrap(), "1 PR");
    assert_eq!(ctx.t("prs", &[("count", 22)]).unwrap(), "22 PR-y");
    assert_eq!(ctx.t("prs", &[("count", 12)]).unwrap(), "12 PR-ów");
    assert_eq!(
        ctx.t(
            "prs",
            &[
                ("count", TranslateArg::from(5)),
                ("name", TranslateArg::from("x"))
            ]
        )
        .unwrap(),
        "5 PR-ów"
    );
    assert_eq!(ctx.text("settings.apiToken").unwrap(), "Api token");
    // Refused locally: no host call.
    let calls = ctx.calls();
    for (key, params) in [
        ("a b", vec![]),
        ("../core.key", vec![]),
        ("ok", vec![("not-a-name", TranslateArg::from(1))]),
        ("ok", vec![("x", TranslateArg::from("y".repeat(257)))]),
    ] {
        let error = ctx.t(key, &params).unwrap_err();
        assert_eq!(error.kind(), CapabilityErrorKind::BadRequest, "{key}");
    }
    assert_eq!(ctx.calls(), calls);
    // The requests name a key only: no plugin, no namespace.
    let translate = host
        .requests()
        .iter()
        .find_map(|r| match r {
            CapabilityRequest::LocaleTranslate(t) if t.key == "greeting" => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        serde_json::to_value(translate).unwrap(),
        json!({"key": "greeting", "params": {"name": "Ala"}})
    );

    // An agent before 1.6 does not know the operations.
    let mut old = MockHost::new(|_| {
        deckpilot_plugin_sdk::CallResult::error(
            deckpilot_plugin_sdk::CallErrorCode::UnsupportedOp,
            "unknown op",
        )
    });
    let mut ctx = old.context();
    assert_eq!(
        ctx.locale().unwrap_err().kind(),
        CapabilityErrorKind::Unsupported
    );
    assert_eq!(
        ctx.text("greeting").unwrap_err().kind(),
        CapabilityErrorKind::Unsupported
    );
}
