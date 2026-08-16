use anyhow::{Context, Result};
use seccompiler::{
    apply_filter_all_threads, BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp,
    SeccompCondition, SeccompFilter, SeccompRule,
};
use std::collections::BTreeMap;

#[repr(C)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

#[repr(C)]
struct CapHeader {
    version: u32,
    pid: i32,
}

const CAP_DAC_READ_SEARCH: u32 = 2;
const CAP_NET_ADMIN: u32 = 12;
const CAP_NET_RAW: u32 = 13;
const CAP_SYS_MODULE: u32 = 16;
const CAP_SYS_RAWIO: u32 = 17;
const CAP_SYS_PTRACE: u32 = 19;
const CAP_SYS_ADMIN: u32 = 21;
const CAP_SYS_BOOT: u32 = 22;
const CAP_SYS_TIME: u32 = 25;
const CAP_MKNOD: u32 = 27;
const CAP_AUDIT_WRITE: u32 = 29;
const CAP_AUDIT_CONTROL: u32 = 30;
const CAP_SETFCAP: u32 = 31;
const CAP_SYSLOG: u32 = 34;

// P0-3: caps that must be absent from the sandbox. `CAP_DAC_READ_SEARCH`
// pairs with blocking `open_by_handle_at` to close a classic container
// escape; `CAP_NET_RAW` is unnecessary once the netns is loopback-only;
// `CAP_SETFCAP` prevents planting setcap binaries for later privilege
// escalation.
const DANGEROUS_CAPS: [u32; 14] = [
    CAP_DAC_READ_SEARCH,
    CAP_NET_ADMIN,
    CAP_NET_RAW,
    CAP_SYS_MODULE,
    CAP_SYS_RAWIO,
    CAP_SYS_PTRACE,
    CAP_SYS_ADMIN,
    CAP_SYS_BOOT,
    CAP_SYS_TIME,
    CAP_MKNOD,
    CAP_AUDIT_WRITE,
    CAP_AUDIT_CONTROL,
    CAP_SETFCAP,
    CAP_SYSLOG,
];

/// Mask of all `CLONE_NEW*` flags (NS|CGROUP|UTS|IPC|USER|PID|NET).
/// `clone(2)` is allowed by seccomp only when none of these bits are set,
/// so a sandboxed process cannot create fresh namespaces to sidestep
/// isolation. `clone3` is absent from the allow-list (its flags live behind
/// a pointer seccomp cannot inspect), so it stays blocked with SIGSYS.
/// See PLAN.md P0-3.
const CLONE_NEWNAMESPACES_MASK: u64 = 0x7E020000;

pub fn drop_capabilities(dangerous: bool) -> Result<()> {
    if dangerous {
        return Ok(());
    }

    let mut mask: u64 = 0;
    for cap in &DANGEROUS_CAPS {
        mask |= 1u64 << cap;
    }

    let caps0 = (mask & 0xFFFFFFFF) as u32;
    let caps1 = (mask >> 32) as u32;

    let mut hdr = CapHeader {
        version: 0x20080522,
        pid: 0,
    };

    let mut data = [
        CapData {
            effective: caps0,
            permitted: caps0,
            inheritable: caps0,
        },
        CapData {
            effective: caps1,
            permitted: caps1,
            inheritable: caps1,
        },
    ];

    let ret = unsafe { libc::syscall(libc::SYS_capget, &mut hdr as *mut _, &mut data as *mut _) };
    if ret != 0 {
        return Err(anyhow::anyhow!(
            "capget failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    data[0].effective &= !caps0;
    data[0].permitted &= !caps0;
    data[0].inheritable &= !caps0;
    data[1].effective &= !caps1;
    data[1].permitted &= !caps1;
    data[1].inheritable &= !caps1;

    let ret = unsafe { libc::syscall(libc::SYS_capset, &mut hdr as *mut _, &data as *const _) };
    if ret != 0 {
        return Err(anyhow::anyhow!(
            "capset failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    unsafe {
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        );
    }

    // SAFETY (P0-4): clear the capability bounding set so a setuid binary
    // exec'd inside the sandbox cannot re-acquire dropped caps from the
    // bounding set on execve(2). capset() above only clears
    // effective/permitted/inheritable/ambient; without this loop the
    // bounding set still grants caps to setuid execs.
    for cap in &DANGEROUS_CAPS {
        let ret = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, *cap as u64, 0, 0, 0) };
        if ret != 0 {
            return Err(anyhow::anyhow!(
                "PR_CAPBSET_DROP for cap {} failed: {}",
                cap,
                std::io::Error::last_os_error()
            ));
        }
    }

    Ok(())
}

pub fn apply_seccomp_filter(dangerous: bool) -> Result<()> {
    if dangerous {
        return Ok(());
    }

    let rules = build_rules()?;

    // SAFETY: this is an allow-list filter; any syscall not explicitly
    // permitted (or, for `clone`, not satisfying the flag mask) is killed
    // with SIGSYS. Residual risk: argument-limited syscalls other than
    // `clone` are unconditionally allowed once the syscall number matches,
    // so a path-based escape (e.g. via still-allowed `open*`) is not
    // prevented here — it is mitigated by the namespace + cap layer. Use
    // `--dangerous` only for trusted debugging.
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Trap,
        SeccompAction::Allow,
        std::env::consts::ARCH.try_into().unwrap(),
    )
    .context("failed to create seccomp filter")?;

    let program: BpfProgram = filter
        .try_into()
        .context("failed to compile seccomp filter")?;

    apply_filter_all_threads(&program).context("failed to apply seccomp filter")?;

    Ok(())
}

/// Build the seccomp allow-list as a syscall→rule map. Extracted so the
/// rule set is unit-testable without applying the filter.
fn build_rules() -> Result<BTreeMap<i64, Vec<SeccompRule>>> {
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();

    let allowed_syscalls = vec![
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_open,
        libc::SYS_close,
        libc::SYS_stat,
        libc::SYS_fstat,
        libc::SYS_lstat,
        libc::SYS_poll,
        libc::SYS_lseek,
        libc::SYS_mmap,
        libc::SYS_mprotect,
        libc::SYS_munmap,
        libc::SYS_brk,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_ioctl,
        libc::SYS_pread64,
        libc::SYS_pwrite64,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_access,
        libc::SYS_pipe,
        libc::SYS_select,
        libc::SYS_sched_yield,
        libc::SYS_mremap,
        libc::SYS_msync,
        libc::SYS_mincore,
        libc::SYS_madvise,
        libc::SYS_dup,
        libc::SYS_dup2,
        libc::SYS_nanosleep,
        libc::SYS_getpid,
        libc::SYS_socket,
        libc::SYS_connect,
        libc::SYS_accept,
        libc::SYS_sendto,
        libc::SYS_recvfrom,
        libc::SYS_sendmsg,
        libc::SYS_recvmsg,
        libc::SYS_shutdown,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_getsockname,
        libc::SYS_getpeername,
        libc::SYS_socketpair,
        libc::SYS_setsockopt,
        libc::SYS_getsockopt,
        // `clone` is intentionally absent from this vector: it is inserted
        // below with a rule that forbids CLONE_NEW* flags (P0-3).
        libc::SYS_fork,
        libc::SYS_vfork,
        libc::SYS_execve,
        libc::SYS_exit,
        libc::SYS_wait4,
        libc::SYS_kill,
        libc::SYS_uname,
        libc::SYS_fcntl,
        libc::SYS_flock,
        libc::SYS_fsync,
        libc::SYS_fdatasync,
        libc::SYS_truncate,
        libc::SYS_ftruncate,
        libc::SYS_getdents,
        libc::SYS_getcwd,
        libc::SYS_chdir,
        libc::SYS_fchdir,
        libc::SYS_rename,
        libc::SYS_mkdir,
        libc::SYS_rmdir,
        libc::SYS_creat,
        libc::SYS_link,
        libc::SYS_unlink,
        libc::SYS_symlink,
        libc::SYS_readlink,
        libc::SYS_chmod,
        libc::SYS_fchmod,
        libc::SYS_chown,
        libc::SYS_fchown,
        libc::SYS_lchown,
        libc::SYS_umask,
        libc::SYS_gettimeofday,
        libc::SYS_getrlimit,
        libc::SYS_getrusage,
        libc::SYS_sysinfo,
        libc::SYS_times,
        libc::SYS_getuid,
        libc::SYS_getgid,
        libc::SYS_setuid,
        libc::SYS_setgid,
        libc::SYS_geteuid,
        libc::SYS_getegid,
        libc::SYS_getppid,
        libc::SYS_getpgrp,
        libc::SYS_setsid,
        libc::SYS_setpgid,
        libc::SYS_getgroups,
        libc::SYS_setgroups,
        libc::SYS_setresuid,
        libc::SYS_getresuid,
        libc::SYS_setresgid,
        libc::SYS_getresgid,
        libc::SYS_getpgid,
        libc::SYS_setfsuid,
        libc::SYS_setfsgid,
        libc::SYS_getsid,
        libc::SYS_capget,
        libc::SYS_capset,
        libc::SYS_rt_sigpending,
        libc::SYS_rt_sigtimedwait,
        libc::SYS_rt_sigqueueinfo,
        libc::SYS_rt_sigsuspend,
        libc::SYS_sigaltstack,
        libc::SYS_personality,
        libc::SYS_statfs,
        libc::SYS_fstatfs,
        libc::SYS_getpriority,
        libc::SYS_setpriority,
        libc::SYS_sched_setparam,
        libc::SYS_sched_getparam,
        libc::SYS_sched_setscheduler,
        libc::SYS_sched_getscheduler,
        libc::SYS_sched_get_priority_max,
        libc::SYS_sched_get_priority_min,
        libc::SYS_sched_rr_get_interval,
        libc::SYS_mlock,
        libc::SYS_munlock,
        libc::SYS_mlockall,
        libc::SYS_munlockall,
        libc::SYS_prctl,
        libc::SYS_arch_prctl,
        libc::SYS_setrlimit,
        libc::SYS_sync,
        libc::SYS_gettid,
        libc::SYS_readahead,
        libc::SYS_setxattr,
        libc::SYS_lsetxattr,
        libc::SYS_fsetxattr,
        libc::SYS_getxattr,
        libc::SYS_lgetxattr,
        libc::SYS_fgetxattr,
        libc::SYS_listxattr,
        libc::SYS_llistxattr,
        libc::SYS_flistxattr,
        libc::SYS_removexattr,
        libc::SYS_lremovexattr,
        libc::SYS_fremovexattr,
        libc::SYS_tkill,
        libc::SYS_time,
        libc::SYS_futex,
        libc::SYS_sched_setaffinity,
        libc::SYS_sched_getaffinity,
        libc::SYS_set_thread_area,
        libc::SYS_io_setup,
        libc::SYS_io_destroy,
        libc::SYS_io_getevents,
        libc::SYS_io_submit,
        libc::SYS_io_cancel,
        libc::SYS_get_thread_area,
        libc::SYS_epoll_create,
        libc::SYS_getdents64,
        libc::SYS_set_tid_address,
        libc::SYS_restart_syscall,
        libc::SYS_semtimedop,
        libc::SYS_fadvise64,
        libc::SYS_timer_create,
        libc::SYS_timer_settime,
        libc::SYS_timer_gettime,
        libc::SYS_timer_getoverrun,
        libc::SYS_timer_delete,
        libc::SYS_clock_settime,
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_clock_nanosleep,
        libc::SYS_exit_group,
        libc::SYS_epoll_wait,
        libc::SYS_epoll_ctl,
        libc::SYS_tgkill,
        libc::SYS_utimes,
        // P0-3: `mbind` / `set_mempolicy` removed (host NUMA interference);
        // `get_mempolicy` (read-only query) is retained.
        libc::SYS_get_mempolicy,
        libc::SYS_mq_open,
        libc::SYS_mq_unlink,
        libc::SYS_mq_timedsend,
        libc::SYS_mq_timedreceive,
        libc::SYS_mq_notify,
        libc::SYS_mq_getsetattr,
        libc::SYS_waitid,
        // P0-3: `ioprio_set` removed (host block-IO interference).
        libc::SYS_ioprio_get,
        libc::SYS_inotify_init,
        libc::SYS_inotify_add_watch,
        libc::SYS_inotify_rm_watch,
        // P0-3: `migrate_pages` / `move_pages` removed (host NUMA interference).
        libc::SYS_openat,
        libc::SYS_mkdirat,
        libc::SYS_mknodat,
        libc::SYS_fchownat,
        libc::SYS_futimesat,
        libc::SYS_newfstatat,
        libc::SYS_unlinkat,
        libc::SYS_renameat,
        libc::SYS_linkat,
        libc::SYS_symlinkat,
        libc::SYS_readlinkat,
        libc::SYS_fchmodat,
        libc::SYS_faccessat,
        libc::SYS_pselect6,
        libc::SYS_ppoll,
        libc::SYS_set_robust_list,
        libc::SYS_get_robust_list,
        libc::SYS_splice,
        libc::SYS_tee,
        libc::SYS_sync_file_range,
        libc::SYS_vmsplice,
        // P0-3: `move_pages` removed (host NUMA interference).
        libc::SYS_utimensat,
        libc::SYS_epoll_pwait,
        libc::SYS_signalfd,
        libc::SYS_timerfd_create,
        libc::SYS_eventfd,
        libc::SYS_fallocate,
        libc::SYS_timerfd_settime,
        libc::SYS_timerfd_gettime,
        libc::SYS_accept4,
        libc::SYS_signalfd4,
        libc::SYS_eventfd2,
        libc::SYS_epoll_create1,
        libc::SYS_dup3,
        libc::SYS_pipe2,
        libc::SYS_inotify_init1,
        libc::SYS_preadv,
        libc::SYS_pwritev,
        libc::SYS_rt_tgsigqueueinfo,
        // P0-3: `perf_event_open` removed (side-channel / host interference).
        libc::SYS_recvmmsg,
        libc::SYS_prlimit64,
        libc::SYS_name_to_handle_at,
        // P0-3: `open_by_handle_at` removed (classic container escape;
        // also CAP_DAC_READ_SEARCH is dropped above).
        libc::SYS_syncfs,
        libc::SYS_sendmmsg,
        libc::SYS_getcpu,
        // P0-3: `process_vm_readv` / `process_vm_writev` removed
        // (cross-process memory read/write).
        libc::SYS_sched_setattr,
        libc::SYS_sched_getattr,
        libc::SYS_renameat2,
        libc::SYS_seccomp,
        libc::SYS_getrandom,
        libc::SYS_memfd_create,
        libc::SYS_execveat,
        libc::SYS_membarrier,
        libc::SYS_mlock2,
        libc::SYS_copy_file_range,
        libc::SYS_preadv2,
        libc::SYS_pwritev2,
        libc::SYS_pkey_mprotect,
        libc::SYS_pkey_alloc,
        libc::SYS_pkey_free,
        libc::SYS_statx,
        libc::SYS_rseq,
    ];

    for syscall in allowed_syscalls {
        rules.insert(syscall, vec![]);
    }

    // P0-3: allow `clone` only when no `CLONE_NEW*` flag is set, so a
    // sandboxed process cannot create fresh namespaces to sidestep the
    // isolation we set up (NEWPID/NEWNS/NEWUTS/NEWNET). Normal fork/exec
    // (e.g. musl's `fork()` = `clone(SIGCHLD)`) is unaffected because it
    // sets none of these bits. `clone3` is not in the allow-list at all.
    let clone_rule = SeccompRule::new(vec![SeccompCondition::new(
        0,
        SeccompCmpArgLen::Qword,
        SeccompCmpOp::MaskedEq(CLONE_NEWNAMESPACES_MASK),
        0,
    )?])?;
    rules.insert(libc::SYS_clone, vec![clone_rule]);

    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seccomp_filter_creation() {
        let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        rules.insert(libc::SYS_read, vec![]);
        rules.insert(libc::SYS_write, vec![]);

        let filter = SeccompFilter::new(
            rules,
            SeccompAction::Trap,
            SeccompAction::Allow,
            std::env::consts::ARCH.try_into().unwrap(),
        );

        assert!(filter.is_ok());
    }

    #[test]
    fn test_dangerous_mode_skips_filter() {
        let result = apply_seccomp_filter(true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_clone_rule_blocks_namespace_flags() {
        // P0-3: `clone` must NOT be unconditionally allowed (`vec![]`); it
        // must carry a condition that forbids CLONE_NEW* flags.
        let rules = build_rules().unwrap();
        let clone_rules = rules
            .get(&libc::SYS_clone)
            .expect("clone must be present in the allow-list");
        assert!(
            !clone_rules.is_empty(),
            "clone must have a flag-condition rule, not an unconditional allow"
        );
    }

    #[test]
    fn test_escape_syscalls_excluded() {
        // P0-3: these escape/interference primitives must be absent from
        // the allow-list.
        let rules = build_rules().unwrap();
        let excluded = [
            libc::SYS_open_by_handle_at,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_perf_event_open,
            libc::SYS_ioprio_set,
            libc::SYS_mbind,
            libc::SYS_set_mempolicy,
            libc::SYS_migrate_pages,
            libc::SYS_move_pages,
        ];
        for sys in excluded {
            assert!(
                !rules.contains_key(&sys),
                "syscall {sys} must be excluded from the allow-list (P0-3)"
            );
        }
    }
}
