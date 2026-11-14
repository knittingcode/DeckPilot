//! A local mock HTTP/1.1 server on 127.0.0.1 with canned responses per
//! path. It records the paths requested and answers every request with
//! `Connection: close`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// How often the accept loop looks for a connection or the stop.
const POLL: Duration = Duration::from_millis(2);
const BIND_ATTEMPTS: u32 = 8;

/// An error that goes away once ports or socket buffers are freed again
/// (a busy Windows runner gives WSAENOBUFS or WSAEADDRINUSE for a moment).
fn transient(error: &std::io::Error) -> bool {
    const NO_BUFFERS: i32 = if cfg!(windows) {
        10_055 // WSAENOBUFS
    } else if cfg!(target_os = "linux") {
        105 // ENOBUFS
    } else {
        55 // ENOBUFS (macOS, BSD)
    };
    matches!(
        error.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::AddrNotAvailable
    ) || error.raw_os_error() == Some(NO_BUFFERS)
}

/// A listener on a free port of 127.0.0.1, retrying transient errors
/// with a growing pause.
pub fn bind_local() -> TcpListener {
    let mut pause = Duration::from_millis(50);
    for _ in 1..BIND_ATTEMPTS {
        match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => return listener,
            Err(error) if transient(&error) => {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_secs(1));
            }
            Err(error) => panic!("binding 127.0.0.1 failed: {error}"),
        }
    }
    TcpListener::bind("127.0.0.1:0").expect("binding 127.0.0.1")
}

/// A canned response.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_owned(),
        }
    }

    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain",
            body: body.to_owned(),
        }
    }
}

type Routes = Arc<Mutex<BTreeMap<String, Vec<Reply>>>>;

pub struct MockServer {
    port: u16,
    routes: Routes,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockServer {
    pub fn start() -> Self {
        let listener = bind_local();
        let port = listener.local_addr().unwrap().port();
        // Polled, so that stopping never needs a connection of its own.
        listener.set_nonblocking(true).unwrap();
        let routes: Routes = Arc::default();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (routes, requests, stop) = (routes.clone(), requests.clone(), stop.clone());
            std::thread::spawn(move || {
                // One connection at a time; the listener closes on stop.
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // Accepted sockets may inherit non-blocking mode.
                            let _ = stream.set_nonblocking(false);
                            serve(stream, &routes, &requests);
                        }
                        Err(_) => std::thread::sleep(POLL),
                    }
                }
            })
        };
        Self {
            port,
            routes,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// Replies to `path`, in order; the last one repeats.
    pub fn route(&self, path: &str, replies: Vec<Reply>) {
        self.routes.lock().unwrap().insert(path.to_owned(), replies);
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Paths requested so far.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(stream: TcpStream, routes: &Routes, requests: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split(' ').nth(1).unwrap_or("/").to_owned();
    let path = path.split('?').next().unwrap_or("/").to_owned();
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    requests.lock().unwrap().push(path.clone());
    let reply = {
        let mut routes = routes.lock().unwrap();
        match routes.get_mut(&path) {
            Some(replies) if replies.len() > 1 => replies.remove(0),
            Some(replies) if !replies.is_empty() => replies[0].clone(),
            _ => Reply::text(404, "not found"),
        }
    };
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        reply.status,
        reply.content_type,
        reply.body.len(),
        reply.body
    );
    let _ = stream.flush();
}
