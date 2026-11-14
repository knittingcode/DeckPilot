//! End to end: plugins built with deckpilot-plugin-sdk (the sample
//! `hello-weather` and the SDK test guest), compiled to wasm by
//! test-fixtures' build script, run in the real host binary under its OS
//! sandbox, driven by the real supervisor and broker. The network executor
//! is a stub: the tests check the broker's decisions and what the agent
//! would execute, never the network.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use deckpilot_plugin_abi::capability::AuthScheme;
use deckpilot_plugin_abi::describe::ParamKind;
use deckpilot_plugin_abi::protocol::{FatalReason, exit_code};
use deckpilot_plugin_abi::{ActionOutcome, CallResult, HttpMethod, ParamValue, Params};
use deckpilot_plugin_broker::{
    Allowed, ApprovedPermissions, AuditDecision, Broker, DenyReason, ManualClock, MemoryAudit,
    Origin, PluginId, RateLimits,
};
use deckpilot_plugin_supervisor::{
    CapabilityExecutor, HostCommand, HostConfig, HostError, PluginHost, RestartPolicy, Supervisor,
    SupervisorError, SupervisorState,
};
use deckpilot_plugin_test_fixtures::{
    HELLO_WEATHER, HELLO_WEATHER_MANIFEST, SDK_TEST_GUEST, TEST_GUEST,
};
use serde_json::{Value, json};

const HOST: &str = env!("CARGO_BIN_EXE_deckpilot-plugin-host");
const T: Duration = Duration::from_secs(20);
const OPEN_METEO: &str = "https://api.open-meteo.com";
const FORECAST: &str = r#"{"current":{"time":"2026-11-14T12:00","temperature_2m":-3.04},"current_units":{"temperature_2m":"°C"}}"#;

/// Stands in for the agent's executors: records what the broker allowed
/// and answers without any I/O.
#[derive(Default)]
struct StubExecutor {
    executed: Vec<Allowed>,
    delay: Duration,
}

impl CapabilityExecutor for StubExecutor {
    fn execute(&mut self, _plugin: &PluginId, allowed: &Allowed) -> CallResult {
        self.executed.push(allowed.clone());
        match allowed {
            Allowed::Fetch(f) if f.origin.host() == "api.open-meteo.com" => {
                CallResult::Ok(json!({"status": 200, "body": FORECAST}))
            }
            Allowed::Fetch(_) => {
                std::thread::sleep(self.delay);
                CallResult::Ok(json!({"status": 200}))
            }
            _ => CallResult::Ok(Value::Null),
        }
    }
}

impl StubExecutor {
    fn fetches(&self) -> Vec<&deckpilot_plugin_broker::AllowedFetch> {
        self.executed
            .iter()
            .filter_map(|a| match a {
                Allowed::Fetch(f) => Some(f),
                _ => None,
            })
            .collect()
    }
}

fn manifest() -> Value {
    serde_json::from_str(HELLO_WEATHER_MANIFEST).unwrap()
}

fn plugin_id() -> PluginId {
    PluginId::new(manifest()["id"].as_str().unwrap()).unwrap()
}

/// The consent the user would give at install: the manifest's network
/// permission, as approved permissions (one exact https origin per host).
fn consent_from_manifest() -> String {
    let m = manifest();
    let approved: Vec<Value> = m["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"] == "network")
        .flat_map(|p| {
            let methods = p["methods"].clone();
            p["hosts"].as_array().unwrap().iter().map(move |h| {
                json!({
                    "kind": "network",
                    "origin": format!("https://{}", h.as_str().unwrap()),
                    "methods": methods,
                })
            })
        })
        .collect();
    serde_json::to_string(&approved).unwrap()
}

fn manifest_action_ids() -> BTreeSet<String> {
    manifest()["contributes"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .collect()
}

struct Fixture {
    broker: Broker,
    audit: Arc<MemoryAudit>,
    clock: ManualClock,
}

fn fixture(perms: &str) -> Fixture {
    let audit = Arc::new(MemoryAudit::new(10_000));
    let clock = ManualClock::default();
    let broker = Broker::new(
        audit.clone(),
        Arc::new(clock.clone()),
        RateLimits::default(),
    );
    broker
        .set_grants(
            &plugin_id(),
            &ApprovedPermissions::from_json(perms).unwrap(),
        )
        .unwrap();
    Fixture {
        broker,
        audit,
        clock,
    }
}

fn config(module: &[u8]) -> HostConfig {
    HostConfig::new(HostCommand::new(HOST), Arc::from(module))
}

fn weather_config() -> HostConfig {
    config(HELLO_WEATHER).expect_actions(manifest_action_ids())
}

fn spawn(cfg: &HostConfig, f: &Fixture) -> PluginHost {
    PluginHost::spawn(cfg, f.broker.session(plugin_id())).unwrap()
}

fn params(v: Value) -> Params {
    serde_json::from_value(v).unwrap()
}

fn data(o: &ActionOutcome) -> &Params {
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
fn sample_loads_sandboxed_and_its_descriptor_matches_the_manifest() {
    let f = fixture("[]");
    let h = spawn(&weather_config(), &f);
    assert_ne!(
        h.sandbox().enforcement,
        deckpilot_plugin_abi::protocol::Enforcement::None
    );
    let d = h.descriptor().expect("the SDK exports dp_describe").clone();
    assert_eq!(d.api_level, "1.0");
    assert!(d.sdk.starts_with("deckpilot-plugin-sdk/"), "{}", d.sdk);
    let m = manifest();
    for (ma, da) in m["contributes"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&d.actions)
    {
        assert_eq!(ma["id"], da.id.as_str());
        for (mp, dp) in ma["parameters"].as_array().unwrap().iter().zip(&da.params) {
            assert_eq!(mp["key"], dp.key.as_str());
            assert_eq!(mp["required"], dp.required);
            let ty = match &dp.kind {
                ParamKind::Text { .. } => "text",
                ParamKind::Choice { options } => {
                    assert_eq!(mp["kind"]["options"], json!(options));
                    "choice"
                }
                other => panic!("{other:?}"),
            };
            assert_eq!(mp["kind"]["type"], ty);
        }
    }
    assert_eq!(h.shutdown(T).and_then(|s| s.code()), Some(exit_code::CLEAN));
}

#[test]
fn modules_that_do_not_implement_the_manifest_are_refused() {
    let f = fixture("[]");
    let wrong = config(HELLO_WEATHER).expect_actions(["greet", "forecast"]);
    match PluginHost::spawn(&wrong, f.broker.session(plugin_id())) {
        Err(HostError::ActionMismatch { expected, actual }) => {
            assert_eq!(expected, vec!["forecast".to_owned(), "greet".to_owned()]);
            assert_eq!(
                actual,
                Some(vec!["greet".to_owned(), "current-temperature".to_owned()])
            );
        }
        other => panic!("{other:?}"),
    }
    // A raw ABI 1 module has no descriptor: refused when ids are expected,
    // accepted (unchanged behaviour) when they are not.
    let raw = config(TEST_GUEST).expect_actions(["echo"]);
    assert!(matches!(
        PluginHost::spawn(&raw, f.broker.session(plugin_id())),
        Err(HostError::ActionMismatch { actual: None, .. })
    ));
    let h = spawn(&config(TEST_GUEST), &f);
    assert!(h.descriptor().is_none());
}

#[test]
fn pure_action_needs_no_capability() {
    let f = fixture("[]");
    let mut h = spawn(&weather_config(), &f);
    let mut ex = StubExecutor::default();
    let o = h
        .invoke(
            "greet",
            &params(json!({"name": "Ada", "style": "excited"})),
            T,
            &mut ex,
        )
        .unwrap();
    assert_eq!(data(&o)["greeting"], text("Hello, Ada!"));
    let o = h
        .invoke("greet", &params(json!({"name": "Grace"})), T, &mut ex)
        .unwrap();
    assert_eq!(data(&o)["greeting"], text("Hello, Grace."));
    assert!(ex.executed.is_empty());
    assert!(f.audit.records().is_empty(), "no capability call at all");
}

#[test]
fn network_action_is_denied_without_the_grant() {
    // Installed, but the network permission was not approved.
    let f = fixture("[]");
    let mut h = spawn(&weather_config(), &f);
    let mut ex = StubExecutor::default();
    let o = h
        .invoke(
            "current-temperature",
            &params(json!({"city": "berlin"})),
            T,
            &mut ex,
        )
        .unwrap();
    let (code, message, retryable) = failure(&o);
    assert_eq!(code, "not-permitted");
    assert!(!retryable);
    assert!(
        message.contains("http.fetch to https://api.open-meteo.com was denied"),
        "{message}"
    );
    assert!(message.contains("origin not granted"), "{message}");
    assert!(ex.fetches().is_empty(), "nothing was executed");
    let denied: Vec<_> = f
        .audit
        .records()
        .into_iter()
        .filter(|r| r.op == "http.fetch")
        .collect();
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].target.as_deref(), Some(OPEN_METEO));
    assert_eq!(
        denied[0].decision,
        AuditDecision::Denied(DenyReason::OriginNotGranted)
    );
    // The host survives a denial.
    assert!(h.is_running());
}

#[test]
fn network_action_is_allowed_with_the_grant_and_revocation_is_immediate() {
    let f = fixture(&consent_from_manifest());
    let mut h = spawn(&weather_config(), &f);
    let pid = h.pid();
    let mut ex = StubExecutor::default();
    let o = h
        .invoke(
            "current-temperature",
            &params(json!({"city": "tokyo", "units": "fahrenheit"})),
            T,
            &mut ex,
        )
        .unwrap();
    let d = data(&o);
    assert_eq!(d["temperature"], text("-3.0"));
    assert_eq!(d["unit"], text("°C"));
    assert_eq!(d["city"], text("Tokyo"));
    let fetches = ex.fetches();
    assert_eq!(fetches.len(), 1);
    let fetch = fetches[0];
    assert_eq!(fetch.origin.to_string(), OPEN_METEO);
    assert_eq!(fetch.method, HttpMethod::Get);
    assert_eq!(fetch.url.path(), "/v1/forecast");
    assert!(
        fetch
            .url
            .query()
            .unwrap()
            .contains("temperature_unit=fahrenheit"),
        "{}",
        fetch.url
    );
    assert_eq!(fetch.auth, None);

    // The user revokes the permission: the next call is denied, in the
    // same host process.
    f.broker
        .revoke_origin(&plugin_id(), &Origin::parse_grant(OPEN_METEO).unwrap());
    let o = h
        .invoke(
            "current-temperature",
            &params(json!({"city": "tokyo"})),
            T,
            &mut ex,
        )
        .unwrap();
    assert_eq!(failure(&o).0, "not-permitted");
    assert_eq!(ex.fetches().len(), 1);
    assert_eq!(h.pid(), pid);
}

#[test]
fn other_methods_and_origins_stay_denied() {
    // Granted GET to Open-Meteo only; the SDK test guest asks for others.
    let f = fixture(&consent_from_manifest());
    let mut h = spawn(&config(SDK_TEST_GUEST), &f);
    let mut ex = StubExecutor::default();
    let o = h.invoke("slow", &Params::new(), T, &mut ex).unwrap();
    let (code, message, _) = failure(&o);
    assert_eq!(code, "not-permitted");
    assert!(message.contains("https://slow.example.com"), "{message}");
    assert!(ex.executed.is_empty());
}

#[test]
fn malformed_params_are_typed_errors_and_the_host_survives() {
    let f = fixture("[]");
    let mut h = spawn(&weather_config(), &f);
    let pid = h.pid();
    let mut ex = StubExecutor::default();
    for (action, p, needle) in [
        ("greet", json!({}), "`name` is required"),
        ("greet", json!({"name": 7}), "must be text"),
        ("greet", json!({"name": ["Ada"]}), "must be text"),
        (
            "greet",
            json!({"name": "Ada", "style": "loud"}),
            "one of: plain, excited",
        ),
        (
            "greet",
            json!({"name": "Ada", "extra": true}),
            "no parameter `extra`",
        ),
        (
            "current-temperature",
            json!({"city": "atlantis"}),
            "one of: berlin",
        ),
        (
            "current-temperature",
            json!({"city": "berlin", "units": 1}),
            "must be a choice",
        ),
    ] {
        let o = h.invoke(action, &params(p), T, &mut ex).unwrap();
        let (code, message, _) = failure(&o);
        assert_eq!(code, "invalid-parameters", "{action}: {message}");
        assert!(message.contains(needle), "{action}: {message}");
    }
    let o = h.invoke("forecast", &Params::new(), T, &mut ex).unwrap();
    assert_eq!(failure(&o).0, "unsupported");
    // Out of ABI bounds: refused by the supervisor before anything is sent.
    assert!(matches!(
        h.invoke("not an id", &Params::new(), T, &mut ex),
        Err(HostError::InvalidParams)
    ));
    assert!(h.is_running());
    assert_eq!(h.pid(), pid);
    assert!(ex.executed.is_empty());
}

#[test]
fn sdk_features_through_the_real_host() {
    let f = fixture("[]");
    let mut h = spawn(&config(SDK_TEST_GUEST), &f);
    let mut ex = StubExecutor::default();
    // Init ran exactly once, at load, before the first invocation.
    for _ in 0..3 {
        let o = h.invoke("inits", &Params::new(), T, &mut ex).unwrap();
        assert_eq!(data(&o)["inits"], ParamValue::Integer(1));
    }
    // Numbers: integers widen, bounds hold, defaults fill.
    let o = h
        .invoke("scale", &params(json!({"x": 2.5})), T, &mut ex)
        .unwrap();
    assert_eq!(data(&o)["y"], ParamValue::Number(5.0));
    let o = h
        .invoke("scale", &params(json!({"x": 3, "factor": 3})), T, &mut ex)
        .unwrap();
    assert_eq!(data(&o)["y"], ParamValue::Number(9.0));
    for p in [
        json!({"x": 11}),
        json!({"x": "2"}),
        json!({"x": 1, "factor": 9}),
    ] {
        let o = h.invoke("scale", &params(p.clone()), T, &mut ex).unwrap();
        assert_eq!(failure(&o).0, "invalid-parameters", "{p}");
    }
    // A code outside the grammar is caught by the SDK.
    let o = h.invoke("bad-code", &Params::new(), T, &mut ex).unwrap();
    assert_eq!(failure(&o).0, "invalid-result");
    // Storage is always granted.
    let o = h.invoke("storage", &Params::new(), T, &mut ex).unwrap();
    assert_eq!(data(&o)["found"], ParamValue::Bool(false));
    // Past the per-invocation call limit the SDK refuses locally instead of
    // letting the host trap.
    let o = h.invoke("many-calls", &Params::new(), T, &mut ex).unwrap();
    assert_eq!(data(&o)["calls"], ParamValue::Integer(256));
    assert_eq!(data(&o)["refusedLocally"], ParamValue::Bool(true));
    assert!(h.is_running());
    assert_eq!(h.shutdown(T).and_then(|s| s.code()), Some(exit_code::CLEAN));
}

#[test]
fn secret_handles_are_injected_only_where_granted() {
    let f = fixture(
        r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]},
            {"kind":"credential","key":"token","origins":["https://api.example.com"],"schemes":["bearer"]}]"#,
    );
    let mut h = spawn(&config(SDK_TEST_GUEST), &f);
    let mut ex = StubExecutor::default();
    let o = h
        .invoke("authed", &params(json!({"token": "token"})), T, &mut ex)
        .unwrap();
    assert_eq!(data(&o)["status"], ParamValue::Integer(200));
    let auth = ex.fetches()[0].auth.clone().unwrap();
    assert_eq!(auth.credential, "token");
    assert_eq!(auth.scheme, AuthScheme::Bearer);

    for p in [
        json!({"token": "token", "scheme": "basic"}),
        json!({"token": "other"}),
    ] {
        let o = h.invoke("authed", &params(p.clone()), T, &mut ex).unwrap();
        let (code, message, _) = failure(&o);
        assert_eq!(code, "not-permitted", "{p}");
        assert!(message.contains("credential"), "{p}: {message}");
    }
    assert_eq!(ex.fetches().len(), 1);
    // A secret parameter carries a slot key, never a secret value.
    let o = h
        .invoke(
            "authed",
            &params(json!({"token": "sk-live 123"})),
            T,
            &mut ex,
        )
        .unwrap();
    assert_eq!(failure(&o).0, "invalid-parameters");
}

#[test]
fn panic_ends_the_host_and_the_supervisor_restarts_it() {
    let f = fixture("[]");
    let mut sup = Supervisor::new(
        config(SDK_TEST_GUEST),
        f.broker.clone(),
        plugin_id(),
        RestartPolicy::default(),
        Arc::new(f.clock.clone()),
    );
    let mut ex = StubExecutor::default();
    assert!(sup.invoke("inits", &Params::new(), T, &mut ex).is_ok());
    let first_pid = sup.host().unwrap().pid();

    let r = sup.invoke("panic", &Params::new(), T, &mut ex);
    match r {
        Err(SupervisorError::Host(HostError::Crashed {
            fatal: Some((FatalReason::Trap, _)),
            ..
        })) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(
        sup.last_exit().and_then(|s| s.code()),
        Some(exit_code::TRAP)
    );
    let SupervisorState::BackingOff { retry_after } = sup.state() else {
        panic!("{:?}", sup.state())
    };
    assert_eq!(retry_after, Duration::from_secs(1));
    f.clock.advance(retry_after);

    // A fresh host and a fresh instance: init ran once in it.
    let o = sup.invoke("inits", &Params::new(), T, &mut ex).unwrap();
    assert_eq!(data(&o)["inits"], ParamValue::Integer(1));
    assert_ne!(sup.host().unwrap().pid(), first_pid);
    assert_eq!(
        sup.shutdown().and_then(|s| s.code()),
        Some(exit_code::CLEAN)
    );
}

#[test]
fn cpu_spin_hits_the_guest_deadline() {
    let f = fixture("[]");
    let mut h = spawn(&config(SDK_TEST_GUEST), &f);
    let mut ex = StubExecutor::default();
    match h.invoke("spin", &Params::new(), T, &mut ex) {
        Err(HostError::Crashed {
            fatal: Some((reason, _)),
            status,
        }) => {
            assert!(
                matches!(reason, FatalReason::Deadline | FatalReason::OutOfFuel),
                "{reason:?}"
            );
            assert_eq!(status.and_then(|s| s.code()), Some(reason.exit_code()));
        }
        other => panic!("{other:?}"),
    }
    assert!(!h.is_running());
}

#[test]
fn slow_host_call_hits_the_wall_clock_timeout() {
    let f =
        fixture(r#"[{"kind":"network","origin":"https://slow.example.com","methods":["GET"]}]"#);
    let mut sup = Supervisor::new(
        config(SDK_TEST_GUEST),
        f.broker.clone(),
        plugin_id(),
        RestartPolicy::default(),
        Arc::new(f.clock.clone()),
    );
    let mut ex = StubExecutor {
        delay: Duration::from_millis(1500),
        ..StubExecutor::default()
    };
    let started = std::time::Instant::now();
    let r = sup.invoke("slow", &Params::new(), Duration::from_millis(500), &mut ex);
    assert!(
        matches!(r, Err(SupervisorError::Host(HostError::Timeout { .. }))),
        "{r:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    // The broker allowed it (decision only); the stub "executed" it.
    assert_eq!(ex.fetches().len(), 1);
    // The killed host counts as a crash.
    assert!(matches!(sup.state(), SupervisorState::BackingOff { .. }));
}

#[test]
fn shutdown_hook_runs_on_clean_shutdown() {
    let f = fixture("[]");
    let h = spawn(&config(SDK_TEST_GUEST), &f);
    assert_eq!(h.shutdown(T).and_then(|s| s.code()), Some(exit_code::CLEAN));

    // A hook that panics turns the clean exit into a trap exit: proof that
    // dp_shutdown ran.
    let mut h = spawn(&config(SDK_TEST_GUEST), &f);
    let mut ex = StubExecutor::default();
    h.invoke("arm-shutdown-panic", &Params::new(), T, &mut ex)
        .unwrap();
    assert_eq!(h.shutdown(T).and_then(|s| s.code()), Some(exit_code::TRAP));
}
