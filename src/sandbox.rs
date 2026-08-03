use anyhow::{bail, Context, Result};
use nix::mount::{mount, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{close, execvp, execvpe, fork, pipe, read, sethostname, ForkResult};
use std::ffi::CString;
use std::os::unix::io::{BorrowedFd, IntoRawFd};
use std::path::PathBuf;

use crate::cgroup::{Cgroup, CgroupConfig};
use crate::network::{self, NetworkConfig};
use crate::rootfs::RootfsConfig;
use crate::seccomp::{apply_seccomp_filter, drop_capabilities};

pub struct SandboxConfig {
    pub command: Vec<String>,
    pub hostname: Option<String>,
    pub rootfs: Option<PathBuf>,
    pub env: Vec<String>,
    pub proxy: Option<String>,
    pub network: Option<String>,
    pub ports: Vec<String>,
    pub volumes: Vec<String>,
    pub memory: Option<u64>,
    pub cpus: Option<f64>,
    pub cpu_quota: Option<i64>,
    pub cpu_period: Option<u64>,
    pub pids_limit: Option<u64>,
    pub dangerous: bool,
}

pub fn run_sandbox(config: &SandboxConfig) -> Result<i32> {
    run_sandbox_with_pid(config, |_| {})
}

pub fn run_sandbox_with_pid<F>(config: &SandboxConfig, on_pid: F) -> Result<i32>
where
    F: FnOnce(nix::unistd::Pid),
{
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

    let needs_cgroup = config.memory.is_some()
        || config.cpus.is_some()
        || config.cpu_quota.is_some()
        || config.pids_limit.is_some();

    let cgroup = if needs_cgroup {
        let cgroup_config = CgroupConfig {
            name: format!("tinybox-{}", std::process::id()),
            memory: config.memory,
            cpus: config.cpus,
            cpu_quota: config.cpu_quota,
            cpu_period: config.cpu_period,
            pids_limit: config.pids_limit,
        };
        Some(Cgroup::new(&cgroup_config)?)
    } else {
        None
    };

    let net_config = if config.network.is_some() {
        network::setup_bridge()?;
        let net_cfg = NetworkConfig::new(std::process::id());
        network::create_veth_pair(&net_cfg)?;
        Some(net_cfg)
    } else {
        None
    };

    let (read_fd, write_fd) = pipe().context("failed to create pipe")?;
    let read_fd = read_fd.into_raw_fd();
    let write_fd = write_fd.into_raw_fd();

    // SAFETY: fork() is safe here as we immediately handle namespace setup in the child.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            close(read_fd).ok();

            if let Some(ref cg) = cgroup {
                cg.add_process(child.as_raw() as u32)?;
            }

            if let Some(ref net_cfg) = net_config {
                network::move_veth_to_ns(net_cfg, child.as_raw() as u32)?;
                for port_spec in &config.ports {
                    if let Some((host_port, container_port)) = parse_port_spec(port_spec) {
                        network::setup_port_mapping(host_port, &net_cfg.container_ip, container_port)?;
                    }
                }
            }

            on_pid(child);

            // SAFETY: write_fd is valid (just created by pipe, not yet closed).
            let borrowed = unsafe { BorrowedFd::borrow_raw(write_fd) };
            nix::unistd::write(borrowed, b"go").ok();
            close(write_fd).ok();

            let status = waitpid(child, None)?;
            if let Some(ref net_cfg) = net_config {
                network::cleanup_veth(net_cfg)?;
            }
            drop(cgroup);
            Ok(exit_code_from_status(status))
        }
        ForkResult::Child => {
            close(write_fd).ok();

            let mut buf = [0u8; 2];
            let _ = read(read_fd, &mut buf);
            close(read_fd).ok();

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
    if config.proxy.is_none() && config.network.is_none() {
        flags.insert(CloneFlags::CLONE_NEWNET);
    }

    unshare(flags)?;

    if config.network.is_some() {
        let net_cfg = NetworkConfig::new(std::process::id());
        network::configure_container_network(&net_cfg)?;
    }

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
            mount_volumes(&config.volumes)?;
            drop_capabilities(config.dangerous)?;
            apply_seccomp_filter(config.dangerous)?;
            let env_values = effective_environment(config);
            if env_values.is_empty() {
                execvp(program, args)?;
            } else {
                let env: Vec<CString> = env_values
                    .iter()
                    .map(|value| CString::new(value.as_str()))
                    .collect::<std::result::Result<_, _>>()?;
                execvpe(program, args, &env)?;
            }
            unreachable!()
        }
    }
}

fn effective_environment(config: &SandboxConfig) -> Vec<String> {
    let mut env = config.env.clone();
    if let Some(proxy) = &config.proxy {
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            env.push(format!("{}={}", key, proxy));
        }
        env.push("NO_PROXY=127.0.0.1,localhost".to_string());
    }
    env
}

fn mount_proc() -> Result<()> {
    std::fs::create_dir_all("/proc").ok();
    mount(
        Some("proc"),
        "/proc",
        Some("proc"),
        MsFlags::empty(),
        None::<&str>,
    )?;
    Ok(())
}

fn mount_volumes(volumes: &[String]) -> Result<()> {
    for vol_spec in volumes {
        let parts: Vec<&str> = vol_spec.split(':').collect();
        if parts.len() < 2 {
            bail!("invalid volume spec: {}", vol_spec);
        }
        let host_path = parts[0];
        let container_path = parts[1];
        let readonly = parts.get(2).map(|s| *s == "ro").unwrap_or(false);

        std::fs::create_dir_all(container_path).ok();

        let mut flags = MsFlags::MS_BIND | MsFlags::MS_REC;
        if readonly {
            flags |= MsFlags::MS_RDONLY;
        }

        mount(
            Some(host_path),
            container_path,
            None::<&str>,
            flags,
            None::<&str>,
        )
        .with_context(|| format!("failed to mount {} to {}", host_path, container_path))?;
    }
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
        Signal::SIGSYS => 31,
        _ => 0,
    }
}

fn parse_port_spec(spec: &str) -> Option<(u16, u16)> {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() == 2 {
        let host_port = parts[0].parse().ok()?;
        let container_port = parts[1].parse().ok()?;
        Some((host_port, container_port))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_code_exited() {
        assert_eq!(
            exit_code_from_status(WaitStatus::Exited(nix::unistd::Pid::from_raw(1), 0)),
            0
        );
        assert_eq!(
            exit_code_from_status(WaitStatus::Exited(nix::unistd::Pid::from_raw(1), 42)),
            42
        );
    }

    #[test]
    fn test_exit_code_signaled() {
        assert_eq!(
            exit_code_from_status(WaitStatus::Signaled(
                nix::unistd::Pid::from_raw(1),
                Signal::SIGKILL,
                false
            )),
            128 + 9
        );
    }

    #[test]
    fn test_empty_command() {
        let config = SandboxConfig {
            command: vec![],
            hostname: None,
            rootfs: None,
            env: Vec::new(),
            proxy: None,
            network: None,
            ports: Vec::new(),
            volumes: Vec::new(),
            memory: None,
            cpus: None,
            cpu_quota: None,
            cpu_period: None,
            pids_limit: None,
            dangerous: false,
        };
        assert!(run_sandbox(&config).is_err());
    }
}
