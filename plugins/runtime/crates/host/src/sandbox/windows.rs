//! Windows: a Job Object the host assigns itself to, and a Low integrity
//! token.
//!
//! * Job: process memory limit, one active process (no child processes),
//!   kill on job close, die on unhandled exception, all UI restrictions.
//! * Low integrity (`S-1-16-4096`): the process can no longer open
//!   Medium-integrity objects for writing (no-write-up), which covers the
//!   user's files and registry keys.
//!
//! **Not covered** (requires launching the host in an AppContainer/LPAC,
//! which is not implemented): reading the user's files and network access.

use std::ptr::null;

use deckpilot_plugin_abi::protocol::{Enforcement, SandboxReport};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows_sys::Win32::Security::{
    GetLengthSid, PSID, SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
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

pub fn apply() -> Result<SandboxReport, SandboxError> {
    job()?;
    low_integrity()?;
    Ok(SandboxReport {
        mechanism: "windows-job-lowil".into(),
        // File reads and network are not restricted without AppContainer.
        enforcement: Enforcement::Partial,
        notes: vec![
            format!(
                "job: memory {} MiB, 1 active process, kill on close, ui restricted",
                PROCESS_MEMORY_LIMIT >> 20
            ),
            "token: low integrity".into(),
            "NOT restricted: file reads, network (AppContainer not implemented)".into(),
        ],
    })
}
