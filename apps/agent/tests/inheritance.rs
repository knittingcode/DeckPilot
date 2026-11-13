//! Programs the agent starts must not inherit its instance lock, its
//! listening socket or its connections. A long-running program (an editor,
//! an app opened with `open`, `docker compose up`) would otherwise keep the
//! lock taken after the agent stopped, so the next agent could not start,
//! and keep connections open, so clients would not notice the agent going.

mod support;

use std::time::Duration;

use deckpilot_agent::InstanceLock;
use deckpilot_core::launch::{LaunchPlan, Launcher, SystemLauncher};
use deckpilot_device::mock::MockBackend;
use deckpilot_ipc::{AgentClient, AgentPaths, ClientKind};
use support::{TIMEOUT, TempDir, TestAgent, create_database};

/// A program that keeps running for a minute, started the way the modules
/// open applications (its own process group, not waited for).
fn long_running_program() -> LaunchPlan {
    if cfg!(windows) {
        LaunchPlan {
            program: "ping".into(),
            args: vec!["-n".into(), "60".into(), "127.0.0.1".into()],
            wait: false,
        }
    } else {
        LaunchPlan {
            program: "sleep".into(),
            args: vec!["60".into()],
            wait: false,
        }
    }
}

/// Ends the `sleep` started by this test process (Unix; on Windows the
/// `ping` ends by itself).
struct Cleanup;

impl Drop for Cleanup {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("pkill")
                .args(["-P", &std::process::id().to_string(), "sleep"])
                .status();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_program_started_by_the_agent_does_not_keep_its_lock_or_endpoint() {
    let dir = TempDir::new("deckpilot-inheritance");
    let paths = AgentPaths::in_dir(dir.path().join("data"));
    paths.create_dirs().unwrap();
    let key = create_database(&paths);
    let agent = TestAgent::start(&paths, &key, &MockBackend::new(), None);
    // A client is connected while programs start, so the agent holds an
    // accepted connection as well as the listener and the lock.
    let mut subscriber = agent.events().await;
    agent.status().await;

    #[cfg(unix)]
    unix::assert_nothing_inherited(&paths);

    let _cleanup = Cleanup;
    SystemLauncher.launch(&long_running_program()).unwrap();

    agent.shutdown().await;
    // The connected client learns that the agent is gone.
    let closed = tokio::time::timeout(TIMEOUT, async {
        while subscriber.next_event().await.is_ok() {}
    })
    .await;
    assert!(
        closed.is_ok(),
        "the connection stayed open after the agent stopped"
    );
    drop(subscriber);
    // The lock is free at once, while the program still runs.
    drop(
        InstanceLock::acquire(&paths.lock_file)
            .expect("the program started by the agent holds the instance lock"),
    );
    // The next agent creates the endpoint again (on Windows a pipe handle
    // held by the program would prevent that).
    let next = TestAgent::start(&paths, &key, &MockBackend::new(), None);
    AgentClient::connect(&paths, ClientKind::Other)
        .await
        .expect("the next agent accepts connections");
    tokio::time::timeout(Duration::from_secs(30), next.shutdown())
        .await
        .unwrap();
}

#[cfg(unix)]
mod unix {
    use std::process::Command;

    use deckpilot_core::process::{ProcessLimits, minimal_environment, run};
    use deckpilot_ipc::AgentPaths;

    /// Lists what the shell running it has open (it opens `/etc/hosts` on
    /// descriptor 9 itself, as a check that the listing works): the targets of its open
    /// descriptors (with the paths of Unix sockets) and its working
    /// directory.
    const LIST_LINUX: &str = r#"
exec 9</etc/hosts
for f in /proc/$$/fd/*; do
  l=$(readlink "$f") || continue
  echo "fd $l"
  case $l in socket:*)
    i=${l#socket:[}; i=${i%]}
    awk -v i="$i" '$7 == i { print "fd unix " $8 }' /proc/net/unix ;;
  esac
done
echo "cwd $(readlink /proc/$$/cwd)"
"#;
    const LIST_MACOS: &str = r#"
exec 9</etc/hosts
lsof -a -p $$ -d '0-9999' -Fn 2>/dev/null | sed -n 's/^n/fd /p'
echo "cwd $(pwd -P)"
"#;

    /// Starts a program through the modules' process path (its own process
    /// group; with and without the minimal environment, which take
    /// different spawn paths in the standard library) and checks that it
    /// inherits neither the instance lock nor the agent's socket.
    pub fn assert_nothing_inherited(paths: &AgentPaths) {
        let script = if cfg!(target_os = "linux") {
            LIST_LINUX
        } else {
            LIST_MACOS
        };
        let data_dir = paths
            .data_dir
            .parent()
            .and_then(|p| p.file_name())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let endpoint = paths.endpoint().display();
        let socket = std::path::Path::new(&endpoint)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for minimal in [false, true] {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            if minimal {
                minimal_environment(&mut command, &[]);
            }
            let output = run(command, None, ProcessLimits::default()).unwrap();
            let listing = output.stdout;
            // The listing works: it shows the working directory and a file
            // the shell opened itself (descriptor 9).
            assert!(
                listing.contains("fd /etc/hosts") || listing.contains("fd /private/etc/hosts"),
                "{listing}"
            );
            assert!(listing.contains("cwd /"), "{listing}");
            assert!(listing.lines().any(|l| l.starts_with("fd ")), "{listing}");
            for line in listing.lines().filter(|l| l.starts_with("fd ")) {
                assert!(
                    !(line.contains(&data_dir) && line.contains("agent.lock")),
                    "a started program inherits the instance lock: {line}"
                );
                assert!(
                    !line.contains(&socket),
                    "a started program inherits the agent's socket: {line}"
                );
            }
            eprintln!("inherited (minimal environment: {minimal}):\n{listing}");
        }
    }
}
