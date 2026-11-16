//! The HTTPS source against a local TLS server (a throwaway CA trusted
//! only here) serving a registry made with the publishing tool and
//! throwaway keys. The registry client does all verification; these tests
//! show that it holds for documents fetched over the network, and that
//! the transport refuses what it must.

use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use deckpilot_plugin_manifest::PackageDigest;
use deckpilot_plugin_package::sha256;
use deckpilot_plugin_registry::publish::{Io, run};
use deckpilot_plugin_registry::roots;
use deckpilot_plugin_registry::{
    FetchError, Freshness, MemoryTrustStore, Policy, Refresh, RegistryClient, RegistryError, Role,
    StaleReason,
};

use super::*;

#[path = "../../../tests/support/https.rs"]
mod https;

use https::{HttpsServer, Route};

const T0: &str = "2026-11-01T00:00:00Z";

fn t0() -> DateTime<Utc> {
    T0.parse().unwrap()
}

fn at(days: i64) -> DateTime<Utc> {
    t0() + TimeDelta::days(days)
}

fn rfc(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A directory removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "deckpilot-fetch-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A registry made with `deckpilot-registry` (as a library): work
/// directory (`site/v1/`, `releases/`) and keys apart.
struct Site {
    scratch: Scratch,
}

impl Site {
    fn work(&self) -> PathBuf {
        self.scratch.0.join("work")
    }

    fn key(&self, name: &str) -> String {
        self.scratch.0.join("keys").join(name).display().to_string()
    }

    fn run(&self, args: &[&str]) -> String {
        let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        if args.first().is_none_or(|c| *c != "keys") {
            all.push("--dir".into());
            all.push(self.work().display().to_string());
        }
        let env = |_: &str| None;
        let mut input: &[u8] = b"";
        let mut out = Vec::new();
        let mut io = Io {
            env: &env,
            stdin: &mut input,
            out: &mut out,
        };
        run(&all, &mut io).unwrap_or_else(|e| panic!("deckpilot-registry {args:?}: {e}"));
        String::from_utf8(out).unwrap()
    }

    /// Key set 1 and seq 1, signed at day 0.
    fn new(name: &str) -> Self {
        let site = Self {
            scratch: Scratch::new(name),
        };
        std::fs::create_dir_all(site.scratch.0.join("keys")).unwrap();
        for role in ["root", "index", "timestamp"] {
            site.run(&[
                "keys",
                "generate",
                "--role",
                role,
                "--out",
                &site.key(&format!("{role}.key")),
                "--no-password",
            ]);
        }
        site.run(&[
            "keyset",
            "init",
            "--root",
            &site.key("root.key.pub"),
            "--index",
            &site.key("index.key.pub"),
            "--timestamp",
            &site.key("timestamp.key.pub"),
        ]);
        site.run(&["keyset", "sign", "--key", &site.key("root.key")]);
        site.publish(0);
        site
    }

    /// A new index and timestamp signed at day `day`.
    fn publish(&self, day: i64) {
        let now = rfc(at(day));
        self.run(&[
            "build-index",
            "--source-commit",
            &"a".repeat(40),
            "--now",
            &now,
        ]);
        self.run(&["sign-index", "--key", &self.key("index.key")]);
        self.timestamp(day);
    }

    /// Re-signs the timestamp at day `day` (valid 7 days).
    fn timestamp(&self, day: i64) {
        self.run(&[
            "timestamp",
            "--key",
            &self.key("timestamp.key"),
            "--now",
            &rfc(at(day)),
        ]);
    }

    fn roots(&self) -> Role {
        serde_json::from_slice(&std::fs::read(self.work().join("roots.json")).unwrap()).unwrap()
    }

    fn client(&self) -> RegistryClient<MemoryTrustStore> {
        RegistryClient::open(self.roots(), MemoryTrustStore::default(), Policy::default()).unwrap()
    }

    fn document(&self, path: &str) -> Vec<u8> {
        std::fs::read(self.work().join("site/v1").join(path)).unwrap()
    }
}

fn source(server: &HttpsServer) -> HttpsSource {
    source_with(server, vec![server.ca_pem.clone()], Vec::new())
}

fn source_with(server: &HttpsServer, roots: Vec<String>, hosts: Vec<String>) -> HttpsSource {
    let mut config = HttpsConfig::new(
        &server.url("site/"),
        Some(&server.url("releases/")),
        TlsRoots::Only(roots),
    )
    .unwrap();
    config.package_redirect_hosts = hosts;
    HttpsSource::new(config).unwrap()
}

fn refused(result: &Result<Refresh, RegistryError>) -> bool {
    matches!(
        result,
        Err(RegistryError::Fetch {
            error: FetchError::Refused(_),
            ..
        })
    )
}

#[test]
fn a_registry_verifies_over_https() {
    let site = Site::new("verifies");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut client = site.client();
    assert_eq!(
        client.refresh(&source, at(1)),
        Ok(Refresh::Updated { seq: 1 })
    );
    assert_eq!(client.freshness(at(1)), Freshness::Fresh);
    let log = server.log();
    for path in [
        "/site/v1/keys/1.json",
        "/site/v1/keys/1.json.minisig",
        "/site/v1/keys/2.json",
        "/site/v1/timestamp.json",
        "/site/v1/timestamp.json.minisig",
        "/site/v1/index/1.json",
        "/site/v1/index/1.json.minisig",
    ] {
        assert!(log.iter().any(|p| p == path), "{path} in {log:?}");
    }
    // Unchanged: only the key set probe and the timestamp.
    assert_eq!(
        client.refresh(&source, at(1)),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    // A new publish.
    site.publish(2);
    assert_eq!(
        client.refresh(&source, at(2)),
        Ok(Refresh::Updated { seq: 2 })
    );
}

#[test]
fn offline_the_verified_state_is_used_until_it_expires() {
    let site = Site::new("offline");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut client = site.client();
    client.refresh(&source, at(1)).unwrap();
    server.set_offline(true);
    assert_eq!(
        client.refresh(&source, at(2)),
        Ok(Refresh::Offline(Freshness::Fresh))
    );
    assert!(source.take_unavailable().is_some());
    assert!(client.index().is_some(), "browsing works from the cache");
    // Timestamp signed at day 0, valid 7 days (plus 5 min skew).
    assert_eq!(
        client.refresh(&source, at(8)),
        Ok(Refresh::Offline(Freshness::Stale(
            StaleReason::TimestampExpired
        )))
    );
    // Back online with a re-signed timestamp: fresh again.
    server.set_offline(false);
    site.timestamp(8);
    assert_eq!(
        client.refresh(&source, at(8)),
        Ok(Refresh::Unchanged { seq: 1 })
    );
    assert_eq!(client.freshness(at(8)), Freshness::Fresh);
}

#[test]
fn a_stale_registry_that_is_reachable_stays_stale() {
    let site = Site::new("stale");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut client = site.client();
    client.refresh(&source, at(1)).unwrap();
    // Nobody re-signed the timestamp: the registry serves the expired one.
    let result = client.refresh(&source, at(8));
    assert!(
        matches!(result, Err(RegistryError::TimestampExpired { .. })),
        "{result:?}"
    );
    assert_eq!(
        client.freshness(at(8)),
        Freshness::Stale(StaleReason::TimestampExpired)
    );
}

#[test]
fn a_freeze_attack_is_refused() {
    let site = Site::new("freeze");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    // A client that never saw the registry is served a timestamp that
    // expired days ago (a frozen mirror): nothing is trusted.
    let mut client = site.client();
    let result = client.refresh(&source, at(10));
    assert!(
        matches!(result, Err(RegistryError::TimestampExpired { .. })),
        "{result:?}"
    );
    assert_eq!(client.freshness(at(10)), Freshness::NeverVerified);
    assert!(client.index().is_none());
}

#[test]
fn a_rollback_attack_is_refused() {
    let site = Site::new("rollback");
    let old = site.scratch.0.join("old");
    site.publish(1);
    copy_dir(&site.work(), &old);
    site.publish(2);
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut client = site.client();
    assert_eq!(
        client.refresh(&source, at(2)),
        Ok(Refresh::Updated { seq: 3 })
    );
    // The network now serves the older, validly signed seq 2 (not
    // expired yet).
    server.set_root(&old);
    let result = client.refresh(&source, at(2));
    assert_eq!(
        result,
        Err(RegistryError::Rollback {
            stored: 3,
            offered: 2
        })
    );
    assert_eq!(client.state().seq, 3);
    assert_eq!(client.index().map(|i| i.seq), Some(3));
}

#[test]
fn tampered_documents_are_refused() {
    let site = Site::new("tampered");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut client = site.client();

    // The index: one byte changed. Its hash no longer matches the
    // timestamp's pin.
    let mut index = site.document("index/1.json");
    index[10] ^= 0x01;
    server.route("/site/v1/index/1.json", Route::Body(index));
    let result = client.refresh(&source, at(1));
    assert!(
        matches!(result, Err(RegistryError::IndexMismatch(_))),
        "{result:?}"
    );
    assert_eq!(client.freshness(at(1)), Freshness::NeverVerified);

    // The timestamp: its signature fails.
    server.clear_routes();
    let mut timestamp = site.document("timestamp.json");
    let last_digit = timestamp.iter().rposition(u8::is_ascii_digit).unwrap();
    timestamp[last_digit] = if timestamp[last_digit] == b'9' {
        b'8'
    } else {
        b'9'
    };
    server.route("/site/v1/timestamp.json", Route::Body(timestamp));
    let result = client.refresh(&source, at(1));
    assert!(
        matches!(result, Err(RegistryError::TimestampSignature(_))),
        "{result:?}"
    );

    // A key set signed by nobody the client trusts (a client that has
    // not stored key set 1 yet).
    server.clear_routes();
    server.route("/site/v1/keys/1.json.minisig", Route::Body(b"[]".to_vec()));
    let result = site.client().refresh(&source, at(1));
    assert!(
        matches!(result, Err(RegistryError::KeysetSignature { .. })),
        "{result:?}"
    );

    server.clear_routes();
    assert_eq!(
        client.refresh(&source, at(1)),
        Ok(Refresh::Updated { seq: 1 })
    );
}

#[test]
fn key_rotation_follows_the_key_set_chain() {
    let site = Site::new("rotation");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut client = site.client();
    client.refresh(&source, at(1)).unwrap();
    site.run(&[
        "keys",
        "generate",
        "--role",
        "timestamp",
        "--out",
        &site.key("timestamp2.key"),
        "--no-password",
    ]);
    site.run(&[
        "keyset",
        "rotate",
        "--timestamp",
        &site.key("timestamp2.key.pub"),
    ]);
    site.run(&["keyset", "sign", "--key", &site.key("root.key")]);
    site.run(&[
        "build-index",
        "--source-commit",
        &"b".repeat(40),
        "--now",
        &rfc(at(2)),
    ]);
    site.run(&["sign-index", "--key", &site.key("index.key")]);
    site.run(&[
        "timestamp",
        "--key",
        &site.key("timestamp2.key"),
        "--now",
        &rfc(at(2)),
    ]);
    assert_eq!(
        client.refresh(&source, at(2)),
        Ok(Refresh::Updated { seq: 2 })
    );
    assert_eq!(client.keys_version(), 2);
}

#[test]
fn redirects_stay_on_the_origin_and_on_https() {
    let site = Site::new("redirects");
    let copy = site.work().join("copy");
    copy_dir(&site.work().join("site/v1"), &copy);
    let server = HttpsServer::start(&site.work());
    let other = HttpsServer::start(&site.work());
    // Trusts both servers' CAs: the redirect is refused for its origin,
    // not for its certificate.
    let source = source_with(
        &server,
        vec![server.ca_pem.clone(), other.ca_pem.clone()],
        vec![],
    );
    let mut client = site.client();

    // To another origin (another port): refused, not followed.
    server.route(
        "/site/v1/timestamp.json",
        Route::Redirect {
            status: 302,
            location: other.url("site/v1/timestamp.json"),
        },
    );
    let result = client.refresh(&source, at(1));
    assert!(refused(&result), "{result:?}");
    assert!(other.log().is_empty(), "{:?}", other.log());

    // Same host name, other origin (`localhost`): refused too.
    server.route(
        "/site/v1/timestamp.json",
        Route::Redirect {
            status: 307,
            location: format!("https://localhost:{}/site/v1/timestamp.json", server.port),
        },
    );
    assert!(refused(&client.refresh(&source, at(1))));

    // Away from https: refused.
    server.route(
        "/site/v1/timestamp.json",
        Route::Redirect {
            status: 301,
            location: format!("http://127.0.0.1:{}/site/v1/timestamp.json", server.port),
        },
    );
    assert!(refused(&client.refresh(&source, at(1))));

    // Endless redirects: refused.
    server.route(
        "/site/v1/timestamp.json",
        Route::Redirect {
            status: 302,
            location: "/site/v1/timestamp.json".into(),
        },
    );
    assert!(refused(&client.refresh(&source, at(1))));
    assert_eq!(client.freshness(at(1)), Freshness::NeverVerified);

    // Within the origin: followed, and the documents still verify.
    server.route(
        "/site/v1/timestamp.json",
        Route::Redirect {
            status: 302,
            location: "/copy/timestamp.json".into(),
        },
    );
    assert_eq!(
        client.refresh(&source, at(1)),
        Ok(Refresh::Updated { seq: 1 })
    );
}

#[test]
fn oversized_and_cut_off_answers_are_refused() {
    let site = Site::new("sizes");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    let mut client = site.client();
    // Larger than the 4 KiB timestamp limit, declared or not.
    for declared in [true, false] {
        server.route(
            "/site/v1/timestamp.json",
            Route::Oversized {
                len: 64 * 1024,
                declared,
            },
        );
        let result = client.refresh(&source, at(1));
        assert!(refused(&result), "{declared}: {result:?}");
    }
    // An index cut off midway: unreachable, so the client stays on what
    // it verified (nothing yet), and nothing partial is stored.
    server.clear_routes();
    server.route("/site/v1/index/1.json", Route::CutOff { sent: 100 });
    assert_eq!(
        client.refresh(&source, at(1)),
        Ok(Refresh::Offline(Freshness::NeverVerified))
    );
    assert!(client.store().index.is_none());
    // Server errors are "unavailable" too.
    server.clear_routes();
    server.route("/site/v1/timestamp.json", Route::Status(503));
    assert_eq!(
        client.refresh(&source, at(1)),
        Ok(Refresh::Offline(Freshness::NeverVerified))
    );
    // Other statuses are refused.
    server.route("/site/v1/timestamp.json", Route::Status(403));
    assert!(refused(&client.refresh(&source, at(1))));
    server.clear_routes();
    assert_eq!(
        client.refresh(&source, at(1)),
        Ok(Refresh::Updated { seq: 1 })
    );
}

#[test]
fn an_untrusted_certificate_is_unreachable() {
    let site = Site::new("untrusted");
    let server = HttpsServer::start(&site.work());
    let stranger = HttpsServer::start(&site.work());
    // Trusts only another CA: the TLS handshake fails, nothing is read.
    let source = source_with(&server, vec![stranger.ca_pem.clone()], vec![]);
    let mut client = site.client();
    assert_eq!(
        client.refresh(&source, at(1)),
        Ok(Refresh::Offline(Freshness::NeverVerified))
    );
    let why = source.take_unavailable().unwrap();
    assert!(why.contains("timestamp") || why.contains("keys"), "{why}");
    assert_eq!(server.log(), Vec::<String>::new());
}

#[test]
fn paths_and_urls_are_checked() {
    let site = Site::new("paths");
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    for bad in [
        "",
        "../roots.json",
        "a/../b",
        "/etc/passwd",
        "a//b",
        "https://evil.example/x",
        "a?b",
        "a%2e%2e/b",
        "a\\b",
        "./a",
    ] {
        assert!(
            matches!(source.fetch(bad, 1024), Err(FetchError::Refused(_))),
            "{bad}"
        );
    }
    assert!(server.log().is_empty(), "{:?}", server.log());
    for bad in [
        "http://registry.example/",
        "ftp://registry.example/",
        "https://user:pw@registry.example/",
        "https://registry.example/?a=1",
        "registry.example",
    ] {
        assert!(https_base(bad, "url").is_err(), "{bad}");
    }
    assert_eq!(
        https_base("https://registry.example/sub", "url")
            .unwrap()
            .as_str(),
        "https://registry.example/sub/"
    );
}

// ------------------------------------------------------------- packages

const PINNED: &str = "https://github.com/knittingcode/deckpilot-plugins/releases/download/acme.x--v1.0.0/acme.x-1.0.0.dpplugin";
const SERVED: &str = "/releases/acme.x--v1.0.0/acme.x-1.0.0.dpplugin";

/// A release asset (any bytes: the download checks only size and hash).
fn release(site: &Site) -> (Vec<u8>, PackageDigest) {
    let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let path = site.work().join(SERVED.trim_start_matches('/'));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let digest = PackageDigest {
        sha256: sha256(&bytes),
        size: bytes.len() as u64,
    };
    (bytes, digest)
}

#[test]
fn a_package_is_downloaded_only_as_pinned() {
    let site = Site::new("package");
    let (bytes, digest) = release(&site);
    let server = HttpsServer::start(&site.work());
    let source = source(&server);
    assert_eq!(source.package(PINNED, &digest), Ok(bytes.clone()));
    assert!(server.log().iter().any(|p| p == SERVED));

    // Same length, other bytes: corrupt.
    let mut tampered = bytes.clone();
    tampered[1234] ^= 0x80;
    server.route(SERVED, Route::Body(tampered));
    assert!(matches!(
        source.package(PINNED, &digest),
        Err(DownloadError::Corrupt(m)) if m.contains("sha256")
    ));
    // Shorter (no length declared would end early): incomplete.
    server.route(SERVED, Route::Body(bytes[..1000].to_vec()));
    assert!(matches!(
        source.package(PINNED, &digest),
        Err(DownloadError::Corrupt(m)) if m.contains("incomplete")
    ));
    // Cut off midway: unavailable, nothing returned.
    server.route(SERVED, Route::CutOff { sent: 50_000 });
    assert!(matches!(
        source.package(PINNED, &digest),
        Err(DownloadError::Unavailable(_))
    ));
    // Larger than pinned: refused before or while reading.
    for declared in [true, false] {
        server.route(
            SERVED,
            Route::Oversized {
                len: bytes.len() + 1,
                declared,
            },
        );
        assert!(matches!(
            source.package(PINNED, &digest),
            Err(DownloadError::Refused(_))
        ));
    }
    // Not a release asset of the registry.
    server.clear_routes();
    for url in [
        "https://evil.example/acme.x-1.0.0.dpplugin",
        "https://github.com/knittingcode/deckpilot-plugins/releases/download/../x",
    ] {
        assert!(matches!(
            source.package(url, &digest),
            Err(DownloadError::Refused(_))
        ));
    }
    assert_eq!(source.package(PINNED, &digest), Ok(bytes));
}

#[test]
fn a_package_redirect_goes_only_to_allowed_hosts() {
    let site = Site::new("package-redirect");
    let (bytes, digest) = release(&site);
    let server = HttpsServer::start(&site.work());
    let elsewhere = format!("https://localhost:{}/asset.bin", server.port);
    std::fs::write(site.work().join("asset.bin"), &bytes).unwrap();
    server.route(
        SERVED,
        Route::Redirect {
            status: 302,
            location: elsewhere.clone(),
        },
    );
    // Not allowed: refused.
    let strict = source(&server);
    assert!(matches!(
        strict.package(PINNED, &digest),
        Err(DownloadError::Refused(m)) if m.contains("another origin")
    ));
    // An allowed asset host (as GitHub's for the pinned URL): followed,
    // and the bytes still have to be the pinned ones.
    let allowing = source_with(
        &server,
        vec![server.ca_pem.clone()],
        vec!["localhost".into()],
    );
    assert_eq!(allowing.package(PINNED, &digest), Ok(bytes));
    server.route(
        SERVED,
        Route::Redirect {
            status: 302,
            location: format!("http://localhost:{}/asset.bin", server.port),
        },
    );
    assert!(matches!(
        allowing.package(PINNED, &digest),
        Err(DownloadError::Refused(_))
    ));
}

#[test]
fn github_asset_hosts_only_for_the_pinned_url() {
    let pinned = HttpsConfig::new("https://registry.example/", None, TlsRoots::System).unwrap();
    assert_eq!(
        pinned.package_redirect_hosts,
        GITHUB_ASSET_HOSTS
            .iter()
            .map(|h| (*h).to_owned())
            .collect::<Vec<_>>()
    );
    let mirrored = HttpsConfig::new(
        "https://registry.example/",
        Some("https://mirror.example/releases/"),
        TlsRoots::System,
    )
    .unwrap();
    assert_eq!(mirrored.package_redirect_hosts, Vec::<String>::new());
    assert!(
        HttpsConfig::new(
            "https://registry.example/",
            Some("http://mirror.example/"),
            TlsRoots::System
        )
        .is_err()
    );
}

#[test]
fn with_roots_and_no_registry_named_the_official_site_is_used() {
    use crate::plugins::PluginSettings;
    use crate::plugins::registry::{DEFAULT_REGISTRY_URL, Registry};
    let site = Site::new("default-url");
    let state = site.scratch.0.join("state");
    let roots = site.scratch.0.join("trusted-roots.json");
    std::fs::copy(site.work().join("roots.json"), &roots).unwrap();
    // Opening fetches nothing; the name is the official site.
    let registry = Registry::from_settings(
        &PluginSettings {
            registry_roots: Some(roots),
            ..PluginSettings::default()
        },
        &state,
    )
    .unwrap()
    .expect("a registry");
    assert_eq!(registry.name, DEFAULT_REGISTRY_URL);
    // Without roots (none embedded, none configured): none.
    assert!(
        Registry::from_choice(
            &PluginSettings::default(),
            &roots::choose_from(None, true, None),
            &state
        )
        .unwrap()
        .is_none()
    );
}

/// Roots embedded at build time are the trust anchor of a registry they
/// signed: a release build ignores a configured `roots.json`, and a root
/// rotation is followed through the signed key set chain. (The build-time
/// embedding itself: `crates/deckpilot-plugin-registry/tests/embedded_roots.rs`.)
#[test]
fn embedded_roots_trust_the_registry_they_signed() {
    use crate::plugins::PluginSettings;
    use crate::plugins::registry::Registry;
    use deckpilot_plugin_registry::roots::RootsSource;
    let site = Site::new("embedded-roots");
    let other = Site::new("embedded-roots-other");
    let today = (Utc::now() - t0()).num_days();
    site.publish(today);
    other.publish(today);
    // The registry directory layout: site/v1 and releases/.
    let settings = |site: &Site| PluginSettings {
        registry: Some(site.work()),
        // In a release build this is ignored (the embedded roots win).
        registry_roots: Some(other.work().join("roots.json")),
        ..PluginSettings::default()
    };
    let embedded: &'static str = Box::leak(
        std::fs::read_to_string(site.work().join("roots.json"))
            .unwrap()
            .into_boxed_str(),
    );
    let release = roots::choose_from(
        Some(embedded),
        false,
        Some(&other.work().join("roots.json")),
    );
    assert_eq!(release.source, Some(RootsSource::Embedded(embedded)));
    assert!(release.ignored.is_some());

    let state = site.scratch.0.join("state");
    let mut registry = Registry::from_choice(&settings(&site), &release, &state)
        .unwrap()
        .expect("a registry");
    assert!(matches!(registry.refresh(), Ok(Refresh::Updated { .. })));
    assert_eq!(registry.client().keys_version(), 1);

    // The other registry is not signed by the embedded roots.
    let mut refused =
        Registry::from_choice(&settings(&other), &release, &other.scratch.0.join("state"))
            .unwrap()
            .expect("a registry");
    assert!(refused.refresh().is_err());

    // Root rotation: key set 2 names a new root, signed by the old and the
    // new one; the embedded roots stay the anchor of the chain.
    site.run(&[
        "keys",
        "generate",
        "--role",
        "root",
        "--out",
        &site.key("root2.key"),
        "--no-password",
    ]);
    site.run(&["keyset", "rotate", "--root", &site.key("root2.key.pub")]);
    site.run(&["keyset", "sign", "--key", &site.key("root.key")]);
    site.run(&["keyset", "sign", "--key", &site.key("root2.key")]);
    site.publish(today);
    let mut registry = Registry::from_choice(&settings(&site), &release, &state)
        .unwrap()
        .expect("a registry");
    assert!(matches!(registry.refresh(), Ok(Refresh::Updated { .. })));
    assert_eq!(registry.client().keys_version(), 2);
}
