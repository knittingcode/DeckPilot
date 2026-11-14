//! The documents this crate reads and the test registry writes agree with
//! the #191 JSON Schemas bundled in `schema/`, and documents the schemas
//! refuse are refused by the Rust model too.

mod common;

use common::*;
use deckpilot_plugin_registry::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn parses<T: DeserializeOwned>(bytes: &[u8], validate: fn(&T) -> Result<(), String>) -> bool {
    serde_json::from_slice::<T>(bytes)
        .map_err(|e| e.to_string())
        .and_then(|doc| validate(&doc))
        .is_ok()
}

fn keyset_ok(bytes: &[u8]) -> bool {
    parses::<Keyset>(bytes, Keyset::validate)
}
fn timestamp_ok(bytes: &[u8]) -> bool {
    parses::<Timestamp>(bytes, Timestamp::validate)
}
fn index_ok(bytes: &[u8]) -> bool {
    parses::<Index>(bytes, Index::validate)
}
fn detail_ok(bytes: &[u8]) -> bool {
    parses::<PluginDetail>(bytes, PluginDetail::validate)
}

/// A registry that has used every optional feature once.
fn full_registry() -> Registry {
    let mut registry = Registry::new();
    let mut spec = plugin("acme.github-prs", &["1.0.0", "1.1.0", "1.2.0"]);
    spec.versions[2].yanked = Some("crashes on start".into());
    spec.versions[1].platforms = Some(vec![deckpilot_plugin_manifest::Platform::Linux]);
    let mut old = plugin("acme.clock", &["0.1.0"]);
    old.deprecated = true;
    registry.publish(
        t0(),
        &[spec, old],
        &[
            revocation(
                "acme.github-prs",
                Some(&["1.0.0"]),
                RevocationAction::Disable,
            ),
            revocation("acme.clock", None, RevocationAction::Yank),
        ],
    );
    let root = registry.root_role();
    let doc = registry.keyset_doc(&root, 1, true);
    registry.publish_keyset(&doc, &[&registry.roots[0]]);
    registry.publish(
        t0() + days(1),
        &[plugin("acme.clock", &["0.1.0"])],
        &[
            revocation(
                "acme.github-prs",
                Some(&["1.0.0"]),
                RevocationAction::Disable,
            ),
            revocation("acme.clock", None, RevocationAction::Yank),
        ],
    );
    registry
}

#[test]
fn every_published_document_matches_its_schema() {
    let registry = full_registry();
    let keyset = validator(schemas::KEYSET);
    let timestamp = validator(schemas::TIMESTAMP);
    let index = validator(schemas::INDEX);
    let detail = validator(schemas::PLUGIN);
    let published = registry.published.borrow();
    let mut seen = [0; 4];
    for (path, bytes) in published.iter() {
        let (schema, ours, slot) = if path.starts_with("keys/") {
            (&keyset, keyset_ok(bytes), 0)
        } else if path == "timestamp.json" {
            (&timestamp, timestamp_ok(bytes), 1)
        } else if path.starts_with("index/") {
            (&index, index_ok(bytes), 2)
        } else {
            (&detail, detail_ok(bytes), 3)
        };
        assert_eq!(schema_errors(schema, bytes), Vec::<String>::new(), "{path}");
        assert!(ours, "{path} is refused by the model");
        seen[slot] += 1;
    }
    assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
    // And the client accepts the whole chain.
    let mut c = client(&registry);
    assert_eq!(
        c.refresh(&registry.site, t0() + days(1)),
        Ok(Refresh::Updated { seq: 2 })
    );
}

#[test]
fn the_key_set_signature_file_is_an_array() {
    let registry = setup_simple();
    let signatures: Value =
        serde_json::from_slice(&registry.site.get("keys/1.json.minisig")).unwrap();
    assert!(signatures.is_array());
}

fn setup_simple() -> Registry {
    let mut registry = Registry::new();
    registry.publish(t0(), &[plugin("acme.clock", &["1.0.0"])], &[]);
    registry
}

fn mutated(bytes: &[u8], pointer: &str, value: Option<Value>) -> Vec<u8> {
    let mut doc: Value = serde_json::from_slice(bytes).unwrap();
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let object = doc.pointer_mut(parent).unwrap();
    match (object, value) {
        (Value::Object(map), Some(value)) => {
            map.insert(key.to_owned(), value);
        }
        (Value::Object(map), None) => {
            map.remove(key).unwrap();
        }
        (Value::Array(list), Some(value)) => list[key.parse::<usize>().unwrap()] = value,
        (other, _) => panic!("cannot mutate {pointer} in {other}"),
    }
    serde_json::to_vec(&doc).unwrap()
}

type Case = (&'static str, Option<Value>);

fn both_refuse(name: &str, bytes: &[u8], schema: &str, ours: fn(&[u8]) -> bool, cases: &[Case]) {
    let validator = validator(schema);
    assert!(
        schema_errors(&validator, bytes).is_empty() && ours(bytes),
        "{name} base"
    );
    for (pointer, value) in cases {
        let bad = mutated(bytes, pointer, value.clone());
        assert!(
            !schema_errors(&validator, &bad).is_empty(),
            "{name} {pointer} = {value:?}: the schema accepts it"
        );
        assert!(
            !ours(&bad),
            "{name} {pointer} = {value:?}: the model accepts it"
        );
    }
}

#[test]
fn key_sets_refused_by_the_schema_are_refused() {
    let registry = setup_simple();
    let other = ThrowawayKey::generate();
    both_refuse(
        "keys/1.json",
        &registry.site.get("keys/1.json"),
        schemas::KEYSET,
        keyset_ok,
        &[
            ("/_type", Some(json!("deckpilot-registry-index"))),
            ("/spec_version", Some(json!(2))),
            ("/version", Some(json!(0))),
            ("/extra", Some(json!(true))),
            ("/seq_floor", None),
            ("/seq_floor", Some(json!(-1))),
            ("/root/threshold", Some(json!(0))),
            ("/root/threshold", Some(json!(5))),
            ("/root/keys", Some(json!([]))),
            ("/index/keys", Some(json!(["short"]))),
            ("/index/keys", Some(json!([other.public(), other.public()]))),
            (
                "/index/keys",
                Some(json!([
                    other.public(),
                    other.public(),
                    other.public(),
                    other.public(),
                    other.public()
                ])),
            ),
            ("/timestamp/extra", Some(json!(1))),
            ("/seq_floor_reset", Some(json!("yes"))),
        ],
    );
}

#[test]
fn timestamps_refused_by_the_schema_are_refused() {
    let registry = setup_simple();
    both_refuse(
        "timestamp.json",
        &registry.site.get("timestamp.json"),
        schemas::TIMESTAMP,
        timestamp_ok,
        &[
            ("/_type", Some(json!("deckpilot-registry-keys"))),
            ("/seq", Some(json!(0))),
            ("/keys_version", Some(json!(0))),
            ("/signed_at", Some(json!("yesterday"))),
            ("/expires", None),
            ("/index/path", Some(json!("index/01.json"))),
            ("/index/path", Some(json!("../index/1.json"))),
            ("/index/sha256", Some(json!("ABC"))),
            ("/index/length", Some(json!(0))),
            ("/index/length", Some(json!(4_194_305))),
            ("/index/extra", Some(json!(1))),
            ("/extra", Some(json!(1))),
        ],
    );
}

#[test]
fn indexes_refused_by_the_schema_are_refused() {
    let registry = full_registry();
    both_refuse(
        "index/1.json",
        &registry.site.get("index/1.json"),
        schemas::INDEX,
        index_ok,
        &[
            ("/_type", Some(json!("deckpilot-registry-timestamp"))),
            ("/download_prefix", Some(json!("https://evil.example/"))),
            ("/source_commit", Some(json!("abc"))),
            ("/generated_at", Some(json!(12))),
            ("/min_app_version", Some(json!("one"))),
            ("/publishers/0/namespace", Some(json!("Acme"))),
            ("/publishers/0/login", Some(json!("bad login"))),
            ("/publishers/0/github_id", Some(json!(0))),
            ("/plugins/0/id", Some(json!("Acme.Clock"))),
            ("/plugins/0/name", Some(json!(""))),
            ("/plugins/0/summary", Some(json!("x".repeat(301)))),
            ("/plugins/0/categories", Some(json!(["a", "b", "c", "d"]))),
            ("/plugins/0/categories", Some(json!(["Tools"]))),
            ("/plugins/0/latest", Some(json!({}))),
            ("/plugins/0/latest/nightly", Some(json!("1.0.0"))),
            ("/plugins/0/detail/path", Some(json!("../etc/passwd"))),
            ("/plugins/0/detail/length", Some(json!(262_145))),
            ("/plugins/1/deprecated/reason", Some(json!(""))),
            ("/revocations/0/action", Some(json!("delete"))),
            ("/revocations/0/versions", Some(json!([]))),
            ("/revocations/0/versions", Some(json!("all"))),
            ("/revocations/0/versions", Some(json!(["1.0.0", "1.0.0"]))),
            ("/revocations/0/advisory", Some(json!("http://example.com"))),
            ("/revocations/0/reason", None),
            ("/extra", Some(json!(1))),
        ],
    );
}

#[test]
fn details_refused_by_the_schema_are_refused() {
    let registry = full_registry();
    let path = registry
        .published
        .borrow()
        .iter()
        .find(|(p, _)| p.starts_with("plugins/acme.github-prs/"))
        .unwrap()
        .0
        .clone();
    both_refuse(
        &path,
        &registry.site.get(&path),
        schemas::PLUGIN,
        detail_ok,
        &[
            ("/_type", Some(json!("deckpilot-registry-index"))),
            ("/id", Some(json!("acme"))),
            ("/license", Some(json!(""))),
            ("/homepage", Some(json!("http://acme.example"))),
            ("/source_repository", Some(json!("no-slash"))),
            ("/versions", Some(json!([]))),
            ("/versions/0/version", Some(json!("1.0"))),
            ("/versions/0/engines/deckpilot", None),
            ("/versions/0/engines/extra", Some(json!(1))),
            ("/versions/0/platforms", Some(json!(["amiga"]))),
            (
                "/versions/0/permissions/0",
                Some(json!({"kind": "filesystem"})),
            ),
            ("/versions/0/manifest_sha256", Some(json!("0"))),
            ("/versions/0/source/commit", Some(json!("main"))),
            ("/versions/0/source/tag", Some(json!(""))),
            (
                "/versions/0/package/url",
                Some(json!("http://github.com/x.dpplugin")),
            ),
            (
                "/versions/0/package/url",
                Some(json!(
                    "https://github.com/evil/deckpilot-plugins/releases/download/acme.github-prs--v1.0.0/a.dpplugin"
                )),
            ),
            ("/versions/0/package/size", Some(json!(0))),
            ("/versions/0/package/size", Some(json!(5_242_881))),
            ("/versions/0/builder", Some(json!("latest"))),
            ("/versions/0/attestation", Some(json!("ftp://x"))),
            ("/versions/2/yanked/reason", Some(json!(""))),
            ("/versions/0/extra", Some(json!(1))),
        ],
    );
}

#[test]
fn the_bundled_schemas_are_the_191_ones() {
    // Byte-identical copies; the README records the source commit. Here:
    // they parse, are draft 2020-12 and carry the registry `$id`s.
    for (name, text) in [
        ("registry-keyset", schemas::KEYSET),
        ("registry-timestamp", schemas::TIMESTAMP),
        ("registry-index", schemas::INDEX),
        ("registry-plugin", schemas::PLUGIN),
    ] {
        let schema: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(
            schema["$id"],
            format!(
                "https://knittingcode.github.io/deckpilot-plugins/v1/schemas/{name}.schema.json"
            )
        );
    }
}
