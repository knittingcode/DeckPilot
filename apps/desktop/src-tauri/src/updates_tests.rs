//! The update flow of the app against a loopback update server: the real
//! Tauri updater checks, downloads and verifies, the app's state handling
//! and messages are checked, and installation replaces a throwaway copy of
//! an installed app.
//!
//! Every key here is generated for the test and never stored; the release
//! signing key is not involved. Nothing outside a temporary directory is
//! installed or replaced.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use tauri::{AppHandle, Manager};

use deckpilot_ipc::AgentPaths;
use deckpilot_presence::Unavailable;

use deckpilot_update::channel::Channel;

use super::{Endpoint, Phase, Source, UPDATE_LOG, UpdateManager, check_from, download, may_start};
use crate::state::AppState;

/// The announced version: newer than any real one.
const VERSION: &str = "99.0.0";

/// What the server answers for a path.
#[derive(Clone)]
enum Reply {
    Ok(Vec<u8>),
    /// Announces the full length, sends half and closes the connection.
    Truncated(Vec<u8>),
    /// Sends the headers and half of the body, then nothing for
    /// `STALL` (far longer than the short read timeout of the tests that
    /// use it).
    Stalled(Vec<u8>),
    Status(u16),
}

/// How long a stalled reply sends nothing.
const STALL: Duration = Duration::from_secs(30);
/// Read timeout of the stalled cases: a stall must end the request well
/// before `STALL`.
const SHORT_READ_TIMEOUT: Duration = Duration::from_secs(2);
/// Timeouts of every other request: generous, so that a slow, busy CI
/// runner (Windows) never turns a normal response into a timeout.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// A minimal HTTP/1.1 server on 127.0.0.1 with replies that can be changed
/// between requests. It is listening when `start` returns, serves every
/// connection on its own thread (a stalled one never blocks the next), and
/// records what it saw for failure messages.
struct Server {
    port: u16,
    replies: Arc<Mutex<HashMap<String, Reply>>>,
    events: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let replies: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let (shared, seen) = (replies.clone(), events.clone());
        std::thread::spawn(move || {
            // Keeps accepting after failed accepts too.
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let (replies, seen) = (shared.clone(), seen.clone());
                        std::thread::spawn(move || serve(stream, &replies, &seen));
                    }
                    Err(error) => record(&seen, &format!("accept failed: {error}")),
                }
            }
        });
        Self {
            port,
            replies,
            events,
        }
    }

    /// Everything the server saw so far, one line per event.
    fn seen(&self) -> String {
        self.events.lock().unwrap().join("\n")
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/{path}", self.port)
    }

    fn reply(&self, path: &str, reply: Reply) {
        self.replies
            .lock()
            .unwrap()
            .insert(format!("/{path}"), reply);
    }
}

fn record(events: &Mutex<Vec<String>>, event: &str) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    events.lock().unwrap().push(format!("{at} {event}"));
}

fn serve(
    mut stream: TcpStream,
    replies: &Mutex<HashMap<String, Reply>>,
    events: &Mutex<Vec<String>>,
) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) => {
                record(events, "connection closed before a complete request");
                return;
            }
            Err(error) => {
                record(events, &format!("reading the request failed: {error}"));
                return;
            }
            Ok(n) => request.extend_from_slice(&buffer[..n]),
        }
    }
    let text = String::from_utf8_lossy(&request);
    let path = text.split(' ').nth(1).unwrap_or_default().to_owned();
    let reply = replies
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap_or(Reply::Status(404));
    let kind = match &reply {
        Reply::Ok(_) => "ok".to_owned(),
        Reply::Truncated(_) => "truncated".to_owned(),
        Reply::Stalled(_) => "stalled".to_owned(),
        Reply::Status(code) => format!("status {code}"),
    };
    record(events, &format!("GET {path}: {kind}"));
    let head = |status: &str, length: usize| {
        format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
    };
    let result = match reply {
        Reply::Ok(body) => stream
            .write_all(head("200 OK", body.len()).as_bytes())
            .and_then(|()| stream.write_all(&body)),
        Reply::Truncated(body) => stream
            .write_all(head("200 OK", body.len()).as_bytes())
            .and_then(|()| stream.write_all(&body[..body.len() / 2])),
        Reply::Stalled(body) => {
            let sent = stream
                .write_all(head("200 OK", body.len()).as_bytes())
                .and_then(|()| stream.write_all(&body[..body.len() / 2]))
                .and_then(|()| stream.flush());
            std::thread::sleep(STALL);
            sent
        }
        Reply::Status(code) => stream.write_all(head(&format!("{code} Failure"), 0).as_bytes()),
    };
    match result {
        Ok(()) => record(events, &format!("GET {path}: {kind} sent")),
        Err(error) => record(
            events,
            &format!("GET {path}: {kind}, writing failed: {error}"),
        ),
    }
}

/// A minisign key pair made for one test.
struct Keys {
    public: String,
    secret: minisign::SecretKey,
}

fn keys() -> Keys {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    Keys {
        public: STANDARD.encode(pair.pk.to_box().unwrap().to_string()),
        secret: pair.sk,
    }
}

/// A signature as the Tauri CLI writes it: base64 of the minisign file,
/// the version in the signed comment.
fn sign(keys: &Keys, data: &[u8], version: &str) -> String {
    let comment = format!("timestamp:1\tfile:DeckPilot.app.tar.gz\tversion:{version}");
    let signature = minisign::sign(
        None,
        &keys.secret,
        std::io::Cursor::new(data),
        Some(&comment),
        None,
    )
    .unwrap();
    STANDARD.encode(signature.to_string())
}

fn manifest(server: &Server, signature: &str) -> Vec<u8> {
    manifest_of(server, signature, VERSION)
}

fn manifest_of(server: &Server, signature: &str, version: &str) -> Vec<u8> {
    let platform = json!({ "url": server.url("package"), "signature": signature });
    let target = deckpilot_update::manifest::current_target();
    serde_json::to_vec(&json!({
        "version": version,
        "notes": "Test update",
        "pub_date": "2026-11-12T00:00:00Z",
        "platforms": {
            target.clone(): platform,
            format!("{target}-app"): platform,
            format!("{target}-appimage"): platform,
        }
    }))
    .unwrap()
}

struct Harness {
    app: tauri::App<MockRuntime>,
    paths: AgentPaths,
    server: Server,
    keys: Keys,
    source: Source,
}

impl Harness {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-update-test-{}",
            deckpilot_core::ProfileId::new()
        ));
        let paths = AgentPaths::in_dir(dir.clone());
        let mut context = mock_context(noop_assets());
        // As tauri.conf.json, but plain HTTP for the loopback server.
        context.config_mut().plugins.0.insert(
            "updater".into(),
            json!({
                "pubkey": "",
                "requireSignedVersion": true,
                "dangerousInsecureTransportProtocol": true
            }),
        );
        let app = mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .manage(AppState::with_presence(paths.clone(), Arc::new(Unavailable)).unwrap())
            .manage(UpdateManager::default())
            .build(context)
            .unwrap();
        let server = Server::start();
        let keys = keys();
        let source = Source {
            key: keys.public.clone(),
            endpoint: Endpoint::Manifest(server.url("latest.json")),
            channel: Channel::Stable,
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: READ_TIMEOUT,
            executable: Some(dir.join("DeckPilot.app/Contents/MacOS/deckpilot-desktop")),
        };
        Self {
            app,
            paths,
            server,
            keys,
            source,
        }
    }

    fn handle(&self) -> &AppHandle<MockRuntime> {
        self.app.handle()
    }

    /// Serves `package` with `signature` as the update.
    fn offer(&self, package: &[u8], signature: &str) {
        self.server
            .reply("latest.json", Reply::Ok(manifest(&self.server, signature)));
        self.server.reply("package", Reply::Ok(package.to_vec()));
    }

    fn check(&self) -> Result<Option<String>, String> {
        tauri::async_runtime::block_on(check_from(self.handle(), &self.source))
            .map(|info| info.map(|info| info.version))
            .map_err(|e| e.to_string())
    }

    /// Checks with the short read timeout of the stalled cases (the
    /// update found carries the timeouts of the check into its download).
    fn check_with_short_timeout(&mut self) -> Result<Option<String>, String> {
        self.source.read_timeout = SHORT_READ_TIMEOUT;
        let result = self.check();
        self.source.read_timeout = READ_TIMEOUT;
        result
    }

    fn download(&self) -> Result<(), String> {
        tauri::async_runtime::block_on(download(self.handle())).map_err(|e| e.to_string())
    }

    /// A download that must succeed; otherwise fails with what the server
    /// saw.
    fn download_ok(&self) {
        if let Err(error) = self.download() {
            panic!("{error}\nthe test server saw:\n{}", self.server.seen());
        }
    }

    fn phase(&self) -> Phase {
        self.handle().state::<UpdateManager>().lock().phase
    }

    fn shown_error(&self) -> Option<String> {
        self.handle().state::<UpdateManager>().lock().error.clone()
    }

    fn has_package(&self) -> bool {
        self.handle()
            .state::<UpdateManager>()
            .lock()
            .bytes
            .is_some()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.paths.log_dir.join(UPDATE_LOG)).unwrap_or_default()
    }

    /// The failure left the updater idle, showing `message`, with no
    /// package kept, and ready for the next attempt.
    fn assert_failed_cleanly(&self, error: &str, expected: &str) {
        assert!(
            error.contains(expected),
            "{error}\nexpected: {expected}\nthe test server saw:\n{}",
            self.server.seen()
        );
        assert_eq!(self.phase(), Phase::Idle);
        assert!(may_start(self.phase()));
        assert_eq!(self.shown_error().as_deref(), Some(error));
        assert!(!self.has_package());
        assert!(self.log().contains(error), "not logged: {}", self.log());
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.paths.data_dir);
    }
}

#[test]
fn a_correctly_signed_update_is_downloaded_and_verified() {
    let h = Harness::new();
    let package = b"DeckPilot update package".repeat(1000);
    h.offer(&package, &sign(&h.keys, &package, VERSION));

    assert_eq!(h.check(), Ok(Some(VERSION.to_owned())));
    assert_eq!(h.phase(), Phase::Idle);
    h.download_ok();
    assert_eq!(h.phase(), Phase::Ready);
    assert_eq!(h.shown_error(), None);
    assert_eq!(
        h.handle().state::<UpdateManager>().lock().bytes.as_deref(),
        Some(&package[..])
    );
    assert!(h.log().contains("signature verified"), "{}", h.log());
}

/// The prerelease channel reads GitHub's release list from the server.
fn prerelease_channel(h: &mut Harness) {
    h.source.channel = Channel::Prerelease;
    h.source.endpoint = Endpoint::Releases {
        list: h.server.url("releases"),
        downloads: h.server.url(""),
    };
}

fn releases(list: &[(&str, bool, bool)]) -> Reply {
    let list: Vec<_> = list
        .iter()
        .map(|(tag, draft, prerelease)| json!({"tag_name": tag, "draft": draft, "prerelease": prerelease}))
        .collect();
    Reply::Ok(serde_json::to_vec(&list).unwrap())
}

#[test]
fn the_stable_channel_never_installs_a_pre_release() {
    let h = Harness::new();
    let package = b"pre-release package".to_vec();
    // Even if a pre-release manifest were served as latest.json.
    let pre = "99.0.0-beta.1";
    h.server.reply(
        "latest.json",
        Reply::Ok(manifest_of(&h.server, &sign(&h.keys, &package, pre), pre)),
    );
    h.server.reply("package", Reply::Ok(package));
    assert_eq!(h.check(), Ok(None));
    assert_eq!(h.phase(), Phase::Idle);
}

#[test]
fn the_prerelease_channel_offers_the_newest_release_and_moves_on_to_stable() {
    let mut h = Harness::new();
    prerelease_channel(&mut h);
    let package = b"pre-release package".to_vec();
    let alpha = "99.0.0-alpha.2";
    h.server.reply(
        "releases",
        releases(&[
            ("v99.0.0-beta.1", true, true), // a draft: never offered
            ("v99.0.0-alpha.2", false, true),
            ("v99.0.0-alpha.1", false, true),
            ("v0.5.0-2", true, true),
        ]),
    );
    h.server.reply(
        "v99.0.0-alpha.2/prerelease.json",
        Reply::Ok(manifest_of(
            &h.server,
            &sign(&h.keys, &package, alpha),
            alpha,
        )),
    );
    h.server.reply("package", Reply::Ok(package.clone()));
    assert_eq!(h.check(), Ok(Some(alpha.to_owned())));
    h.download_ok();
    assert_eq!(h.phase(), Phase::Ready);

    // Once the stable release is out, it is what the channel offers.
    h.server.reply(
        "releases",
        releases(&[("v99.0.0", false, false), ("v99.0.0-alpha.2", false, true)]),
    );
    h.server.reply(
        "v99.0.0/latest.json",
        Reply::Ok(manifest(&h.server, &sign(&h.keys, &package, VERSION))),
    );
    assert_eq!(h.check(), Ok(Some(VERSION.to_owned())));

    // Nothing newer than the running version: nothing is offered and no
    // manifest is read.
    h.server
        .reply("releases", releases(&[("v0.0.1", false, false)]));
    h.server.reply("v0.0.1/latest.json", Reply::Status(500));
    assert_eq!(h.check(), Ok(None));
    assert_eq!(h.shown_error(), None);
}

#[test]
fn an_unreadable_release_list_fails_the_check_cleanly() {
    let mut h = Harness::new();
    prerelease_channel(&mut h);
    for reply in [
        Reply::Status(403),
        Reply::Ok(b"<html>Sign in to the network</html>".to_vec()),
    ] {
        h.server.reply("releases", reply);
        let error = h.check().unwrap_err();
        assert!(
            error.contains("list of DeckPilot releases on GitHub could not be read"),
            "{error}"
        );
        assert_eq!(h.phase(), Phase::Idle);
        assert!(h.log().contains(&error));
    }
}

#[test]
fn corrupted_or_foreign_updates_are_rejected() {
    let h = Harness::new();
    let package = b"DeckPilot update package".repeat(1000);
    let mut tampered = package.clone();
    tampered[10] ^= 0x01;
    let other = keys();
    let cases: [(&str, Vec<u8>, String); 5] = [
        ("tampered", tampered, sign(&h.keys, &package, VERSION)),
        (
            "truncated file",
            package[..package.len() / 2].to_vec(),
            sign(&h.keys, &package, VERSION),
        ),
        ("empty", Vec::new(), sign(&h.keys, &package, VERSION)),
        (
            "signed with another key",
            package.clone(),
            sign(&other, &package, VERSION),
        ),
        (
            "older release announced as newer",
            package.clone(),
            sign(&h.keys, &package, "98.0.0"),
        ),
    ];
    for (case, served, signature) in cases {
        h.offer(&served, &signature);
        assert_eq!(h.check(), Ok(Some(VERSION.to_owned())), "{case}");
        let error = h.download().expect_err(case);
        h.assert_failed_cleanly(&error, "failed signature verification");
        // Still offered: the user can try again.
        assert!(h.handle().state::<UpdateManager>().lock().update.is_some());
    }
}

#[test]
fn a_malformed_signature_is_rejected() {
    let h = Harness::new();
    h.offer(b"package", &STANDARD.encode("not a minisign signature"));
    h.check().unwrap();
    let error = h.download().unwrap_err();
    h.assert_failed_cleanly(&error, "failed signature verification");
}

#[test]
fn interrupted_downloads_fail_cleanly_and_can_be_retried() {
    let mut h = Harness::new();
    let package = vec![7u8; 256 * 1024];
    let signature = sign(&h.keys, &package, VERSION);
    h.offer(&package, &signature);
    h.check().unwrap();

    // The connection closes before the announced length arrived.
    h.server.reply("package", Reply::Truncated(package.clone()));
    let error = h.download().unwrap_err();
    h.assert_failed_cleanly(&error, "update download failed");

    // The server stops sending: the read timeout ends the download instead
    // of keeping the updater busy forever.
    h.check_with_short_timeout().unwrap();
    h.server.reply("package", Reply::Stalled(package.clone()));
    let started = Instant::now();
    let error = h.download().unwrap_err();
    assert!(
        started.elapsed() < STALL / 2,
        "{:?}\nthe test server saw:\n{}",
        started.elapsed(),
        h.server.seen()
    );
    h.assert_failed_cleanly(&error, "stopped responding");

    // The server refuses (checked again with the normal timeouts).
    h.check().unwrap();
    h.server.reply("package", Reply::Status(503));
    let error = h.download().unwrap_err();
    h.assert_failed_cleanly(&error, "could not be reached or refused");

    // Nothing partial was kept: the next attempt starts over and succeeds.
    h.server.reply("package", Reply::Ok(package.clone()));
    h.download_ok();
    assert_eq!(h.phase(), Phase::Ready);
    assert_eq!(
        h.handle().state::<UpdateManager>().lock().bytes.as_deref(),
        Some(&package[..])
    );
}

#[test]
fn failed_checks_leave_the_updater_usable() {
    let mut h = Harness::new();
    let cases = [
        (
            Reply::Ok(b"<html>Sign in to the network</html>".to_vec()),
            "update check failed",
        ),
        (Reply::Status(500), "update check failed"),
        (
            Reply::Stalled(b"{\"version\": \"99.0.0\"}".to_vec()),
            "stopped responding",
        ),
    ];
    for (reply, expected) in cases {
        let stalled = matches!(reply, Reply::Stalled(_));
        h.server.reply("latest.json", reply);
        let error = if stalled {
            h.check_with_short_timeout().unwrap_err()
        } else {
            h.check().unwrap_err()
        };
        assert!(
            error.contains(expected),
            "{error}\nexpected: {expected}\nthe test server saw:\n{}",
            h.server.seen()
        );
        assert_eq!(h.phase(), Phase::Idle);
        assert_eq!(h.shown_error().as_deref(), Some(&error[..]));
        assert!(h.log().contains(&error));
    }
    // Afterwards a check works.
    let package = b"package".to_vec();
    h.offer(&package, &sign(&h.keys, &package, VERSION));
    assert_eq!(h.check(), Ok(Some(VERSION.to_owned())));
    assert_eq!(h.shown_error(), None);
}

#[test]
fn installing_needs_a_verified_download() {
    let h = Harness::new();
    let error = tauri::async_runtime::block_on(super::replace(h.handle())).unwrap_err();
    assert!(error.to_string().contains("download the update first"));
    assert_eq!(h.phase(), Phase::Idle);
}

/// Installation into a throwaway copy of an installed DeckPilot. Windows
/// installs through the system installer, which is not run in tests.
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod install {
    use super::*;
    use crate::updates::replace;
    use std::path::{Path, PathBuf};

    use deckpilot_agent::{Agent, AgentOptions, InstanceLock};
    use deckpilot_device::{mock::MockBackend, models};

    /// The installed app with `content` in the app and the agent: an app
    /// bundle on macOS, an `AppImage` file on Linux.
    fn install_app(h: &Harness, content: &str) -> PathBuf {
        let executable = h.source.executable.clone().unwrap();
        if cfg!(target_os = "macos") {
            let dir = executable.parent().unwrap();
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(&executable, content).unwrap();
            std::fs::write(dir.join("deckpilot-agent"), content).unwrap();
            executable
        } else {
            let appimage = h.paths.data_dir.join("DeckPilot.AppImage");
            std::fs::create_dir_all(&h.paths.data_dir).unwrap();
            std::fs::write(&appimage, appimage_bytes(content)).unwrap();
            appimage
        }
    }

    /// An `AppImage` is an ELF executable; the updater checks the header.
    fn appimage_bytes(content: &str) -> Vec<u8> {
        let mut bytes = b"\x7fELF".to_vec();
        bytes.resize(64, 0);
        bytes.extend_from_slice(content.as_bytes());
        bytes
    }

    /// The content of the installed app and agent.
    fn installed(executable: &Path) -> (String, String) {
        if cfg!(target_os = "macos") {
            let agent = executable.parent().unwrap().join("deckpilot-agent");
            (
                std::fs::read_to_string(executable).unwrap(),
                std::fs::read_to_string(agent).unwrap(),
            )
        } else {
            let bytes = std::fs::read(executable).unwrap();
            let content = String::from_utf8_lossy(&bytes[64..]).into_owned();
            (content.clone(), content)
        }
    }

    /// The update package for `content`, as the bundler makes it.
    fn package(h: &Harness, content: &str) -> Vec<u8> {
        if cfg!(target_os = "macos") {
            let staging = h.paths.data_dir.join("staging");
            let dir = staging.join("DeckPilot.app/Contents/MacOS");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("deckpilot-desktop"), content).unwrap();
            std::fs::write(dir.join("deckpilot-agent"), content).unwrap();
            let archive = h.paths.data_dir.join("DeckPilot.app.tar.gz");
            // Without AppleDouble (`._*`) entries for extended attributes,
            // which the bundler's archives do not have either.
            let status = std::process::Command::new("tar")
                .env("COPYFILE_DISABLE", "1")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(&staging)
                .arg("DeckPilot.app")
                .status()
                .unwrap();
            assert!(status.success());
            std::fs::read(archive).unwrap()
        } else {
            appimage_bytes(content)
        }
    }

    /// Checks and downloads `package` from the loopback server.
    fn downloaded(h: &mut Harness, executable: PathBuf, package: &[u8]) {
        h.source.executable = Some(executable);
        h.offer(package, &sign(&h.keys, package, VERSION));
        h.check().unwrap();
        h.download_ok();
    }

    /// A real agent with a simulated device on its own runtime.
    fn start_agent(paths: &AgentPaths) -> tokio::runtime::Runtime {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let mock = MockBackend::new();
        mock.plug(&models::stream_deck_mk2(), "TEST-0001");
        let options = AgentOptions {
            paths: paths.clone(),
            backend: Box::new(mock.clone()),
            mock: Some(mock),
            rescan_interval: Duration::from_millis(50),
            autostart: None,
            open_desktop: None,
            open_window: None,
            key_store: None,
            auto_lock_check: Duration::from_millis(50),
            cancel_grace: Duration::from_secs(10),
            clock: deckpilot_agent::system_clock(),
            screen_lock: None,
            notifier: None,
        };
        paths.create_dirs().unwrap();
        let lock = InstanceLock::acquire(&paths.lock_file).unwrap();
        runtime.block_on(async {
            let agent = Agent::start(options, lock).unwrap();
            tokio::spawn(agent.run(std::future::pending()));
        });
        runtime
    }

    fn agent_running(paths: &AgentPaths) -> bool {
        tauri::async_runtime::block_on(crate::agent::connection(paths)).is_ok_and(|c| c.running)
    }

    #[test]
    fn an_update_stops_the_agent_and_replaces_app_and_agent_together() {
        let mut h = Harness::new();
        let executable = install_app(&h, "version 1");
        let package = package(&h, "version 2");
        downloaded(&mut h, executable.clone(), &package);
        let _agent = start_agent(&h.paths);
        assert!(agent_running(&h.paths));

        tauri::async_runtime::block_on(replace(h.handle())).unwrap();

        // The agent was asked to stop before anything was replaced (a
        // running executable is never overwritten); the restart that
        // follows in the app starts the new one.
        assert!(!agent_running(&h.paths));
        assert_eq!(
            installed(&executable),
            ("version 2".to_owned(), "version 2".to_owned())
        );
        assert!(h.log().contains("installing update 99.0.0"), "{}", h.log());
    }

    #[test]
    fn a_failed_installation_keeps_the_previous_version() {
        let mut h = Harness::new();
        let executable = install_app(&h, "version 1");
        // Correctly signed, but not an installable package.
        downloaded(
            &mut h,
            executable.clone(),
            b"not an app archive or AppImage",
        );
        let _agent = start_agent(&h.paths);
        assert!(agent_running(&h.paths));

        let error = tauri::async_runtime::block_on(replace(h.handle()))
            .unwrap_err()
            .to_string();

        assert!(error.starts_with("update installation failed"), "{error}");
        // The test has no agent executable to start again; the message says
        // so and what to do.
        assert!(
            error.contains("the agent could not be started again")
                && error.contains("restart DeckPilot"),
            "{error}"
        );
        assert_eq!(
            installed(&executable),
            ("version 1".to_owned(), "version 1".to_owned())
        );
        // No half state: nothing else appeared next to the installation.
        let dir = executable.parent().unwrap();
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        if cfg!(target_os = "macos") {
            assert_eq!(entries, ["deckpilot-agent", "deckpilot-desktop"]);
        }
        // The download stays ready to retry; the updater is not stuck.
        assert_eq!(h.phase(), Phase::Ready);
        assert!(h.has_package());
        assert_eq!(h.shown_error().as_deref(), Some(&error[..]));
        assert!(h.log().contains(&error));
    }
}
