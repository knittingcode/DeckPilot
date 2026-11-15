//! Linux: process attributes, Landlock and a seccomp-bpf allowlist.
//!
//! Landlock domains and `no_new_privs` are per thread and inherited by new
//! threads, so [`begin`] runs while the process is still single-threaded;
//! the epoch ticker thread started afterwards inherits them. The seccomp
//! filter is installed by [`finish`] with `SECCOMP_FILTER_FLAG_TSYNC`, so it
//! covers every thread, and allows no thread or process creation at all.
//!
//! Enforcement is `full` when Landlock enforces every right this code asks
//! for (ABI 9), or when the kernel provides exactly Landlock ABI 8 and the
//! seccomp allowlist has no socket, connect, open or exec call
//! ([`seccomp_covers_resolve_unix`]): ABI 9 adds only
//! `AccessFs::ResolveUnix` (connecting to pathname UNIX sockets), which
//! needs `socket` and `connect`, and seccomp denies both outright. Anything
//! else (no Landlock, an ABI below 8, any other missing right) is
//! `partial`.

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
/// support (best effort).
const LANDLOCK_ABI: ABI = ABI::V9;

/// The oldest kernel Landlock ABI that still counts as `full`, given
/// [`seccomp_covers_resolve_unix`]: it lacks only V9's `ResolveUnix`.
const LANDLOCK_ABI_COVERED_BY_SECCOMP: i64 = 8;

/// `LANDLOCK_CREATE_RULESET_VERSION`: `landlock_create_ruleset` returns the
/// kernel's ABI version instead of creating a ruleset.
const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1;

/// How much of the requested Landlock ruleset the kernel enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landlock {
    /// Every requested right (ABI 9 or newer).
    Full,
    /// Kernel ABI 8: everything but `ResolveUnix`.
    MissingResolveUnix,
    /// Anything less.
    Partial,
}

#[derive(Debug)]
pub struct Pending {
    notes: Vec<String>,
    landlock: Landlock,
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

    // The kernel's ABI, probed before restricting (a negative value: no
    // Landlock).
    let kernel_abi = landlock_abi();

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
    notes.push(format!("landlock: {:?}", status.ruleset));
    let not_enforced = status.ruleset == RulesetStatus::NotEnforced;
    let landlock = classify(status.ruleset, kernel_abi);
    match landlock {
        Landlock::Full => {}
        Landlock::MissingResolveUnix => notes.push(
            "landlock ABI 8: ResolveUnix unavailable, covered by seccomp (no socket/connect)"
                .into(),
        ),
        Landlock::Partial if not_enforced => {
            notes.push("landlock unavailable: seccomp alone denies open/socket/exec".into());
        }
        Landlock::Partial => notes.push(format!("landlock ABI {kernel_abi}: rights missing")),
    }
    Ok(Pending { notes, landlock })
}

/// The kernel's Landlock ABI version, or a negative errno-style value when
/// Landlock is unavailable.
fn landlock_abi() -> i64 {
    // SAFETY: with a null attribute pointer, size 0 and the VERSION flag the
    // syscall only returns the ABI version (or an error); nothing is read.
    let v = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if v < 0 {
        -i64::from(std::io::Error::last_os_error().raw_os_error().unwrap_or(1))
    } else {
        v
    }
}

/// `full` needs every requested right, or kernel ABI 8 (only `ResolveUnix`
/// missing) with a seccomp allowlist that denies what `ResolveUnix` would.
fn classify(ruleset: RulesetStatus, kernel_abi: i64) -> Landlock {
    match ruleset {
        RulesetStatus::FullyEnforced => Landlock::Full,
        RulesetStatus::PartiallyEnforced
            if kernel_abi == LANDLOCK_ABI_COVERED_BY_SECCOMP && seccomp_covers_resolve_unix() =>
        {
            Landlock::MissingResolveUnix
        }
        _ => Landlock::Partial,
    }
}

/// Syscalls the seccomp filter must never allow while the `ResolveUnix`
/// exception applies: creating or connecting a socket, opening a path,
/// executing a program.
fn denied_for_resolve_unix() -> Vec<i64> {
    let mut v = vec![
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_sendto,
        libc::SYS_sendmsg,
        libc::SYS_openat,
        libc::SYS_openat2,
        libc::SYS_open_by_handle_at,
        libc::SYS_execve,
        libc::SYS_execveat,
    ];
    // `open` exists only on x86_64 (aarch64 has `openat` alone).
    #[cfg(target_arch = "x86_64")]
    v.push(libc::SYS_open);
    v.sort_unstable();
    v
}

/// Whether the seccomp filter (unconditional and conditional rules) allows
/// none of [`denied_for_resolve_unix`].
fn seccomp_covers_resolve_unix() -> bool {
    let denied = denied_for_resolve_unix();
    allowed_syscalls()
        .into_iter()
        .chain(CONDITIONAL_SYSCALLS)
        .all(|s| !denied.contains(&s))
}

/// Allowed only with an argument condition (see [`filter`]).
const CONDITIONAL_SYSCALLS: [i64; 1] = [libc::SYS_tgkill];

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
    rules.insert(CONDITIONAL_SYSCALLS[0], vec![own]);
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
        enforcement: match pending.landlock {
            Landlock::Full | Landlock::MissingResolveUnix => Enforcement::Full,
            Landlock::Partial => Enforcement::Partial,
        },
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_allowlist_has_no_socket_connect_open_or_exec() {
        let allowed: Vec<i64> = allowed_syscalls()
            .into_iter()
            .chain(CONDITIONAL_SYSCALLS)
            .collect();
        for s in denied_for_resolve_unix() {
            assert!(!allowed.contains(&s), "syscall {s} is allowed");
        }
        assert!(seccomp_covers_resolve_unix());
    }

    #[test]
    fn abi_9_adds_only_resolve_unix() {
        // The exception is sound only while V9's sole addition over V8 is
        // `ResolveUnix`.
        assert_eq!(
            AccessFs::from_all(ABI::V9) & !AccessFs::from_all(ABI::V8),
            AccessFs::ResolveUnix
        );
        assert_eq!(AccessNet::from_all(ABI::V9), AccessNet::from_all(ABI::V8));
        assert_eq!(Scope::from_all(ABI::V9), Scope::from_all(ABI::V8));
        assert_eq!(LANDLOCK_ABI, ABI::V9);
    }

    #[test]
    fn only_abi_8_with_seccomp_counts_as_full() {
        use RulesetStatus::{FullyEnforced, NotEnforced, PartiallyEnforced};
        assert_eq!(classify(FullyEnforced, 9), Landlock::Full);
        assert_eq!(classify(PartiallyEnforced, 8), Landlock::MissingResolveUnix);
        for abi in [-38, 1, 4, 6, 7, 9, 10] {
            assert_eq!(classify(PartiallyEnforced, abi), Landlock::Partial, "{abi}");
        }
        assert_eq!(classify(NotEnforced, -38), Landlock::Partial);
        assert_eq!(classify(NotEnforced, 8), Landlock::Partial);
    }
}
