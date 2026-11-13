//! The built agent executable starts at all. Windows refuses to load it
//! when a DLL import cannot be resolved (e.g. `comctl32.dll` version 6
//! without an application manifest), long before `main` runs; other tests
//! use the agent as a library and would not notice.

use std::process::Command;

#[test]
fn the_executable_starts_and_prints_its_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
        .arg("--version")
        .output()
        .expect("the executable can be started");
    assert!(
        output.status.success(),
        "deckpilot-agent --version failed with {} (on Windows 0xc0000139 means a missing DLL entry point)\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with(&format!("deckpilot-agent {}", env!("CARGO_PKG_VERSION"))),
        "{stdout}"
    );
}
