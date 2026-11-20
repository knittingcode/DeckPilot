//! Round-trip measurements through the real, sandboxed host under the real
//! supervisor and broker. It replaces the retired spike's `ipc-bench`
//! binary; the spike's numbers are kept in `docs/marketplace/spike-findings.md`.
//!
//! Ignored by default (it measures, it asserts nothing about speed). Run it
//! in release for meaningful numbers:
//!
//! ```sh
//! cargo test --locked --release -p deckpilot-plugin-host --test ipc_bench -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use deckpilot_plugin_abi::{ActionOutcome, CallResult, ParamValue, Params};
use deckpilot_plugin_broker::{
    Allowed, ApprovedPermissions, Broker, ManualClock, MemoryAudit, PluginId, Rate, RateLimits,
};
use deckpilot_plugin_supervisor::{CapabilityExecutor, HostCommand, HostConfig, PluginHost};
use deckpilot_plugin_test_fixtures::TEST_GUEST;
use serde_json::json;

const HOST: &str = env!("CARGO_BIN_EXE_deckpilot-plugin-host");
const T: Duration = Duration::from_secs(10);
const N: usize = 1000;
const SPAWNS: usize = 20;

/// Answers every allowed request at once, without any network.
struct Stub;

impl CapabilityExecutor for Stub {
    fn execute(&mut self, _plugin: &PluginId, _allowed: &Allowed) -> CallResult {
        CallResult::Ok(json!({"status": 200}))
    }
}

fn plugin() -> PluginId {
    PluginId::new("acme.weather").unwrap()
}

fn broker() -> Broker {
    // The manual clock never refills the buckets, so the limits must cover
    // the whole run.
    let plenty = Rate::new(1_000_000, Duration::from_secs(1));
    let limits = RateLimits {
        any: plenty,
        http: plenty,
        notify: plenty,
        log: plenty,
        storage: plenty,
        variables: plenty,
        open_url: plenty,
        open_url_window: plenty,
        time: plenty,
        random_bytes: plenty,
        dialog: plenty,
        tool_run: plenty,
        tool_modify: plenty,
        resource_open: plenty,
        project_switch: plenty,
    };
    let b = Broker::new(
        Arc::new(MemoryAudit::new(10)),
        Arc::new(ManualClock::default()),
        limits,
    );
    let perms = r#"[{"kind":"network","origin":"https://api.example.com","methods":["GET"]}]"#;
    b.set_grants(&plugin(), &ApprovedPermissions::from_json(perms).unwrap())
        .unwrap();
    b
}

fn config() -> HostConfig {
    HostConfig::new(HostCommand::new(HOST), Arc::from(TEST_GUEST))
}

fn params(v: serde_json::Value) -> Params {
    serde_json::from_value(v).unwrap()
}

fn report(label: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    let pct = |p: f64| samples[((samples.len() as f64 - 1.0) * p).round() as usize];
    let mean = samples.iter().sum::<Duration>() / samples.len() as u32;
    println!(
        "{label:<44} n={:<5} mean={:>9.1}us p50={:>9.1}us p95={:>9.1}us p99={:>9.1}us max={:>9.1}us",
        samples.len(),
        us(mean),
        us(pct(0.50)),
        us(pct(0.95)),
        us(pct(0.99)),
        us(pct(1.0)),
    );
}

fn time(n: usize, mut f: impl FnMut()) -> Vec<Duration> {
    (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect()
}

fn ok_flag(o: &ActionOutcome) -> bool {
    match o {
        ActionOutcome::Ok { data, .. } => data["ok"] == ParamValue::Bool(true),
        other => panic!("expected ok, got {other:?}"),
    }
}

#[test]
#[ignore = "measurement; run with --ignored --nocapture in release"]
fn ipc_round_trips() {
    let b = broker();
    let cfg = config();
    println!("test guest: {} bytes", TEST_GUEST.len());

    // Spawn, sandbox, handshake, load (compiles the module) until ready.
    let spawns = time(SPAWNS, || {
        let h = PluginHost::spawn(&cfg, b.session(plugin())).unwrap();
        let _ = h.shutdown(T);
    });
    report("spawn + sandbox + load until ready", spawns);

    let mut h = PluginHost::spawn(&cfg, b.session(plugin())).unwrap();
    println!("sandbox: {:?}", h.sandbox());
    let mut ex = Stub;
    let add = params(json!({"a": 2, "b": 3}));
    report(
        "invoke add (no capability call)",
        time(N, || {
            h.invoke("add", &add, T, &mut ex).unwrap();
        }),
    );
    let allowed = params(json!({"url": "https://api.example.com/x"}));
    report(
        "invoke fetch, allowed (stub executor)",
        time(N, || {
            assert!(ok_flag(&h.invoke("fetch", &allowed, T, &mut ex).unwrap()));
        }),
    );
    let denied = params(json!({"url": "https://evil.net/"}));
    report(
        "invoke fetch, denied by the broker",
        time(N, || {
            let o = h.invoke("fetch", &denied, T, &mut ex).unwrap();
            assert!(!ok_flag(&o));
        }),
    );
    let _ = h.shutdown(T);
}
