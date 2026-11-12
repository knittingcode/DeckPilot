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

use super::{Phase, Source, UPDATE_LOG, UpdateManager, check_from, download, may_start};
use crate::state::AppState;

/// The announced version: newer than any real one.
const VERSION: &str = "99.0.0";

/// What the server answers for a path.
#[derive(Clone)]
enum Reply {
    Ok(Vec<u8>),
    /// Announces the full length, sends half and closes the connection.
    Truncated(Vec<u8>),
    /// Sends the headers and half of the body, then nothing for a while.
    Stalled(Vec<u8>),
    Status(u16),
}

/// A minimal HTTP/1.1 server on 127.0.0.1 with replies that can be changed
/// between requests.
struct Server {
    port: u16,
    replies: Arc<Mutex<HashMap<String, Reply>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let replies: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
        let shared = replies.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let replies = shared.clone();
                std::thread::spawn(move || serve(stream, &replies));
            }
        });
        Self { port, replies }
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

fn serve(mut stream: TcpStream, replies: &Mutex<HashMap<String, Reply>>) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
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
    let head = |status: &str, length: usize| {
        format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
    };
    let _ = match reply {
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
            std::thread::sleep(Duration::from_secs(10));
            sent
        }
        Reply::Status(code) => stream.write_all(head(&format!("{code} Failure"), 0).as_bytes()),
    };
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
    let platform = json!({ "url": server.url("package"), "signature": signature });
    let target = deckpilot_update::manifest::current_target();
    serde_json::to_vec(&json!({
        "version": VERSION,
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
            endpoint: server.url("latest.json"),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(1),
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

    fn download(&self) -> Result<(), String> {
        tauri::async_runtime::block_on(download(self.handle())).map_err(|e| e.to_string())
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
        assert!(error.contains(expected), "{error}");
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
    h.download().unwrap();
    assert_eq!(h.phase(), Phase::Ready);
    assert_eq!(h.shown_error(), None);
    assert_eq!(
        h.handle().state::<UpdateManager>().lock().bytes.as_deref(),
        Some(&package[..])
    );
    assert!(h.log().contains("signature verified"), "{}", h.log());
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
    let h = Harness::new();
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
    h.server.reply("package", Reply::Stalled(package.clone()));
    let started = Instant::now();
    let error = h.download().unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
    h.assert_failed_cleanly(&error, "stopped responding");

    // The server refuses.
    h.server.reply("package", Reply::Status(503));
    let error = h.download().unwrap_err();
    h.assert_failed_cleanly(&error, "could not be reached or refused");

    // Nothing partial was kept: the next attempt starts over and succeeds.
    h.server.reply("package", Reply::Ok(package.clone()));
    h.download().unwrap();
    assert_eq!(h.phase(), Phase::Ready);
    assert_eq!(
        h.handle().state::<UpdateManager>().lock().bytes.as_deref(),
        Some(&package[..])
    );
}

#[test]
fn failed_checks_leave_the_updater_usable() {
    let h = Harness::new();
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
        h.server.reply("latest.json", reply);
        let error = h.check().unwrap_err();
        assert!(error.contains(expected), "{error}");
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
        h.download().unwrap();
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
