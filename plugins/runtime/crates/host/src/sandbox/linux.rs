//! Linux: process attributes, Landlock and a seccomp-bpf allowlist.
//!
//! Landlock domains and `no_new_privs` are per thread and inherited by new
//! threads, so [`begin`] runs while the process is still single-threaded;
//! the epoch ticker thread started afterwards inherits them. The seccomp
//! filter is installed by [`finish`] with `SECCOMP_FILTER_FLAG_TSYNC`, so it
//! covers every thread, and allows no thread or process creation at all.

use std::collections::BTreeMap;

use deckpilot_plugin_abi::protocol::{Enforcement, SandboxReport};
use landlock::{
    ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetStatus,
    Scope,
};
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule, TargetArch,
};

use super::SandboxError;

/// The newest Landlock ABI this code asks for; older kernels get what they
/// support (best effort) and the report says `partial`.
const LANDLOCK_ABI: ABI = ABI::V9;

#[derive(Debug)]
pub struct Pending {
    notes: Vec<String>,
    landlock_full: bool,
}

fn os_err(what: &str) -> SandboxError {
    SandboxError(format!("{what}: {}", std::io::Error::last_os_error()))
}

pub fn begin() -> Result<Pending, SandboxError> {
    let mut notes = Vec::new();
    // SAFETY: plain prctl/setrlimit calls with constant arguments.
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(os_err("PR_SET_NO_NEW_PRIVS"));
        }
        if libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0 {
            return Err(os_err("PR_SET_DUMPABLE"));
        }
        let zero = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &zero) != 0 {
            return Err(os_err("RLIMIT_CORE"));
        }
    }
    notes.push("no_new_privs, dumpable=0, RLIMIT_CORE=0".into());

    // Handle every access right the kernel knows and add no rule: all file
    // system access, TCP bind/connect and scoped IPC are denied.
    let status = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(LANDLOCK_ABI))
        .and_then(|r| r.handle_access(AccessNet::from_all(LANDLOCK_ABI)))
        .and_then(|r| r.scope(Scope::from_all(LANDLOCK_ABI)))
        .and_then(|r| r.create())
        .and_then(|r| r.restrict_self())
        .map_err(|e| SandboxError(format!("landlock: {e}")))?;
    let landlock_full = status.ruleset == RulesetStatus::FullyEnforced;
    notes.push(format!("landlock: {:?}", status.ruleset));
    if status.ruleset == RulesetStatus::NotEnforced {
        notes.push("landlock unavailable: seccomp alone denies open/socket/exec".into());
    }
    Ok(Pending {
        notes,
        landlock_full,
    })
}

fn allowed_syscalls() -> Vec<i64> {
    let mut v = vec![
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_close,
        libc::SYS_mmap,
        libc::SYS_munmap,
        libc::SYS_mprotect,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_brk,
        libc::SYS_futex,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_sched_yield,
        libc::SYS_nanosleep,
        libc::SYS_clock_nanosleep,
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_gettimeofday,
        libc::SYS_getrandom,
        libc::SYS_gettid,
        libc::SYS_getpid,
        libc::SYS_membarrier,
        libc::SYS_sched_getaffinity,
        libc::SYS_rseq,
        libc::SYS_set_robust_list,
        libc::SYS_restart_syscall,
    ];
    #[cfg(target_arch = "x86_64")]
    v.push(libc::SYS_arch_prctl);
    v.sort_unstable();
    v.dedup();
    v
}

fn filter() -> Result<BpfProgram, SandboxError> {
    let arch = TargetArch::try_from(std::env::consts::ARCH)
        .map_err(|e| SandboxError(format!("seccomp arch: {e}")))?;
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = allowed_syscalls()
        .into_iter()
        .map(|s| (s, vec![]))
        .collect();
    // abort() signals the process itself; allow tgkill/tkill only for our
    // own thread group.
    // SAFETY: getpid has no preconditions.
    let pid = unsafe { libc::getpid() } as u64;
    let own = SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, pid)
        .and_then(|c| SeccompRule::new(vec![c]))
        .map_err(|e| SandboxError(format!("seccomp rule: {e}")))?;
    rules.insert(libc::SYS_tgkill, vec![own]);
    SeccompFilter::new(
        rules,
        SeccompAction::Errno(libc::EPERM as u32),
        SeccompAction::Allow,
        arch,
    )
    .and_then(BpfProgram::try_from)
    .map_err(|e| SandboxError(format!("seccomp filter: {e}")))
}

impl Pending {
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
}

pub fn finish(pending: Pending) -> Result<SandboxReport, SandboxError> {
    let program = filter()?;
    seccompiler::apply_filter_all_threads(&program)
        .map_err(|e| SandboxError(format!("seccomp: {e}")))?;
    let mut notes = pending.notes;
    notes.push("seccomp allowlist (TSYNC), default EPERM".into());
    Ok(SandboxReport {
        mechanism: "linux-landlock-seccomp".into(),
        enforcement: if pending.landlock_full {
            Enforcement::Full
        } else {
            Enforcement::Partial
        },
        notes,
    })
}
