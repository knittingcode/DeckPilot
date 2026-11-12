//! An agent and a client started with a temporary directory so long that a
//! socket inside it would not fit in `sun_path` (104 bytes on macOS, 108 on
//! Linux) still find each other. The environment is process-wide, so the
//! test re-runs itself in a child process with `TMPDIR` set.
#![cfg(unix)]

use deckpilot_ipc::{AgentPaths, SharedStream};
use interprocess::local_socket::tokio::prelude::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const CHILD: &str = "DECKPILOT_IPC_LONG_TMPDIR_CHILD";
const TEST: &str = "agent_and_client_meet_with_a_long_tmpdir";

#[test]
fn agent_and_client_meet_with_a_long_tmpdir() {
    if std::env::var_os(CHILD).is_some() {
        child();
        return;
    }
    let mut long = std::env::temp_dir().join(format!("deckpilot-long-{}", std::process::id()));
    while long.as_os_str().len() < 160 {
        long = long.join("a-rather-long-directory-name");
    }
    std::fs::create_dir_all(&long).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("TMPDIR", &long)
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("deckpilot-long-{}", std::process::id())),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "child failed\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("endpoint: /tmp/deckpilot-"), "{stdout}");
}

fn child() {
    let temp = std::env::temp_dir();
    assert!(temp.as_os_str().len() >= 160, "{}", temp.display());
    let data_dir = temp.join("data");
    let paths = AgentPaths::in_dir(&data_dir);
    paths.create_dirs().unwrap();
    let endpoint = paths.endpoint();
    println!("endpoint: {}", endpoint.display());
    assert!(endpoint.display().len() < 104, "{}", endpoint.display());
    // A client in another process computes the same endpoint from the same
    // data directory and environment.
    assert_eq!(endpoint, AgentPaths::in_dir(&data_dir).endpoint());

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let listener = endpoint.listen().unwrap();
            let server = tokio::spawn(async move {
                let mut write = SharedStream::new(listener.accept().await.unwrap());
                let read = write.clone();
                let mut line = String::new();
                BufReader::new(read).read_line(&mut line).await.unwrap();
                write.write_all(line.as_bytes()).await.unwrap();
            });
            let mut write = SharedStream::new(endpoint.connect().await.unwrap());
            let read = write.clone();
            write.write_all(b"ping\n").await.unwrap();
            let mut line = String::new();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            assert_eq!(line, "ping\n");
            server.await.unwrap();
        });
}
