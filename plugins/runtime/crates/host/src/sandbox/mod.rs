//! OS sandbox the host applies to itself before it reads any plugin byte
//! (design §3.5, layer 4: defense in depth behind the wasm layer).
//!
//! | Platform | Applied here | Denies |
//! | --- | --- | --- |
//! | macOS | Seatbelt via `sandbox_init` with a `(deny default)` profile; `RLIMIT_CORE = 0` | file reads and writes, network, process exec and fork, mach-lookup, IPC |
//! | Linux | `PR_SET_NO_NEW_PRIVS`, `PR_SET_DUMPABLE = 0`, `RLIMIT_CORE = 0`, Landlock (best available ABI: all file access, TCP bind/connect on ABI 4+, abstract-socket and signal scoping on ABI 6+), seccomp-bpf allowlist | `open*`, `socket`, `connect`, `execve`, `fork`/`clone`, `ptrace` and everything else not on the allowlist (EPERM) |
//! | Windows | Job Object (process memory limit, one active process, kill on close, die on unhandled exception, UI restrictions) and the process token lowered to Low integrity | creating processes; writes to Medium-integrity objects (the user's files). **Not** denied: reading the user's files and network access (needs AppContainer, not implemented) |
//!
//! [`apply`] returns a [`SandboxReport`] that the host sends in `ready`;
//! the supervisor refuses a host without one unless configured otherwise.

use deckpilot_plugin_abi::protocol::{Enforcement, SandboxReport};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Whether to apply the sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Enforce,
    /// Development and tests only (`--no-os-sandbox`). The report says
    /// `none`, so a supervisor that requires a sandbox refuses the host.
    Disabled,
}

#[derive(Debug)]
pub struct SandboxError(pub String);

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sandbox: {}", self.0)
    }
}

impl std::error::Error for SandboxError {}

/// Process memory limit enforced by the OS where available (Windows Job
/// Object). Linear memory is capped separately at 64 MiB per Store.
pub const PROCESS_MEMORY_LIMIT: usize = 256 << 20;

/// First stage, while the process is still single-threaded (Linux:
/// `no_new_privs`, rlimits, Landlock, which new threads inherit). Create the
/// engine and its epoch ticker thread after this, then call
/// [`Pending::finish`] before reading any plugin byte.
pub fn begin(mode: Mode) -> Result<Pending, SandboxError> {
    if mode == Mode::Disabled {
        return Ok(Pending { inner: None });
    }
    #[cfg(target_os = "linux")]
    let inner = linux::begin()?;
    #[cfg(not(target_os = "linux"))]
    let inner = ();
    Ok(Pending { inner: Some(inner) })
}

#[cfg(target_os = "linux")]
type PendingInner = linux::Pending;
#[cfg(not(target_os = "linux"))]
type PendingInner = ();

/// A sandbox whose first stage is applied. Irreversible.
#[derive(Debug)]
pub struct Pending {
    inner: Option<PendingInner>,
}

impl Pending {
    /// What the first stage did (Linux: Landlock status); empty elsewhere.
    pub fn notes(&self) -> Vec<String> {
        #[cfg(target_os = "linux")]
        if let Some(inner) = &self.inner {
            return inner.notes().to_vec();
        }
        Vec::new()
    }

    /// Second stage, for the whole process (all threads).
    pub fn finish(self) -> Result<SandboxReport, SandboxError> {
        let Some(inner) = self.inner else {
            return Ok(SandboxReport {
                mechanism: "none".into(),
                enforcement: Enforcement::None,
                notes: vec!["disabled by --no-os-sandbox".into()],
            });
        };
        #[cfg(target_os = "macos")]
        {
            let () = inner;
            macos::apply()
        }
        #[cfg(target_os = "linux")]
        {
            linux::finish(inner)
        }
        #[cfg(windows)]
        {
            let () = inner;
            windows::apply()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            let () = inner;
            Err(SandboxError(
                "no sandbox implementation for this platform".into(),
            ))
        }
    }
}

/// Both stages at once, for a process that starts no threads in between.
pub fn apply(mode: Mode) -> Result<SandboxReport, SandboxError> {
    begin(mode)?.finish()
}
