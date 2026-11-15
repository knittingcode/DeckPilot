//! Windows. Two cases, told apart by the process token:
//!
//! * **Launched by the supervisor** (production): the host already runs in
//!   an AppContainer without capabilities, inside a Job Object the
//!   supervisor holds (`deckpilot-plugin-supervisor`, `windows.rs`). Here
//!   the host only *verifies* that: the token is an AppContainer token with
//!   zero capabilities, and the job it is in has one active process, kill
//!   on close and the process memory limit. If the job cannot be verified
//!   the host assigns itself to its own (nested) job as below. Reported as
//!   `windows-appcontainer-job`, `full`: the user's files (read and write),
//!   the network and process creation are denied by the OS. Which job
//!   applies is in the notes: `job (supervisor): ...` when the supervisor's
//!   job was verified, `job (own, supervisor job not verified: ...)` when
//!   the host had to add its own (the tests require the former).
//! * **Started any other way** (no AppContainer): a Job Object the host
//!   assigns itself to and a Low integrity token, as before. This denies
//!   process creation and writes to Medium-integrity objects (the user's
//!   files), but **not** reading the user's files or network access, so it
//!   is reported as `windows-job-lowil`, `partial`.
use std::ffi::c_void;
use std::ptr::null;

use deckpilot_plugin_abi::protocol::{Enforcement, SandboxReport};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, PSID, SID_AND_ATTRIBUTES, SetTokenInformation,
    TOKEN_ADJUST_DEFAULT, TOKEN_GROUPS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenCapabilities,
    TokenIntegrityLevel, TokenIsAppContainer,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{PROCESS_MEMORY_LIMIT, SandboxError};

/// `SE_GROUP_INTEGRITY` (winnt.h).
const SE_GROUP_INTEGRITY: u32 = 0x20;

fn last_error(what: &str) -> SandboxError {
    // SAFETY: no preconditions.
    SandboxError(format!("{what}: error {}", unsafe { GetLastError() }))
}

fn job() -> Result<(), SandboxError> {
    // SAFETY: Win32 calls with valid, zero-initialized structures; the job
    // handle is intentionally kept open for the life of the process.
    unsafe {
        let job: HANDLE = CreateJobObjectW(null(), null());
        if job.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        info.BasicLimitInformation.ActiveProcessLimit = 1;
        info.ProcessMemoryLimit = PROCESS_MEMORY_LIMIT;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject(limits)"));
        }
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                | JOB_OBJECT_UILIMIT_EXITWINDOWS
                | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_HANDLES
                | JOB_OBJECT_UILIMIT_READCLIPBOARD
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
        };
        if SetInformationJobObject(
            job,
            JobObjectBasicUIRestrictions,
            &ui as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject(ui)"));
        }
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            return Err(last_error("AssignProcessToJobObject"));
        }
    }
    Ok(())
}

fn low_integrity() -> Result<(), SandboxError> {
    // SAFETY: Win32 calls; the SID from ConvertStringSidToSidW is freed with
    // LocalFree and the token handle is closed.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_DEFAULT | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return Err(last_error("OpenProcessToken"));
        }
        let sddl: Vec<u16> = "S-1-16-4096\0".encode_utf16().collect();
        let mut sid: PSID = std::ptr::null_mut();
        if ConvertStringSidToSidW(sddl.as_ptr(), &mut sid) == 0 {
            CloseHandle(token);
            return Err(last_error("ConvertStringSidToSidW"));
        }
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: sid,
                Attributes: SE_GROUP_INTEGRITY,
            },
        };
        let size = std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(sid);
        let ok = SetTokenInformation(
            token,
            TokenIntegrityLevel,
            &label as *const _ as *const _,
            size,
        );
        LocalFree(sid as _);
        CloseHandle(token);
        if ok == 0 {
            return Err(last_error("SetTokenInformation(TokenIntegrityLevel)"));
        }
    }
    Ok(())
}

/// What the token says about the AppContainer: `None` when the process is
/// not in one, otherwise the number of capabilities.
fn app_container() -> Result<Option<u32>, SandboxError> {
    // SAFETY: Win32 calls with out buffers of the stated sizes; the token
    // handle is closed on every path.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(last_error("OpenProcessToken"));
        }
        let mut is_ac = 0u32;
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenIsAppContainer,
            &mut is_ac as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
            &mut len,
        );
        if ok == 0 {
            CloseHandle(token);
            return Err(last_error("GetTokenInformation(TokenIsAppContainer)"));
        }
        if is_ac == 0 {
            CloseHandle(token);
            return Ok(None);
        }
        GetTokenInformation(token, TokenCapabilities, std::ptr::null_mut(), 0, &mut len);
        if len < std::mem::size_of::<u32>() as u32 {
            CloseHandle(token);
            return Err(last_error("GetTokenInformation(TokenCapabilities, size)"));
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(
            token,
            TokenCapabilities,
            buf.as_mut_ptr() as *mut c_void,
            len,
            &mut len,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err(last_error("GetTokenInformation(TokenCapabilities)"));
        }
        Ok(Some((*(buf.as_ptr() as *const TOKEN_GROUPS)).GroupCount))
    }
}

/// Whether the job the process is in (the innermost one) has the limits
/// the supervisor sets. `Err` when it cannot be queried.
fn supervisor_job() -> Result<bool, SandboxError> {
    // SAFETY: Win32 calls with out buffers of the stated sizes; a null job
    // handle means the calling process's job.
    unsafe {
        let mut in_job = 0;
        if IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) == 0 {
            return Err(last_error("IsProcessInJob"));
        }
        if in_job == 0 {
            return Ok(false);
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        if QueryInformationJobObject(
            std::ptr::null_mut(),
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(last_error("QueryInformationJobObject"));
        }
        let flags = info.BasicLimitInformation.LimitFlags;
        let need = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        Ok(flags & need == need
            && info.BasicLimitInformation.ActiveProcessLimit == 1
            && info.ProcessMemoryLimit != 0
            && info.ProcessMemoryLimit <= PROCESS_MEMORY_LIMIT)
    }
}

pub fn apply() -> Result<SandboxReport, SandboxError> {
    let memory = PROCESS_MEMORY_LIMIT >> 20;
    match app_container()? {
        Some(0) => {
            let job_note = match supervisor_job() {
                Ok(true) => format!(
                    "job (supervisor): memory {memory} MiB, 1 active process, kill on close"
                ),
                verified => {
                    // Not verifiable: add our own (nested) job.
                    job()?;
                    format!(
                        "job (own, supervisor job not verified: {verified:?}): memory {memory} MiB, 1 active process, kill on close, ui restricted"
                    )
                }
            };
            Ok(SandboxReport {
                mechanism: "windows-appcontainer-job".into(),
                enforcement: Enforcement::Full,
                notes: vec![
                    "appcontainer: 0 capabilities (no network), low integrity".into(),
                    job_note,
                ],
            })
        }
        Some(n) => Err(SandboxError(format!(
            "AppContainer token has {n} capabilities; the host requires none"
        ))),
        None => {
            job()?;
            low_integrity()?;
            Ok(SandboxReport {
                mechanism: "windows-job-lowil".into(),
                // File reads and network are not restricted without AppContainer.
                enforcement: Enforcement::Partial,
                notes: vec![
                    format!(
                        "job: memory {memory} MiB, 1 active process, kill on close, ui restricted"
                    ),
                    "token: low integrity".into(),
                    "NOT restricted: file reads, network (not launched in an AppContainer by the supervisor)".into(),
                ],
            })
        }
    }
}
