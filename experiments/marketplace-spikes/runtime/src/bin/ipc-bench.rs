//! Measurements for RESULTS.md. Run with
//! `cargo run --release --bin ipc-bench` (builds plugin-host next to it).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use runtime_spike::broker::{Broker, Grant};
use runtime_spike::engine::{GUEST_WASM, Limits, Runtime};
use runtime_spike::supervisor::{PluginProcess, SpawnConfig};

const N: usize = 1000;
const T: Duration = Duration::from_secs(10);

fn host_bin() -> PathBuf {
    let me = std::env::current_exe().unwrap();
    let name = if cfg!(windows) {
        "plugin-host.exe"
    } else {
        "plugin-host"
    };
    me.with_file_name(name)
}

fn report(label: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let pct = |p: f64| samples[((samples.len() as f64 - 1.0) * p).round() as usize];
    let total: Duration = samples.iter().sum();
    println!(
        "{label:<52} n={:<5} mean={:>8.1}us p50={:>8.1}us p95={:>8.1}us p99={:>8.1}us max={:>8.1}us",
        samples.len(),
        total.as_secs_f64() * 1e6 / samples.len() as f64,
        pct(0.50).as_secs_f64() * 1e6,
        pct(0.95).as_secs_f64() * 1e6,
        pct(0.99).as_secs_f64() * 1e6,
        pct(1.0).as_secs_f64() * 1e6,
    );
}

fn time<F: FnMut()>(n: usize, mut f: F) -> Vec<Duration> {
    (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect()
}

fn main() {
    let dir = tempfile_dir();
    std::fs::write(dir.join("config.json"), r#"{"hello":"world"}"#).unwrap();
    let mut broker = Broker::new();
    broker.grant("bench", Grant::fs_read(&dir).unwrap());
    broker.grant("bench", Grant::http(&["https://api.example.com"]));

    println!("guest.wasm size: {} bytes", GUEST_WASM.len());

    // In-process costs.
    let t = Instant::now();
    let rt = Runtime::new(GUEST_WASM).unwrap();
    println!(
        "compile guest (cranelift): {:.2} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    report(
        "in-process: fresh Store + instantiate + add()",
        time(N, || {
            let mut p = rt
                .instantiate(&Limits::default(), Box::new(|_| b"{}".to_vec()))
                .unwrap();
            let _: i32 = p.call("add", (1, 2)).unwrap();
        }),
    );
    let mut p = rt
        .instantiate(
            &Limits {
                epoch_ticks: None,
                ..Limits::default()
            },
            Box::new(|_| b"{}".to_vec()),
        )
        .unwrap();
    let t = Instant::now();
    p.relay_n(br#"{"op":"x"}"#, N as u32).unwrap();
    println!(
        "in-process: guest->host_call->guest x{N}: {:.2} us/call",
        t.elapsed().as_secs_f64() * 1e6 / N as f64
    );

    // Process spawn.
    let spawns = time(20, || {
        let p = PluginProcess::spawn(SpawnConfig::new(host_bin(), "bench")).unwrap();
        drop(p);
    });
    report("spawn plugin-host until ready (unsandboxed)", spawns);

    #[cfg(target_os = "macos")]
    {
        let scratch = tempfile_dir();
        let profile =
            runtime_spike::sandbox::SandboxProfile::macos_deny_default(&host_bin(), &scratch)
                .unwrap();
        let spawns = time(20, || {
            let mut cfg = SpawnConfig::new(host_bin(), "bench");
            cfg.sandbox = Some(profile.clone());
            drop(PluginProcess::spawn(cfg).unwrap());
        });
        report("spawn plugin-host until ready (sandbox-exec)", spawns);
    }

    let mut proc_ = PluginProcess::spawn(SpawnConfig::new(host_bin(), "bench")).unwrap();
    println!("host rss after ready: {}", rss(proc_.pid()));
    for i in 0..50 {
        proc_.ping(i, T).unwrap();
    }
    let mut seq = 1000;
    report(
        "ipc: ping/pong round trip",
        time(N, || {
            seq += 1;
            proc_.ping(seq, T).unwrap();
        }),
    );
    report(
        "ipc: invoke add() (fresh Store per invoke)",
        time(N, || {
            let o = proc_.invoke(&broker, "add", None, T).unwrap();
            assert_eq!(o.result.unwrap(), 5);
        }),
    );
    let req = r#"{"op":"fs.read","args":{"path":"config.json"}}"#;
    report(
        "ipc: invoke relay fs.read (guest->host->broker->file)",
        time(N, || {
            let o = proc_.invoke(&broker, "relay", Some(req), T).unwrap();
            assert!(o.result.unwrap()["ok"]["text"].is_string());
        }),
    );
    let req = r#"{"op":"http.fetch","args":{"url":"https://evil.example/"}}"#;
    report(
        "ipc: invoke relay denied http.fetch",
        time(N, || {
            let o = proc_.invoke(&broker, "relay", Some(req), T).unwrap();
            assert!(o.result.unwrap()["error"].is_string());
        }),
    );
    let req = r#"{"op":"fs.read","args":{"path":"config.json"}}"#;
    let t = Instant::now();
    let o = proc_
        .invoke(
            &broker,
            &format!("relay_n:{N}"),
            Some(req),
            Duration::from_secs(60),
        )
        .unwrap();
    assert_eq!(o.calls, N as u64);
    println!(
        "ipc: {N} capability calls inside one invoke: {:.1} us/call",
        t.elapsed().as_secs_f64() * 1e6 / N as f64
    );

    let t = Instant::now();
    let o = proc_.invoke(&broker, "spin", None, T).unwrap();
    println!(
        "epoch deadline (25 ticks x 10 ms) stopped spin after {:.0} ms: {:?}",
        t.elapsed().as_secs_f64() * 1e3,
        o.result
    );
    let t = Instant::now();
    let o = proc_.invoke(&broker, "memory_bomb", None, T).unwrap();
    println!(
        "memory bomb (64 MiB cap) stopped after {:.0} ms: {:?}",
        t.elapsed().as_secs_f64() * 1e3,
        o.result
    );
    println!(
        "host rss after 3000+ invokes and a memory bomb: {}",
        rss(proc_.pid())
    );
}

fn tempfile_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "rt-spike-bench-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    let d = d.with_extension(format!("{}", rand_suffix()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rand_suffix() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(0u8)
}

/// Resident set size via `ps` (macOS/Linux); "n/a" elsewhere.
fn rss(pid: u32) -> String {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|kb| format!("{:.1} MiB", kb as f64 / 1024.0))
        .unwrap_or_else(|| "n/a".into())
}
