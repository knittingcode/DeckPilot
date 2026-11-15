//! OS sandbox probes. `deckpilot-sandbox-probe` applies exactly the host's
//! sandbox (same code, same two stages, a thread created in between like
//! the epoch ticker) and then tries file, network and process operations
//! from native code on both threads, i.e. as a plugin that escaped wasm.
//! The same probes without the sandbox are the control: they must succeed,
//! so a failure under the sandbox is the sandbox's doing.
//!
//! On Windows the isolation comes from how the supervisor launches the
//! host (AppContainer, Job Object), so the sandboxed probe is started with
//! `HostCommand::spawn`, exactly like the host; started directly it only
//! gets the host's fallback (own job, Low integrity).

use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

const PROBE: &str = env!("CARGO_BIN_EXE_deckpilot-sandbox-probe");

struct Fixture {
    dir: tempfile::TempDir,
    _listener: TcpListener,
    addr: String,
    #[cfg(unix)]
    _unix: std::os::unix::net::UnixListener,
    #[cfg(unix)]
    unix_path: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.txt"), b"user data").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    #[cfg(unix)]
    let (unix, unix_path) = {
        // Short path: sun_path is limited to ~104 bytes.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("dp-probe-{}-{n}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        (
            std::os::unix::net::UnixListener::bind(&p).unwrap(),
            p.to_string_lossy().into_owned(),
        )
    };
    Fixture {
        dir,
        _listener: listener,
        addr,
        #[cfg(unix)]
        _unix: unix,
        #[cfg(unix)]
        unix_path,
    }
}

fn exec_target() -> &'static str {
    if cfg!(windows) {
        "C:\\Windows\\System32\\whoami.exe"
    } else {
        "/bin/echo"
    }
}

/// Runs the probe; returns the sandbox report and `(kind, thread) -> ok`.
fn run(sandboxed: bool, probes: &[(&str, String)]) -> (Value, Vec<(String, String, bool, String)>) {
    run_with(if sandboxed { &[] } else { &["--no-os-sandbox"] }, probes)
}

fn run_with(
    flags: &[&str],
    probes: &[(&str, String)],
) -> (Value, Vec<(String, String, bool, String)>) {
    let mut c = Command::new(PROBE);
    c.args(flags);
    for (k, t) in probes {
        c.arg(k).arg(t);
    }
    let out = c.stdin(Stdio::null()).output().unwrap();
    parse(&String::from_utf8_lossy(&out.stdout))
}

type Results = Vec<(String, String, bool, String)>;

fn parse(text: &str) -> (Value, Results) {
    let mut lines = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    let report = lines.next().expect("probe printed nothing");
    assert!(report.get("error").is_none(), "sandbox failed: {report}");
    let results = lines
        .map(|l| {
            (
                l["kind"].as_str().unwrap().to_owned(),
                l["thread"].as_str().unwrap().to_owned(),
                l["ok"].as_bool().unwrap(),
                l["detail"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    (report["sandbox"].clone(), results)
}

fn probes(f: &Fixture, write_name: &str) -> Vec<(&'static str, String)> {
    let p = |x: &Path| x.to_string_lossy().into_owned();
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut v = vec![
        ("read_file", p(&f.dir.path().join("secret.txt"))),
        ("write_file", p(&f.dir.path().join(write_name))),
        ("list_dir", p(f.dir.path())),
        ("connect_tcp", f.addr.clone()),
        ("exec", exec_target().to_owned()),
    ];
    #[cfg(unix)]
    {
        v.push(("read_file", "/etc/passwd".to_owned()));
        v.push(("connect_unix", f.unix_path.clone()));
    }
    v
}

#[test]
fn control_without_sandbox_can_do_everything() {
    let f = fixture();
    let (report, results) = run(false, &probes(&f, "control.txt"));
    assert_eq!(report["enforcement"], "none");
    for (kind, thread, ok, detail) in &results {
        assert!(ok, "control {kind} on {thread} failed: {detail}");
    }
    assert!(f.dir.path().join("control.txt").exists());
}

fn assert_denied(results: &[(String, String, bool, String)], kinds: &[&str]) {
    for (kind, thread, ok, detail) in results {
        if kinds.contains(&kind.as_str()) {
            assert!(!ok, "sandboxed {kind} on {thread} succeeded: {detail}");
        }
    }
    for k in kinds {
        assert!(
            results.iter().filter(|r| r.0 == *k).count() >= 2,
            "{k} was not probed on both threads"
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn sandbox_denies_files_network_and_exec() {
    let f = fixture();
    let (report, results) = run(true, &probes(&f, "sandboxed.txt"));
    let mechanism = if cfg!(target_os = "macos") {
        "macos-seatbelt"
    } else {
        "linux-landlock-seccomp"
    };
    assert_eq!(report["mechanism"], mechanism, "{report}");
    assert_ne!(report["enforcement"], "none");
    assert_denied(
        &results,
        &[
            "read_file",
            "write_file",
            "list_dir",
            "connect_tcp",
            "connect_unix",
            "exec",
        ],
    );
    assert!(!f.dir.path().join("sandboxed.txt").exists());
}

/// The probe launched like the host by the supervisor: in a fresh
/// AppContainer without capabilities inside the supervisor's Job Object.
#[cfg(windows)]
fn run_like_the_host(probes: &[(&str, String)]) -> (Value, Vec<Probed>) {
    let (report, results, _) = run_like_the_host_status(probes);
    (report, results)
}

/// As [`run_like_the_host`], plus the probe's exit status.
#[cfg(windows)]
fn run_like_the_host_status(
    probes: &[(&str, String)],
) -> (Value, Vec<Probed>, std::process::ExitStatus) {
    use std::io::Read;
    let mut c = deckpilot_plugin_supervisor::HostCommand::new(PROBE);
    for (k, t) in probes {
        c = c.arg(k).arg(t);
    }
    let mut child = c.spawn().expect("spawn in AppContainer");
    drop(child.stdin.take());
    let mut text = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let status = child.wait().unwrap();
    eprintln!("probe exit status: {status:?}\n{text}");
    let mut lines = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap());
    let report = lines.next().expect("probe printed nothing");
    assert!(report.get("error").is_none(), "sandbox failed: {report}");
    let results = lines
        .map(|l| Probed {
            kind: l["kind"].as_str().unwrap().to_owned(),
            target: l["target"].as_str().unwrap().to_owned(),
            _thread: l["thread"].as_str().unwrap().to_owned(),
            ok: l["ok"].as_bool().unwrap(),
            detail: l["detail"].as_str().unwrap().to_owned(),
        })
        .collect();
    (report["sandbox"].clone(), results, status)
}

#[cfg(windows)]
#[derive(Debug)]
struct Probed {
    kind: String,
    target: String,
    /// Shown in assertion messages (Debug).
    _thread: String,
    ok: bool,
    detail: String,
}

/// Started directly (not by the supervisor), the probe gets the host's
/// fallback: its own job and a Low integrity token. That denies writes to
/// the user's files and exec, not reads or network (reported as partial).
#[cfg(windows)]
#[test]
fn fallback_without_app_container_denies_writes_and_exec() {
    let f = fixture();
    let (report, results) = run(true, &probes(&f, "sandboxed.txt"));
    assert_eq!(report["mechanism"], "windows-job-lowil", "{report}");
    assert_eq!(report["enforcement"], "partial", "{report}");
    assert_denied(&results, &["write_file", "exec"]);
    assert!(!f.dir.path().join("sandboxed.txt").exists());
}

/// The production launch: user files in the temp directory and in the
/// user profile (read, write, list), TCP to loopback and process creation
/// are denied, on both threads; a system file stays readable and the
/// pipes work (this output arrived over them), so the denials are the
/// AppContainer's, not a broken process.
#[cfg(windows)]
#[test]
fn app_container_denies_user_files_network_and_exec() {
    let f = fixture();
    let home = std::path::PathBuf::from(std::env::var_os("USERPROFILE").expect("USERPROFILE"));
    let in_home = tempfile::Builder::new()
        .prefix("dp-probe-")
        .tempdir_in(&home)
        .unwrap();
    std::fs::write(in_home.path().join("secret.txt"), b"user data").unwrap();
    let system_root =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap_or("C:\\Windows".into()));
    let system_file = system_root
        .join("System32")
        .join("kernel32.dll")
        .to_string_lossy()
        .into_owned();
    let p = |x: &Path| x.to_string_lossy().into_owned();
    let home_probes = |write_name: &str| {
        vec![
            ("read_file", p(&in_home.path().join("secret.txt"))),
            ("write_file", p(&in_home.path().join(write_name))),
            ("list_dir", p(in_home.path())),
            ("list_dir", p(&home)),
        ]
    };

    // Control: outside the AppContainer every probe succeeds.
    let mut control = probes(&f, "control.txt");
    control.extend(home_probes("control.txt"));
    let (_, results) = run(false, &control);
    for (kind, thread, ok, detail) in &results {
        assert!(ok, "control {kind} on {thread} failed: {detail}");
    }

    let mut sandboxed = probes(&f, "sandboxed.txt");
    sandboxed.extend(home_probes("sandboxed.txt"));
    sandboxed.push(("read_file", system_file.clone()));
    sandboxed.push(("env_count", "-".to_owned()));
    let (report, results) = run_like_the_host(&sandboxed);
    assert_eq!(report["mechanism"], "windows-appcontainer-job", "{report}");
    assert_eq!(report["enforcement"], "full", "{report}");
    assert_eq!(results.len(), sandboxed.len() * 2, "{results:?}");
    for r in &results {
        let allowed = r.kind == "env_count" || (r.kind == "read_file" && r.target == system_file);
        if allowed {
            assert!(r.ok, "{r:?} should work");
        } else {
            assert!(!r.ok, "sandboxed probe succeeded: {r:?}");
        }
        if r.kind == "connect_tcp" {
            // 10106: Winsock could not load its providers (environment),
            // which would make the denial meaningless.
            assert!(!r.detail.contains("10106"), "{r:?}");
        }
    }
    for kind in ["read_file", "write_file", "list_dir", "connect_tcp", "exec"] {
        let denied = results.iter().filter(|r| r.kind == kind && !r.ok).count();
        assert!(denied >= 2, "{kind} was not denied on both threads");
    }
    assert!(!f.dir.path().join("sandboxed.txt").exists());
    assert!(!in_home.path().join("sandboxed.txt").exists());
}

/// Public addresses on the HTTPS port (anycast resolvers, no DNS needed).
const EXTERNAL: [&str; 2] = ["1.1.1.1:443", "8.8.8.8:443"];

/// The control outside the sandbox: every probe must succeed. A transient
/// failure is retried twice; a machine without outbound network fails the
/// test here (loudly) instead of passing it vacuously.
fn internet_control(probes: &[(&str, String)]) {
    let mut last = Vec::new();
    for _ in 0..3 {
        let (_, results) = run(false, probes);
        if results.iter().all(|r| r.2) && results.len() == probes.len() * 2 {
            return;
        }
        last = results;
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    panic!(
        "control: outbound TCP to {EXTERNAL:?} fails outside the sandbox, so this machine \
         has no internet access and egress denial cannot be shown: {last:?}"
    );
}

/// TCP to the internet (not only loopback) is denied inside the sandbox,
/// on both threads, while the same probe outside it connects.
#[test]
fn internet_egress_is_denied() {
    let probes: Vec<(&str, String)> = EXTERNAL
        .iter()
        .map(|a| ("connect_tcp", (*a).to_owned()))
        .collect();
    internet_control(&probes);
    #[cfg(unix)]
    {
        let (report, results) = run(true, &probes);
        eprintln!("sandboxed egress: {report} {results:?}");
        assert_ne!(report["enforcement"], "none", "{report}");
        assert_eq!(results.len(), probes.len() * 2, "{results:?}");
        assert_denied(&results, &["connect_tcp"]);
    }
    #[cfg(windows)]
    {
        let (report, results) = run_like_the_host(&probes);
        assert_eq!(report["mechanism"], "windows-appcontainer-job", "{report}");
        assert_eq!(report["enforcement"], "full", "{report}");
        assert_eq!(results.len(), probes.len() * 2, "{results:?}");
        for r in &results {
            assert!(!r.ok, "internet connection from the AppContainer: {r:?}");
            assert!(!r.detail.contains("10106"), "{r:?}");
        }
    }
}

/// Windows-only checks of the launch from the outside and the inside.
#[cfg(windows)]
mod windows_launch {
    use std::os::windows::io::AsRawHandle;
    use std::ptr::{null, null_mut};

    use deckpilot_plugin_abi::limits::HOST_PROCESS_MEMORY;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GlobalFree, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, GlobalAddAtomW, GlobalDeleteAtom,
        GlobalFindAtomW, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
        JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES,
        JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
        JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

    use super::{exec_target, run, run_like_the_host, run_like_the_host_status};

    /// A name no other run uses.
    fn unique(what: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("dp-probe-{what}-{}-{nanos}", std::process::id())
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// `key=value` fields of a probe detail.
    fn field(detail: &str, key: &str) -> String {
        detail
            .split_whitespace()
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("{key} missing in {detail:?}"))
            .to_owned()
    }

    fn hex(s: &str) -> u32 {
        u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
    }

    /// The report says the supervisor's job was verified (the host did not
    /// fall back to a job of its own).
    fn assert_supervisor_job(report: &serde_json::Value) {
        let notes: Vec<&str> = report["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert!(
            notes.iter().any(|n| n.starts_with("job (supervisor):")),
            "{report}"
        );
        assert!(!notes.iter().any(|n| n.contains("job (own")), "{report}");
    }

    /// Inside the AppContainer the probe is in the supervisor's job (as
    /// reported, and the innermost job has exactly the supervisor's limits
    /// and UI restrictions with the probe as its only process), the child
    /// process policy is set, and `CreateProcessW` itself is refused (not
    /// merely `std::process::Command`'s opening of `NUL`).
    #[test]
    fn supervisor_job_and_child_policy_apply() {
        let exec = exec_target().to_owned();
        let probes = vec![
            ("job_info", "-".to_owned()),
            ("child_policy", "-".to_owned()),
            ("create_process", exec.clone()),
        ];
        // Control: started directly the probe has no child process policy
        // and can create processes.
        let (_, control) = run(false, &probes[1..]);
        for (kind, thread, ok, detail) in &control {
            assert!(ok, "control {kind} on {thread}: {detail}");
            if kind == "child_policy" {
                assert_eq!(hex(&field(detail, "flags")) & 1, 0, "{detail}");
            }
        }

        let (report, results) = run_like_the_host(&probes);
        assert_eq!(report["mechanism"], "windows-appcontainer-job", "{report}");
        assert_eq!(report["enforcement"], "full", "{report}");
        assert_supervisor_job(&report);
        assert_eq!(results.len(), probes.len() * 2, "{results:?}");
        let ui_all = JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_EXITWINDOWS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;
        let limits = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        for r in &results {
            match r.kind.as_str() {
                "job_info" => {
                    assert!(r.ok, "{r:?}");
                    assert_eq!(field(&r.detail, "in_job"), "1", "{r:?}");
                    assert_eq!(hex(&field(&r.detail, "flags")), limits, "{r:?}");
                    assert_eq!(field(&r.detail, "active_limit"), "1", "{r:?}");
                    assert_eq!(
                        field(&r.detail, "memory"),
                        HOST_PROCESS_MEMORY.to_string(),
                        "{r:?}"
                    );
                    assert_eq!(hex(&field(&r.detail, "ui")), ui_all, "{r:?}");
                    // Only the probe itself ever ran in this job.
                    assert_eq!(field(&r.detail, "total"), "1", "{r:?}");
                    assert_eq!(field(&r.detail, "active"), "1", "{r:?}");
                }
                "child_policy" => {
                    assert!(r.ok, "{r:?}");
                    // NoChildProcessCreation.
                    assert_eq!(hex(&field(&r.detail, "flags")) & 1, 1, "{r:?}");
                }
                "create_process" => {
                    assert!(!r.ok, "process created in the AppContainer: {r:?}");
                    // Refused by the child process policy, which is checked
                    // before the job's process limit (that one would say
                    // ERROR_NOT_ENOUGH_QUOTA, 1816; it is shown on its own
                    // in process.rs, host_runs_alone_in_the_supervisors_job).
                    assert!(!r.detail.contains("1816"), "{r:?}");
                    eprintln!("create_process in the AppContainer: {}", r.detail);
                }
                _ => unreachable!(),
            }
        }
    }

    /// The job's process memory limit (`HOST_PROCESS_MEMORY`, 256 MiB)
    /// stops native allocations outside wasm: 192 MiB fit, 320 MiB do not;
    /// started directly, the same probe allocates 320 MiB.
    #[test]
    fn process_memory_limit_stops_native_allocations() {
        assert_eq!(HOST_PROCESS_MEMORY, 256 << 20);
        let (_, control) = run(false, &[("alloc_mib", "320".to_owned())]);
        assert_eq!(control.len(), 2, "{control:?}");
        for (kind, thread, ok, detail) in &control {
            assert!(ok, "control {kind} on {thread}: {detail}");
        }
        let probes = vec![
            ("alloc_mib", "192".to_owned()),
            ("alloc_mib", "320".to_owned()),
        ];
        let (report, results) = run_like_the_host(&probes);
        assert_supervisor_job(&report);
        assert_eq!(results.len(), 4, "{results:?}");
        for r in &results {
            if r.target == "192" {
                assert!(r.ok, "below the limit: {r:?}");
            } else {
                assert!(!r.ok, "allocated beyond the job's memory limit: {r:?}");
                assert!(r.detail.starts_with("OutOfMemory"), "{r:?}");
            }
        }
    }

    /// The child gets only its two pipe handles (handle list): an
    /// inheritable event and an inheritable file handle of this process are
    /// not usable in it. Control: a child started with `std::process::Command`
    /// (no handle list) inherits both and can signal the event and write the
    /// file through the same handle values, so the check would see a leak.
    #[test]
    fn only_the_pipe_handles_are_inherited() {
        let inherit = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        // SAFETY: valid attributes; a manual-reset event, initially unset.
        let event: HANDLE = unsafe { CreateEventW(&inherit, 1, 0, null()) };
        assert!(!event.is_null());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inherited.txt");
        let file = std::fs::File::create(&path).unwrap();
        // SAFETY: a valid handle owned by `file`.
        assert_ne!(
            unsafe {
                SetHandleInformation(
                    file.as_raw_handle(),
                    HANDLE_FLAG_INHERIT,
                    HANDLE_FLAG_INHERIT,
                )
            },
            0
        );
        let probes = vec![
            ("signal_event", (event as usize).to_string()),
            ("write_handle", (file.as_raw_handle() as usize).to_string()),
        ];
        // SAFETY (all WaitForSingleObject below): a valid event handle.
        let signaled = || unsafe { WaitForSingleObject(event, 0) };

        let (_, control) = run(false, &probes);
        for (kind, thread, ok, detail) in &control {
            assert!(ok, "control {kind} on {thread}: {detail}");
        }
        assert_eq!(signaled(), WAIT_OBJECT_0, "control did not signal");
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        // SAFETY: a valid event handle.
        unsafe { ResetEvent(event) };
        file.set_len(0).unwrap();
        assert_eq!(signaled(), WAIT_TIMEOUT);

        // One probe per run: using a value that names no handle in the
        // AppContainer process may end it with STATUS_INVALID_HANDLE
        // (strict handle checking) before it reports, which is the OS
        // saying the same thing. Either way the decisive check is ours:
        // our event stays unsignaled and our file empty.
        const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
        for probe in &probes {
            let (report, results, status) = run_like_the_host_status(std::slice::from_ref(probe));
            assert_supervisor_job(&report);
            for r in &results {
                assert!(!r.ok, "inherited handle usable in the AppContainer: {r:?}");
            }
            if results.len() != 2 {
                assert_eq!(
                    status.code().map(|c| c as u32),
                    Some(STATUS_INVALID_HANDLE),
                    "{probe:?}: {results:?} {status:?}"
                );
            }
        }
        assert_eq!(signaled(), WAIT_TIMEOUT, "the host signaled our event");
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            0,
            "the host wrote to our file"
        );
        drop(file);
        // SAFETY: created above, closed once.
        unsafe { CloseHandle(event) };
    }

    /// Runs `f` with the clipboard open, retrying while another process
    /// holds it.
    fn with_clipboard<T>(f: impl FnOnce() -> T) -> T {
        for _ in 0..50 {
            // SAFETY: no owner window.
            if unsafe { OpenClipboard(null_mut()) } != 0 {
                let r = f();
                // SAFETY: opened above on this thread.
                unsafe { CloseClipboard() };
                return r;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!(
            "control: OpenClipboard fails outside the sandbox: {}",
            std::io::Error::last_os_error()
        );
    }

    fn set_clipboard(text: &str) {
        let w = wide(text);
        with_clipboard(|| {
            // SAFETY: the clipboard is open; the memory is sized for `w` and
            // owned by the clipboard after SetClipboardData.
            unsafe {
                assert_ne!(EmptyClipboard(), 0, "{}", std::io::Error::last_os_error());
                let mem = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
                assert!(!mem.is_null());
                let p = GlobalLock(mem) as *mut u16;
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                GlobalUnlock(mem);
                if SetClipboardData(13, mem).is_null() {
                    let e = std::io::Error::last_os_error();
                    GlobalFree(mem);
                    panic!("control: SetClipboardData: {e}");
                }
            }
        });
    }

    fn clipboard() -> Option<String> {
        with_clipboard(|| {
            // SAFETY: the clipboard is open; read while locked up to NUL.
            unsafe {
                let h = GetClipboardData(13);
                if h.is_null() {
                    return None;
                }
                let p = GlobalLock(h) as *const u16;
                if p.is_null() {
                    return None;
                }
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                GlobalUnlock(h);
                Some(s)
            }
        })
    }

    fn atom_exists(name: &str) -> bool {
        let w = wide(name);
        // SAFETY: a NUL-terminated string.
        unsafe { GlobalFindAtomW(w.as_ptr()) != 0 }
    }

    /// The job's UI restrictions (and the AppContainer): inside, the probe
    /// can neither read nor replace the clipboard, sees none of the
    /// window station's global atoms and adds none to it, and cannot create
    /// a desktop. Control: the same probe started directly can do each.
    #[test]
    fn clipboard_atoms_and_desktops_are_out_of_reach() {
        let secret = unique("clipboard");
        set_clipboard(&secret);
        assert_eq!(clipboard().as_deref(), Some(secret.as_str()));
        let outside_atom = unique("atom-out");
        let w = wide(&outside_atom);
        // SAFETY: a NUL-terminated string.
        let atom = unsafe { GlobalAddAtomW(w.as_ptr()) };
        assert_ne!(atom, 0);

        // Control.
        let replaced = unique("clipboard-control");
        let (_, control) = run(
            false,
            &[
                ("clipboard_read", "-".to_owned()),
                ("atom_find", outside_atom.clone()),
                ("create_desktop", unique("desktop-control")),
            ],
        );
        for (kind, thread, ok, detail) in &control {
            assert!(ok, "control {kind} on {thread}: {detail}");
            if kind == "clipboard_read" {
                assert!(detail.contains(&secret), "control read {detail}");
            }
        }
        let (_, control) = run(false, &[("clipboard_write", replaced.clone())]);
        for (kind, thread, ok, detail) in &control {
            assert!(ok, "control {kind} on {thread}: {detail}");
        }
        assert_eq!(clipboard().as_deref(), Some(replaced.as_str()));
        set_clipboard(&secret);

        // Inside the AppContainer and the supervisor's job.
        let overwrite = unique("clipboard-sandboxed");
        let inside_atom = unique("atom-in");
        let probes = vec![
            ("clipboard_read", "-".to_owned()),
            ("clipboard_write", overwrite.clone()),
            ("atom_find", outside_atom.clone()),
            ("atom_add", inside_atom.clone()),
            ("create_desktop", unique("desktop-sandboxed")),
        ];
        let (report, results) = run_like_the_host(&probes);
        assert_supervisor_job(&report);
        assert_eq!(results.len(), probes.len() * 2, "{results:?}");
        for r in &results {
            match r.kind.as_str() {
                "clipboard_read" => {
                    assert!(!r.detail.contains(&secret), "clipboard read: {r:?}");
                    assert!(!r.ok, "{r:?}");
                }
                "atom_find" | "create_desktop" => assert!(!r.ok, "{r:?}"),
                // Judged from the outside below.
                "clipboard_write" | "atom_add" => {}
                _ => unreachable!(),
            }
        }
        assert_eq!(
            clipboard().as_deref(),
            Some(secret.as_str()),
            "the sandboxed probe replaced the clipboard: {results:?}"
        );
        assert!(
            !atom_exists(&inside_atom),
            "an atom added in the job is visible outside: {results:?}"
        );
        // SAFETY: added above.
        unsafe { GlobalDeleteAtom(atom) };
        set_clipboard("");
    }
}

/// Landlock on its own (no seccomp): file access is denied by Landlock
/// (EACCES), independently of the seccomp layer (EPERM).
#[cfg(target_os = "linux")]
#[test]
fn landlock_alone_denies_file_access() {
    let f = fixture();
    let (report, results) = run_with(&["--stage1-only"], &probes(&f, "landlock.txt"));
    let notes = report["notes"].to_string();
    if notes.contains("NotEnforced") {
        eprintln!("SKIPPED: Landlock is not available on this kernel ({notes})");
        return;
    }
    for (kind, thread, ok, detail) in &results {
        if ["read_file", "write_file", "list_dir", "exec"].contains(&kind.as_str()) {
            assert!(!ok, "{kind} on {thread} succeeded under Landlock: {detail}");
            assert!(
                detail.contains("os error 13"),
                "{kind}: not a Landlock denial: {detail}"
            );
        }
    }
    assert!(!f.dir.path().join("landlock.txt").exists());
}
