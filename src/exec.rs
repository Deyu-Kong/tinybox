use anyhow::{bail, Context, Result};
use nix::sched::{setns, CloneFlags};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{execvp, fork, setsid, ForkResult};
use std::ffi::CString;
use std::fs;

/// Exec a command inside a running tinybox sandbox by `setns`-ing into the
/// target's namespaces directly (no `nsenter` binary dependency). P1-5:
/// replaces the old 23-line `nsenter` wrapper — now namespace-complete
/// (mnt/uts/net/ipc/cgroup/user/pid), PID-validated, and acquires a
/// controlling tty for interactive use.
pub fn exec_in_container(pid: u32, command: &[String]) -> Result<i32> {
    if command.is_empty() {
        bail!("no command specified");
    }

    // Validate the target is a tinybox sandbox: its /proc/<pid>/cgroup must
    // reference a tinybox-* cgroup. This prevents `tinybox exec` from being
    // aimed at an arbitrary host process (a privilege footgun).
    let cgroup_path = format!("/proc/{pid}/cgroup");
    let cgroup = fs::read_to_string(&cgroup_path)
        .with_context(|| format!("failed to read {cgroup_path} (is pid {pid} live?)"))?;
    if !cgroup.contains("tinybox-") {
        bail!(
            "pid {pid} is not in a tinybox cgroup; refusing to exec into an arbitrary host process"
        );
    }

    // Enter each namespace of the target via setns(2). Compare namespace
    // inodes first: setns to the *same* namespace (e.g. the sandbox didn't
    // unshare user/ipc/cgroup, so they equal the host's) returns EINVAL, so
    // we skip those. The PID namespace is entered last and only affects
    // forked children, so we fork below.
    let ns_entries: [(&str, CloneFlags); 7] = [
        ("user", CloneFlags::CLONE_NEWUSER),
        ("mnt", CloneFlags::CLONE_NEWNS),
        ("uts", CloneFlags::CLONE_NEWUTS),
        ("net", CloneFlags::CLONE_NEWNET),
        ("ipc", CloneFlags::CLONE_NEWIPC),
        ("cgroup", CloneFlags::CLONE_NEWCGROUP),
        ("pid", CloneFlags::CLONE_NEWPID),
    ];
    for (name, flag) in ns_entries {
        let self_link = format!("/proc/self/ns/{name}");
        let tgt_link = format!("/proc/{pid}/ns/{name}");
        // Same namespace → no setns needed (and setns would EINVAL).
        if let (Ok(s), Ok(t)) = (
            fs::read_link(&self_link),
            fs::read_link(&tgt_link),
        ) {
            if s == t {
                continue;
            }
        }
        let f = fs::File::open(&tgt_link)
            .with_context(|| format!("failed to open {tgt_link}"))?;
        setns(f, flag)
            .with_context(|| format!("failed to setns({name}) for pid {pid}"))?;
    }

    let program = CString::new(command[0].as_str())?;
    let args: Vec<CString> = command
        .iter()
        .map(|s| CString::new(s.as_str()))
        .collect::<std::result::Result<_, _>>()?;

    // SAFETY: fork after setns(CLONE_NEWPID) so the child is born in the
    // target's PID namespace.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            let status = waitpid(child, None)?;
            Ok(exit_code_from(status))
        }
        ForkResult::Child => {
            // Acquire a controlling terminal for interactive shells.
            // SAFETY: isatty/stdin is a read-only query; setsid + TIOCSCTTY
            // give the child a controlling tty on fd 0.
            if unsafe { libc::isatty(0) } == 1 {
                let _ = setsid();
                unsafe {
                    libc::ioctl(0, libc::TIOCSCTTY, 0i32);
                }
            }
            execvp(&program, &args)?;
            unreachable!()
        }
    }
}

fn exit_code_from(status: WaitStatus) -> i32 {
    match status {
        WaitStatus::Exited(_, code) => code,
        WaitStatus::Signaled(_, signal, _) => 128 + signal as i32,
        _ => 1,
    }
}
