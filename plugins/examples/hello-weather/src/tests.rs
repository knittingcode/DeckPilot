//! Native unit tests: the definition agrees with manifest.json, the
//! manifest is valid against the #191 schema, and both actions behave,
//! with the agent replaced by `MockHost`.

use deckpilot_plugin_sdk::testing::MockHost;
use deckpilot_plugin_sdk::{
    ActionOutcome, CallResult, CapabilityRequest, HttpMethod, ParamKind, ParamValue, Params, codes,
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

#[test]
fn definition_is_valid() {
    plugin().validate().unwrap();
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
    // The validator does catch mistakes.
    let mut bad = m.clone();
    bad["permissions"][0]["hosts"] = json!(["*"]);
    assert!(!validator.is_valid(&bad));
}

#[test]
fn definition_matches_the_manifest() {
    let m = manifest();
    let p = plugin();
    assert_eq!(m["id"], p.id());
    assert_eq!(m["version"], p.version());
    assert_eq!(
        m["engines"]["pluginApi"],
        format!("^{}", deckpilot_plugin_sdk::API_LEVEL)
    );
    assert_eq!(m["engines"]["sdk"], deckpilot_plugin_sdk::SDK_VERSION);
    let actions = m["contributes"]["actions"].as_array().unwrap();
    assert_eq!(actions.len(), p.actions().len());
    for (ma, a) in actions.iter().zip(p.actions()) {
        assert_eq!(ma["id"], a.id());
        let params = ma["parameters"].as_array().unwrap();
        assert_eq!(params.len(), a.params().len(), "{}", a.id());
        for (mp, sp) in params.iter().zip(a.params()) {
            let at = format!("{}.{}", a.id(), sp.key());
            assert_eq!(mp["key"], sp.key(), "{at}");
            assert_eq!(mp["required"], sp.is_required(), "{at}");
            let kind = &mp["kind"];
            match sp.kind() {
                ParamKind::Text { max_length } => {
                    assert_eq!(kind["type"], "text", "{at}");
                    assert_eq!(
                        kind["maxLength"].as_u64(),
                        max_length.map(u64::from),
                        "{at}"
                    );
                }
                ParamKind::Choice { options } => {
                    assert_eq!(kind["type"], "choice", "{at}");
                    assert_eq!(kind["options"], json!(options), "{at}");
                }
                ParamKind::Integer { min, max } => {
                    assert_eq!(kind["type"], "integer", "{at}");
                    assert_eq!(kind["min"].as_i64(), *min, "{at}");
                    assert_eq!(kind["max"].as_i64(), *max, "{at}");
                }
                ParamKind::Boolean {} => assert_eq!(kind["type"], "boolean", "{at}"),
                other => panic!("{at}: {other:?} has no manifest kind"),
            }
            let default = sp.default_value().map(|v| serde_json::to_value(v).unwrap());
            assert_eq!(mp["default"].get("value").cloned(), default, "{at}");
        }
    }
}

#[test]
fn error_codes_and_permissions_are_declared() {
    let m = manifest();
    let declared = m["contributes"]["errorCodes"].as_object().unwrap();
    for code in PLUGIN_CODES {
        assert!(declared.contains_key(*code), "{code} is not in errorCodes");
    }
    for code in declared.keys() {
        assert!(!codes::is_shared(code), "{code} redeclares a shared code");
    }
    let net = &m["permissions"][0];
    assert_eq!(net["kind"], "network");
    assert_eq!(net["methods"], json!(["GET"]));
    assert_eq!(
        format!("https://{}", net["hosts"][0].as_str().unwrap()),
        WEATHER_ORIGIN
    );
}

#[test]
fn icon_is_a_128px_png() {
    let png = include_bytes!("../assets/icon-128.png");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((w, h), (128, 128));
    assert!(png.len() <= 256 << 10);
}

#[test]
fn greet_is_pure() {
    let p = plugin();
    // A host that denies everything: greet never asks for anything.
    let mut host = MockHost::deny_all();
    let o = host.invoke(&p, "greet", json!({"name": "  Ada "}));
    assert_eq!(ok_data(&o)["greeting"], text("Hello, Ada."));
    let o = host.invoke(&p, "greet", json!({"name": "Ada", "style": "excited"}));
    assert_eq!(ok_data(&o)["greeting"], text("Hello, Ada!"));
    assert!(host.requests().is_empty());
}

#[test]
fn malformed_params_are_typed_errors() {
    let p = plugin();
    let mut host = MockHost::deny_all();
    for (action, params, needle) in [
        ("greet", json!({}), "`name` is required"),
        ("greet", json!({"name": 42}), "must be text"),
        ("greet", json!({"name": "x".repeat(65)}), "longer than 64"),
        (
            "greet",
            json!({"name": "Ada", "style": "shouty"}),
            "one of: plain, excited",
        ),
        ("greet", json!({"name": "   "}), "must not be blank"),
        (
            "greet",
            json!({"name": "Ada", "colour": "red"}),
            "no parameter `colour`",
        ),
        (
            "current-temperature",
            json!({"city": "paris"}),
            "one of: berlin",
        ),
        ("current-temperature", json!({}), "`city` is required"),
    ] {
        let o = host.invoke(&p, action, params);
        let (code, message, retryable) = failure(&o);
        assert_eq!(code, codes::INVALID_PARAMETERS, "{action}: {message}");
        assert!(message.contains(needle), "{action}: {message}");
        assert!(!retryable);
    }
    let o = host.invoke(&p, "forecast", json!({}));
    assert_eq!(failure(&o).0, codes::UNSUPPORTED);
    assert!(host.requests().is_empty(), "nothing reached the host");
}

fn weather_host(status: u16, body: &'static str) -> MockHost {
    MockHost::new(move |req| match req {
        CapabilityRequest::HttpFetch(_) => CallResult::Ok(json!({"status": status, "body": body})),
        _ => CallResult::Ok(Value::Null),
    })
}

const FORECAST: &str = r#"{"current":{"time":"2026-11-14T12:00","temperature_2m":7.25},
    "current_units":{"temperature_2m":"°C"}}"#;

#[test]
fn temperature_is_fetched_from_the_declared_origin() {
    let p = plugin();
    let mut host = weather_host(200, FORECAST);
    let o = host.invoke(&p, "current-temperature", json!({"city": "berlin"}));
    let data = ok_data(&o);
    assert_eq!(data["temperature"], text("7.2"));
    assert_eq!(data["unit"], text("°C"));
    assert_eq!(data["city"], text("Berlin"));
    assert_eq!(data["observedAt"], text("2026-11-14T12:00"));
    let fetches: Vec<_> = host
        .requests()
        .iter()
        .filter_map(|r| match r {
            CapabilityRequest::HttpFetch(f) => Some(f),
            _ => None,
        })
        .collect();
    assert_eq!(fetches.len(), 1);
    let f = fetches[0];
    assert_eq!(f.method, HttpMethod::Get);
    assert!(
        f.url.starts_with(&format!("{WEATHER_ORIGIN}/v1/forecast?")),
        "{}",
        f.url
    );
    assert!(
        f.url.contains("latitude=52.52&longitude=13.41"),
        "{}",
        f.url
    );
    assert!(f.url.contains("temperature_unit=celsius"), "{}", f.url);
    assert_eq!(f.auth, None);
}

#[test]
fn denial_is_a_clear_typed_error() {
    let p = plugin();
    let mut host = MockHost::deny_all();
    let o = host.invoke(&p, "current-temperature", json!({"city": "tokyo"}));
    let (code, message, retryable) = failure(&o);
    assert_eq!(code, codes::NOT_PERMITTED);
    assert!(!retryable);
    assert!(
        message.contains("http.fetch to https://api.open-meteo.com was denied"),
        "{message}"
    );
    assert!(message.contains("`network` permission"), "{message}");
}

#[test]
fn service_errors_are_typed() {
    let p = plugin();
    for (status, body, code, retryable) in [
        (500, "", WEATHER_SERVICE_ERROR, true),
        (404, "", WEATHER_SERVICE_ERROR, false),
        (429, "", codes::THROTTLED, true),
        (200, "<html>", INVALID_RESPONSE, false),
        (200, r#"{"current":{}}"#, INVALID_RESPONSE, false),
    ] {
        let mut host = weather_host(status, body);
        let o = host.invoke(&p, "current-temperature", json!({"city": "london"}));
        let (c, m, r) = failure(&o);
        assert_eq!((c, r), (code, retryable), "{status} {body}: {m}");
    }
}
