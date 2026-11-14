//! The publishing tool end to end (feature `publish`): throwaway keys,
//! key sets, real packages from `deckpilot-plugin-package`, index,
//! signatures and timestamp, then the CLIENT verifier on the result.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use common::{days, id, package, t0, v};
use deckpilot_plugin_package::{Limits, verify_package};
use deckpilot_plugin_registry::publish::{Io, SiteSource, Workspace, run};
use deckpilot_plugin_registry::*;
use tempfile::TempDir;

const ROOT_PASSWORD: &str = "correct horse battery staple";

fn time(at: DateTime<Utc>) -> String {
    at.to_rfc3339()
}

/// A registry working directory plus a separate key directory (secrets
/// never sit in the working directory).
struct Fixture {
    work: TempDir,
    keys: TempDir,
    env: HashMap<String, String>,
}

impl Fixture {
    fn new() -> Self {
        let mut env = HashMap::new();
        env.insert(
            "DECKPILOT_REGISTRY_KEY_PASSWORD".to_owned(),
            ROOT_PASSWORD.to_owned(),
        );
        Self {
            work: TempDir::new().unwrap(),
            keys: TempDir::new().unwrap(),
            env,
        }
    }

    fn dir(&self) -> &Path {
        self.work.path()
    }

    fn ws(&self) -> Workspace {
        Workspace::new(self.dir())
    }

    fn site(&self) -> PathBuf {
        self.ws().site()
    }

    fn key(&self, name: &str) -> String {
        self.keys.path().join(name).display().to_string()
    }

    fn try_run_in(&self, dir: &Path, args: &[&str], stdin: &str) -> Result<String, String> {
        let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        if args.first().is_none_or(|c| *c != "keys") {
            all.push("--dir".into());
            all.push(dir.display().to_string());
        }
        let env = |name: &str| self.env.get(name).cloned();
        let mut input = stdin.as_bytes();
        let mut out = Vec::new();
        let mut io = Io {
            env: &env,
            stdin: &mut input,
            out: &mut out,
        };
        run(&all, &mut io).map_err(|e| e.to_string())?;
        Ok(String::from_utf8(out).unwrap())
    }

    fn try_run(&self, args: &[&str]) -> Result<String, String> {
        self.try_run_in(self.dir(), args, "")
    }

    fn run(&self, args: &[&str]) -> String {
        self.try_run(args)
            .unwrap_or_else(|e| panic!("{args:?} failed: {e}"))
    }

    fn fails(&self, args: &[&str]) -> String {
        match self.try_run(args) {
            Ok(out) => panic!("{args:?} succeeded: {out}"),
            Err(e) => e,
        }
    }

    /// Unencrypted throwaway key (fast); roots get a password.
    fn generate(&self, role: &str, name: &str) {
        self.run(&[
            "keys",
            "generate",
            "--role",
            role,
            "--out",
            &self.key(name),
            "--no-password",
        ]);
    }

    fn package_file(&self, plugin: &str, version: &str) -> String {
        let path = self
            .keys
            .path()
            .join(format!("{plugin}-{version}.dpplugin"));
        std::fs::write(&path, package(plugin, version).bytes).unwrap();
        path.display().to_string()
    }

    fn plugin_detail(&self, plugin: &str) -> String {
        let path = self.keys.path().join(format!("{plugin}.plugin.json"));
        let info = serde_json::json!({
            "id": plugin,
            "name": format!("Plugin {plugin}"),
            "summary": "A test plugin",
            "description": "A test plugin built by the publishing tests.",
            "license": "MIT",
            "homepage": "https://acme.example/plugin",
            "source_repository": "someone/test-plugin",
            "categories": ["developer-tools"]
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&info).unwrap()).unwrap();
        path.display().to_string()
    }

    fn add_version(&self, plugin: &str, version: &str, at: DateTime<Utc>) -> String {
        let package = self.package_file(plugin, version);
        let detail = self.plugin_detail(plugin);
        let tag = format!("v{version}");
        let commit = "1".repeat(40);
        self.run(&[
            "add-version",
            "--package",
            &package,
            "--plugin-detail",
            &detail,
            "--source-repository",
            "someone/test-plugin",
            "--tag",
            &tag,
            "--commit",
            &commit,
            "--builder",
            "1.0.0",
            "--now",
            &time(at),
        ])
    }

    fn build_index(&self, at: DateTime<Utc>) -> String {
        let commit = "a".repeat(40);
        self.run(&[
            "build-index",
            "--source-commit",
            &commit,
            "--now",
            &time(at),
        ])
    }

    fn sign_index(&self, key: &str) -> String {
        self.run(&["sign-index", "--key", &self.key(key)])
    }

    fn timestamp(&self, key: &str, at: DateTime<Utc>) -> String {
        self.run(&["timestamp", "--key", &self.key(key), "--now", &time(at)])
    }

    /// `publish.yml`: index, signature, timestamp.
    fn publish(&self, index_key: &str, at: DateTime<Utc>) {
        self.build_index(at);
        self.sign_index(index_key);
        self.timestamp("timestamp.key", at);
    }

    fn verify(&self, extra: &[&str], at: DateTime<Utc>) -> Result<String, String> {
        let now = time(at);
        let mut args = vec!["verify-site", "--now", &now];
        args.extend_from_slice(extra);
        self.try_run(&args)
    }

    fn state(&self) -> String {
        self.keys.path().join("client-state").display().to_string()
    }

    /// The state an existing client keeps between runs.
    fn verify_as_client(&self, at: DateTime<Utc>) -> Result<String, String> {
        let state = self.state();
        self.verify(&["--state", &state], at)
    }

    fn client(&self) -> RegistryClient<FileTrustStore> {
        let roots: Role =
            serde_json::from_slice(&std::fs::read(self.ws().roots_path()).unwrap()).unwrap();
        RegistryClient::open(roots, FileTrustStore::new(self.state()), Policy::default()).unwrap()
    }

    fn read_site(&self, path: &str) -> Vec<u8> {
        std::fs::read(self.site().join(path)).unwrap()
    }
}

/// Keys, key set 1 signed by one of two roots, two publishers, two
/// plugins with five versions, published at t0.
fn setup() -> Fixture {
    let fx = setup_unpublished();
    fx.publish("index.key", t0());
    fx
}

fn setup_unpublished() -> Fixture {
    let fx = Fixture::new();
    // Roots: encrypted, password from the environment and from stdin.
    fx.run(&[
        "keys",
        "generate",
        "--role",
        "root",
        "--out",
        &fx.key("root1.key"),
    ]);
    fx.try_run_in(
        fx.dir(),
        &[
            "keys",
            "generate",
            "--role",
            "root",
            "--out",
            &fx.key("root2.key"),
            "--password-stdin",
        ],
        "another password\n",
    )
    .unwrap();
    fx.generate("index", "index.key");
    fx.generate("timestamp", "timestamp.key");
    fx.run(&[
        "keyset",
        "init",
        "--root",
        &fx.key("root1.key.pub"),
        "--root",
        &fx.key("root2.key.pub"),
        "--root-threshold",
        "1",
        "--index",
        &fx.key("index.key.pub"),
        "--timestamp",
        &fx.key("timestamp.key.pub"),
    ]);
    let status = fx.run(&["keyset", "sign", "--key", &fx.key("root1.key")]);
    assert!(status.contains("previous roots 1/1 (complete)"), "{status}");
    for (namespace, gid) in [("acme", "1001"), ("zeta", "1002")] {
        fx.run(&[
            "publisher",
            "--namespace",
            namespace,
            "--github-id",
            gid,
            "--login",
            namespace,
            "--verified",
        ]);
    }
    for (plugin, version) in [
        ("acme.alpha", "1.0.0"),
        ("acme.alpha", "1.1.0"),
        ("acme.alpha", "2.0.0-beta.1"),
        ("zeta.beta", "0.1.0"),
        ("zeta.beta", "0.2.0"),
    ] {
        fx.add_version(plugin, version, t0() - days(1));
    }
    fx
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn detail_path(fx: &Fixture, plugin: &str) -> String {
    let index: Index = serde_json::from_slice(&fx.read_site("index/1.json")).unwrap();
    index.plugin(&id(plugin)).unwrap().detail.path.clone()
}

#[test]
fn a_published_site_is_accepted_by_the_client() {
    let fx = setup();
    let report = fx.verify(&["--require-packages"], t0()).unwrap();
    assert!(report.contains("key set 1, seq 1 (new)"), "{report}");
    assert!(report.contains("2 plugin(s), 5 installable version(s), 5 package(s) verified"));

    // The client directly, as the agent will use it.
    let mut client = fx.client();
    let refresh = client.refresh(&SiteSource::new(fx.site()), t0()).unwrap();
    assert_eq!(refresh, Refresh::Updated { seq: 1 });
    let index = client.index().unwrap().clone();
    let alpha = index.plugin(&id("acme.alpha")).unwrap();
    assert_eq!(alpha.latest.stable, Some(v("1.1.0")));
    assert_eq!(alpha.latest.prerelease, Some(v("2.0.0-beta.1")));
    assert_eq!(
        index.plugin(&id("zeta.beta")).unwrap().latest.stable,
        Some(v("0.2.0"))
    );
    let detail = fx.read_site(&alpha.detail.path);
    let target = client
        .resolve_install(&id("acme.alpha"), &v("1.1.0"), &detail, None, t0())
        .unwrap();
    let release = fx.ws().release_file(&target.url).unwrap();
    assert!(release.ends_with("acme.alpha--v1.1.0/acme.alpha-1.1.0.dpplugin"));
    let bytes = std::fs::read(release).unwrap();
    let verified = verify_package(&bytes, &target.expected, &Limits::default()).unwrap();
    assert_eq!(verified.manifest().version, v("1.1.0"));

    // Every generated document also passes the bundled #191 schemas.
    for (path, schema) in [
        ("keys/1.json", schemas::KEYSET),
        ("index/1.json", schemas::INDEX),
        ("timestamp.json", schemas::TIMESTAMP),
    ] {
        let errors = common::schema_errors(&common::validator(schema), &fx.read_site(path));
        assert!(errors.is_empty(), "{path}: {errors:?}");
    }
    let plugin_schema = common::validator(schemas::PLUGIN);
    for entry in &index.plugins {
        let errors = common::schema_errors(&plugin_schema, &fx.read_site(&entry.detail.path));
        assert!(errors.is_empty(), "{}: {errors:?}", entry.detail.path);
    }
}

#[test]
fn documents_are_deterministic() {
    let fx = setup_unpublished();
    let copy = TempDir::new().unwrap();
    copy_dir(fx.dir(), copy.path());
    fx.build_index(t0());
    let commit = "a".repeat(40);
    fx.try_run_in(
        copy.path(),
        &[
            "build-index",
            "--source-commit",
            &commit,
            "--now",
            &time(t0()),
        ],
        "",
    )
    .unwrap();
    let other = Workspace::new(copy.path());
    assert_eq!(
        fx.read_site("index/1.json"),
        std::fs::read(other.index_path(1)).unwrap()
    );
    for plugin in ["acme.alpha", "zeta.beta"] {
        let path = detail_path(&fx, plugin);
        assert_eq!(
            fx.read_site(&path),
            std::fs::read(other.site().join(&path)).unwrap()
        );
    }
    // Re-adding the same package records nothing new; another package for
    // a published version, or a lower version, is refused.
    let again = fx.add_version("acme.alpha", "1.1.0", t0());
    assert!(again.contains("already recorded"), "{again}");
    let mut tampered = package("acme.alpha", "1.1.0").bytes;
    let package_path = fx.keys.path().join("other.dpplugin");
    tampered.extend_from_slice(b"x");
    std::fs::write(&package_path, &tampered).unwrap();
    let detail = fx.plugin_detail("acme.alpha");
    let commit = "1".repeat(40);
    let add = |package: &str| {
        fx.try_run(&[
            "add-version",
            "--package",
            package,
            "--plugin-detail",
            &detail,
            "--source-repository",
            "someone/test-plugin",
            "--tag",
            "v1",
            "--commit",
            &commit,
            "--builder",
            "1.0.0",
        ])
    };
    let error = add(&package_path.display().to_string()).unwrap_err();
    assert!(error.contains("the package is invalid"), "{error}");
    let lower = fx.package_file("acme.alpha", "1.0.5");
    let error = add(&lower).unwrap_err();
    assert!(error.contains("append-only"), "{error}");
}

#[test]
fn revocations_and_yanks_reach_clients() {
    let fx = setup();
    fx.verify_as_client(t0()).unwrap();

    fx.run(&[
        "revoke",
        "--id",
        "acme.alpha",
        "--versions",
        "1.0.0",
        "--action",
        "disable",
        "--reason",
        "Sends the token to the wrong host",
        "--advisory",
        "https://example.invalid/advisory/1",
        "--now",
        &time(t0() + days(1)),
    ]);
    fx.run(&[
        "yank",
        "--id",
        "zeta.beta",
        "--version",
        "0.2.0",
        "--reason",
        "Crashes on start",
        "--now",
        &time(t0() + days(1)),
    ]);
    fx.publish("index.key", t0() + days(1));
    let report = fx.verify_as_client(t0() + days(1)).unwrap();
    assert!(report.contains("seq 2 (new)"), "{report}");
    assert!(report.contains("2 revocation(s)"), "{report}");

    let client = fx.client();
    assert_eq!(
        client.installed_verdict(&id("acme.alpha"), &v("1.0.0")),
        InstalledVerdict::Disable {
            reason: "Sends the token to the wrong host".into(),
            advisory: Some("https://example.invalid/advisory/1".into()),
        }
    );
    assert!(matches!(
        client.installed_verdict(&id("zeta.beta"), &v("0.2.0")),
        InstalledVerdict::RunWithNotice { .. }
    ));
    assert_eq!(
        client.installed_verdict(&id("acme.alpha"), &v("1.1.0")),
        InstalledVerdict::Run
    );
    let index = client.index().unwrap();
    let zeta = index.plugin(&id("zeta.beta")).unwrap();
    assert_eq!(zeta.latest.stable, Some(v("0.1.0")));
    let detail = fx.read_site(&zeta.detail.path);
    assert!(matches!(
        client.resolve_install(&id("zeta.beta"), &v("0.2.0"), &detail, None, t0() + days(1)),
        Err(InstallError::Yanked { .. })
    ));
    let alpha = index.plugin(&id("acme.alpha")).unwrap();
    let detail = fx.read_site(&alpha.detail.path);
    assert!(matches!(
        client.resolve_install(
            &id("acme.alpha"),
            &v("1.0.0"),
            &detail,
            None,
            t0() + days(1)
        ),
        Err(InstallError::Revoked { .. })
    ));

    // Weakening is refused by the tool, as the client would refuse it.
    let path = fx.ws().revocations_path();
    let mut list: Vec<Revocation> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    list[0].action = RevocationAction::Yank;
    std::fs::write(&path, serde_json::to_vec_pretty(&list).unwrap()).unwrap();
    let commit = "a".repeat(40);
    let error = fx.fails(&["build-index", "--source-commit", &commit]);
    assert!(error.contains("drops or weakens"), "{error}");
    std::fs::write(&path, b"[]").unwrap();
    let error = fx.fails(&["build-index", "--source-commit", &commit]);
    assert!(error.contains("drops or weakens"), "{error}");
    let error = fx.fails(&["lift-revocation", "--id", "acme.alpha"]);
    assert!(error.contains("seq_floor_reset"), "{error}");
}

#[test]
fn key_rotation_needs_old_and_new_roots() {
    let fx = setup();
    fx.verify_as_client(t0()).unwrap();
    fx.generate("index", "index2.key");
    fx.run(&[
        "keys",
        "generate",
        "--role",
        "root",
        "--out",
        &fx.key("root3.key"),
    ]);
    let out = fx.run(&[
        "keyset",
        "rotate",
        "--root",
        &fx.key("root1.key.pub"),
        "--root",
        &fx.key("root3.key.pub"),
        "--index",
        &fx.key("index2.key.pub"),
    ]);
    assert!(out.contains("keys/2.json (seq_floor 1)"), "{out}");
    let commit = "a".repeat(40);
    let error = fx.fails(&["build-index", "--source-commit", &commit]);
    assert!(
        error.contains("keys/2.json is not accepted by clients"),
        "{error}"
    );

    let status = fx.run(&["keyset", "sign", "--key", &fx.key("root3.key")]);
    assert!(
        status.contains("previous roots 0/1, new roots 1/1 (more"),
        "{status}"
    );
    let error = fx.fails(&["build-index", "--source-commit", &commit]);
    assert!(error.contains("not accepted by clients"), "{error}");
    let status = fx.run(&["keyset", "sign", "--key", &fx.key("root1.key")]);
    assert!(
        status.contains("previous roots 1/1, new roots 2/1 (complete)"),
        "{status}"
    );
    let error = fx.fails(&["keyset", "sign", "--key", &fx.key("index2.key")]);
    assert!(error.contains("not a root key"), "{error}");

    // The timestamp cannot pin the old index under the new key set.
    let error = fx.fails(&[
        "timestamp",
        "--key",
        &fx.key("timestamp.key"),
        "--now",
        &time(t0() + days(1)),
    ]);
    assert!(error.contains("not signed for key set 2"), "{error}");
    fx.build_index(t0() + days(1));
    let error = fx.fails(&["sign-index", "--key", &fx.key("index.key")]);
    assert!(error.contains("not an index key of keys/2.json"), "{error}");
    fx.sign_index("index2.key");
    fx.timestamp("timestamp.key", t0() + days(1));
    let report = fx.verify_as_client(t0() + days(1)).unwrap();
    assert!(report.contains("key set 2, seq 2 (new)"), "{report}");
    assert_eq!(fx.client().keys_version(), 2);
    // A fresh client walks the chain from the embedded roots too.
    fx.verify(&[], t0() + days(1)).unwrap();
}

#[test]
fn a_rollback_is_refused_by_the_client() {
    let fx = setup();
    fx.verify_as_client(t0()).unwrap();
    let snapshot = TempDir::new().unwrap();
    copy_dir(&fx.site(), snapshot.path());

    fx.add_version("zeta.beta", "0.3.0", t0() + days(1));
    fx.publish("index.key", t0() + days(1));
    fx.verify_as_client(t0() + days(1)).unwrap();

    // A mirror (or attacker) serves the older, validly signed site.
    std::fs::remove_dir_all(fx.site()).unwrap();
    copy_dir(snapshot.path(), &fx.site());
    let error = fx.verify_as_client(t0() + days(2)).unwrap_err();
    assert!(
        error.contains("rollback: seq 1 is below the stored seq 2"),
        "{error}"
    );
    // Only the stored floor tells: a fresh client accepts the old site.
    fx.verify(&[], t0() + days(2)).unwrap();
    assert_eq!(fx.client().state().seq, 2);
}

#[test]
fn the_tool_never_rolls_back() {
    let fx = setup();
    fx.add_version("zeta.beta", "0.3.0", t0() + days(1));
    fx.publish("index.key", t0() + days(1));
    for file in ["index/2.json", "index/2.json.minisig"] {
        std::fs::remove_file(fx.site().join(file)).unwrap();
    }
    let error = fx.fails(&[
        "timestamp",
        "--key",
        &fx.key("timestamp.key"),
        "--now",
        &time(t0() + days(2)),
    ]);
    assert!(error.contains("refusing to roll back"), "{error}");
    // The next index continues after the live timestamp's seq.
    let out = fx.build_index(t0() + days(2));
    assert!(out.contains("index/3.json"), "{out}");
    let error = fx.fails(&["build-index", "--source-commit", &"a".repeat(40)]);
    assert!(error.contains("not signed yet"), "{error}");
}

#[test]
fn a_tampered_site_is_refused() {
    let fx = setup();
    fx.verify(&["--require-packages"], t0()).unwrap();
    let detail = detail_path(&fx, "acme.alpha");
    let release = "releases/acme.alpha--v1.0.0/acme.alpha-1.0.0.dpplugin".to_owned();
    for (path, expected) in [
        ("site/v1/keys/1.json".to_owned(), "keys/1.json"),
        ("site/v1/timestamp.json".to_owned(), "timestamp signature"),
        (
            "site/v1/index/1.json".to_owned(),
            "does not match the timestamp",
        ),
        (format!("site/v1/{detail}"), "does not match the index"),
        (release, "release asset fails verification"),
    ] {
        let copy = TempDir::new().unwrap();
        copy_dir(fx.dir(), copy.path());
        let file = copy.path().join(&path);
        let mut bytes = std::fs::read(&file).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        std::fs::write(&file, bytes).unwrap();
        let now = time(t0());
        let error = fx
            .try_run_in(
                copy.path(),
                &["verify-site", "--now", &now, "--require-packages"],
                "",
            )
            .unwrap_err();
        assert!(error.contains(expected), "{path}: {error}");
    }
    // A missing signature is refused too.
    let copy = TempDir::new().unwrap();
    copy_dir(fx.dir(), copy.path());
    std::fs::remove_file(copy.path().join("site/v1/index/1.json.minisig")).unwrap();
    let error = fx
        .try_run_in(copy.path(), &["verify-site", "--now", &time(t0())], "")
        .unwrap_err();
    assert!(error.contains("index/1.json.minisig"), "{error}");
}

#[test]
fn a_timestamp_refresh_keeps_clients_fresh_without_reindexing() {
    let fx = setup();
    fx.verify_as_client(t0()).unwrap();
    let before = TempDir::new().unwrap();
    copy_dir(fx.dir(), before.path());
    let index = fx.read_site("index/1.json");
    let signature = fx.read_site("index/1.json.minisig");

    // What the daily job runs: the timestamp key only.
    let out = fx.timestamp("timestamp.key", t0() + days(6));
    assert!(out.contains("seq 1, key set 1"), "{out}");
    assert_eq!(fx.read_site("index/1.json"), index);
    assert_eq!(fx.read_site("index/1.json.minisig"), signature);

    let report = fx.verify_as_client(t0() + days(10)).unwrap();
    assert!(report.contains("seq 1 (unchanged)"), "{report}");
    assert_eq!(fx.client().freshness(t0() + days(10)), Freshness::Fresh);
    // Without the refresh the same client would be stale by now.
    let error = fx
        .try_run_in(
            before.path(),
            &["verify-site", "--now", &time(t0() + days(10))],
            "",
        )
        .unwrap_err();
    assert!(error.contains("expired"), "{error}");
    // The warning threshold of the scheduled check.
    let error = fx
        .verify(&["--min-remaining", "3d"], t0() + days(12))
        .unwrap_err();
    assert!(error.contains("sooner than required"), "{error}");

    // Longer than the client's 14-day cap, or back in time: refused.
    let key = fx.key("timestamp.key");
    let error = fx.fails(&["timestamp", "--key", &key, "--expires-in", "15d"]);
    assert!(error.contains("at most 14 days"), "{error}");
    let error = fx.fails(&["timestamp", "--key", &key, "--now", &time(t0())]);
    assert!(error.contains("check the clock"), "{error}");
    let error = fx.fails(&["timestamp", "--key", &fx.key("index.key")]);
    assert!(error.contains("not a timestamp key"), "{error}");
}

#[test]
fn revocations_are_lifted_only_after_a_root_reset() {
    let fx = setup();
    fx.run(&[
        "revoke",
        "--id",
        "acme.alpha",
        "--versions",
        "1.0.0",
        "--action",
        "disable",
        "--reason",
        "Mistaken revocation",
        "--now",
        &time(t0() + days(1)),
    ]);
    fx.publish("index.key", t0() + days(1));
    fx.verify_as_client(t0() + days(1)).unwrap();
    assert!(matches!(
        fx.client()
            .installed_verdict(&id("acme.alpha"), &v("1.0.0")),
        InstalledVerdict::Disable { .. }
    ));

    // Index-key compromise recovery: new index key, seq floor reset.
    fx.generate("index", "index2.key");
    fx.run(&[
        "keyset",
        "rotate",
        "--index",
        &fx.key("index2.key.pub"),
        "--seq-floor-reset",
    ]);
    // root2's password is not the one in the environment.
    let error = fx.fails(&["keyset", "sign", "--key", &fx.key("root2.key")]);
    assert!(error.contains("Wrong password"), "{error}");
    fx.try_run_in(
        fx.dir(),
        &[
            "keyset",
            "sign",
            "--key",
            &fx.key("root2.key"),
            "--password-stdin",
        ],
        "another password\n",
    )
    .unwrap();
    let out = fx.run(&["lift-revocation", "--id", "acme.alpha"]);
    assert!(out.contains("removed 1 revocation(s)"), "{out}");
    fx.publish("index2.key", t0() + days(2));
    let report = fx.verify_as_client(t0() + days(2)).unwrap();
    assert!(report.contains("key set 2, seq 3 (new)"), "{report}");
    assert_eq!(
        fx.client()
            .installed_verdict(&id("acme.alpha"), &v("1.0.0")),
        InstalledVerdict::Run
    );

    // Once an index is out under the reset key set, cumulative again.
    fx.run(&[
        "revoke",
        "--id",
        "acme.alpha",
        "--versions",
        "1.0.0",
        "--action",
        "yank",
        "--reason",
        "Old",
    ]);
    fx.publish("index2.key", t0() + days(3));
    let error = fx.fails(&["lift-revocation", "--id", "acme.alpha"]);
    assert!(error.contains("seq_floor_reset"), "{error}");
}

#[test]
#[allow(clippy::too_many_lines)] // one real process per step
fn the_binary_keeps_secrets_out_of_its_output() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let bin = env!("CARGO_BIN_EXE_deckpilot-registry");
    let keys = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let key = keys.path().join("root.key");
    let password = "pa55-never-printed";
    let mut child = Command::new(bin)
        .args([
            "keys",
            "generate",
            "--role",
            "root",
            "--password-stdin",
            "--out",
        ])
        .arg(&key)
        .env_remove("DECKPILOT_REGISTRY_KEY_PASSWORD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{password}\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let secret = std::fs::read_to_string(&key).unwrap();
    let secret_line = secret.lines().nth(1).unwrap().to_owned();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(!secret.contains(password));

    let public = format!("{}.pub", key.display());
    let ok = Command::new(bin)
        .args([
            "keyset",
            "init",
            "--root",
            &public,
            "--index",
            &public,
            "--timestamp",
            &public,
        ])
        .args(["--dir".as_ref(), work.path().as_os_str()])
        .output()
        .unwrap();
    // One key in every role is refused (separation of privilege).
    assert!(!ok.status.success());
    assert!(String::from_utf8_lossy(&ok.stderr).contains("shares a key"));

    let index = keys.path().join("index.key");
    let timestamp = keys.path().join("timestamp.key");
    for (role, path) in [("index", &index), ("timestamp", &timestamp)] {
        let out = Command::new(bin)
            .args(["keys", "generate", "--no-password", "--role", role, "--out"])
            .arg(path)
            .env_remove("DECKPILOT_REGISTRY_KEY_PASSWORD")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    let out = Command::new(bin)
        .args(["keyset", "init", "--root", &public])
        .arg("--index")
        .arg(format!("{}.pub", index.display()))
        .arg("--timestamp")
        .arg(format!("{}.pub", timestamp.display()))
        .args(["--dir".as_ref(), work.path().as_os_str()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");

    let sign = |password: &str| {
        Command::new(bin)
            .args([
                "keyset",
                "sign",
                "--password-env",
                "TEST_ROOT_PASSWORD",
                "--key",
            ])
            .arg(&key)
            .args(["--dir".as_ref(), work.path().as_os_str()])
            .env("TEST_ROOT_PASSWORD", password)
            .output()
            .unwrap()
    };
    let wrong = sign("wrong password");
    assert!(!wrong.status.success());
    let right = sign(password);
    assert!(right.status.success(), "{right:?}");
    for output in [&output, &wrong, &right] {
        for stream in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(stream);
            assert!(!text.contains(password), "{text}");
            assert!(!text.contains("wrong password"), "{text}");
            assert!(!text.contains(&secret_line), "{text}");
        }
    }
    let stdout = String::from_utf8_lossy(&right.stdout);
    assert!(stdout.contains("(complete)"), "{stdout}");
}

#[test]
fn bad_input_is_refused() {
    let mut fx = Fixture::new();
    fx.env.clear();
    let error = fx.fails(&["keys", "generate", "--role", "index", "--out", &fx.key("k")]);
    assert!(error.contains("a password is required"), "{error}");
    fx.generate("index", "k");
    let error = fx.fails(&[
        "keys",
        "generate",
        "--role",
        "index",
        "--out",
        &fx.key("k"),
        "--no-password",
    ]);
    assert!(error.contains("refusing to overwrite"), "{error}");
    let error = fx.fails(&["keys", "generate", "--role", "admin", "--out", &fx.key("x")]);
    assert!(error.contains("unknown role"), "{error}");
    let error = fx.fails(&["build-index", "--source-commit", "abc", "--colour", "red"]);
    assert!(error.contains("unknown option --colour"), "{error}");
    let error = fx.fails(&["publish-everything"]);
    assert!(error.contains("unknown command"), "{error}");
    let error = fx.fails(&["verify-site"]);
    assert!(error.contains("roots.json is missing"), "{error}");
    assert!(fx.run(&["help"]).contains("verify-site"));
    // An encrypted key without its password.
    fx.env
        .insert("DECKPILOT_REGISTRY_KEY_PASSWORD".into(), "pw".into());
    fx.run(&["keys", "generate", "--role", "root", "--out", &fx.key("r")]);
    fx.env.clear();
    fx.generate("timestamp", "t");
    fx.run(&[
        "keyset",
        "init",
        "--root",
        &fx.key("r.pub"),
        "--index",
        &fx.key("k.pub"),
        "--timestamp",
        &fx.key("t.pub"),
    ]);
    let error = fx.fails(&["keyset", "sign", "--key", &fx.key("r")]);
    assert!(error.contains("is encrypted"), "{error}");
    let error = fx.fails(&["keyset", "sign", "--key", &fx.key("k")]);
    assert!(error.contains("not a root key"), "{error}");
}

#[test]
fn package_builds_from_a_plugin_directory() {
    let fx = Fixture::new();
    let reference = package("acme.alpha", "1.0.0");
    let files =
        deckpilot_plugin_package::read_archive(&reference.bytes, &Limits::default()).unwrap();
    let plugin = fx.keys.path().join("plugin");
    for (path, bytes) in &files {
        if path == "plugin.wasm" {
            continue;
        }
        let target = plugin.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, bytes).unwrap();
    }
    // Sources and build output next to the manifest are not packaged.
    std::fs::create_dir_all(plugin.join("src")).unwrap();
    std::fs::write(plugin.join("src/lib.rs"), "// source").unwrap();
    let module = fx.keys.path().join("plugin.wasm");
    std::fs::write(&module, &files["plugin.wasm"]).unwrap();
    let out = fx.keys.path().join("out.dpplugin");
    let args = |dir: &Path| {
        vec![
            "package".to_owned(),
            "--plugin-dir".to_owned(),
            dir.display().to_string(),
            "--module".to_owned(),
            module.display().to_string(),
            "--out".to_owned(),
            out.display().to_string(),
        ]
    };
    let report = fx.run(&args(&plugin).iter().map(String::as_str).collect::<Vec<_>>());
    assert!(report.contains("acme.alpha 1.0.0 packaged"), "{report}");
    // Deterministic: the same bytes as the builder's.
    assert_eq!(std::fs::read(&out).unwrap(), reference.bytes);

    // A module that is not plugin ABI 1 is refused.
    std::fs::write(&module, b"\0asm\x01\0\0\0").unwrap();
    let error = fx.fails(&args(&plugin).iter().map(String::as_str).collect::<Vec<_>>());
    assert!(error.contains("plugin.wasm"), "{error}");

    // A manifest of the #191 JavaScript contract is refused, naming the
    // contract.
    std::fs::write(&module, &files["plugin.wasm"]).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&files["manifest.json"]).unwrap();
    manifest.as_object_mut().unwrap().remove("runtime");
    std::fs::write(
        plugin.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let error = fx.fails(&args(&plugin).iter().map(String::as_str).collect::<Vec<_>>());
    assert!(error.contains("artifact-contract.md"), "{error}");
}
