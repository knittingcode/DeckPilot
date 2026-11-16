//! Root keys embedded at build time (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`,
//! `build.rs`, `src/roots.rs`).
//!
//! The test signs two local registry sites with throwaway keys made by the
//! publishing tool (`deckpilot-registry keys generate`, in memory through
//! `publish::run`), then builds this crate's `registry_roots` example with
//! Cargo, in its own target directory, and runs it against them:
//!
//! * built with site A's roots embedded, it trusts site A and refuses
//!   site B;
//! * a release build ignores a `roots.json` given at run time
//!   (`DECKPILOT_PLUGIN_REGISTRY_ROOTS`) when it embeds roots; a debug
//!   build, and a release build with `test-hooks`, use it instead;
//! * without embedded roots nothing is fetched unless a `roots.json` is
//!   given (a release build still accepts one: self-hosted registries);
//! * a malformed value fails the build, and nothing is embedded.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use deckpilot_plugin_registry::publish::{Io, run};
use tempfile::TempDir;

const BUILD_ENV: &str = "DECKPILOT_REGISTRY_ROOT_PUBKEYS";
const RUNTIME_ENV: &str = "DECKPILOT_PLUGIN_REGISTRY_ROOTS";

/// A registry site signed by its own throwaway keys.
struct Site {
    dir: TempDir,
}

impl Site {
    fn tool(&self, args: &[&str]) {
        let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        if args.first() != Some(&"keys") {
            all.extend(["--dir".to_owned(), self.work().display().to_string()]);
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
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn key(&self, name: &str) -> String {
        self.dir
            .path()
            .join("keys")
            .join(name)
            .display()
            .to_string()
    }

    fn create() -> Self {
        let site = Self {
            dir: TempDir::new().unwrap(),
        };
        std::fs::create_dir_all(site.work()).unwrap();
        std::fs::create_dir_all(site.dir.path().join("keys")).unwrap();
        for role in ["root", "index", "timestamp"] {
            let out = site.key(&format!("{role}.key"));
            site.tool(&[
                "keys",
                "generate",
                "--role",
                role,
                "--out",
                &out,
                "--no-password",
            ]);
        }
        site.tool(&[
            "keyset",
            "init",
            "--root",
            &site.key("root.key.pub"),
            "--index",
            &site.key("index.key.pub"),
            "--timestamp",
            &site.key("timestamp.key.pub"),
        ]);
        site.tool(&["keyset", "sign", "--key", &site.key("root.key")]);
        site.tool(&["build-index", "--source-commit", &"a".repeat(40)]);
        site.tool(&["sign-index", "--key", &site.key("index.key")]);
        site.tool(&["timestamp", "--key", &site.key("timestamp.key")]);
        site
    }

    /// `roots.json` as the tool wrote it (`keys/roots.json` of the
    /// registry repository).
    fn roots_file(&self) -> PathBuf {
        self.work().join("roots.json")
    }

    fn roots(&self) -> String {
        std::fs::read_to_string(self.roots_file()).unwrap()
    }

    fn v1(&self) -> PathBuf {
        self.work().join("site").join("v1")
    }
}

/// One target directory per test, so that the tests can run in parallel.
fn target_dir(test: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("registry-roots-{test}"))
}

/// Builds the example; `embed` is the value of the build variable.
fn cargo_build(test: &str, release: bool, features: &[&str], embed: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "build",
            "--locked",
            "--quiet",
            "-p",
            "deckpilot-plugin-registry",
            "--example",
            "registry_roots",
            "--target-dir",
        ])
        .arg(target_dir(test))
        .env("CARGO_INCREMENTAL", "0")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove(BUILD_ENV)
        .env_remove(RUNTIME_ENV);
    if release {
        command.arg("--release");
    }
    if !features.is_empty() {
        command.args(["--features", &features.join(",")]);
    }
    if let Some(value) = embed {
        command.env(BUILD_ENV, value);
    }
    command.output().expect("cargo runs")
}

/// Builds the example and returns a copy of it under `name`.
fn build(test: &str, release: bool, features: &[&str], embed: Option<&str>, name: &str) -> PathBuf {
    let output = cargo_build(test, release, features, embed);
    assert!(
        output.status.success(),
        "the example builds: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let profile = if release { "release" } else { "debug" };
    let exe = format!("registry_roots{}", std::env::consts::EXE_SUFFIX);
    let built = target_dir(test).join(profile).join("examples").join(&exe);
    let copy = target_dir(test).join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(&built, &copy).unwrap();
    copy
}

/// Runs a built example against `site` with a fresh state, and the
/// runtime `roots.json` `configured`.
fn verify(exe: &Path, site: &Site, configured: Option<&Path>) -> String {
    let state = TempDir::new().unwrap();
    let mut command = Command::new(exe);
    command
        .arg(site.v1())
        .arg(state.path())
        .env_remove(RUNTIME_ENV)
        .env_remove(BUILD_ENV);
    if let Some(file) = configured {
        command.env(RUNTIME_ENV, file);
    }
    let output = command.output().expect("the example runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn assert_starts(line: &str, prefix: &str) {
    assert!(
        line.starts_with(prefix),
        "expected `{prefix}…`, got `{line}`"
    );
}

#[test]
fn release_builds_trust_embedded_roots_only() {
    let a = Site::create();
    let b = Site::create();
    let test = "release";

    // Site A's roots embedded (the value is the file's content, as the
    // repository variable holds it).
    let exe = build(test, true, &[], Some(&a.roots()), "embedded");
    let line = verify(&exe, &a, None);
    assert_starts(&line, "roots=embedded ignored=false");
    assert!(line.contains("result=updated seq 1"), "{line}");
    let line = verify(&exe, &b, None);
    assert_starts(&line, "roots=embedded ignored=false");
    assert!(
        line.contains("result=refused"),
        "site B is not signed by A's roots: {line}"
    );
    // The runtime override is ignored: B stays refused, A trusted.
    let line = verify(&exe, &b, Some(&b.roots_file()));
    assert_starts(&line, "roots=embedded ignored=true");
    assert!(line.contains("result=refused"), "{line}");
    let line = verify(&exe, &a, Some(&b.roots_file()));
    assert_starts(&line, "roots=embedded ignored=true");
    assert!(line.contains("result=updated seq 1"), "{line}");

    // Nothing embedded: nothing fetched without roots; a release build
    // still takes a configured roots.json (self-hosted registries).
    let exe = build(test, true, &[], None, "none");
    assert_eq!(
        verify(&exe, &a, None),
        "roots=none ignored=false fetches=0 result=not-configured"
    );
    let line = verify(&exe, &b, Some(&b.roots_file()));
    assert_starts(&line, "roots=file ignored=false");
    assert!(line.contains("result=updated seq 1"), "{line}");
    // An empty value embeds nothing either.
    let exe = build(test, true, &[], Some(""), "empty");
    assert_eq!(
        verify(&exe, &a, None),
        "roots=none ignored=false fetches=0 result=not-configured"
    );

    // test-hooks (the agent's test builds): the override wins again.
    let exe = build(test, true, &["test-hooks"], Some(&a.roots()), "test-hooks");
    let line = verify(&exe, &b, Some(&b.roots_file()));
    assert_starts(&line, "roots=file ignored=false");
    assert!(line.contains("result=updated seq 1"), "{line}");
}

#[test]
fn debug_builds_let_the_runtime_roots_override() {
    let a = Site::create();
    let b = Site::create();
    let test = "debug";

    // The value may also name the file (absolute path).
    let path = a.roots_file().display().to_string();
    let exe = build(test, false, &[], Some(&path), "embedded");
    let line = verify(&exe, &a, None);
    assert_starts(&line, "roots=embedded ignored=false");
    assert!(line.contains("result=updated seq 1"), "{line}");
    let line = verify(&exe, &b, Some(&b.roots_file()));
    assert_starts(&line, "roots=file ignored=false");
    assert!(line.contains("result=updated seq 1"), "{line}");

    let exe = build(test, false, &[], None, "none");
    assert_eq!(
        verify(&exe, &a, None),
        "roots=none ignored=false fetches=0 result=not-configured"
    );
}

#[test]
fn malformed_roots_fail_the_build() {
    let a = Site::create();
    let test = "malformed";
    let roots: serde_json::Value = serde_json::from_str(&a.roots()).unwrap();
    let key = roots["keys"][0].as_str().unwrap().to_owned();
    let missing = a.dir.path().join("missing-roots.json");
    let cases = [
        ("not JSON".to_owned(), "neither a root role"),
        ("{\"keys\": [".to_owned(), "is not a root role"),
        (
            format!("{{\"keys\":[\"{key}\"],\"threshold\":1,\"index\":[]}}"),
            "unknown field",
        ),
        ("{\"keys\":[],\"threshold\":1}".to_owned(), "has 0 keys"),
        (
            format!("{{\"keys\":[\"{key}\"],\"threshold\":2}}"),
            "has threshold 2 for 1 keys",
        ),
        (
            format!("{{\"keys\":[\"{key}\",\"{key}\"],\"threshold\":1}}"),
            "listed twice",
        ),
        (
            format!(
                "{{\"keys\":[\"{}\"],\"threshold\":1}}",
                &key[..key.len() - 4]
            ),
            "minisign public key",
        ),
        ("roots.json".to_owned(), "neither a root role"),
        (missing.display().to_string(), "cannot be read"),
    ];
    for (value, expected) in cases {
        let output = cargo_build(test, false, &[], Some(&value));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{value:?} builds");
        assert!(
            stderr.contains(BUILD_ENV) && stderr.contains(expected),
            "{value:?}: expected `{expected}` in {stderr}"
        );
    }
    // And the same target directory still builds a valid value.
    let exe = build(test, false, &[], Some(&a.roots()), "valid");
    assert_starts(&verify(&exe, &a, None), "roots=embedded");
}
