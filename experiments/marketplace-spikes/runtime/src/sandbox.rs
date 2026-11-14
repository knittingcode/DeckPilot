//! OS sandbox around the plugin host process (design §3.5, layer 4).
//!
//! Implemented here only for macOS, by launching the host under
//! `/usr/bin/sandbox-exec` with a deny-default SBPL profile. `sandbox-exec`
//! and SBPL are deprecated and undocumented by Apple. The design's v1 plan
//! is `sandbox_init` applied by the host to itself; both use the same
//! Seatbelt profile language.
//!
//! Not implemented (not verified by this spike):
//! - Windows: AppContainer/LPAC with no capabilities + Job Object
//!   (`windows` / `windows-sys` crates: `CreateAppContainerProfile`,
//!   `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`, `CreateJobObjectW`,
//!   `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`).
//! - Linux: `landlock` crate (file system and, ABI v4+, TCP bind/connect),
//!   `seccompiler` or `libseccomp` crate (syscall allowlist), `rustix`/`libc`
//!   for `PR_SET_NO_NEW_PRIVS`, `PR_SET_DUMPABLE` and rlimits.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct SandboxProfile {
    /// Seatbelt profile source.
    pub sbpl: String,
}

impl SandboxProfile {
    /// True if this platform has an implementation in the spike.
    pub const SUPPORTED: bool = cfg!(target_os = "macos");

    /// Deny-default profile: the host may read its own binary and system
    /// libraries, read and write `scratch`, and nothing else. No network,
    /// no process execution after the initial exec, no mach-lookup.
    #[cfg(target_os = "macos")]
    pub fn macos_deny_default(host_bin: &Path, scratch: &Path) -> std::io::Result<Self> {
        let host = host_bin.canonicalize()?;
        let scratch = scratch.canonicalize()?;
        let q = |p: &Path| {
            p.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let sbpl = format!(
            r#"(version 1)
(deny default)
(allow process-exec (literal "{host}"))
(allow file-read*
    (literal "{host}")
    (subpath "/usr/lib")
    (subpath "/System/Library")
    (subpath "/System/Cryptexes")
    (subpath "/System/Volumes/Preboot/Cryptexes")
    (subpath "/private/var/db/dyld")
    (literal "/dev/null")
    (literal "/dev/urandom")
    (literal "/dev/random"))
(allow file-read-metadata (literal "/") (literal "/private") (literal "/private/var") (literal "/var"))
; Found by bisection: the process aborts at start-up unless it may read the
; root directory itself (names of top-level entries only, not their contents).
(allow file-read-data (literal "/"))
(allow file-read* file-write* (subpath "{scratch}"))
(allow sysctl-read)
"#,
            host = q(host.as_path()),
            scratch = q(scratch.as_path()),
        );
        Ok(Self { sbpl })
    }

    /// Command that starts `host_bin` inside this sandbox.
    pub fn command(&self, host_bin: &Path) -> Command {
        #[cfg(target_os = "macos")]
        {
            let mut c = Command::new("/usr/bin/sandbox-exec");
            c.arg("-p").arg(&self.sbpl).arg(host_bin);
            c
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = host_bin;
            panic!("OS sandbox not implemented on this platform in the spike");
        }
    }
}
