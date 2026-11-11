//! The client against a real agent binary of an earlier release, started
//! by `tools/compat/matrix.sh` (which builds it from its tag). Skipped
//! unless `DECKPILOT_COMPAT_DATA_DIR` names the data directory of such an
//! agent and `DECKPILOT_COMPAT_AGENT_VERSION` its version.

use deckpilot_ipc::{
    AgentClient, AgentPaths, Capability, ClientError, ClientKind, Request, Response,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_client_negotiates_with_a_real_older_agent() {
    let (Some(dir), Some(version)) = (
        std::env::var_os("DECKPILOT_COMPAT_DATA_DIR"),
        std::env::var("DECKPILOT_COMPAT_AGENT_VERSION").ok(),
    ) else {
        eprintln!("skipped: DECKPILOT_COMPAT_DATA_DIR is not set");
        return;
    };
    let protocol: u32 = std::env::var("DECKPILOT_COMPAT_AGENT_PROTOCOL")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(4);
    let paths = AgentPaths::in_dir(dir);
    let mut client = AgentClient::connect(&paths, ClientKind::Other)
        .await
        .unwrap();
    assert_eq!(client.agent_version(), version);
    assert_eq!(client.protocol(), protocol);
    assert_eq!(client.capabilities(), Capability::of_protocol(protocol));
    let Response::Status { status } = client.request(Request::GetStatus).await.unwrap() else {
        panic!("status expected");
    };
    assert_eq!(status.version, version);
    assert_eq!(status.running_executions, None, "older agents cannot say");
    if protocol < 5 {
        let refused = client
            .request(Request::ListExecutions {
                workflow: None,
                limit: None,
            })
            .await;
        assert!(
            matches!(refused, Err(ClientError::Unsupported { .. })),
            "{refused:?}"
        );
        // The connection is still usable afterwards.
        assert!(matches!(
            client.request(Request::GetStatus).await,
            Ok(Response::Status { .. })
        ));
    }
    eprintln!(
        "real agent {version}: protocol {}, capabilities {:?}",
        client.protocol(),
        client.capabilities()
    );
}
