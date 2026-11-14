//! Measurements for RESULTS.md. Run with
//! `cargo run --release --example measure`.

#[path = "../tests/common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::*;
use marketplace_trust_spike::client::{Client, Refresh};
use marketplace_trust_spike::package::{self, ExpectedPackage};
use marketplace_trust_spike::registry::sha256_hex;
use marketplace_trust_spike::sig;
use marketplace_trust_spike::signer::ThrowawayKey;

fn median(mut f: impl FnMut(), runs: usize) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}

fn main() {
    // Spike 4: ~5 MiB package (incompressible asset: worst case for size,
    // best case for gunzip) and a compressible one (realistic bundle).
    for (label, files) in [
        (
            "incompressible 4.9 MiB asset",
            plugin_files("acme.clock", "1.0.0", 4_900 * 1024),
        ),
        ("compressible: 2 MiB plugin.js + 3 MiB asset", {
            let mut f = plugin_files("acme.clock", "1.0.0", 3 * 1024 * 1024);
            let js: String = (0..)
                .map(|i| format!("export const v{i} = ctx => ctx.log('value {i}');\n"))
                .take_while({
                    let mut n = 0;
                    move |s: &String| {
                        n += s.len();
                        n < 2 * 1024 * 1024 - 64
                    }
                })
                .collect();
            f[1] = package::PackageFile::new("plugin.js", js);
            f
        }),
    ] {
        let bytes = package::build(&files).unwrap();
        let key = ThrowawayKey::generate();
        let hash = sha256_hex(&bytes);
        let signature = key.sign(&bytes, &sig::package_comment("acme.clock", "1.0.0", &hash));
        let mut e = ExpectedPackage {
            id: "acme.clock".into(),
            version: "1.0.0".into(),
            url: release_url("acme.clock", "1.0.0"),
            sha256: hash,
            size: bytes.len() as u64,
            manifest_sha256: sha256_hex(&manifest_bytes("acme.clock", "1.0.0")),
            publisher: None,
        };
        let build = median(|| drop(package::build(&files).unwrap()), 11);
        let pinned = median(|| package::verify_bytes(&bytes, &e).unwrap(), 51);
        let full = median(|| drop(package::verify(&bytes, &e).unwrap()), 51);
        e.publisher = Some((key.public().to_owned(), Some(signature)));
        let pinned_sig = median(|| package::verify_bytes(&bytes, &e).unwrap(), 51);
        let full_sig = median(|| drop(package::verify(&bytes, &e).unwrap()), 51);
        println!("[package] {label}: {} bytes", bytes.len());
        println!("  build {build:?}; size+sha256 {pinned:?}; +publisher sig {pinned_sig:?}");
        println!("  full verify {full:?}; full verify +publisher sig {full_sig:?}");
    }

    // Spike 5: refresh cost for a large index.
    for n in [100usize, 1000, 3000] {
        let mut reg = Registry::new();
        let plugins: Vec<_> = (0..n)
            .map(|i| PluginSpec {
                id: format!("pub{}.plugin{i}", i % 50),
                publisher: format!("pub{}", i % 50),
                versions: vec![],
            })
            .collect();
        reg.publish(T0, &plugins, &[]);
        let index_len = reg.site.get_bytes("index/1.json").len();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        let first = median(
            || {
                let _ = std::fs::remove_file(&path);
                let mut c = Client::open(reg.embedded_roots(), &path).unwrap();
                assert_eq!(c.refresh(&reg.site, T0), Ok(Refresh::Updated { seq: 1 }));
            },
            11,
        );
        let mut c = Client::open(reg.embedded_roots(), &path).unwrap();
        let again = median(|| assert!(c.refresh(&reg.site, T0).is_ok()), 11);
        let state = std::fs::metadata(&path).unwrap().len();
        println!(
            "[registry] {n} plugins: index {index_len} bytes; first refresh {first:?}; \
             unchanged refresh {again:?}; state file {state} bytes"
        );
    }
}
