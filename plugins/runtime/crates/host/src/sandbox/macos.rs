//! macOS: Seatbelt applied to the running process with `sandbox_init`.
//!
//! `sandbox_init` with a profile string (flags 0) is deprecated and not
//! documented by Apple, like `sandbox-exec`, but it is the only way for a
//! process to confine itself after start-up with a custom profile. Because
//! the process has already started, the profile does not need to allow
//! loading the binary or system libraries.

use std::ffi::{CStr, CString, c_char, c_int};

use deckpilot_plugin_abi::protocol::{Enforcement, SandboxReport};

use super::SandboxError;

unsafe extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// Deny everything. Reading the root directory and sysctl are not needed
/// after start-up (the spike's `sandbox-exec` profile needed them only to
/// exec the binary).
pub const PROFILE: &str = "(version 1)\n(deny default)\n";

pub fn apply() -> Result<SandboxReport, SandboxError> {
    let core = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: plain syscall with a valid struct.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &core) } != 0 {
        return Err(SandboxError("setrlimit(RLIMIT_CORE)".into()));
    }
    let profile = CString::new(PROFILE).expect("profile has no NUL");
    let mut err: *mut c_char = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated profile and out-pointer.
    let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut err) };
    if rc != 0 {
        let msg = if err.is_null() {
            "sandbox_init failed".to_owned()
        } else {
            // SAFETY: sandbox_init returned a C string; freed right after.
            let m = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { sandbox_free_error(err) };
            m
        };
        return Err(SandboxError(msg));
    }
    Ok(SandboxReport {
        mechanism: "macos-seatbelt".into(),
        enforcement: Enforcement::Full,
        notes: vec!["sandbox_init (deny default)".into(), "RLIMIT_CORE=0".into()],
    })
}
