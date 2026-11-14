//! Spike 3: deny-by-default permission broker, unit level and end to end
//! (guest -> host_call import -> plugin host -> frame -> supervisor broker).

use std::time::Duration;

use runtime_spike::broker::{Broker, Decision, Grant};
use runtime_spike::supervisor::{PluginProcess, SpawnConfig};
use serde_json::{Value, json};

const HOST: &str = env!("CARGO_BIN_EXE_plugin-host");
const T: Duration = Duration::from_secs(10);

struct Fixture {
    _tmp: tempfile::TempDir,
    data: std::path::PathBuf,
    broker: Broker,
}

/// <tmp>/data            granted to plugin "a"
/// <tmp>/data/config.json
/// <tmp>/data/sub/note.txt
/// <tmp>/data-evil/secret.txt   (sibling with a common name prefix)
/// <tmp>/secret.txt
fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(data.join("sub")).unwrap();
    std::fs::create_dir_all(tmp.path().join("data-evil")).unwrap();
    std::fs::write(data.join("config.json"), r#"{"k":1}"#).unwrap();
    std::fs::write(data.join("sub/note.txt"), "note").unwrap();
    std::fs::write(tmp.path().join("secret.txt"), "SECRET").unwrap();
    std::fs::write(tmp.path().join("data-evil/secret.txt"), "SECRET2").unwrap();
    let mut broker = Broker::new();
    broker.grant("a", Grant::fs_read(&data).unwrap());
    broker.grant("a", Grant::http(&["https://api.example.com"]));
    Fixture {
        _tmp: tmp,
        data,
        broker,
    }
}

fn fs(path: &str) -> Value {
    json!({ "path": path })
}
fn http(url: &str) -> Value {
    json!({ "url": url })
}
fn denied(d: Decision) -> bool {
    matches!(d, Decision::Deny(_))
}

#[test]
fn unknown_plugin_and_unknown_op_are_denied() {
    let f = fixture();
    assert!(denied(f.broker.authorize(
        "b",
        "fs.read",
        &fs("config.json")
    )));
    assert!(denied(f.broker.authorize(
        "a",
        "process.exec",
        &json!({"cmd":"sh"})
    )));
    assert!(denied(f.broker.authorize(
        "a",
        "keystore.getDatabaseKey",
        &Value::Null
    )));
    assert!(denied(f.broker.authorize("", "", &Value::Null)));
}

#[test]
fn grant_of_one_kind_does_not_imply_another() {
    let mut b = Broker::new();
    b.grant("net-only", Grant::http(&["https://api.example.com"]));
    assert!(denied(b.authorize("net-only", "fs.read", &fs("x"))));
    let tmp = tempfile::tempdir().unwrap();
    b.grant("fs-only", Grant::fs_read(tmp.path()).unwrap());
    assert!(denied(b.authorize(
        "fs-only",
        "http.fetch",
        &http("https://api.example.com/")
    )));
}

#[test]
fn fs_read_inside_granted_directory_is_allowed() {
    let f = fixture();
    assert_eq!(
        f.broker.authorize("a", "fs.read", &fs("config.json")),
        Decision::Allow
    );
    assert_eq!(
        f.broker.authorize("a", "fs.read", &fs("sub/note.txt")),
        Decision::Allow
    );
    assert_eq!(
        f.broker.authorize("a", "fs.read", &fs("./sub/note.txt")),
        Decision::Allow
    );
    let v = f.broker.handle("a", "fs.read", &fs("config.json")).unwrap();
    assert_eq!(v["text"], r#"{"k":1}"#);
}

#[test]
fn fs_read_traversal_is_denied() {
    let f = fixture();
    for p in [
        "../secret.txt",
        "sub/../../secret.txt",
        "../data-evil/secret.txt",
        "sub/../config.json", // conservative: any `..` is rejected
        "..",
        "",
        "config.json\0",
    ] {
        assert!(denied(f.broker.authorize("a", "fs.read", &fs(p))), "{p:?}");
    }
    assert!(
        f.broker
            .handle("a", "fs.read", &fs("../secret.txt"))
            .is_err()
    );
}

#[test]
fn fs_read_absolute_paths_are_denied() {
    let f = fixture();
    let inside = f.data.join("config.json");
    for p in [
        inside.to_str().unwrap(),
        "/etc/passwd",
        "/",
        "C:\\Windows\\win.ini",
        "\\\\server\\share\\x",
    ] {
        // On Unix a Windows path is a (nonexistent) relative file name:
        // still denied, for a different reason.
        assert!(denied(f.broker.authorize("a", "fs.read", &fs(p))), "{p:?}");
    }
}

#[test]
fn fs_read_of_directory_or_missing_file_is_denied() {
    let f = fixture();
    assert!(denied(f.broker.authorize("a", "fs.read", &fs("sub"))));
    assert!(denied(f.broker.authorize("a", "fs.read", &fs("."))));
    assert!(denied(f.broker.authorize(
        "a",
        "fs.read",
        &fs("missing.txt")
    )));
    assert!(denied(f.broker.authorize(
        "a",
        "fs.read",
        &json!({"path": 5})
    )));
    assert!(denied(f.broker.authorize("a", "fs.read", &Value::Null)));
}

#[cfg(unix)]
#[test]
fn fs_read_symlink_escapes_are_denied() {
    use std::os::unix::fs::symlink;
    let f = fixture();
    let root = f.data.parent().unwrap();
    symlink(root.join("secret.txt"), f.data.join("link-file")).unwrap();
    symlink(root.join("data-evil"), f.data.join("link-dir")).unwrap();
    symlink("../secret.txt", f.data.join("sub/rel-link")).unwrap();
    symlink("/etc/passwd", f.data.join("passwd")).unwrap();
    for p in ["link-file", "link-dir/secret.txt", "sub/rel-link", "passwd"] {
        assert!(denied(f.broker.authorize("a", "fs.read", &fs(p))), "{p}");
        assert!(f.broker.handle("a", "fs.read", &fs(p)).is_err(), "{p}");
    }
    // A symlink that stays inside the granted directory is fine.
    symlink(f.data.join("sub/note.txt"), f.data.join("inside-link")).unwrap();
    assert_eq!(
        f.broker.authorize("a", "fs.read", &fs("inside-link")),
        Decision::Allow
    );
}

#[cfg(unix)]
#[test]
fn granted_root_reached_through_symlink_is_canonicalized() {
    // The grant stores the canonical root, so a symlinked grant path does
    // not make the comparison fail open or closed incorrectly.
    use std::os::unix::fs::symlink;
    let f = fixture();
    let alias = f.data.parent().unwrap().join("alias");
    symlink(&f.data, &alias).unwrap();
    let mut b = Broker::new();
    b.grant("c", Grant::fs_read(&alias).unwrap());
    assert_eq!(
        b.authorize("c", "fs.read", &fs("config.json")),
        Decision::Allow
    );
    assert!(denied(b.authorize("c", "fs.read", &fs("../secret.txt"))));
}

#[test]
fn http_fetch_only_to_allowlisted_origin() {
    let f = fixture();
    for u in [
        "https://api.example.com/",
        "https://api.example.com/v1/items?q=1",
        "https://API.EXAMPLE.COM/x",     // host is normalized
        "https://api.example.com:443/x", // explicit default port
    ] {
        assert_eq!(
            f.broker.authorize("a", "http.fetch", &http(u)),
            Decision::Allow,
            "{u}"
        );
    }
    for u in [
        "http://api.example.com/",           // scheme
        "https://api.example.com:8443/",     // port
        "https://example.com/",              // parent domain
        "https://evil.api.example.com/",     // subdomain
        "https://api.example.com.evil.net/", // suffix trick
        "https://api.example.com@evil.net/", // userinfo trick: host is evil.net
        "https://user:pw@api.example.com/",  // userinfo on the right host
        "https://evil.net/?u=https://api.example.com/",
        "ftp://api.example.com/",
        "file:///etc/passwd",
        "https://127.0.0.1/",
        "not a url",
        "",
    ] {
        assert!(
            denied(f.broker.authorize("a", "http.fetch", &http(u))),
            "{u}"
        );
    }
    assert!(denied(f.broker.authorize(
        "a",
        "http.fetch",
        &json!({"href":"https://api.example.com/"})
    )));
}

#[test]
fn revocation_takes_effect_on_next_call() {
    let mut f = fixture();
    assert_eq!(
        f.broker.authorize("a", "fs.read", &fs("config.json")),
        Decision::Allow
    );
    f.broker.revoke_all("a");
    assert!(denied(f.broker.authorize(
        "a",
        "fs.read",
        &fs("config.json")
    )));
}

#[test]
fn audit_records_decisions_without_values() {
    let f = fixture();
    f.broker.handle("a", "fs.read", &fs("config.json")).unwrap();
    let _ = f.broker.handle(
        "a",
        "http.fetch",
        &json!({"url":"https://evil.net/?token=abc", "body":"secret-body"}),
    );
    let audit = f.broker.audit();
    assert_eq!(audit.len(), 2);
    assert!(audit[0].allowed);
    assert!(!audit[1].allowed);
    assert_eq!(audit[1].target, "evil.net");
    let dump = format!("{audit:?}");
    assert!(!dump.contains("abc") && !dump.contains("secret-body") && !dump.contains(r#"{"k":1}"#));
}

// ---- End to end through the guest, the plugin host and the supervisor ----

fn spawn(plugin: &str) -> PluginProcess {
    PluginProcess::spawn(SpawnConfig::new(HOST, plugin)).unwrap()
}

fn relay(p: &mut PluginProcess, b: &Broker, req: Value) -> Value {
    let out = p.invoke(b, "relay", Some(&req.to_string()), T).unwrap();
    assert_eq!(out.calls, 1);
    out.result.unwrap()
}

#[test]
fn e2e_guest_request_is_allowed_or_denied_by_broker() {
    let f = fixture();
    let mut a = spawn("a");
    let ok = relay(
        &mut a,
        &f.broker,
        json!({"op":"fs.read","args":{"path":"config.json"}}),
    );
    assert_eq!(ok["ok"]["text"], r#"{"k":1}"#);
    let no = relay(
        &mut a,
        &f.broker,
        json!({"op":"fs.read","args":{"path":"../secret.txt"}}),
    );
    assert!(no["error"].as_str().unwrap().starts_with("not-permitted"));
    let no = relay(
        &mut a,
        &f.broker,
        json!({"op":"http.fetch","args":{"url":"https://evil.net/"}}),
    );
    assert!(no["error"].as_str().unwrap().starts_with("not-permitted"));
    let yes = relay(
        &mut a,
        &f.broker,
        json!({"op":"http.fetch","args":{"url":"https://api.example.com/x"}}),
    );
    assert_eq!(yes["ok"]["notExecuted"], true);
}

#[test]
fn e2e_identity_comes_from_the_channel_not_the_request() {
    let f = fixture();
    // Process "b" has no grants. Claiming to be "a" inside the request or
    // the args changes nothing.
    let mut b = spawn("b");
    for req in [
        json!({"op":"fs.read","args":{"path":"config.json"}}),
        json!({"op":"fs.read","plugin":"a","args":{"path":"config.json","plugin":"a"}}),
    ] {
        let r = relay(&mut b, &f.broker, req);
        assert!(r["error"].as_str().unwrap().contains("no grants"), "{r}");
    }
    assert!(f.broker.audit().iter().all(|e| e.plugin == "b"));
}

#[test]
fn e2e_malformed_guest_request_never_reaches_broker() {
    let f = fixture();
    let mut a = spawn("a");
    let out = a.invoke(&f.broker, "relay", Some("not json"), T).unwrap();
    assert_eq!(out.calls, 0);
    assert_eq!(out.result.unwrap()["error"], "bad request");
    assert!(f.broker.audit().is_empty());
}

#[test]
fn e2e_many_calls_in_one_invocation() {
    let f = fixture();
    let mut a = spawn("a");
    let req = json!({"op":"fs.read","args":{"path":"sub/note.txt"}}).to_string();
    let out = a.invoke(&f.broker, "relay_n:200", Some(&req), T).unwrap();
    assert_eq!(out.calls, 200);
    assert_eq!(f.broker.audit().len(), 200);
}
