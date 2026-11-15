//! The host dies with the agent. This test binary re-executes itself as a
//! helper "agent" (`--helper <program> <args>...`) that starts a host
//! through the supervisor (`HostCommand::spawn`, exactly as the agent
//! does), waits until it has produced output (it is up and sandboxed),
//! prints its pid and then holds it. The test kills the helper outright
//! (no cleanup code runs in it) and requires the host to be gone within a
//! few seconds.
//!
//! * Windows: kill on job close. The helper's death closes the only handle
//!   to the supervisor's Job Object, and the job kills the host. Shown with
//!   the sandbox probe in `sleep` mode, which never reads stdin, so nothing
//!   but the job can end it; and with the real host.
//! * macOS and Linux: there is no kill-on-close. The host ends because its
//!   stdin reaches end-of-file when the agent's pipe end closes with the
//!   agent, so this holds for a host that is reading (idle between
//!   invocations), which is what is tested. A host stuck in a long native
//!   computation would outlive the agent until it next reads (README).
//!
//! `harness = false`: the helper's stdout must carry only the pid.

use std::io::{BufRead, BufReader, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use deckpilot_plugin_supervisor::HostCommand;

const HOST: &str = env!("CARGO_BIN_EXE_deckpilot-plugin-host");
#[cfg(windows)]
const PROBE: &str = env!("CARGO_BIN_EXE_deckpilot-sandbox-probe");

/// How long the host may take to go after the agent was killed.
const GRACE: Duration = Duration::from_secs(5);

fn helper(args: &[String]) -> ! {
    let mut c = HostCommand::new(&args[0]);
    for a in &args[1..] {
        c = c.arg(a);
    }
    let mut child = c.spawn().expect("spawn host");
    // The first byte of output: the host (or probe) has applied or
    // verified its sandbox and is running.
    let mut b = [0u8; 1];
    child
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut b)
        .expect("host output");
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", child.id()).unwrap();
    out.flush().unwrap();
    // Hold the host (and on Windows its job) until this process is killed.
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

/// Watches a process by pid without being its parent.
#[cfg(windows)]
struct Watch(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Watch {
    fn open(pid: u32) -> Self {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        };
        // SAFETY: OpenProcess with a pid; the handle is closed in Drop.
        let h = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        assert!(
            !h.is_null(),
            "OpenProcess({pid}): {}",
            std::io::Error::last_os_error()
        );
        Self(h)
    }

    fn alive(&self) -> bool {
        use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: a valid process handle.
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
    }

    fn exit_code(&self) -> u32 {
        let mut code = 0;
        // SAFETY: a valid process handle and an out pointer.
        unsafe { windows_sys::Win32::System::Threading::GetExitCodeProcess(self.0, &mut code) };
        code
    }
}

#[cfg(windows)]
impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: opened in `open`, closed once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(unix)]
struct Watch(libc::pid_t);

#[cfg(unix)]
impl Watch {
    fn open(pid: u32) -> Self {
        Self(pid as libc::pid_t)
    }

    /// Running (a zombie waiting for its new parent to reap it counts as
    /// gone: it has exited).
    fn alive(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", self.0)) else {
                return false;
            };
            let state = stat
                .rsplit_once(')')
                .and_then(|(_, r)| r.trim().chars().next());
            !matches!(state, Some('Z' | 'X') | None)
        }
        #[cfg(not(target_os = "linux"))]
        {
            // SAFETY: signal 0 only checks for existence.
            unsafe { libc::kill(self.0, 0) == 0 }
        }
    }

    fn exit_code(&self) -> &'static str {
        "(not our child)"
    }
}

fn host_dies_with_the_agent(program: &str, args: &[&str]) {
    let mut helper = Command::new(std::env::current_exe().unwrap())
        .arg("--helper")
        .arg(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(helper.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let pid: u32 = line
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("helper said {line:?}"));
    let host = Watch::open(pid);
    std::thread::sleep(Duration::from_millis(500));
    assert!(host.alive(), "host {pid} is not running before the kill");

    helper.kill().unwrap();
    helper.wait().unwrap();
    let t = Instant::now();
    while host.alive() {
        assert!(
            t.elapsed() < GRACE,
            "host {pid} still runs {GRACE:?} after its agent was killed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    eprintln!(
        "host {pid} ({program} {args:?}) gone {:?} after the agent was killed, exit code {}",
        t.elapsed(),
        host.exit_code()
    );
    #[cfg(windows)]
    delete_profile(helper.id());
}

/// The helper died without deleting its host's AppContainer profile (its
/// first and only one); remove it so no profile outlives the test.
#[cfg(windows)]
fn delete_profile(helper_pid: u32) {
    let name = format!(
        "{}.{helper_pid:x}.0",
        deckpilot_plugin_supervisor::APP_CONTAINER_PROFILE_PREFIX
    );
    let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: a NUL-terminated profile name.
    let hr =
        unsafe { windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile(w.as_ptr()) };
    assert!(hr >= 0, "DeleteAppContainerProfile({name}): {hr:#x}");
}

fn idle_host_exits_when_its_agent_is_killed() {
    host_dies_with_the_agent(HOST, &[]);
}

/// Windows: a host that never reads stdin is killed by the job alone.
#[cfg(windows)]
fn busy_host_is_killed_with_its_agents_job() {
    host_dies_with_the_agent(PROBE, &["sleep", "600"]);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--helper") {
        helper(&args[1..]);
    }
    // libtest flags (`--nocapture`, filters) are ignored: every test runs.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut tests: Vec<(&str, fn())> = vec![(
        "idle_host_exits_when_its_agent_is_killed",
        idle_host_exits_when_its_agent_is_killed,
    )];
    #[cfg(windows)]
    tests.push((
        "busy_host_is_killed_with_its_agents_job",
        busy_host_is_killed_with_its_agents_job,
    ));
    let mut failed = 0;
    for (name, test) in &tests {
        let ok = catch_unwind(AssertUnwindSafe(test)).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        failed += usize::from(!ok);
    }
    println!(
        "\ntest result: {}. {} passed; {failed} failed",
        if failed == 0 { "ok" } else { "FAILED" },
        tests.len() - failed
    );
    if failed > 0 {
        std::process::exit(101);
    }
}
