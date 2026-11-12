//! Agent lifecycle: starting over what a crashed agent left behind, and a
//! restart that begins before the previous agent has fully stopped.

mod support;

use std::time::{Duration, Instant};

use deckpilot_agent::InstanceLock;
use deckpilot_device::mock::MockBackend;
use deckpilot_ipc::framing::{read_message, write_message};
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, ClientMessage, ErrorCode, PROTOCOL_VERSION,
    ServerMessage, SharedStream, token,
};
use support::{TempDir, TestAgent, create_database};
use tokio::io::BufReader;

fn data_dir(dir: &TempDir) -> AgentPaths {
    let paths = AgentPaths::in_dir(dir.path().join("data"));
    paths.create_dirs().unwrap();
    paths
}

/// Sends a `hello` with `secret` and returns the agent's answer.
async fn hello(paths: &AgentPaths, secret: &str) -> ServerMessage {
    let stream = SharedStream::new(paths.endpoint().connect().await.unwrap());
    let mut reader = BufReader::new(stream.clone());
    let mut writer = stream;
    write_message(
        &mut writer,
        &ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            token: secret.to_owned(),
            client: ClientKind::Other,
        },
    )
    .await
    .unwrap();
    read_message::<_, ServerMessage>(&mut reader)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_starts_over_the_leftovers_of_a_crashed_one() {
    let dir = TempDir::new("deckpilot-lifecycle-crash");
    let paths = data_dir(&dir);
    let key = create_database(&paths);

    // What a killed agent leaves: its token, the lock file (the OS lock went
    // with the process) and, on Unix, a socket file nobody listens on.
    let stale = token::create(&paths.token_file).unwrap();
    std::fs::write(&paths.lock_file, b"").unwrap();
    #[cfg(unix)]
    {
        drop(paths.endpoint().listen().unwrap());
        let socket = std::os::unix::net::UnixListener::bind(paths.endpoint().display()).unwrap();
        drop(socket);
        assert!(std::path::Path::new(&paths.endpoint().display()).exists());
    }
    // Clients see no agent rather than an error.
    assert!(matches!(
        AgentClient::connect(&paths, ClientKind::Desktop).await,
        Err(ClientError::NotRunning)
    ));

    let agent = TestAgent::start(&paths, &key, &MockBackend::new(), None);
    let status = agent.status().await;
    assert_eq!(status.version, env!("CARGO_PKG_VERSION"));
    let fresh = token::read(&paths.token_file).unwrap();
    assert_ne!(fresh, stale, "a new token replaces the stale one");
    // The token of the crashed agent no longer grants access.
    match hello(&paths, &stale).await {
        ServerMessage::Error { error, .. } => assert_eq!(error.code, ErrorCode::Unauthorized),
        other => panic!("stale token accepted: {other:?}"),
    }
    assert!(matches!(
        hello(&paths, &fresh).await,
        ServerMessage::Welcome { .. }
    ));
    agent.shutdown().await;
    assert!(!paths.token_file.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_waits_for_the_previous_agent_to_finish_stopping() {
    let dir = TempDir::new("deckpilot-lifecycle-restart");
    let paths = data_dir(&dir);
    let key = create_database(&paths);
    let agent = TestAgent::start(&paths, &key, &MockBackend::new(), None);
    agent.shutdown().await;

    // The previous agent's endpoint is closed, but it still holds the lock
    // for a moment while it finishes.
    let stopping = InstanceLock::acquire(&paths.lock_file).unwrap();
    assert!(matches!(
        AgentClient::connect(&paths, ClientKind::Desktop).await,
        Err(ClientError::NotRunning)
    ));
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        drop(stopping);
    });
    let started = Instant::now();
    let lock = InstanceLock::acquire_waiting(&paths.lock_file, deckpilot_agent::STARTUP_LOCK_WAIT)
        .expect("the next agent starts once the previous one has stopped");
    assert!(started.elapsed() >= Duration::from_millis(300));
    release.join().unwrap();
    drop(lock);

    let agent = TestAgent::start(&paths, &key, &MockBackend::new(), None);
    agent.status().await;
    agent.shutdown().await;
}
