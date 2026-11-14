//! Windows: the supervisor launches the host inside an AppContainer and a
//! Job Object it holds itself (`std::process::Command` can do neither).
//!
//! * **AppContainer** with **no capabilities** (not even `internetClient`):
//!   a fresh profile per host process, `DeckPilot.PluginHost.<pid>.<n>`,
//!   created with `CreateAppContainerProfile` and deleted with
//!   `DeleteAppContainerProfile` after the process has exited. One profile
//!   per process means two plugin hosts never share an AppContainer SID
//!   (processes of the same AppContainer could open each other), and the
//!   container's private storage (`%LOCALAPPDATA%\Packages\<name>`) does not
//!   survive the process. A profile left behind by a crashed agent is
//!   deleted and recreated when its name is reused; others stay until
//!   removed (they hold no data the host could not write anyway).
//!   The token is Low integrity; file and registry access needs an ACE for
//!   the container SID or `ALL APPLICATION PACKAGES`, which the user's files
//!   do not have; network access needs a capability (loopback is blocked
//!   for every AppContainer).
//! * **Job Object** (held by the supervisor, assigned at creation with
//!   `PROC_THREAD_ATTRIBUTE_JOB_LIST`): one active process, committed memory
//!   limit, kill on job close (the host dies with the supervisor), die on
//!   unhandled exception, UI restrictions.
//! * `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY`: child process creation
//!   restricted (a second layer behind the job's process limit).
//! * `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`: the child inherits exactly the
//!   two pipe ends (stdin read end, stdout write end); stderr is not set.
//! * Environment: `SystemRoot` (Winsock and other system DLLs need it, so
//!   that a denied connection is the sandbox's doing), and `TEMP`, `TMP` and
//!   `LOCALAPPDATA`, which CreateProcessW requires for an AppContainer
//!   process and rewrites to the container's own folders (without them it
//!   fails with ERROR_ENVVAR_NOT_FOUND, 203). They are paths, not access:
//!   what the container may open is decided by its SID. Working directory:
//!   the system directory, which every AppContainer may read.
//!
//! The host binary itself must be readable and executable by the container.
//! Under `Program Files` that is already the case (`ALL APPLICATION
//! PACKAGES` has read and execute); elsewhere (development, tests, per-user
//! installs) the supervisor adds that ACE to the host executable only (not
//! its directory), best effort, once per path and process.
//!
//! Race: the two child pipe ends are inheritable from their creation until
//! `CreateProcessW` returns. A `std::process::Command` spawned by another
//! thread in that window would inherit them too (it does not use a handle
//! list), which only delays end-of-file on the pipe until that process
//! exits; it gives no access to the host.

#![allow(unsafe_code)]

use std::ffi::{OsStr, OsString, c_void};
use std::fmt;
use std::io::{self, PipeReader, PipeWriter};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use deckpilot_plugin_abi::limits::HOST_PROCESS_MEMORY;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, HANDLE_FLAG_INHERIT, LocalFree,
    SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW,
    NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
    TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, FreeSid, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_CAPABILITIES,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemInformation::{
    GetSystemDirectoryW, GetSystemWindowsDirectoryW,
};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows_sys::Win32::System::WindowsProgramming::PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;

/// Prefix of every AppContainer profile name the supervisor creates.
pub const PROFILE_PREFIX: &str = "DeckPilot.PluginHost";

/// `ALL APPLICATION PACKAGES`.
const ALL_APP_PACKAGES: &str = "S-1-15-2-1";

/// Exit code of a host killed by the supervisor (as `Child::kill`).
const KILLED: u32 = 1;

fn last_error(what: &str) -> io::Error {
    let e = io::Error::last_os_error();
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn dir_from(f: unsafe extern "system" fn(*mut u16, u32) -> u32) -> PathBuf {
    let mut buf = vec![0u16; 512];
    // SAFETY: the buffer is valid for its length.
    let n = unsafe { f(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return PathBuf::from("C:\\Windows\\System32");
    }
    PathBuf::from(OsString::from_wide(&buf[..n]))
}

/// The host's working directory: the system directory (`C:\Windows\System32`).
pub(crate) fn system_dir() -> PathBuf {
    dir_from(GetSystemDirectoryW)
}

// ------------------------------------------------------------ AppContainer

/// An AppContainer profile owned by one host process.
struct Profile {
    name: Vec<u16>,
    sid: PSID,
}

// SAFETY: the SID is allocated by CreateAppContainerProfile, owned by this
// value, never mutated, and freed once in Drop.
unsafe impl Send for Profile {}
// SAFETY: see above; shared access only reads it.
unsafe impl Sync for Profile {}

static PROFILE_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Profile {
    fn create() -> io::Result<Self> {
        let n = PROFILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("{PROFILE_PREFIX}.{:x}.{n:x}", std::process::id());
        let wname = wide(OsStr::new(&name));
        let display = wide(OsStr::new("DeckPilot plugin host"));
        let already_exists = (0x8007_0000u32 | ERROR_ALREADY_EXISTS) as i32;
        let mut sid: PSID = null_mut();
        // SAFETY: valid NUL-terminated strings, no capabilities, out pointer.
        let mut hr = unsafe {
            CreateAppContainerProfile(
                wname.as_ptr(),
                display.as_ptr(),
                display.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        if hr == already_exists {
            // Left behind by an earlier agent process with the same pid.
            // SAFETY: valid NUL-terminated string.
            unsafe { DeleteAppContainerProfile(wname.as_ptr()) };
            // SAFETY: as above.
            hr = unsafe {
                CreateAppContainerProfile(
                    wname.as_ptr(),
                    display.as_ptr(),
                    display.as_ptr(),
                    null(),
                    0,
                    &mut sid,
                )
            };
        }
        if hr < 0 || sid.is_null() {
            return Err(io::Error::other(format!(
                "CreateAppContainerProfile({name}): HRESULT {:#010x}",
                hr as u32
            )));
        }
        Ok(Self { name: wname, sid })
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        // SAFETY: the SID came from CreateAppContainerProfile (FreeSid);
        // the name is NUL-terminated.
        unsafe {
            FreeSid(self.sid);
            DeleteAppContainerProfile(self.name.as_ptr());
        }
    }
}

/// Grants `ALL APPLICATION PACKAGES` read and execute on the host
/// executable (only that file), once per path. Errors are ignored: where
/// the ACE cannot be added (e.g. `Program Files` without administrator
/// rights) it is normally inherited already; otherwise `CreateProcessW`
/// fails and reports it.
fn grant_executable(program: &Path) {
    static DONE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let mut done = DONE.lock().unwrap_or_else(|e| e.into_inner());
    if done.iter().any(|p| p == program) {
        return;
    }
    if add_read_execute_ace(program).is_ok() {
        done.push(program.to_path_buf());
    }
}

fn add_read_execute_ace(path: &Path) -> io::Result<()> {
    let wpath = wide(path.as_os_str());
    let wsid = wide(OsStr::new(ALL_APP_PACKAGES));
    // SAFETY: Win32 calls with valid pointers; every allocation (SID,
    // security descriptor, new ACL) is released with LocalFree.
    unsafe {
        let mut sid: PSID = null_mut();
        if ConvertStringSidToSidW(wsid.as_ptr(), &mut sid) == 0 {
            return Err(last_error("ConvertStringSidToSidW"));
        }
        let mut old: *mut ACL = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        let r = GetNamedSecurityInfoW(
            wpath.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut old,
            null_mut(),
            &mut sd,
        );
        if r != 0 {
            LocalFree(sid as _);
            return Err(io::Error::from_raw_os_error(r as i32));
        }
        let ea = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: sid as *mut u16,
            },
        };
        let mut new: *mut ACL = null_mut();
        let r = SetEntriesInAclW(1, &ea, old, &mut new);
        let r = if r == 0 {
            let r = SetNamedSecurityInfoW(
                wpath.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                new,
                null(),
            );
            LocalFree(new as _);
            r
        } else {
            r
        };
        LocalFree(sd as _);
        LocalFree(sid as _);
        if r != 0 {
            return Err(io::Error::from_raw_os_error(r as i32));
        }
    }
    Ok(())
}

// --------------------------------------------------------------------- job

fn create_job(o: &Options) -> io::Result<OwnedHandle> {
    // SAFETY: Win32 calls with valid, zero-initialized structures; the
    // handle is owned by the returned value.
    unsafe {
        let raw = CreateJobObjectW(null(), null());
        if raw.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let job = OwnedHandle::from_raw_handle(raw);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        if o.memory {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        }
        if o.one_process {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        }
        info.BasicLimitInformation.ActiveProcessLimit = 1;
        info.ProcessMemoryLimit = HOST_PROCESS_MEMORY;
        if SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject(limits)"));
        }
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: o.ui,
        };
        if SetInformationJobObject(
            raw,
            JobObjectBasicUIRestrictions,
            &ui as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject(ui)"));
        }
        Ok(job)
    }
}

// ------------------------------------------------------------------- spawn

/// Synchronous anonymous pipe; neither end inheritable.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut r: HANDLE = null_mut();
    let mut w: HANDLE = null_mut();
    // SAFETY: out pointers to locals; the handles are owned right after.
    unsafe {
        if CreatePipe(&mut r, &mut w, null(), 0) == 0 {
            return Err(last_error("CreatePipe"));
        }
        Ok((
            OwnedHandle::from_raw_handle(r),
            OwnedHandle::from_raw_handle(w),
        ))
    }
}

fn set_inheritable(h: &OwnedHandle) -> io::Result<()> {
    // SAFETY: a valid handle owned by the caller.
    if unsafe { SetHandleInformation(h.as_raw_handle(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }
        == 0
    {
        return Err(last_error("SetHandleInformation"));
    }
    Ok(())
}

struct AttributeList {
    buf: Vec<usize>,
}

impl AttributeList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0usize;
        // SAFETY: size query with a null list (fails with
        // ERROR_INSUFFICIENT_BUFFER and sets `size`).
        unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size) };
        if size == 0 {
            return Err(last_error("InitializeProcThreadAttributeList(size)"));
        }
        let mut buf = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        // SAFETY: the buffer is at least `size` bytes and pointer-aligned.
        if unsafe {
            InitializeProcThreadAttributeList(buf.as_mut_ptr().cast(), count, 0, &mut size)
        } == 0
        {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }
        Ok(Self { buf })
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buf.as_mut_ptr().cast()
    }

    /// # Safety
    /// `value` must stay valid until the list is no longer used.
    unsafe fn set<T>(&mut self, attribute: u32, value: *const T, size: usize) -> io::Result<()> {
        // SAFETY: an initialized list; the caller keeps `value` alive.
        let ok = unsafe {
            UpdateProcThreadAttribute(
                self.as_ptr(),
                0,
                attribute as usize,
                value.cast(),
                size,
                null_mut(),
                null(),
            )
        };
        if ok == 0 {
            return Err(last_error(&format!(
                "UpdateProcThreadAttribute({attribute:#x})"
            )));
        }
        Ok(())
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: initialized in `new`.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) }
    }
}

/// Appends one argument with the quoting `CommandLineToArgvW` and the MSVC
/// runtime undo (the same rules as the standard library).
fn append_arg(cmd: &mut Vec<u16>, arg: &OsStr, force_quotes: bool) -> io::Result<()> {
    let w: Vec<u16> = arg.encode_wide().collect();
    if w.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "argument contains NUL",
        ));
    }
    let quote = force_quotes
        || w.is_empty()
        || w.iter()
            .any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'"' as u16);
    if quote {
        cmd.push(b'"' as u16);
    }
    let mut backslashes = 0usize;
    for &c in &w {
        if c == b'\\' as u16 {
            backslashes += 1;
        } else {
            if c == b'"' as u16 {
                cmd.extend(std::iter::repeat_n(b'\\' as u16, backslashes + 1));
            }
            backslashes = 0;
        }
        cmd.push(c);
    }
    if quote {
        cmd.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
        cmd.push(b'"' as u16);
    }
    Ok(())
}

fn command_line(program: &Path, args: &[OsString]) -> io::Result<Vec<u16>> {
    let mut cmd = Vec::new();
    append_arg(&mut cmd, program.as_os_str(), true)?;
    for a in args {
        cmd.push(b' ' as u16);
        append_arg(&mut cmd, a, false)?;
    }
    cmd.push(0);
    Ok(cmd)
}

/// Variables CreateProcessW needs for an AppContainer process; it rewrites
/// them to the container's own folders.
const CONTAINER_REDIRECTED: [&str; 3] = ["LOCALAPPDATA", "TEMP", "TMP"];

/// `SystemRoot=<Windows directory>`, the container-redirected variables
/// that are set in the supervisor's environment, then the block terminator.
fn environment() -> Vec<u16> {
    let mut env: Vec<u16> = OsStr::new("SystemRoot=").encode_wide().collect();
    env.extend(
        dir_from(GetSystemWindowsDirectoryW)
            .as_os_str()
            .encode_wide(),
    );
    env.push(0);
    for name in CONTAINER_REDIRECTED {
        if let Some(value) = std::env::var_os(name).filter(|v| !v.is_empty()) {
            env.extend(OsStr::new(name).encode_wide());
            env.push(u16::from(b'='));
            env.extend(value.encode_wide());
            env.push(0);
        }
    }
    env.push(0);
    env
}

/// A host process in its AppContainer and Job Object.
pub(crate) struct Process {
    // Drop order matters: process and job before the profile.
    process: OwnedHandle,
    job: OwnedHandle,
    pid: u32,
    status: Option<ExitStatus>,
    _profile: Profile,
}

impl fmt::Debug for Process {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Process")
            .field("pid", &self.pid)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

pub(crate) struct Spawned {
    pub process: Process,
    pub stdin: PipeWriter,
    pub stdout: PipeReader,
}

/// Temporary (diagnosis of STATUS_DLL_INIT_FAILED): the parts of the
/// launch that can be toggled one at a time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    pub app_container: bool,
    pub job: bool,
    pub memory: bool,
    pub one_process: bool,
    pub ui: u32,
    pub child_policy: bool,
    pub console: u32,
}

const ALL_UI: u32 = JOB_OBJECT_UILIMIT_DESKTOP
    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
    | JOB_OBJECT_UILIMIT_EXITWINDOWS
    | JOB_OBJECT_UILIMIT_GLOBALATOMS
    | JOB_OBJECT_UILIMIT_HANDLES
    | JOB_OBJECT_UILIMIT_READCLIPBOARD
    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;

pub(crate) const PRODUCTION: Options = Options {
    app_container: true,
    job: true,
    memory: true,
    one_process: true,
    ui: ALL_UI,
    child_policy: true,
    console: DETACHED_PROCESS,
};

pub(crate) fn spawn(program: &Path, args: &[OsString], cwd: &Path) -> io::Result<Spawned> {
    spawn_with(program, args, cwd, &PRODUCTION)
}

pub(crate) fn spawn_with(
    program: &Path,
    args: &[OsString],
    cwd: &Path,
    o: &Options,
) -> io::Result<Spawned> {
    let program = std::path::absolute(program)?;
    grant_executable(&program);
    let profile = Profile::create()?;
    let job = create_job(o)?;
    let (child_stdin, our_stdin) = pipe()?;
    let (our_stdout, child_stdout) = pipe()?;
    set_inheritable(&child_stdin)?;
    set_inheritable(&child_stdout)?;

    let app = wide(program.as_os_str());
    let mut cmd = command_line(&program, args)?;
    let env = environment();
    let wcwd = wide(cwd.as_os_str());

    let caps = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid,
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let handles: [HANDLE; 2] = [child_stdin.as_raw_handle(), child_stdout.as_raw_handle()];
    let jobs: [HANDLE; 1] = [job.as_raw_handle()];
    let child_policy: u32 = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;

    let mut attrs = AttributeList::new(4)?;
    // SAFETY: every value outlives `attrs`' use in CreateProcessW below.
    unsafe {
        if o.app_container {
            attrs.set(
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
                &caps,
                std::mem::size_of_val(&caps),
            )?;
        }
        attrs.set(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            handles.as_ptr(),
            std::mem::size_of_val(&handles),
        )?;
        if o.job {
            attrs.set(
                PROC_THREAD_ATTRIBUTE_JOB_LIST,
                jobs.as_ptr(),
                std::mem::size_of_val(&jobs),
            )?;
        }
        if o.child_policy {
            attrs.set(
                PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,
                &child_policy,
                std::mem::size_of_val(&child_policy),
            )?;
        }
    }

    // SAFETY: zeroed STARTUPINFOEXW is a valid "nothing set" value.
    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = child_stdin.as_raw_handle();
    si.StartupInfo.hStdOutput = child_stdout.as_raw_handle();
    si.StartupInfo.hStdError = null_mut();
    si.lpAttributeList = attrs.as_ptr();
    // SAFETY: zeroed PROCESS_INFORMATION is filled by CreateProcessW.
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: all strings NUL-terminated, the command line is mutable, the
    // environment block is double-NUL-terminated UTF-16, the attribute list
    // and its values are alive.
    let ok = unsafe {
        CreateProcessW(
            app.as_ptr(),
            cmd.as_mut_ptr(),
            null(),
            null(),
            1, // inherit: limited to `handles` by the handle list
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | o.console,
            env.as_ptr().cast(),
            wcwd.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(last_error("CreateProcessW (AppContainer)"));
    }
    drop(attrs);
    drop(child_stdin);
    drop(child_stdout);
    // SAFETY: handles returned by CreateProcessW, owned from here on.
    let process = unsafe {
        CloseHandle(pi.hThread);
        OwnedHandle::from_raw_handle(pi.hProcess)
    };
    Ok(Spawned {
        process: Process {
            process,
            job,
            pid: pi.dwProcessId,
            status: None,
            _profile: profile,
        },
        stdin: PipeWriter::from(our_stdin),
        stdout: PipeReader::from(our_stdout),
    })
}

impl Process {
    pub(crate) fn id(&self) -> u32 {
        self.pid
    }

    /// Terminates every process in the job.
    pub(crate) fn kill(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        // SAFETY: a valid job handle.
        if unsafe { TerminateJobObject(self.job.as_raw_handle(), KILLED) } == 0 {
            return Err(last_error("TerminateJobObject"));
        }
        Ok(())
    }

    fn wait_ms(&mut self, ms: u32) -> io::Result<Option<ExitStatus>> {
        if let Some(s) = self.status {
            return Ok(Some(s));
        }
        // SAFETY: a valid process handle.
        let r = unsafe { WaitForSingleObject(self.process.as_raw_handle(), ms) };
        if r != WAIT_OBJECT_0 {
            return Ok(None);
        }
        let mut code = 0u32;
        // SAFETY: a valid process handle and an out pointer.
        if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
            return Err(last_error("GetExitCodeProcess"));
        }
        let s = ExitStatus::from_raw(code);
        self.status = Some(s);
        Ok(Some(s))
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.wait_ms(0)
    }

    pub(crate) fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(s) = self.wait_ms(INFINITE)? {
                return Ok(s);
            }
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Kill-on-close would do this when the job handle closes; doing it
        // first lets the profile be deleted after the process is gone.
        let _ = self.kill();
        let _ = self.wait_ms(5_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    fn line(args: &[&str]) -> String {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let w = command_line(Path::new("C:\\a b\\host.exe"), &args).unwrap();
        String::from_utf16(&w[..w.len() - 1]).unwrap()
    }

    /// Temporary diagnosis: which part of the launch makes the host fail
    /// with STATUS_DLL_INIT_FAILED. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn diagnose_launch() {
        let deps = std::env::current_exe().unwrap();
        let debug = deps.parent().unwrap().parent().unwrap().to_path_buf();
        let programs = [
            debug.join("deckpilot-plugin-host.exe"),
            debug.join("deckpilot-sandbox-probe.exe"),
            system_dir().join("hostname.exe"),
        ];
        let p = PRODUCTION;
        let mut configs: Vec<(&str, Options)> = vec![
            ("production(detached)", p),
            (
                "no_window",
                Options {
                    console: CREATE_NO_WINDOW,
                    ..p
                },
            ),
            ("plain_console_flags", Options { console: 0, ..p }),
            (
                "no_window+one_process=off",
                Options {
                    console: CREATE_NO_WINDOW,
                    one_process: false,
                    ..p
                },
            ),
            (
                "no_window+child_policy=off",
                Options {
                    console: CREATE_NO_WINDOW,
                    child_policy: false,
                    ..p
                },
            ),
            (
                "no_window+ui=0",
                Options {
                    console: CREATE_NO_WINDOW,
                    ui: 0,
                    ..p
                },
            ),
            (
                "no_window+job=off",
                Options {
                    console: CREATE_NO_WINDOW,
                    job: false,
                    ..p
                },
            ),
            ("ui=0", Options { ui: 0, ..p }),
            ("memory=off", Options { memory: false, ..p }),
            (
                "one_process=off",
                Options {
                    one_process: false,
                    ..p
                },
            ),
            (
                "child_policy=off",
                Options {
                    child_policy: false,
                    ..p
                },
            ),
            ("job=off", Options { job: false, ..p }),
            (
                "app_container=off",
                Options {
                    app_container: false,
                    ..p
                },
            ),
            (
                "everything_off",
                Options {
                    app_container: false,
                    job: false,
                    child_policy: false,
                    ..p
                },
            ),
        ];
        for bit in [
            JOB_OBJECT_UILIMIT_DESKTOP,
            JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
            JOB_OBJECT_UILIMIT_EXITWINDOWS,
            JOB_OBJECT_UILIMIT_GLOBALATOMS,
            JOB_OBJECT_UILIMIT_HANDLES,
            JOB_OBJECT_UILIMIT_READCLIPBOARD,
            JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
            JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
        ] {
            configs.push((
                Box::leak(format!("ui_without_{bit:#x}").into_boxed_str()),
                Options {
                    ui: ALL_UI & !bit,
                    ..p
                },
            ));
        }
        for program in &programs {
            for (name, o) in &configs {
                let r = match spawn_with(program, &[], &system_dir(), o) {
                    Err(e) => format!("spawn error: {e}"),
                    Ok(mut s) => {
                        drop(s.stdin);
                        let mut out = String::new();
                        let _ = std::io::Read::read_to_string(&mut s.stdout, &mut out);
                        match s.process.wait_ms(10_000) {
                            Ok(Some(st)) => format!(
                                "exit {:#x} out {:?}",
                                st.code().unwrap_or(0) as u32,
                                out.chars().take(200).collect::<String>()
                            ),
                            Ok(None) => "still running after 10 s".to_owned(),
                            Err(e) => format!("wait error: {e}"),
                        }
                    }
                };
                println!(
                    "DIAG {} {name}: {r}",
                    program.file_name().unwrap().to_string_lossy()
                );
            }
        }
    }

    #[test]
    fn command_line_quoting() {
        assert_eq!(line(&[]), r#""C:\a b\host.exe""#);
        assert_eq!(line(&["x", ""]), r#""C:\a b\host.exe" x """#);
        assert_eq!(
            line(&["a b", r#"q"t"#]),
            r#""C:\a b\host.exe" "a b" "q\"t""#
        );
        assert_eq!(line(&[r"c:\d\ e\"]), r#""C:\a b\host.exe" "c:\d\ e\\""#);
        assert_eq!(line(&[r"c:\d\"]), r#""C:\a b\host.exe" c:\d\"#);
    }

    #[test]
    fn environment_is_system_root_and_container_folders_only() {
        let env = environment();
        let s = String::from_utf16(&env).unwrap();
        assert!(s.starts_with("SystemRoot="), "{s}");
        assert!(s.ends_with("\0\0"));
        let names: Vec<&str> = s
            .trim_end_matches('\0')
            .split('\0')
            .map(|entry| entry.split('=').next().unwrap())
            .collect();
        for name in &names[1..] {
            assert!(CONTAINER_REDIRECTED.contains(name), "{name} in {s}");
        }
    }
}
