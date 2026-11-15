//! A small HTTPS server for the registry tests: rustls with a throwaway CA
//! (made per server, trusted only by the test that passes its PEM to the
//! agent), serving the files of a directory, with per-path overrides for
//! the failure cases (redirects, oversized or cut-off answers, statuses)
//! and an offline switch.
//!
//! Not part of `support/mod.rs`: only the registry tests include it
//! (`#[path]`), so no other test binary needs rcgen or rustls.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// What a path answers instead of its file.
#[derive(Debug, Clone)]
pub enum Route {
    /// `status` with `Location: location`.
    Redirect { status: u16, location: String },
    /// A bare status, no body.
    Status(u16),
    /// `len` bytes of `x`; with `declared`, a matching `Content-Length`,
    /// otherwise none (the connection closes at the end).
    Oversized { len: usize, declared: bool },
    /// Declares the file's full length but sends only `sent` bytes of it,
    /// then closes the connection (a download cut off midway).
    CutOff { sent: usize },
    /// These bytes instead of the file.
    Body(Vec<u8>),
}

#[derive(Debug, Default)]
struct State {
    root: PathBuf,
    routes: BTreeMap<String, Route>,
    offline: bool,
    log: Vec<String>,
}

/// A running server; stopped when dropped.
pub struct HttpsServer {
    pub port: u16,
    /// The throwaway CA's certificate (PEM), the only root the client
    /// should trust.
    pub ca_pem: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// A CA and a leaf certificate for `127.0.0.1` and `localhost`.
fn certificates() -> (String, CertificateDer<'static>, PrivateKeyDer<'static>) {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
        KeyPair, KeyUsagePurpose,
    };
    let ca_key = KeyPair::generate().unwrap();
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.distinguished_name
        .push(DnType::CommonName, "DeckPilot registry test CA (throwaway)");
    ca.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca, ca_key);
    let leaf_key = KeyPair::generate().unwrap();
    let mut leaf =
        CertificateParams::new(vec!["127.0.0.1".to_owned(), "localhost".to_owned()]).unwrap();
    leaf.distinguished_name
        .push(DnType::CommonName, "127.0.0.1");
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_cert = leaf.signed_by(&leaf_key, &issuer).unwrap();
    (
        ca_cert.pem(),
        leaf_cert.der().clone(),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    )
}

fn below(root: &Path, path: &str) -> Option<PathBuf> {
    let relative = Path::new(path.trim_start_matches('/'));
    relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| root.join(relative))
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn head(status: u16, length: Option<usize>, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut text = format!(
        "HTTP/1.1 {status} {}\r\nConnection: close\r\n",
        status_text(status)
    );
    if let Some(length) = length {
        let _ = write!(text, "Content-Length: {length}\r\n");
    }
    for (name, value) in extra {
        let _ = write!(text, "{name}: {value}\r\n");
    }
    text.push_str("\r\n");
    text.into_bytes()
}

fn serve(
    mut stream: rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
    state: &Mutex<State>,
) {
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
        if request.len() > 16 * 1024 {
            return;
        }
    }
    let text = String::from_utf8_lossy(&request);
    let path = text
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("/")
        .to_owned();
    let (route, file) = {
        let mut state = state.lock().unwrap();
        state.log.push(path.clone());
        (state.routes.get(&path).cloned(), below(&state.root, &path))
    };
    let file = file.and_then(|f| std::fs::read(f).ok());
    let mut answer = Vec::new();
    match route {
        Some(Route::Redirect { status, location }) => {
            answer.extend(head(status, Some(0), &[("Location", &location)]));
        }
        Some(Route::Status(status)) => answer.extend(head(status, Some(0), &[])),
        Some(Route::Oversized { len, declared }) => {
            answer.extend(head(200, declared.then_some(len), &[]));
            answer.extend(std::iter::repeat_n(b'x', len));
        }
        Some(Route::CutOff { sent }) => {
            let body = file.unwrap_or_default();
            answer.extend(head(200, Some(body.len()), &[]));
            answer.extend_from_slice(&body[..sent.min(body.len())]);
        }
        Some(Route::Body(body)) => {
            answer.extend(head(200, Some(body.len()), &[]));
            answer.extend(body);
        }
        None => match file {
            Some(body) => {
                answer.extend(head(200, Some(body.len()), &[]));
                answer.extend(body);
            }
            None => answer.extend(head(404, Some(0), &[])),
        },
    }
    let _ = stream.write_all(&answer);
    let _ = stream.flush();
    stream.conn.send_close_notify();
    let _ = stream.conn.complete_io(&mut stream.sock);
    let _ = stream.sock.shutdown(Shutdown::Both);
}

impl HttpsServer {
    /// Serves the files below `root` at `https://127.0.0.1:<port>/`.
    pub fn start(root: &Path) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (ca_pem, cert, key) = certificates();
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State {
            root: root.to_path_buf(),
            ..State::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let state = state.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let socket = match listener.accept() {
                        Ok((socket, _)) => socket,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(_) => continue,
                    };
                    if state.lock().unwrap().offline {
                        // Unreachable: the connection is dropped before
                        // any TLS.
                        let _ = socket.shutdown(Shutdown::Both);
                        continue;
                    }
                    let _ = socket.set_nonblocking(false);
                    let _ = socket.set_read_timeout(Some(Duration::from_secs(10)));
                    let _ = socket.set_write_timeout(Some(Duration::from_secs(10)));
                    let config = config.clone();
                    let state = state.clone();
                    std::thread::spawn(move || {
                        let Ok(connection) = rustls::ServerConnection::new(config) else {
                            return;
                        };
                        serve(rustls::StreamOwned::new(connection, socket), &state);
                    });
                }
            })
        };
        Self {
            port,
            ca_pem,
            state,
            stop,
            thread: Some(thread),
        }
    }

    /// `https://127.0.0.1:<port>/<path>`.
    pub fn url(&self, path: &str) -> String {
        format!(
            "https://127.0.0.1:{}/{}",
            self.port,
            path.trim_start_matches('/')
        )
    }

    /// `path` (absolute, e.g. `/site/v1/timestamp.json`) answers `route`.
    pub fn route(&self, path: &str, route: Route) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(path.to_owned(), route);
    }

    pub fn clear_routes(&self) {
        self.state.lock().unwrap().routes.clear();
    }

    /// Serves another directory from now on.
    pub fn set_root(&self, root: &Path) {
        self.state.lock().unwrap().root = root.to_path_buf();
    }

    /// While offline every connection is closed at once.
    pub fn set_offline(&self, offline: bool) {
        self.state.lock().unwrap().offline = offline;
    }

    /// Every requested path so far.
    pub fn log(&self) -> Vec<String> {
        self.state.lock().unwrap().log.clone()
    }

    /// Writes the CA certificate to `path` (for the agent).
    pub fn write_ca(&self, path: &Path) {
        std::fs::write(path, &self.ca_pem).unwrap();
    }
}

impl Drop for HttpsServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
