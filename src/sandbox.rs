use anyhow::{bail, Context, Result};
use nix::mount::{mount, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{execvp, fork, sethostname, ForkResult};
use std::ffi::CString;
use std::path::PathBuf;

use crate::cgroup::{Cgroup, CgroupConfig};
use crate::rootfs::RootfsConfig;

pub struct SandboxConfig {
    pub command: Vec<String>,
    pub hostname: Option<String>,
    pub rootfs: Option<PathBuf>,
    pub memory: Option<u64>,
    pub cpus: Option<f64>,
    pub pids_limit: Option<u64>,
}

pub fn run_sandbox(config: &SandboxConfig) -> Result<i32> {
    if !cfg!(target_os = "linux") {
        bail!("tinybox only supports Linux");
    }

    if config.command.is_empty() {
        bail!("no command specified");
    }

    let program = CString::new(config.command[0].as_str())?;
    let args: Vec<CString> = config
        .command
        .iter()
        .map(|s| CString::new(s.as_str()).unwrap())
        .collect();

    // SAFETY: fork() is safe here as we immediately handle namespace setup in the child.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            let needs_cgroup = config.memory.is_some() || config.cpus.is_some() || config.pids_limit.is_some();
            let cgroup = if needs_cgroup {
                let cgroup_config = CgroupConfig {
                    name: format!("tinybox-{}", child),
                    memory: config.memory,
                    cpus: config.cpus,
                    pids_limit: config.pids_limit,
                };
                let cg = Cgroup::new(&cgroup_config)?;
                cg.add_process(child.as_raw() as u32)?;
                Some(cg)
            } else {
                None
            };

            let status = waitpid(child, None)?;
            drop(cgroup);
            Ok(exit_code_from_status(status))
        }
        ForkResult::Child => {
            if let Err(e) = child_main(config, &program, &args) {
                eprintln!("tinybox: {}", e);
                std::process::exit(1);
            }
            unreachable!()
        }
    }
}

fn child_main(config: &SandboxConfig, program: &CString, args: &[CString]) -> Result<()> {
    let mut flags = CloneFlags::empty();
    flags.insert(CloneFlags::CLONE_NEWPID);
    flags.insert(CloneFlags::CLONE_NEWNS);
    flags.insert(CloneFlags::CLONE_NEWUTS);

    unshare(flags)?;

    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("failed to make mounts private")?;

    if let Some(ref hostname) = config.hostname {
        sethostname(hostname)?;
    }

    let rootfs_config = if let Some(ref rootfs_path) = config.rootfs {
        let rootfs = RootfsConfig::new(rootfs_path.clone())?;
        rootfs.setup()?;
        Some(rootfs)
    } else {
        None
    };

    // SAFETY: fork() after unshare(CLONE_NEWPID) creates a process that is PID 1
    // in the new PID namespace.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            let status = waitpid(child, None)?;
            let code = exit_code_from_status(status);
            drop(rootfs_config);
            std::process::exit(code);
        }
        ForkResult::Child => {
            if let Some(ref rootfs) = rootfs_config {
                rootfs.pivot()?;
            }
            drop(rootfs_config);
            mount_proc()?;
            execvp(program, args)?;
            unreachable!()
        }
    }
}

fn mount_proc() -> Result<()> {
    mount(
        Some("proc"),
        "/proc",
        Some("proc"),
        MsFlags::empty(),
        None::<&str>,
    )?;
    Ok(())
}

fn exit_code_from_status(status: WaitStatus) -> i32 {
    match status {
        WaitStatus::Exited(_, code) => code,
        WaitStatus::Signaled(_, signal, _) => 128 + signal_to_int(signal),
        _ => 1,
    }
}

fn signal_to_int(signal: Signal) -> i32 {
    match signal {
        Signal::SIGHUP => 1,
        Signal::SIGINT => 2,
        Signal::SIGQUIT => 3,
        Signal::SIGILL => 4,
        Signal::SIGTRAP => 5,
        Signal::SIGABRT => 6,
        Signal::SIGBUS => 7,
        Signal::SIGFPE => 8,
        Signal::SIGKILL => 9,
        Signal::SIGUSR1 => 10,
        Signal::SIGSEGV => 11,
        Signal::SIGUSR2 => 12,
        Signal::SIGPIPE => 13,
        Signal::SIGALRM => 14,
        Signal::SIGTERM => 15,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_code_exited() {
        assert_eq!(exit_code_from_status(WaitStatus::Exited(nix::unistd::Pid::from_raw(1), 0)), 0);
        assert_eq!(exit_code_from_status(WaitStatus::Exited(nix::unistd::Pid::from_raw(1), 42)), 42);
    }

    #[test]
    fn test_exit_code_signaled() {
        assert_eq!(
            exit_code_from_status(WaitStatus::Signaled(nix::unistd::Pid::from_raw(1), Signal::SIGKILL, false)),
            128 + 9
        );
    }

    #[test]
    fn test_empty_command() {
        let config = SandboxConfig {
            command: vec![],
            hostname: None,
            rootfs: None,
            memory: None,
            cpus: None,
            pids_limit: None,
        };
        assert!(run_sandbox(&config).is_err());
    }
}
