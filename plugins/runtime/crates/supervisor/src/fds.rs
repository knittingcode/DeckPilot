//! The host inherits standard input, output and error only.
//!
//! `Command` opens its own descriptors close-on-exec, but descriptors the
//! supervisor's process inherited without that flag (for example a build
//! tool's jobserver pipes, or anything the agent's parent left open) would
//! otherwise reach the plugin host. Before `exec`, the child marks every
//! descriptor from 3 up close-on-exec: they vanish at `exec`, while the
//! standard library's own exec-error pipe (already close-on-exec) keeps
//! working.
//!
//! This is the only `unsafe` code in the crate: `pre_exec` runs between
//! `fork` and `exec`, where only async-signal-safe calls are allowed;
//! `close_range` and `fcntl` are.

use std::process::Command;

/// Upper bound for the per-descriptor fallback (when `close_range` is not
/// available); descriptor numbers above it are not inherited in practice.
#[cfg(unix)]
const MAX_FALLBACK_FD: libc::c_int = 65_536;

#[cfg(unix)]
#[allow(unsafe_code)]
pub(crate) fn inherit_only_stdio(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure only calls the async-signal-safe functions
    // close_range(2), getrlimit(2) and fcntl(2), allocates nothing and
    // touches no shared state.
    unsafe {
        command.pre_exec(|| {
            mark_close_on_exec_from(3);
            Ok(())
        });
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn mark_close_on_exec_from(first: libc::c_int) {
    #[cfg(target_os = "linux")]
    {
        // Linux 5.11+: one call. CLOSE_RANGE_CLOEXEC = 1 << 2.
        const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
        // SAFETY: raw syscall with integer arguments.
        let r = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                first as libc::c_uint,
                libc::c_uint::MAX,
                CLOSE_RANGE_CLOEXEC,
            )
        };
        if r == 0 {
            return;
        }
    }
    // SAFETY: getrlimit writes into the struct we pass.
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let max = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0 {
        libc::c_int::try_from(limit.rlim_cur)
            .unwrap_or(MAX_FALLBACK_FD)
            .min(MAX_FALLBACK_FD)
    } else {
        MAX_FALLBACK_FD
    };
    for fd in first..max {
        // SAFETY: fcntl on a descriptor number; EBADF for unused numbers
        // is expected and ignored.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

/// Windows: `Command` only passes handles that are explicitly inheritable
/// and listed for the child (stdio); nothing else to do.
#[cfg(not(unix))]
pub(crate) fn inherit_only_stdio(_command: &mut Command) {}
