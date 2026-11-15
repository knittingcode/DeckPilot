//! Test-only probe (not shipped): applies exactly the host's sandbox
//! (`deckpilot_plugin_host::sandbox`, same two stages, with a thread started
//! in between like the host's epoch ticker), then attempts operations from
//! native code, i.e. it models a plugin that has escaped the wasm layer.
//!
//! Usage: `deckpilot-sandbox-probe [--no-os-sandbox] [--stage1-only] (<kind> <target>)...`
//! Kinds: `read_file`, `write_file`, `list_dir`, `connect_tcp` (`ip:port`),
//! `connect_unix` (Unix only), `exec`, `env_count`, `open_fds` (Unix only),
//! `alloc_mib` (allocate and touch that many MiB natively), `sleep`
//! (seconds, without reading stdin). Windows only: `signal_event` and
//! `write_handle` (a raw handle value the parent created, in decimal),
//! `job_info` (the limits of the job the process is in), `child_policy`
//! (the child process mitigation policy), `clipboard_read`,
//! `clipboard_write` (text), `atom_find`, `atom_add` (global atom name) and
//! `create_desktop` (name).
//! Each probe runs on the main thread and on the helper thread. Output: one
//! JSON line with the sandbox report, then one per probe and thread:
//! `{"kind","target","thread","ok","detail"}`.

use std::io::{self, Write};
use std::sync::mpsc;
use std::time::Duration;

use deckpilot_plugin_host::sandbox::{self, Mode};
use serde_json::json;

fn probe(kind: &str, target: &str) -> (bool, String) {
    let r: io::Result<String> = match kind {
        "read_file" => std::fs::read(target).map(|b| format!("read {} bytes", b.len())),
        "write_file" => std::fs::write(target, b"probe").map(|_| "written".into()),
        "list_dir" => std::fs::read_dir(target).map(|d| format!("{} entries", d.count())),
        "connect_tcp" => target
            .parse()
            .map_err(io::Error::other)
            .and_then(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(3)))
            .map(|_| "connected".into()),
        "exec" => std::process::Command::new(target)
            .output()
            .map(|o| format!("exit {}", o.status)),
        #[cfg(unix)]
        "connect_unix" => {
            std::os::unix::net::UnixStream::connect(target).map(|_| "connected".into())
        }
        "env_count" => Ok(format!("{}", std::env::vars_os().count())),
        #[cfg(unix)]
        "open_fds" => Ok(open_fds().to_string()),
        "alloc_mib" => target
            .parse::<usize>()
            .map_err(io::Error::other)
            .and_then(alloc_mib),
        "sleep" => target.parse::<u64>().map_err(io::Error::other).map(|s| {
            std::thread::sleep(Duration::from_secs(s));
            "slept".into()
        }),
        #[cfg(windows)]
        "signal_event" => win::signal_event(target),
        #[cfg(windows)]
        "write_handle" => win::write_handle(target),
        #[cfg(windows)]
        "job_info" => win::job_info(),
        #[cfg(windows)]
        "child_policy" => win::child_policy(),
        #[cfg(windows)]
        "clipboard_read" => win::clipboard_read(),
        #[cfg(windows)]
        "clipboard_write" => win::clipboard_write(target),
        #[cfg(windows)]
        "atom_find" => win::atom_find(target),
        #[cfg(windows)]
        "atom_add" => win::atom_add(target),
        #[cfg(windows)]
        "create_desktop" => win::create_desktop(target),
        _ => Err(io::Error::other("unknown probe")),
    };
    match r {
        Ok(d) => (true, d),
        Err(e) => (false, format!("{:?}: {e}", e.kind())),
    }
}

/// Commits and touches `mib` MiB of native heap (outside wasm), then
/// frees it. Fails when the allocator cannot commit the memory.
fn alloc_mib(mib: usize) -> io::Result<String> {
    let bytes = mib << 20;
    let mut v: Vec<u8> = Vec::new();
    v.try_reserve_exact(bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::OutOfMemory, e.to_string()))?;
    v.resize(bytes, 1);
    Ok(format!("allocated {mib} MiB ({} bytes)", v.len()))
}

/// Descriptors other than 0, 1 and 2 that are open. Meaningful only
/// without the sandbox (under seccomp `fcntl` itself is denied).
#[cfg(unix)]
fn open_fds() -> usize {
    (3..1024)
        // SAFETY: F_GETFD on any integer is harmless.
        .filter(|fd| unsafe { libc::fcntl(*fd, libc::F_GETFD) } != -1)
        .count()
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mode = if args.first().is_some_and(|a| a == "--no-os-sandbox") {
        args.remove(0);
        Mode::Disabled
    } else {
        Mode::Enforce
    };
    // `--stage1-only`: apply only the first stage (Linux: Landlock and
    // process attributes, no seccomp), to test that layer on its own.
    let stage1_only = args.first().is_some_and(|a| a == "--stage1-only");
    if stage1_only {
        args.remove(0);
    }
    let probes: Vec<(String, String)> = args
        .chunks(2)
        .map(|c| (c[0].clone(), c.get(1).cloned().unwrap_or_default()))
        .collect();

    let pending = match sandbox::begin(mode) {
        Ok(p) => p,
        Err(e) => {
            println!("{}", json!({"error": e.to_string()}));
            std::process::exit(5);
        }
    };
    // Like the host's epoch ticker: a thread created between the stages.
    let (to_helper, helper_rx) = mpsc::channel::<(String, String)>();
    let (helper_tx, from_helper) = mpsc::channel::<(bool, String)>();
    std::thread::spawn(move || {
        for (k, t) in helper_rx {
            let _ = helper_tx.send(probe(&k, &t));
        }
    });
    let report = if stage1_only {
        json!({ "stage1Only": true, "notes": pending.notes() })
    } else {
        match pending.finish() {
            Ok(r) => serde_json::to_value(r).unwrap_or_default(),
            Err(e) => {
                println!("{}", json!({"error": e.to_string()}));
                std::process::exit(5);
            }
        }
    };
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{}", json!({ "sandbox": report }));
    for (kind, target) in probes {
        let (ok, detail) = probe(&kind, &target);
        let _ = writeln!(
            out,
            "{}",
            json!({"kind": kind, "target": target, "thread": "main", "ok": ok, "detail": detail})
        );
        let _ = to_helper.send((kind.clone(), target.clone()));
        let (ok, detail) = from_helper
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or((false, "helper thread did not answer".into()));
        let _ = writeln!(
            out,
            "{}",
            json!({"kind": kind, "target": target, "thread": "helper", "ok": ok, "detail": detail})
        );
    }
    let _ = out.flush();
}

/// Windows probes. User32 is loaded only by the UI probes (dynamically), so
/// the other probes run without a window station connection, like the host.
#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::io;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{GetHandleInformation, GlobalFree, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::WriteFile;
    use windows_sys::Win32::System::DataExchange::{GlobalAddAtomW, GlobalFindAtomW};
    use windows_sys::Win32::System::JobObjects::{
        IsProcessInJob, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_UI_RESTRICTIONS,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
        JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessMitigationPolicy, ProcessChildProcessPolicy, SetEvent,
    };

    /// `CF_UNICODETEXT`.
    const CF_UNICODETEXT: u32 = 13;
    /// `GENERIC_ALL`.
    const GENERIC_ALL: u32 = 0x1000_0000;

    type Proc = unsafe extern "system" fn() -> isize;
    type OpenClipboard = unsafe extern "system" fn(*mut c_void) -> i32;
    type NoArgs = unsafe extern "system" fn() -> i32;
    type GetClipboardData = unsafe extern "system" fn(u32) -> HANDLE;
    type SetClipboardData = unsafe extern "system" fn(u32, HANDLE) -> HANDLE;
    type CreateDesktopW = unsafe extern "system" fn(
        *const u16,
        *const u16,
        *const c_void,
        u32,
        u32,
        *const c_void,
    ) -> HANDLE;
    type CloseDesktop = unsafe extern "system" fn(HANDLE) -> i32;

    fn os_error(what: &str) -> io::Error {
        let e = io::Error::last_os_error();
        io::Error::new(e.kind(), format!("{what}: {e}"))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn handle(value: &str) -> io::Result<HANDLE> {
        value
            .parse::<usize>()
            .map(|v| v as HANDLE)
            .map_err(io::Error::other)
    }

    fn handle_valid(h: HANDLE) -> bool {
        let mut flags = 0u32;
        // SAFETY: GetHandleInformation only reads the handle table.
        unsafe { GetHandleInformation(h, &mut flags) != 0 }
    }

    /// `SetEvent` on a handle value the parent passed. Had the event been
    /// inherited, this would signal the parent's event (which it checks).
    pub fn signal_event(value: &str) -> io::Result<String> {
        let h = handle(value)?;
        let valid = handle_valid(h);
        // SAFETY: SetEvent on an arbitrary value fails with
        // ERROR_INVALID_HANDLE unless it names an event of this process.
        if unsafe { SetEvent(h) } == 0 {
            return Err(os_error(&format!("SetEvent (handle valid: {valid})")));
        }
        Ok(format!("SetEvent succeeded (handle valid: {valid})"))
    }

    /// `WriteFile` on a handle value the parent passed. Had the file handle
    /// been inherited, this would write to the parent's file (it checks).
    pub fn write_handle(value: &str) -> io::Result<String> {
        let h = handle(value)?;
        let valid = handle_valid(h);
        let data = b"leaked";
        let mut written = 0u32;
        // SAFETY: WriteFile on an arbitrary value fails unless it names a
        // writable object of this process; the buffer is valid.
        if unsafe {
            WriteFile(
                h,
                data.as_ptr(),
                data.len() as u32,
                &mut written,
                null_mut(),
            )
        } == 0
        {
            return Err(os_error(&format!("WriteFile (handle valid: {valid})")));
        }
        Ok(format!("wrote {written} bytes (handle valid: {valid})"))
    }

    /// # Safety
    /// `T` must be the information class's structure.
    unsafe fn query_job<T>(class: i32, out: &mut T, what: &str) -> io::Result<()> {
        // SAFETY: an out buffer of T's size; a null job handle means the
        // caller's (innermost) job.
        if unsafe {
            QueryInformationJobObject(
                null_mut(),
                class,
                out as *mut T as *mut c_void,
                std::mem::size_of::<T>() as u32,
                null_mut(),
            )
        } == 0
        {
            return Err(os_error(what));
        }
        Ok(())
    }

    /// The limits of the innermost job this process is in, as
    /// `in_job=1 flags=0x.. active_limit=.. memory=.. ui=0x.. total=.. active=..`.
    pub fn job_info() -> io::Result<String> {
        let mut in_job = 0;
        // SAFETY: the current process pseudo handle and an out pointer.
        if unsafe { IsProcessInJob(GetCurrentProcess(), null_mut(), &mut in_job) } == 0 {
            return Err(os_error("IsProcessInJob"));
        }
        if in_job == 0 {
            return Ok("in_job=0".into());
        }
        // SAFETY: zeroed plain C structures, each queried with its class.
        let (ext, ui, acct) = unsafe {
            let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            query_job(JobObjectExtendedLimitInformation, &mut ext, "limits")?;
            let mut ui: JOBOBJECT_BASIC_UI_RESTRICTIONS = std::mem::zeroed();
            query_job(JobObjectBasicUIRestrictions, &mut ui, "ui")?;
            let mut acct: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            query_job(JobObjectBasicAccountingInformation, &mut acct, "accounting")?;
            (ext, ui, acct)
        };
        Ok(format!(
            "in_job=1 flags={:#x} active_limit={} memory={} ui={:#x} total={} active={}",
            ext.BasicLimitInformation.LimitFlags,
            ext.BasicLimitInformation.ActiveProcessLimit,
            ext.ProcessMemoryLimit,
            ui.UIRestrictionsClass,
            acct.TotalProcesses,
            acct.ActiveProcesses,
        ))
    }

    /// `PROCESS_MITIGATION_CHILD_PROCESS_POLICY` flags (bit 0:
    /// `NoChildProcessCreation`), as `flags=0x..`.
    pub fn child_policy() -> io::Result<String> {
        let mut flags = 0u32;
        // SAFETY: the policy structure is a single DWORD.
        if unsafe {
            GetProcessMitigationPolicy(
                GetCurrentProcess(),
                ProcessChildProcessPolicy,
                &mut flags as *mut u32 as *mut c_void,
                std::mem::size_of::<u32>(),
            )
        } == 0
        {
            return Err(os_error("GetProcessMitigationPolicy"));
        }
        Ok(format!("flags={flags:#x}"))
    }

    /// A user32 export, loaded on first use of a UI probe.
    fn user32(name: &str) -> io::Result<Proc> {
        let lib = wide("user32.dll");
        // SAFETY: a NUL-terminated name; the module stays loaded.
        let module = unsafe { LoadLibraryW(lib.as_ptr()) };
        if module.is_null() {
            return Err(os_error("LoadLibraryW(user32.dll)"));
        }
        let cname: Vec<u8> = name.bytes().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated name in a loaded module.
        unsafe { GetProcAddress(module, cname.as_ptr()) }
            .ok_or_else(|| os_error(&format!("GetProcAddress({name})")))
    }

    /// Runs `f` with the clipboard open on this thread.
    fn with_clipboard(f: impl FnOnce() -> io::Result<String>) -> io::Result<String> {
        // SAFETY: the signatures are those of user32's exports.
        let (open, close) = unsafe {
            (
                std::mem::transmute::<Proc, OpenClipboard>(user32("OpenClipboard")?),
                std::mem::transmute::<Proc, NoArgs>(user32("CloseClipboard")?),
            )
        };
        // SAFETY: OpenClipboard without an owner window.
        if unsafe { open(null_mut()) } == 0 {
            return Err(os_error("OpenClipboard"));
        }
        let r = f();
        // SAFETY: the clipboard is open on this thread.
        unsafe { close() };
        r
    }

    /// Reads the clipboard's text.
    pub fn clipboard_read() -> io::Result<String> {
        // SAFETY: user32's GetClipboardData signature.
        let get =
            unsafe { std::mem::transmute::<Proc, GetClipboardData>(user32("GetClipboardData")?) };
        with_clipboard(|| {
            // SAFETY: the clipboard is open; the memory is only read while
            // locked, up to its NUL or 4096 units.
            unsafe {
                let h = get(CF_UNICODETEXT);
                if h.is_null() {
                    return Err(os_error("GetClipboardData"));
                }
                let p = GlobalLock(h) as *const u16;
                if p.is_null() {
                    return Err(os_error("GlobalLock"));
                }
                let mut n = 0;
                while n < 4096 && *p.add(n) != 0 {
                    n += 1;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                GlobalUnlock(h);
                Ok(format!("read {text:?}"))
            }
        })
    }

    /// Replaces the clipboard's content with `text`.
    pub fn clipboard_write(text: &str) -> io::Result<String> {
        // SAFETY: user32's signatures.
        let (empty, set) = unsafe {
            (
                std::mem::transmute::<Proc, NoArgs>(user32("EmptyClipboard")?),
                std::mem::transmute::<Proc, SetClipboardData>(user32("SetClipboardData")?),
            )
        };
        let w = wide(text);
        with_clipboard(|| {
            // SAFETY: the clipboard is open; the global memory is sized for
            // `w`, written while locked, owned by the clipboard after a
            // successful SetClipboardData and freed here otherwise.
            unsafe {
                if empty() == 0 {
                    return Err(os_error("EmptyClipboard"));
                }
                let mem = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
                if mem.is_null() {
                    return Err(os_error("GlobalAlloc"));
                }
                let p = GlobalLock(mem) as *mut u16;
                if p.is_null() {
                    let e = os_error("GlobalLock");
                    GlobalFree(mem);
                    return Err(e);
                }
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                GlobalUnlock(mem);
                if set(CF_UNICODETEXT, mem).is_null() {
                    let e = os_error("SetClipboardData");
                    GlobalFree(mem);
                    return Err(e);
                }
                Ok("written".into())
            }
        })
    }

    /// Looks a global atom up by name.
    pub fn atom_find(name: &str) -> io::Result<String> {
        let w = wide(name);
        // SAFETY: a NUL-terminated string.
        match unsafe { GlobalFindAtomW(w.as_ptr()) } {
            0 => Err(os_error("GlobalFindAtomW")),
            a => Ok(format!("found atom {a:#x}")),
        }
    }

    /// Adds a global atom (to the job's own table when the job restricts
    /// global atoms).
    pub fn atom_add(name: &str) -> io::Result<String> {
        let w = wide(name);
        // SAFETY: a NUL-terminated string.
        match unsafe { GlobalAddAtomW(w.as_ptr()) } {
            0 => Err(os_error("GlobalAddAtomW")),
            a => Ok(format!("added atom {a:#x}")),
        }
    }

    /// Creates (and closes) a desktop in the process's window station.
    pub fn create_desktop(name: &str) -> io::Result<String> {
        // SAFETY: user32's signatures.
        let (create, close) = unsafe {
            (
                std::mem::transmute::<Proc, CreateDesktopW>(user32("CreateDesktopW")?),
                std::mem::transmute::<Proc, CloseDesktop>(user32("CloseDesktop")?),
            )
        };
        let w = wide(name);
        // SAFETY: a NUL-terminated name, no device or mode, default
        // security; the handle is closed right away.
        unsafe {
            let desk = create(w.as_ptr(), null(), null(), 0, GENERIC_ALL, null());
            if desk.is_null() {
                return Err(os_error("CreateDesktopW"));
            }
            close(desk);
        }
        Ok("created".into())
    }
}
