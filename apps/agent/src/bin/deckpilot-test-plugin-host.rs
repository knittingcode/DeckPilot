//! Test only (features `plugins` and `test-hooks`, never in a release
//! build): a stand-in for `deckpilot-plugin-host` whose `ready` message
//! reports a **partial** OS sandbox, as the real host does on Linux without
//! Landlock or on Windows outside an `AppContainer`. `plugins_e2e` points the
//! agent at it to check that such a host is stopped before it receives a
//! plugin. It runs no plugin code: it exits with code 3 if the supervisor
//! ever sends it anything but `shutdown`.

use std::io::{self, BufReader};
use std::process::ExitCode;

use deckpilot_plugin_abi::frame::{read_frame, write_frame};
use deckpilot_plugin_abi::limits::{MAX_FRAME_TO_SUPERVISOR, MAX_LOAD_FRAME};
use deckpilot_plugin_abi::protocol::{
    Enforcement, PROTOCOL_VERSION, SandboxReport, ToHost, ToSupervisor,
};

fn main() -> ExitCode {
    let ready = ToSupervisor::Ready {
        protocol: PROTOCOL_VERSION,
        host_version: "test-partial".into(),
        engine: "none".into(),
        sandbox: SandboxReport {
            mechanism: "test-partial".into(),
            enforcement: Enforcement::Partial,
            notes: vec!["a test stand-in for a host without every sandbox layer".into()],
        },
    };
    if write_frame(&mut io::stdout().lock(), &ready, MAX_FRAME_TO_SUPERVISOR).is_err() {
        return ExitCode::from(2);
    }
    let mut input = BufReader::new(io::stdin().lock());
    match read_frame::<_, ToHost>(&mut input, MAX_LOAD_FRAME) {
        // Killed or closed by the supervisor: what the agent must do.
        Ok(None | Some(ToHost::Shutdown {})) | Err(_) => ExitCode::SUCCESS,
        // A module or an invocation reached a host with a partial sandbox.
        Ok(Some(_)) => ExitCode::from(3),
    }
}
