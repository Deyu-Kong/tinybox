use anyhow::{bail, Context, Result};
use nix::fcntl::OFlag;
use nix::mount::{mount, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{
    chdir, close, execvp, execvpe, fork, pipe, pipe2, read, setgid, sethostname, setuid, ForkResult,
};
use std::ffi::CString;
use std::fmt;
use std::io::Read as IoRead;
use std::os::fd::AsRawFd;
use std::os::unix::io::{BorrowedFd, FromRawFd, IntoRawFd};
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::audit::AuditSink;
use crate::broker;
use crate::cgroup::{Cgroup, CgroupConfig};
use crate::landlock;
use crate::oci::NamespaceType;
use crate::policy::{FsRule, NetworkRule};
use crate::proxy;
use crate::rootfs::{mount_volumes, RootfsConfig};
use crate::seccomp::{apply_seccomp_filter, drop_capabilities};

pub struct SandboxConfig {
    pub cgroup_name: Option<String>,
    pub command: Vec<String>,
    pub hostname: Option<String>,
    pub rootfs: Option<PathBuf>,
    pub root_readonly: bool,
    pub env: Vec<String>,
    pub proxy: Option<String>,
    pub volumes: Vec<String>,
    pub memory: Option<u64>,
    pub cpus: Option<f64>,
    pub cpu_quota: Option<i64>,
    pub cpu_period: Option<u64>,
    pub pids_limit: Option<u64>,
    pub dangerous: bool,
    pub filesystem_policy: Option<Vec<FsRule>>,
    pub network_policy: Option<Arc<RwLock<Vec<NetworkRule>>>>,
    pub audit: Option<AuditSink>,
    /// None = unshare all (tinybox default, max isolation). Some(set) = honor
    /// only the listed OCI namespace types ("pid"/"mount"/"uts"/"net"/"ipc"/
    /// "user"/"cgroup").
    pub namespaces: Option<Vec<NamespaceType>>,
    pub cwd: Option<String>,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug)]
pub struct SetupError(String);

impl fmt::Display for SetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "sandbox setup failed: {}", self.0)
    }
}

impl std::error::Error for SetupError {}

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

    validate_namespaces(config)?;

    let program = CString::new(config.command[0].as_str())?;
    let args: Vec<CString> = config
        .command
        .iter()
        .map(|s| CString::new(s.as_str()))
        .collect::<std::result::Result<_, _>>()?;

    // Always create a tracking cgroup for the sandbox (even with no limits):
    // (a) it gives `tinybox exec` a reliable marker to validate a target pid
    // belongs to a tinybox sandbox via /proc/<pid>/cgroup (P1-5), and
    // (b) it lets the daemon kill/track every sandbox by cgroup. The cost is
    // one mkdir in /sys/fs/cgroup per sandbox.
    let cgroup_config = CgroupConfig {
        name: config
            .cgroup_name
            .clone()
            .unwrap_or_else(|| format!("tinybox-{}", std::process::id())),
        memory: config.memory,
        cpus: config.cpus,
        cpu_quota: config.cpu_quota,
        cpu_period: config.cpu_period,
        pids_limit: config.pids_limit,
    };
    let cgroup = Some(Cgroup::new(&cgroup_config)?);
    if let Some(audit) = &config.audit {
        audit.record(
            "cgroup",
            "allow",
            "resources",
            cgroup_config.name.clone(),
            Some("resources:ceiling".into()),
            "cgroup created with requested limits",
        );
    }

    // Fail fast: validate the rootfs BEFORE forking, so a bad rootfs surfaces
    // as a runtime error (Err → daemon "failed" status) rather than a child
    // exit code 1 (which would be mislabeled "completed"). P1-3.
    if let Some(ref rootfs_path) = config.rootfs {
        if !rootfs_path.exists() {
            bail!("rootfs path does not exist: {:?}", rootfs_path);
        }
        if !rootfs_path.is_dir() {
            bail!("rootfs path is not a directory: {:?}", rootfs_path);
        }
    }

    let (read_fd, write_fd) = pipe().context("failed to create pipe")?;
    let read_fd = read_fd.into_raw_fd();
    let write_fd = write_fd.into_raw_fd();
    let (setup_read_fd, setup_write_fd) =
        pipe2(OFlag::O_CLOEXEC).context("failed to create setup error pipe")?;
    let setup_read_fd = setup_read_fd.into_raw_fd();
    let setup_write_fd = setup_write_fd.into_raw_fd();
    let (mut broker_host, mut broker_child) = match &config.network_policy {
        Some(_) => {
            let (host, child) = UnixDatagram::pair().context("failed to create broker channel")?;
            (Some(host), Some(child))
        }
        _ => (None, None),
    };

    // SAFETY: fork() is safe here as we immediately handle namespace setup in the child.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            close(read_fd).ok();
            close(setup_write_fd).ok();
            drop(broker_child.take());
            let broker_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let broker_thread = broker_host.take().map(|channel| {
                let rules = config.network_policy.clone().unwrap();
                let stop = broker_stop.clone();
                let audit = config.audit.clone();
                std::thread::spawn(move || broker::serve(channel, rules, stop, audit))
            });

            if let Some(ref cg) = cgroup {
                cg.add_process(child.as_raw() as u32)?;
            }

            on_pid(child);

            // SAFETY: write_fd is valid (just created by pipe, not yet closed).
            let borrowed = unsafe { BorrowedFd::borrow_raw(write_fd) };
            nix::unistd::write(borrowed, b"go").ok();
            close(write_fd).ok();

            let mut setup_message = String::new();
            // SAFETY: setup_read_fd is owned by this branch and converted once.
            let mut setup_reader = unsafe { std::fs::File::from_raw_fd(setup_read_fd) };
            setup_reader
                .read_to_string(&mut setup_message)
                .context("failed to read child setup status")?;
            let status = waitpid(child, None)?;
            broker_stop.store(true, std::sync::atomic::Ordering::Release);
            if let Some(thread) = broker_thread {
                match thread.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => eprintln!("tinybox broker: {error:#}"),
                    Err(_) => eprintln!("tinybox broker thread panicked"),
                }
            }
            drop(cgroup);
            if !setup_message.is_empty() {
                if let Some(audit) = &config.audit {
                    audit.record(
                        "runtime",
                        "deny",
                        "sandbox.setup",
                        "payload",
                        None,
                        setup_message.clone(),
                    );
                }
                return Err(SetupError(setup_message).into());
            }
            if let Some(audit) = &config.audit {
                if let Some(rules) = &config.filesystem_policy {
                    audit.record(
                        "landlock",
                        "allow",
                        "filesystem.ceiling",
                        format!("{} rules", rules.len()),
                        Some("filesystem:ceiling".into()),
                        "Landlock ruleset installed before payload exec",
                    );
                }
                audit.record(
                    "runtime",
                    "allow",
                    "sandbox.setup",
                    "payload",
                    None,
                    "namespace, filesystem, Landlock, capability and seccomp setup completed",
                );
            }
            Ok(exit_code_from_status(status))
        }
        ForkResult::Child => {
            close(write_fd).ok();
            close(setup_read_fd).ok();
            drop(broker_host.take());

            let mut buf = [0u8; 2];
            let _ = read(read_fd, &mut buf);
            close(read_fd).ok();

            if let Err(e) = child_main(config, &program, &args, setup_write_fd, broker_child.take())
            {
                report_setup_error(setup_write_fd, &format!("{e:#}"));
                eprintln!("tinybox: {}", e);
                std::process::exit(1);
            }
            unreachable!()
        }
    }
}

fn child_main(
    config: &SandboxConfig,
    program: &CString,
    args: &[CString],
    setup_write_fd: i32,
    broker_channel: Option<UnixDatagram>,
) -> Result<()> {
    // SAFETY: unshare the requested namespace set. `config.namespaces == None`
    // is tinybox's default — the original four (NEWPID/NEWNS/NEWUTS/NEWNET),
    // maximum isolation WITHOUT NEWUSER (uid/gid mapping is R5/rootless).
    // When an OCI bundle lists a namespace subset (P1-1), honor only those —
    // that is OCI semantics (the bundle author is responsible for the right
    // set). `CLONE_NEWUSER` is intentionally never unshared here: doing so
    // without writing uid_map leaves the process unmapped and unable to
    // mount; proper rootless user-ns is deferred to R5.
    let mut flags = CloneFlags::empty();
    match &config.namespaces {
        None => {
            flags.insert(
                CloneFlags::CLONE_NEWPID
                    | CloneFlags::CLONE_NEWNS
                    | CloneFlags::CLONE_NEWUTS
                    | CloneFlags::CLONE_NEWNET,
            );
        }
        Some(namespaces) => {
            for namespace in namespaces {
                match namespace {
                    NamespaceType::Pid => {
                        flags.insert(CloneFlags::CLONE_NEWPID);
                    }
                    NamespaceType::Mount => {
                        flags.insert(CloneFlags::CLONE_NEWNS);
                    }
                    NamespaceType::Uts => {
                        flags.insert(CloneFlags::CLONE_NEWUTS);
                    }
                    NamespaceType::Network => {
                        flags.insert(CloneFlags::CLONE_NEWNET);
                    }
                    NamespaceType::Ipc => {
                        flags.insert(CloneFlags::CLONE_NEWIPC);
                    }
                    NamespaceType::Cgroup => {
                        flags.insert(CloneFlags::CLONE_NEWCGROUP);
                    }
                }
            }
        }
    }

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
        let rootfs = RootfsConfig::new(rootfs_path.clone(), config.root_readonly)?;
        rootfs.setup()?;
        // P2-1: mount /dev /tmp /sys /proc under merged BEFORE pivot, while the
        // host's /dev device nodes are still reachable as bind sources.
        rootfs.setup_special_fs()?;
        mount_volumes(&config.volumes, &rootfs.merged_path())?;
        Some(rootfs)
    } else {
        None
    };

    let has_rootfs = rootfs_config.is_some();

    // SAFETY: fork() after unshare(CLONE_NEWPID) creates a process that is PID 1
    // in the new PID namespace.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            close(setup_write_fd).ok();
            drop(broker_channel);
            let status = waitpid(child, None)?;
            let code = exit_code_from_status(status);
            drop(rootfs_config);
            std::process::exit(code);
        }
        ForkResult::Child => {
            if let Some(ref rootfs) = rootfs_config {
                rootfs.pivot()?;
                drop(rootfs_config);
            } else {
                // No isolated rootfs: mount /proc in-place in the new mount ns.
                // (No /dev/tmp/sys hardening — the no-rootfs path shares the
                // host rootfs and is inherently less isolated; the hardened
                // path is rootfs+pivot above.)
                mount_proc()?;
            }
            if !has_rootfs {
                mount_volumes(&config.volumes, std::path::Path::new("/"))?;
            }
            let proxy_enabled = if let Some(channel) = broker_channel {
                start_proxy_helper(channel)?;
                true
            } else {
                false
            };
            // P1-1: honor OCI process.user — set gid then uid while we still
            // hold CAP_SETUID/CAP_SETGID (caps dropped below). gid must be set
            // before uid per setuid(2) rules.
            if config.gid != 0 {
                setgid(nix::unistd::Gid::from_raw(config.gid))?;
            }
            if config.uid != 0 {
                setuid(nix::unistd::Uid::from_raw(config.uid))?;
            }
            if let Some(rules) = &config.filesystem_policy {
                landlock::enforce(rules)?;
            }
            drop_capabilities(config.dangerous)?;
            apply_seccomp_filter(config.dangerous)?;
            // P1-1: honor OCI process.cwd.
            if let Some(ref cwd) = config.cwd {
                chdir(cwd.as_str())?;
            }
            let env_values = effective_environment(config, proxy_enabled);
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

fn validate_namespaces(config: &SandboxConfig) -> Result<()> {
    if !config.volumes.is_empty() && config.rootfs.is_none() {
        bail!("volume mounts require an isolated rootfs");
    }
    if let Some(namespaces) = &config.namespaces {
        if !namespaces.contains(&NamespaceType::Mount) {
            bail!("a private mount namespace is required for sandbox filesystem setup");
        }
    }
    Ok(())
}

fn report_setup_error(fd: i32, message: &str) {
    let bytes = message.as_bytes();
    // SAFETY: fd is the write end of the setup pipe and bytes is valid for its length.
    unsafe {
        libc::write(fd, bytes.as_ptr().cast(), bytes.len());
    }
    close(fd).ok();
}

fn start_proxy_helper(channel: UnixDatagram) -> Result<()> {
    let (ready_read, ready_write) = pipe().context("failed to create proxy readiness pipe")?;
    match unsafe { fork() }? {
        ForkResult::Parent { .. } => {
            drop(ready_write);
            drop(channel);
            let mut byte = [0u8; 1];
            let count = read(ready_read.as_raw_fd(), &mut byte)?;
            if count != 1 {
                bail!("sandbox proxy failed before becoming ready");
            }
            Ok(())
        }
        ForkResult::Child => {
            drop(ready_read);
            let ready_fd = ready_write.into_raw_fd();
            if let Err(error) = proxy::serve(channel, ready_fd) {
                eprintln!("tinybox proxy: {error:#}");
                std::process::exit(1);
            }
            unreachable!()
        }
    }
}

fn effective_environment(config: &SandboxConfig, proxy_enabled: bool) -> Vec<String> {
    let mut env = config.env.clone();
    let proxy = if proxy_enabled {
        Some(format!("http://{}", proxy::PROXY_ADDRESS))
    } else {
        config.proxy.clone()
    };
    if let Some(proxy) = proxy {
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
            cgroup_name: None,
            command: vec![],
            hostname: None,
            rootfs: None,
            root_readonly: false,
            env: Vec::new(),
            proxy: None,
            volumes: Vec::new(),
            memory: None,
            cpus: None,
            cpu_quota: None,
            cpu_period: None,
            pids_limit: None,
            dangerous: false,
            filesystem_policy: None,
            network_policy: None,
            audit: None,
            namespaces: None,
            cwd: None,
            uid: 0,
            gid: 0,
        };
        assert!(run_sandbox(&config).is_err());
    }

    #[test]
    fn explicit_namespaces_require_mount_namespace() {
        let config = SandboxConfig {
            cgroup_name: None,
            command: vec!["true".into()],
            hostname: None,
            rootfs: None,
            root_readonly: false,
            env: Vec::new(),
            proxy: None,
            volumes: Vec::new(),
            memory: None,
            cpus: None,
            cpu_quota: None,
            cpu_period: None,
            pids_limit: None,
            dangerous: false,
            filesystem_policy: None,
            network_policy: None,
            audit: None,
            namespaces: Some(vec![NamespaceType::Pid]),
            cwd: None,
            uid: 0,
            gid: 0,
        };
        assert!(validate_namespaces(&config).is_err());
    }

    #[test]
    fn volumes_require_isolated_rootfs() {
        let config = SandboxConfig {
            cgroup_name: None,
            command: vec!["true".into()],
            hostname: None,
            rootfs: None,
            root_readonly: false,
            env: Vec::new(),
            proxy: None,
            volumes: vec!["/host:/sandbox".into()],
            memory: None,
            cpus: None,
            cpu_quota: None,
            cpu_period: None,
            pids_limit: None,
            dangerous: false,
            filesystem_policy: None,
            network_policy: None,
            audit: None,
            namespaces: None,
            cwd: None,
            uid: 0,
            gid: 0,
        };
        assert!(validate_namespaces(&config).is_err());
    }
}
