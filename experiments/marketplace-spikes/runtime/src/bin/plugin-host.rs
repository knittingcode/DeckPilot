//! Spike plugin host: one process per plugin. Reads frames on stdin, writes
//! frames on stdout, runs the embedded guest in wasmtime with a fresh Store
//! per invocation, and turns `host_call` imports into `call` frames.

use std::cell::RefCell;
use std::io::{self, BufReader, BufWriter, Stdin, Stdout};
use std::rc::Rc;
use std::time::Duration;

use runtime_spike::engine::{EpochTicker, GUEST_WASM, Limits, Runtime, trap_of};
use runtime_spike::protocol::{MAX_TO_HOST, ToHost, ToSupervisor, read_frame, write_frame};
use serde_json::{Value, json};

struct Io {
    input: BufReader<Stdin>,
    output: BufWriter<Stdout>,
}

impl Io {
    fn send(&mut self, msg: &ToSupervisor) {
        if write_frame(&mut self.output, msg).is_err() {
            std::process::exit(2);
        }
    }

    fn recv(&mut self) -> ToHost {
        match read_frame::<_, ToHost>(&mut self.input, MAX_TO_HOST) {
            Ok(Some(ToHost::Shutdown)) | Ok(None) => std::process::exit(0),
            Ok(Some(m)) => m,
            // Malformed frame: close the channel.
            Err(_) => std::process::exit(3),
        }
    }
}

fn main() {
    let io = Rc::new(RefCell::new(Io {
        input: BufReader::new(io::stdin()),
        output: BufWriter::new(io::stdout()),
    }));
    let runtime = match Runtime::new(GUEST_WASM) {
        Ok(r) => r,
        Err(_) => std::process::exit(4),
    };
    let _ticker = EpochTicker::start(&runtime.engine);
    let sandbox = if std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some() {
        "app-sandbox"
    } else {
        "applied-by-launcher-or-none"
    };
    io.borrow_mut().send(&ToSupervisor::Ready {
        host_version: env!("CARGO_PKG_VERSION").into(),
        engine: "wasmtime-50.0.1/spike-guest".into(),
        sandbox: sandbox.into(),
    });

    loop {
        let msg = io.borrow_mut().recv();
        match msg {
            ToHost::Ping { seq } => io.borrow_mut().send(&ToSupervisor::Pong { seq }),
            ToHost::Invoke {
                invocation_id,
                entry,
                input,
            } => {
                let result = invoke(&runtime, &io, invocation_id, &entry, input.as_deref());
                let (ok, error) = match result {
                    Ok(v) => (Some(v), None),
                    Err(e) => (None, Some(e)),
                };
                io.borrow_mut().send(&ToSupervisor::Result {
                    invocation_id,
                    ok,
                    error,
                });
            }
            ToHost::Probe { kind, target } => {
                let (ok, detail) = probe(&kind, &target);
                io.borrow_mut()
                    .send(&ToSupervisor::ProbeResult { ok, detail });
            }
            ToHost::Crash => std::process::abort(),
            ToHost::Hang => loop {
                std::thread::sleep(Duration::from_secs(3600));
            },
            ToHost::Reply { .. } => std::process::exit(3), // reply without a call
            ToHost::Shutdown => std::process::exit(0),
        }
    }
}

fn invoke(
    runtime: &Runtime,
    io: &Rc<RefCell<Io>>,
    invocation_id: u64,
    entry: &str,
    input: Option<&str>,
) -> Result<Value, String> {
    let io = io.clone();
    let mut next_call = 0u64;
    let handler = Box::new(move |request: &[u8]| -> Vec<u8> {
        let parsed: Option<(String, Value)> =
            serde_json::from_slice::<Value>(request).ok().and_then(|v| {
                Some((
                    v.get("op")?.as_str()?.to_owned(),
                    v.get("args").cloned().unwrap_or(Value::Null),
                ))
            });
        let Some((op, args)) = parsed else {
            return br#"{"error":"bad request"}"#.to_vec();
        };
        next_call += 1;
        let call_id = next_call;
        let mut io = io.borrow_mut();
        io.send(&ToSupervisor::Call {
            invocation_id,
            call_id,
            op,
            args,
        });
        loop {
            match io.recv() {
                ToHost::Reply {
                    call_id: c,
                    ok,
                    error,
                } if c == call_id => {
                    let v = match error {
                        Some(e) => json!({ "error": e }),
                        None => json!({ "ok": ok }),
                    };
                    return serde_json::to_vec(&v).unwrap();
                }
                ToHost::Ping { seq } => io.send(&ToSupervisor::Pong { seq }),
                _ => std::process::exit(3),
            }
        }
    });
    let mut plugin = runtime
        .instantiate(&Limits::default(), handler)
        .map_err(|e| format!("instantiate: {e}"))?;
    let describe = |e: runtime_spike::engine::Error| match trap_of(&e) {
        Some(t) => format!("trap: {t:?}"),
        None => format!("error: {e:#}"),
    };
    let r: Result<Value, String> = match entry {
        "add" => plugin
            .call::<(i32, i32), i32>("add", (2, 3))
            .map(Value::from)
            .map_err(describe),
        "grow_raw" => plugin
            .call::<(), u32>("grow_raw", ())
            .map(Value::from)
            .map_err(describe),
        "spin" => plugin
            .call::<(), i32>("spin", ())
            .map(Value::from)
            .map_err(describe),
        "memory_bomb" => plugin
            .call::<(), u32>("memory_bomb", ())
            .map(Value::from)
            .map_err(describe),
        "trap_panic" | "trap_unreachable" => plugin
            .call::<(), ()>(entry, ())
            .map(|_| Value::Null)
            .map_err(describe),
        "relay" => {
            let input = input.unwrap_or("");
            plugin
                .relay(input.as_bytes())
                .map_err(describe)
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
        }
        e if e.starts_with("relay_n:") => {
            let n: u32 = e[8..].parse().map_err(|_| "bad count".to_string())?;
            plugin
                .relay_n(input.unwrap_or("").as_bytes(), n)
                .map(Value::from)
                .map_err(describe)
        }
        _ => Err(format!("unknown entry {entry}")),
    };
    // The Store is dropped here; it is never reused after a trap.
    r
}

/// Native operations attempted by the host process itself (outside wasm),
/// used to check the OS sandbox.
fn probe(kind: &str, target: &str) -> (bool, String) {
    let r: io::Result<String> = match kind {
        "read_file" => std::fs::read(target).map(|b| format!("read {} bytes", b.len())),
        "write_file" => std::fs::write(target, b"spike").map(|_| "written".into()),
        "list_dir" => std::fs::read_dir(target).map(|d| format!("{} entries", d.count())),
        "connect" => target
            .parse()
            .map_err(io::Error::other)
            .and_then(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(2)))
            .map(|_| "connected".into()),
        "exec" => std::process::Command::new(target)
            .output()
            .map(|o| format!("exit {}", o.status)),
        #[cfg(unix)]
        "connect_unix" => {
            std::os::unix::net::UnixStream::connect(target).map(|_| "connected".into())
        }
        "env" => Ok(format!("{} vars", std::env::vars_os().count())),
        _ => Err(io::Error::other("unknown probe")),
    };
    match r {
        Ok(d) => (true, d),
        Err(e) => (false, format!("{:?}: {e}", e.kind())),
    }
}
