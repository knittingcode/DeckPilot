//! The client against simulated agents of earlier releases: they accept
//! `hello` only up to their own protocol, answer like those releases did
//! (fixtures recorded from the real DeckPilot 0.5.0-2 agent), and record
//! every request they receive, so a test can prove that nothing newer than
//! their protocol was ever sent.

use std::sync::{Arc, Mutex};

use deckpilot_ipc::framing::{read_message, write_message};
use deckpilot_ipc::{
    AgentClient, AgentPaths, Capability, ClientError, ClientKind, ClientMessage, ErrorCode,
    IpcError, Request, Response, ServerMessage, SharedStream, token,
};
use interprocess::local_socket::tokio::prelude::*;
use tokio::io::BufReader;

/// The status a 0.5.0-2 agent sent, as recorded from the real binary.
const STATUS_0_5_0_2: &str = include_str!("fixtures/agent-0.5.0-2-status.json");
/// The `welcome` a 0.5.0-2 agent sent to a protocol 4 client.
const WELCOME_0_5_0_2: &str = include_str!("fixtures/agent-0.5.0-2-welcome.json");

struct Peer {
    paths: AgentPaths,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    hellos: Arc<Mutex<Vec<u32>>>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.paths.data_dir);
    }
}

/// An agent that speaks protocols up to `max`, like a release before
/// protocol negotiation: it refuses a newer `hello` outright.
/// Must be called inside a Tokio runtime.
fn peer(max: u32, version: &'static str) -> Peer {
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-ipc-peer-{}-{}",
        std::process::id(),
        getrandom_suffix()
    ));
    let paths = AgentPaths::in_dir(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let secret = token::create(&paths.token_file).unwrap();
    let listener = paths.endpoint().listen().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let hellos = Arc::new(Mutex::new(Vec::new()));
    let (seen, greeted) = (received.clone(), hellos.clone());
    tokio::spawn(async move {
        loop {
            let Ok(stream) = listener.accept().await else {
                return;
            };
            let (seen, greeted, secret) = (seen.clone(), greeted.clone(), secret.clone());
            tokio::spawn(async move {
                let stream = SharedStream::new(stream);
                let mut reader = BufReader::new(stream.clone());
                let mut writer = stream;
                let Ok(Some(ClientMessage::Hello {
                    protocol, token, ..
                })) = read_message::<_, ClientMessage>(&mut reader).await
                else {
                    return;
                };
                greeted.lock().unwrap().push(protocol);
                if !(1..=max).contains(&protocol) || token != secret {
                    let error = ServerMessage::Error {
                        id: None,
                        error: IpcError::new(
                            ErrorCode::UnsupportedProtocol,
                            format!("protocol {protocol} is not supported; expected 1 to {max}"),
                        ),
                    };
                    let _ = write_message(&mut writer, &error).await;
                    return;
                }
                // Recorded from the real 0.5.0-2 agent; releases before
                // protocol 6 send no capabilities.
                let mut welcome: serde_json::Value = serde_json::from_str(WELCOME_0_5_0_2).unwrap();
                welcome["protocol"] = max.into();
                welcome["agent_version"] = version.into();
                let _ = write_message(&mut writer, &welcome).await;
                while let Ok(Some(message)) =
                    read_message::<_, serde_json::Value>(&mut reader).await
                {
                    seen.lock().unwrap().push(message.clone());
                    let id = message["id"].as_u64().unwrap_or_default();
                    let reply = match message["request"]["type"].as_str() {
                        Some("getStatus") => serde_json::json!({
                            "type": "response", "id": id,
                            "response": {"type": "status", "status":
                                serde_json::from_str::<serde_json::Value>(STATUS_0_5_0_2).unwrap()},
                        }),
                        // As a protocol 7 or 8 agent lists the catalog:
                        // system ids only, without owners.
                        Some("listVariables") if max >= 7 => serde_json::json!({
                            "type": "response", "id": id,
                            "response": {"type": "variables", "entries": [{
                                "reference": "system.project.current.key",
                                "label": "system.project.current.key",
                                "description": "Key of the project in the context",
                                "group": "system",
                                "valueType": {"type": "string"},
                                "dynamic": true,
                                "availability": "notInContext",
                            }]},
                        }),
                        _ => serde_json::json!({
                            "type": "error", "id": id,
                            "error": {"code": "invalidRequest", "message": "invalid request"},
                        }),
                    };
                    if write_message(&mut writer, &reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    Peer {
        paths,
        received,
        hellos,
    }
}

fn getrandom_suffix() -> u64 {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_4_agent_is_used_with_protocol_4_only() {
    let peer = peer(4, "0.5.0-2");
    let mut client = AgentClient::connect(&peer.paths, ClientKind::Desktop)
        .await
        .unwrap();
    // The client stepped down from its own protocol until accepted.
    assert_eq!(*peer.hellos.lock().unwrap(), [9, 8, 7, 6, 5, 4]);
    assert_eq!(client.protocol(), 4);
    assert_eq!(client.agent_protocol(), 4);
    assert_eq!(client.agent_version(), "0.5.0-2");
    assert!(client.supports(Capability::EncryptedDatabase));
    assert!(!client.supports(Capability::BackgroundExecutions));
    assert!(!client.supports(Capability::Notifications));

    // The status of the real 0.5.0-2 agent is read; what it cannot say
    // stays unknown instead of being guessed.
    let Response::Status { status } = client.request(Request::GetStatus).await.unwrap() else {
        panic!("status expected")
    };
    assert_eq!(status.version, "0.5.0-2");
    assert_eq!(status.running_executions, None);

    // Background runs need protocol 5: refused before anything is sent.
    let refused = client
        .request(Request::ListExecutions {
            workflow: None,
            limit: None,
        })
        .await;
    assert!(
        matches!(
            &refused,
            Err(ClientError::Unsupported {
                needs: 5,
                protocol: 4,
                ..
            })
        ),
        "{refused:?}"
    );
    let sent: Vec<String> = peer
        .received
        .lock()
        .unwrap()
        .iter()
        .map(|m| m["request"]["type"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(sent, ["getStatus"]);
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("restart the agent")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_older_than_protocol_4_need_replacing() {
    let peer = peer(3, "0.3.0");
    let refused = AgentClient::connect(&peer.paths, ClientKind::Desktop).await;
    assert!(
        matches!(&refused, Err(ClientError::Agent(e)) if e.code == ErrorCode::UnsupportedProtocol),
        "{refused:?}"
    );
    // It never went below the oldest protocol the app works with.
    assert_eq!(*peer.hellos.lock().unwrap(), [9, 8, 7, 6, 5, 4]);
    // Protocol 1 still works for stopping it during an update.
    let mut stopper = AgentClient::connect_with_protocol(&peer.paths, ClientKind::Desktop, 1)
        .await
        .unwrap();
    assert_eq!(stopper.protocol(), 1);
    assert_eq!(stopper.capabilities(), []);
    assert!(matches!(
        stopper.request(Request::PrepareForUpdate).await,
        Err(ClientError::Unsupported { needs: 2, .. })
    ));
}

/// An agent that is stopping accepts a connection and closes it before it
/// answers the `hello` (on Windows the pipe of a stopped agent can still be
/// opened for a moment). Clients are told that no agent runs, quickly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_that_closes_before_answering_is_not_running() {
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-ipc-closing-{}-{}",
        std::process::id(),
        getrandom_suffix()
    ));
    let paths = AgentPaths::in_dir(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    token::create(&paths.token_file).unwrap();
    let listener = paths.endpoint().listen().unwrap();
    let closing = tokio::spawn(async move {
        for _ in 0..2 {
            let Ok(stream) = listener.accept().await else {
                return;
            };
            drop(stream);
        }
    });
    let started = std::time::Instant::now();
    for _ in 0..2 {
        let refused = AgentClient::connect(&paths, ClientKind::Desktop).await;
        assert!(
            matches!(refused, Err(ClientError::NotRunning)),
            "{refused:?}"
        );
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    closing.await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_5_agent_has_no_notifications() {
    let peer = peer(5, "0.5.0-dev");
    let client = AgentClient::connect(&peer.paths, ClientKind::Desktop)
        .await
        .unwrap();
    assert_eq!(client.protocol(), 5);
    assert!(client.supports(Capability::BackgroundExecutions));
    assert!(!client.supports(Capability::Notifications));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_6_agent_is_never_asked_about_variables() {
    let peer = peer(6, "0.6.0");
    let mut client = AgentClient::connect(&peer.paths, ClientKind::Desktop)
        .await
        .unwrap();
    assert_eq!(*peer.hellos.lock().unwrap(), [9, 8, 7, 6]);
    assert_eq!(client.protocol(), 6);
    assert!(client.supports(Capability::Notifications));
    assert!(!client.supports(Capability::Variables));
    let context =
        deckpilot_core::variables::ResolutionContext::preview(deckpilot_core::ProfileId::new());
    for request in [
        Request::ListVariables {
            context: Box::new(context.clone()),
            filter: Box::default(),
        },
        Request::PreviewReferences {
            context: Box::new(context.clone()),
            items: Vec::new(),
        },
    ] {
        let refused = client.request(request).await;
        assert!(
            matches!(
                &refused,
                Err(ClientError::Unsupported {
                    needs: 7,
                    protocol: 6,
                    ..
                })
            ),
            "{refused:?}"
        );
    }
    // Refused before anything was sent.
    assert_eq!(peer.received.lock().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_7_agent_is_never_asked_about_the_work_session() {
    let peer = peer(7, "0.6.0");
    let mut client = AgentClient::connect(&peer.paths, ClientKind::Desktop)
        .await
        .unwrap();
    assert_eq!(*peer.hellos.lock().unwrap(), [9, 8, 7]);
    assert_eq!(client.protocol(), 7);
    assert!(client.supports(Capability::Variables));
    assert!(!client.supports(Capability::WorkSession));
    let profile = deckpilot_core::ProfileId::new();
    for request in [
        Request::GetWorkSession,
        Request::ListWorkSessions {
            limit: 10,
            before: None,
        },
        Request::GetWorkSessionSettings { profile },
        Request::SaveWorkSessionSettings {
            settings: Box::new(deckpilot_ipc::work_session::WorkSessionSettings {
                profile,
                start_workflow: None,
                verification_workflow: None,
                approval_ttl_seconds: 600,
                review_gap_seconds: 900,
                elapsed_format: deckpilot_ipc::work_session::ElapsedFormat::Minutes,
                updated_at_ms: None,
            }),
        },
        Request::ResolveWorkSessionReview {
            session: deckpilot_core::WorkSessionId::new(),
            review: 1,
            count: false,
        },
        Request::ClearWorkSessionNotices { session: None },
    ] {
        let refused = client.request(request).await;
        assert!(
            matches!(
                &refused,
                Err(ClientError::Unsupported {
                    needs: 8,
                    protocol: 7,
                    ..
                })
            ),
            "{refused:?}"
        );
    }
    // Refused before anything was sent.
    assert_eq!(peer.received.lock().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_8_agent_lists_variables_without_the_registry() {
    let peer = peer(8, "0.6.0");
    let mut client = AgentClient::connect(&peer.paths, ClientKind::Desktop)
        .await
        .unwrap();
    assert_eq!(*peer.hellos.lock().unwrap(), [9, 8]);
    assert_eq!(client.protocol(), 8);
    assert!(client.supports(Capability::WorkSession));
    assert!(!client.supports(Capability::VariableRegistry));
    let context =
        deckpilot_core::variables::ResolutionContext::preview(deckpilot_core::ProfileId::new());
    let response = client
        .request(Request::ListVariables {
            context: Box::new(context),
            filter: Box::default(),
        })
        .await
        .unwrap();
    let Response::Variables { entries } = response else {
        panic!("variables: {response:?}");
    };
    // The legacy catalog reads as entries without owners or canonical ids.
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].reference.to_string(),
        "system.project.current.key"
    );
    assert_eq!(entries[0].owner, None);
    assert_eq!(entries[0].deprecated, None);
    assert!(!entries[0].read_only);
    assert_eq!(
        entries[0].reference.registered_id().unwrap().as_str(),
        "projects.current.key"
    );
}
