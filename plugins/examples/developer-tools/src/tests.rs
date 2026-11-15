//! Native unit tests: the definition agrees with manifest.json, the
//! manifest is valid against the schema, and every action behaves, with
//! the agent replaced by `MockHost`.

use std::collections::BTreeMap;

use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};
use deckpilot_plugin_sdk::{
    ActionOutcome, AuthScheme, CallErrorCode, CallResult, CapabilityRequest, HttpMethod,
    ParamValue, Params, codes,
};
use serde_json::{Value, json};

use super::*;

const MANIFEST: &str = include_str!("../manifest.json");
const SCHEMA: &str = include_str!("../../../schemas/plugin-manifest.schema.json");

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).unwrap()
}

fn ok_data(o: &ActionOutcome) -> &Params {
    match o {
        ActionOutcome::Ok { data, .. } => data,
        other => panic!("expected ok, got {other:?}"),
    }
}

fn failure(o: &ActionOutcome) -> (&str, &str, bool) {
    match o {
        ActionOutcome::Failed {
            code,
            message,
            retryable,
        } => (code, message.as_deref().unwrap_or(""), *retryable),
        other => panic!("expected failed, got {other:?}"),
    }
}

fn text(s: &str) -> ParamValue {
    ParamValue::String(s.into())
}

/// Runs a pure action with every capability denied: it must not need any.
fn pure(action: &str, params: Value) -> ActionOutcome {
    let mut host = MockHost::deny_all();
    let out = host.invoke(&plugin(), action, params);
    assert!(host.requests().is_empty(), "{action} asked the host");
    out
}

#[test]
fn definition_is_valid_and_matches_the_manifest() {
    plugin().validate().unwrap();
    assert_eq!(
        manifest_mismatches(&plugin(), &manifest()),
        Vec::<String>::new()
    );
}

#[test]
fn manifest_is_valid_against_the_schema() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let m = manifest();
    let errors: Vec<String> = validator
        .iter_errors(&m)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn permissions_codes_and_slots_are_declared() {
    let m = manifest();
    let declared = m["contributes"]["errorCodes"].as_object().unwrap();
    for code in PLUGIN_CODES {
        assert!(declared.contains_key(*code), "{code} is not in errorCodes");
    }
    for code in declared.keys() {
        assert!(!codes::is_shared(code), "{code} redeclares a shared code");
    }
    let kinds: Vec<&str> = m["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["network", "credentials", "notifications"]);
    assert_eq!(m["permissions"][0]["hosts"], json!(["api.github.com"]));
    assert_eq!(m["permissions"][0]["methods"], json!(["GET"]));
    assert_eq!(
        format!(
            "https://{}",
            m["permissions"][0]["hosts"][0].as_str().unwrap()
        ),
        GITHUB_API
    );
    let slot = &m["contributes"]["credentials"][0];
    assert_eq!(slot["key"], TOKEN_SLOT);
    assert_eq!(slot["hosts"], json!(["api.github.com"]));
    assert_eq!(slot["schemes"], json!(["bearer"]));
    let vars: Vec<&str> = m["contributes"]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap())
        .collect();
    assert_eq!(vars, ["openPrs", "buildStatus"]);
}

#[test]
fn icon_is_a_128px_png() {
    let png = include_bytes!("../assets/icon-128.png");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((w, h), (128, 128));
}

// ------------------------------------------------------------ pure actions

#[test]
fn timestamps_convert_both_ways() {
    let cases = [
        ("0", "auto", "1970-01-01T00:00:00Z", 0, "seconds"),
        (
            "1731661200",
            "auto",
            "2024-11-15T09:00:00Z",
            1_731_661_200,
            "seconds",
        ),
        (
            "1731661200250",
            "auto",
            "2024-11-15T09:00:00.250Z",
            1_731_661_200,
            "milliseconds",
        ),
        (
            "2024-11-15T10:00:00+01:00",
            "auto",
            "2024-11-15T09:00:00Z",
            1_731_661_200,
            "rfc3339",
        ),
        (
            "2000-02-29 23:59:59.5z",
            "rfc3339",
            "2000-02-29T23:59:59.500Z",
            951_868_799,
            "rfc3339",
        ),
        ("-1", "seconds", "1969-12-31T23:59:59Z", -1, "seconds"),
        (
            "253402300799",
            "seconds",
            "9999-12-31T23:59:59Z",
            253_402_300_799,
            "seconds",
        ),
    ];
    for (value, from, iso, seconds, read_as) in cases {
        let out = pure("timestamp", json!({"value": value, "from": from}));
        let data = ok_data(&out);
        assert_eq!(data["iso"], text(iso), "{value}");
        assert_eq!(data["unixSeconds"], ParamValue::Integer(seconds), "{value}");
        assert_eq!(data["readAs"], text(read_as), "{value}");
    }
    for (value, from) in [
        ("2024-02-30T00:00:00Z", "auto"),
        ("2024-11-15T09:00:00", "auto"),
        ("yesterday", "auto"),
        ("1.5", "seconds"),
        ("253402300800", "seconds"),
        ("99999999999999999", "milliseconds"),
        ("", "auto"),
    ] {
        let out = pure("timestamp", json!({"value": value, "from": from}));
        assert_eq!(failure(&out).0, "invalid-input", "{value}");
    }
}

#[test]
fn civil_dates_round_trip() {
    for days in (-719_162..2_932_896).step_by(997) {
        let (y, m, d) = util::civil_from_days(days);
        assert_eq!(util::days_from_civil(y, m, d), days);
    }
}

#[test]
fn base64_encodes_and_decodes() {
    let enc = |t: &str, a: &str| {
        let out = pure("base64", json!({"text": t, "alphabet": a}));
        ok_data(&out)["result"].clone()
    };
    assert_eq!(enc("hello?>", "standard"), text("aGVsbG8/Pg=="));
    assert_eq!(enc("hello?>", "url-safe"), text("aGVsbG8_Pg=="));
    let dec = |t: &str, a: &str| {
        pure(
            "base64",
            json!({"text": t, "mode": "decode", "alphabet": a}),
        )
    };
    assert_eq!(
        ok_data(&dec("aGVsbG8/Pg==", "standard"))["result"],
        text("hello?>")
    );
    assert_eq!(
        ok_data(&dec("aGVsbG8_Pg", "url-safe"))["result"],
        text("hello?>")
    );
    assert_eq!(failure(&dec("@@@", "standard")).0, "invalid-input");
    // Valid base64 of bytes that are not UTF-8.
    let not_utf8 = dec("/w==", "standard");
    let (code, message, _) = failure(&not_utf8);
    assert_eq!(code, "invalid-input");
    assert!(message.contains("UTF-8"), "{message}");
}

#[test]
fn json_is_formatted_or_explained() {
    let out = pure(
        "json-format",
        json!({"json": "{\"b\":1, \"a\":[true,null]}"}),
    );
    assert_eq!(
        ok_data(&out)["result"],
        text("{\n  \"a\": [\n    true,\n    null\n  ],\n  \"b\": 1\n}")
    );
    let out = pure(
        "json-format",
        json!({"json": "{ \"a\" : 1 }", "style": "compact"}),
    );
    assert_eq!(ok_data(&out)["result"], text("{\"a\":1}"));
    let broken = pure("json-format", json!({"json": "{\"a\": }"}));
    let (code, message, _) = failure(&broken);
    assert_eq!(code, "invalid-input");
    assert!(message.contains("line 1, column"), "{message}");
    // Pretty-printing can outgrow what an output holds.
    // 100 nested lists: 200 characters in, about 20 KB of indentation out.
    let deep = format!("{}{}", "[".repeat(100), "]".repeat(100));
    let out = pure("json-format", json!({"json": deep}));
    assert_eq!(failure(&out).0, "output-too-large");
}

#[test]
fn uuid_v5_matches_the_rfc_examples() {
    // RFC 9562 appendix A.4 and Python's documented uuid5 example.
    let out = pure("uuid-v5", json!({"name": "www.example.com"}));
    assert_eq!(
        ok_data(&out)["uuid"],
        text("2ed6657d-e927-568b-95e1-2665a8aea6a2")
    );
    let out = pure("uuid-v5", json!({"name": "python.org", "namespace": "dns"}));
    assert_eq!(
        ok_data(&out)["uuid"],
        text("886313e1-3b8a-5372-9b90-0c9aee199e5d")
    );
    assert_eq!(
        util::sha1(b"abc")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}

#[test]
fn bad_parameters_never_reach_a_handler() {
    for (action, params) in [
        ("timestamp", json!({})),
        ("timestamp", json!({"value": "1", "from": "minutes"})),
        ("base64", json!({"text": "x", "mode": "rot13"})),
        ("uuid-v5", json!({"name": "x", "namespace": "nil"})),
        ("open-prs", json!({"repo": "x".repeat(141)})),
        ("last-values", json!({"extra": true})),
    ] {
        let out = pure(action, params);
        assert_eq!(failure(&out).0, codes::INVALID_PARAMETERS, "{action}");
    }
}

// ----------------------------------------------------------- GitHub checks

/// A scripted agent: settings, credential state, storage in memory and
/// canned GitHub answers by URL prefix. Records nothing itself (MockHost
/// records every request).
fn github_host(
    settings: Value,
    token_stored: bool,
    routes: Vec<(&'static str, u16, Value)>,
) -> MockHost {
    let mut store: BTreeMap<String, Value> = BTreeMap::new();
    MockHost::new(move |req| match req {
        CapabilityRequest::SettingsGet(_) => CallResult::Ok(settings.clone()),
        CapabilityRequest::CredentialHas(c) => {
            assert_eq!(c.credential, TOKEN_SLOT);
            CallResult::Ok(json!(token_stored))
        }
        CapabilityRequest::HttpFetch(f) => {
            match routes.iter().find(|(prefix, ..)| f.url.starts_with(prefix)) {
                Some((_, status, body)) => CallResult::Ok(json!({
                    "status": status,
                    "headers": [{"name": "date", "value": "Sun, 15 Nov 2026 09:00:00 GMT"}],
                    "body": body.to_string()
                })),
                None => CallResult::error(CallErrorCode::NotPermitted, "origin not granted"),
            }
        }
        CapabilityRequest::StorageGet(k) => {
            CallResult::Ok(store.get(&k.key).cloned().unwrap_or(Value::Null))
        }
        CapabilityRequest::StorageSet(s) => {
            store.insert(s.key.clone(), s.value.clone());
            CallResult::Ok(Value::Null)
        }
        CapabilityRequest::StorageUsage(_) => CallResult::Ok(json!({
            "usedBytes": 200, "keys": store.len(), "quotaBytes": 262_144, "maxKeys": 1000
        })),
        _ => CallResult::Ok(Value::Null),
    })
}

const SEARCH: &str =
    "https://api.github.com/search/issues?q=repo%3Aacme%2Fapp%20is%3Apr%20is%3Aopen&per_page=1";

fn fetches(host: &MockHost) -> Vec<&deckpilot_plugin_sdk::CapabilityRequest> {
    host.requests()
        .iter()
        .filter(|r| matches!(r, CapabilityRequest::HttpFetch(_)))
        .collect()
}

#[test]
fn open_prs_uses_the_default_repo_the_token_handle_storage_and_a_variable() {
    let mut host = github_host(
        json!({"defaultRepo": "acme/app", "defaultBranch": "main"}),
        true,
        vec![(SEARCH, 200, json!({"total_count": 4, "items": []}))],
    );
    let out = host.invoke(&plugin(), "open-prs", json!({}));
    let data = ok_data(&out);
    assert_eq!(data["count"], ParamValue::Integer(4));
    assert_eq!(data["repo"], text("acme/app"));
    assert_eq!(data["checkedAt"], text("Sun, 15 Nov 2026 09:00:00 GMT"));
    let CapabilityRequest::HttpFetch(f) = fetches(&host)[0] else {
        unreachable!()
    };
    assert_eq!(f.method, HttpMethod::Get);
    assert_eq!(f.url, SEARCH);
    let auth = f.auth.as_ref().expect("the token handle");
    assert_eq!(
        (auth.credential.as_str(), auth.scheme),
        (TOKEN_SLOT, AuthScheme::Bearer)
    );
    assert!(
        f.headers.iter().any(|h| h.name == "User-Agent"),
        "GitHub needs a User-Agent"
    );
    assert!(
        f.headers
            .iter()
            .all(|h| !h.name.eq_ignore_ascii_case("authorization")),
        "never a header with a secret"
    );
    assert!(host.requests().iter().any(|r| matches!(r,
        CapabilityRequest::VariablesPublish(v) if v.id == "openPrs" && v.value == json!(4))));
    assert!(host.requests().iter().any(|r| matches!(r,
        CapabilityRequest::StorageSet(s) if s.key == "last.openPrs" && s.value["count"] == 4)));

    // The last values come back from storage, without a request.
    let before = fetches(&host).len();
    let out = host.invoke(&plugin(), "last-values", json!({}));
    let data = ok_data(&out);
    assert_eq!(data["openPrs"], ParamValue::Integer(4));
    assert_eq!(data["openPrsRepo"], text("acme/app"));
    assert!(!data.contains_key("buildStatus"));
    assert_eq!(fetches(&host).len(), before);
}

#[test]
fn without_a_token_requests_carry_no_credential() {
    let mut host = github_host(
        json!({}),
        false,
        vec![(SEARCH, 200, json!({"total_count": 0}))],
    );
    let out = host.invoke(&plugin(), "open-prs", json!({"repo": "acme/app"}));
    assert_eq!(ok_data(&out)["count"], ParamValue::Integer(0));
    let CapabilityRequest::HttpFetch(f) = fetches(&host)[0] else {
        unreachable!()
    };
    assert_eq!(f.auth, None);
}

#[test]
fn repositories_are_required_and_checked() {
    let mut host = github_host(json!({}), false, vec![]);
    let out = host.invoke(&plugin(), "open-prs", json!({}));
    assert_eq!(failure(&out).0, "repo-required");
    for bad in ["acme", "acme/app/x", "ac me/app", "acme/..", "-/../x"] {
        let out = host.invoke(&plugin(), "open-prs", json!({"repo": bad}));
        assert_eq!(failure(&out).0, "invalid-input", "{bad}");
    }
    assert!(fetches(&host).is_empty());
}

#[test]
fn github_errors_are_typed() {
    for (status, code, retryable) in [
        (401, "github-unauthorized", false),
        (403, codes::THROTTLED, true),
        (404, "repo-not-found", false),
        (502, "github-error", true),
        (418, "github-error", false),
    ] {
        let mut host = github_host(
            json!({"defaultRepo": "acme/app"}),
            false,
            vec![(SEARCH, status, json!({"message": "no"}))],
        );
        let out = host.invoke(&plugin(), "open-prs", json!({}));
        let (got, _, retry) = failure(&out);
        assert_eq!((got, retry), (code, retryable), "HTTP {status}");
    }
    let mut host = github_host(
        json!({"defaultRepo": "acme/app"}),
        false,
        vec![(SEARCH, 200, json!({"unexpected": true}))],
    );
    let out = host.invoke(&plugin(), "open-prs", json!({}));
    assert_eq!(failure(&out).0, "invalid-response");
}

#[test]
fn a_denied_origin_is_not_permitted() {
    let mut host = MockHost::deny_all();
    let out = host.invoke(&plugin(), "open-prs", json!({"repo": "acme/app"}));
    let (code, message, _) = failure(&out);
    assert_eq!(code, codes::NOT_PERMITTED);
    assert!(message.contains("settings.get"), "{message}");
}

const RUNS: &str =
    "https://api.github.com/repos/acme/app/actions/runs?branch=release%2F1.0&per_page=1";

fn run(status: &str, conclusion: Option<&str>) -> Value {
    json!({"workflow_runs": [{
        "name": "CI", "status": status, "conclusion": conclusion,
        "html_url": "https://github.com/acme/app/actions/runs/1"
    }]})
}

#[test]
fn build_status_publishes_stores_and_notifies_on_change() {
    let settings = json!({"defaultRepo": "acme/app", "defaultBranch": "release/1.0",
        "notifyOnChange": true});
    let mut host = github_host(
        settings.clone(),
        false,
        vec![(RUNS, 200, run("completed", Some("success")))],
    );
    let out = host.invoke(&plugin(), "build-status", json!({}));
    let data = ok_data(&out);
    assert_eq!(data["status"], text("success"));
    assert_eq!(data["branch"], text("release/1.0"));
    assert_eq!(data["workflow"], text("CI"));
    assert_eq!(data["changed"], ParamValue::Bool(false));
    assert!(host.requests().iter().any(|r| matches!(r,
        CapabilityRequest::VariablesPublish(v) if v.id == "buildStatus" && v.value == "success")));
    assert!(
        !host
            .requests()
            .iter()
            .any(|r| matches!(r, CapabilityRequest::Notify(_)))
    );

    // The same storage, a failing run now: changed, and a notification.
    let stored = host
        .requests()
        .iter()
        .find_map(|r| match r {
            CapabilityRequest::StorageSet(s) if s.key == "last.buildStatus" => {
                Some(s.value.clone())
            }
            _ => None,
        })
        .unwrap();
    let mut host = github_host(
        settings,
        false,
        vec![(RUNS, 200, run("completed", Some("failure")))],
    );
    host.context()
        .storage_set("last.buildStatus", stored)
        .unwrap();
    let out = host.invoke(&plugin(), "build-status", json!({}));
    assert_eq!(ok_data(&out)["changed"], ParamValue::Bool(true));
    let notified = host.requests().iter().find_map(|r| match r {
        CapabilityRequest::Notify(n) => Some(n.clone()),
        _ => None,
    });
    let n = notified.expect("a notification");
    assert_eq!(n.title, "Build failure");
    assert!(n.body.contains("success -> failure"), "{}", n.body);
}

#[test]
fn build_status_reads_running_and_missing_runs() {
    let mut host = github_host(
        json!({"defaultRepo": "acme/app"}),
        false,
        vec![
            (
                "https://api.github.com/repos/acme/app/actions/runs?branch=main&",
                200,
                run("in_progress", None),
            ),
            (
                "https://api.github.com/repos/acme/app/actions/runs?branch=dev&",
                200,
                json!({"workflow_runs": []}),
            ),
        ],
    );
    let out = host.invoke(&plugin(), "build-status", json!({}));
    assert_eq!(ok_data(&out)["status"], text("in_progress"));
    let out = host.invoke(&plugin(), "build-status", json!({"branch": "dev"}));
    assert_eq!(ok_data(&out)["status"], text("none"));
    for bad in ["a b", "x..y", "-x", "a:b", "refs/heads/@{1}"] {
        let out = host.invoke(&plugin(), "build-status", json!({"branch": bad}));
        assert_eq!(failure(&out).0, "invalid-input", "{bad}");
    }
}

#[test]
fn last_values_without_checks_is_skipped() {
    let mut host = github_host(json!({}), false, vec![]);
    let out = host.invoke(&plugin(), "last-values", json!({}));
    assert!(matches!(out, ActionOutcome::Skipped { .. }), "{out:?}");
}
