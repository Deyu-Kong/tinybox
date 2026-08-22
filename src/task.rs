use crate::landlock;
use crate::policy::FsRule;
use crate::seccomp::{apply_seccomp_filter, drop_capabilities};
use anyhow::{bail, Context, Result};
use nix::sched::{setns, CloneFlags};
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{chdir, close, dup2, execvpe, fork, pipe, pipe2, read, ForkResult, Pid};
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs;
use std::io::Read as IoRead;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
static NEXT_EXEC_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecRequest {
    pub command: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub output_truncated: bool,
}

#[derive(Debug, Clone)]
pub struct ExecTarget {
    pub keeper_pid: i32,
    pub keeper_start_time: u64,
    pub cgroup_path: PathBuf,
    pub filesystem_policy: Vec<FsRule>,
    pub base_env: Vec<String>,
}

fn default_timeout_ms() -> u64 {
    120_000
}

pub fn process_start_time(pid: i32) -> Result<u64> {
    let path = format!("/proc/{pid}/stat");
    let stat = fs::read_to_string(&path).with_context(|| format!("failed to read {path}"))?;
    // comm is parenthesized and may contain spaces. Field 22 is the 20th token
    // after the closing parenthesis (state is field 3).
    let end = stat
        .rfind(')')
        .context("malformed /proc pid stat: missing comm terminator")?;
    let fields: Vec<&str> = stat[end + 1..].split_whitespace().collect();
    fields
        .get(19)
        .context("malformed /proc pid stat: missing starttime")?
        .parse()
        .context("invalid process starttime")
}

pub fn destroy_task(supervisor_pid: i32, cgroup_path: &Path) -> Result<()> {
    let name = validate_task_cgroup_path(cgroup_path)?;

    if Path::new(&format!("/proc/{supervisor_pid}")).exists() {
        let membership = fs::read_to_string(format!("/proc/{supervisor_pid}/cgroup"))
            .context("failed to verify task supervisor cgroup")?;
        if !membership.lines().any(|line| line.ends_with(name)) {
            bail!("task supervisor is no longer in its recorded cgroup");
        }
        let _ = kill(Pid::from_raw(supervisor_pid), Signal::SIGKILL);
    }
    kill_exec_cgroup(cgroup_path);

    cleanup_task_cgroup(cgroup_path)
}

pub fn cleanup_orphaned_task_cgroups() -> Result<()> {
    let root = Path::new("/sys/fs/cgroup");
    for entry in fs::read_dir(root).context("failed to scan cgroup root")? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if name.starts_with("tinybox-task-") && path.is_dir() {
            let populated = fs::read_to_string(path.join("cgroup.events"))
                .unwrap_or_default()
                .lines()
                .any(|line| line == "populated 1");
            // Another daemon may intentionally use a different listen address.
            // Startup reconciliation must never kill a task it cannot own.
            if !populated {
                cleanup_task_cgroup(&path)?;
            }
        }
    }
    Ok(())
}

fn validate_task_cgroup_path(cgroup_path: &Path) -> Result<&str> {
    let name = cgroup_path
        .file_name()
        .and_then(|value| value.to_str())
        .context("invalid task cgroup path")?;
    if cgroup_path.parent() != Some(Path::new("/sys/fs/cgroup"))
        || !name.starts_with("tinybox-task-")
    {
        bail!("refusing to destroy an unrecognized task cgroup");
    }
    Ok(name)
}

fn cleanup_task_cgroup(cgroup_path: &Path) -> Result<()> {
    validate_task_cgroup_path(cgroup_path)?;

    for _ in 0..100 {
        let populated = fs::read_to_string(cgroup_path.join("cgroup.events"))
            .unwrap_or_default()
            .lines()
            .any(|line| line == "populated 1");
        if !populated {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let still_populated = fs::read_to_string(cgroup_path.join("cgroup.events"))
        .unwrap_or_default()
        .lines()
        .any(|line| line == "populated 1");
    if still_populated {
        bail!("task cgroup remained populated after kill");
    }

    if let Ok(entries) = fs::read_dir(cgroup_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                cleanup_exec_cgroup(&path);
            }
        }
    }
    if let Err(error) = fs::remove_dir(cgroup_path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error).with_context(|| {
                format!("failed to remove task cgroup {}", cgroup_path.display())
            });
        }
    }
    Ok(())
}

pub fn exec_in_task(target: &ExecTarget, request: &ExecRequest) -> Result<ExecResponse> {
    validate_request(request)?;
    validate_target(target)?;

    let root = fs::File::open(format!("/proc/{}/root", target.keeper_pid))
        .context("failed to open task root")?;
    let namespaces = open_namespaces(target.keeper_pid)?;
    if process_start_time(target.keeper_pid)? != target.keeper_start_time {
        bail!("task keeper identity changed while preparing exec");
    }

    let exec_id = NEXT_EXEC_ID.fetch_add(1, Ordering::Relaxed);
    let exec_cgroup = target
        .cgroup_path
        .join(format!("exec-{}-{exec_id}", std::process::id()));
    fs::create_dir(&exec_cgroup)
        .with_context(|| format!("failed to create exec cgroup {}", exec_cgroup.display()))?;

    let stdout_pipe = pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    let stderr_pipe = pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    let (ready_read, ready_write) = pipe()?;
    let (stdout_read, stdout_write) = (stdout_pipe.0.into_raw_fd(), stdout_pipe.1.into_raw_fd());
    let (stderr_read, stderr_write) = (stderr_pipe.0.into_raw_fd(), stderr_pipe.1.into_raw_fd());

    // SAFETY: the child performs only namespace/setup operations and then
    // follows a fork/exec path. The host parent remains outside all namespaces.
    let helper = match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            drop(ready_read);
            close(stdout_write).ok();
            close(stderr_write).ok();
            fs::write(exec_cgroup.join("cgroup.procs"), child.as_raw().to_string())
                .context("failed to move task exec helper into cgroup")?;
            nix::unistd::write(&ready_write, b"g")?;
            drop(ready_write);
            child
        }
        ForkResult::Child => {
            drop(ready_write);
            let mut byte = [0u8; 1];
            if read(ready_read.as_raw_fd(), &mut byte)? != 1 {
                std::process::exit(125);
            }
            drop(ready_read);
            close(stdout_read).ok();
            close(stderr_read).ok();
            dup2(stdout_write, libc::STDOUT_FILENO)?;
            dup2(stderr_write, libc::STDERR_FILENO)?;
            close(stdout_write).ok();
            close(stderr_write).ok();
            if let Err(error) = enter_and_exec(target, request, root, namespaces) {
                eprintln!("tinybox task exec setup failed: {error:#}");
                std::process::exit(125);
            }
            unreachable!()
        }
    };

    let response = collect_helper(
        helper,
        stdout_read,
        stderr_read,
        request.timeout_ms,
        &exec_cgroup,
    );
    kill_exec_cgroup(&exec_cgroup);
    cleanup_exec_cgroup(&exec_cgroup);
    response
}

fn validate_target(target: &ExecTarget) -> Result<()> {
    if process_start_time(target.keeper_pid)? != target.keeper_start_time {
        bail!("task keeper PID was reused or the task has exited");
    }
    let cgroup = fs::read_to_string(format!("/proc/{}/cgroup", target.keeper_pid))?;
    let expected = target
        .cgroup_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("invalid task cgroup path")?;
    if !cgroup.lines().any(|line| line.ends_with(expected)) {
        bail!("task keeper is no longer in its recorded cgroup");
    }
    Ok(())
}

fn open_namespaces(pid: i32) -> Result<Vec<(CloneFlags, fs::File, &'static str)>> {
    let entries = [
        (CloneFlags::CLONE_NEWUSER, "user"),
        (CloneFlags::CLONE_NEWNS, "mnt"),
        (CloneFlags::CLONE_NEWUTS, "uts"),
        (CloneFlags::CLONE_NEWNET, "net"),
        (CloneFlags::CLONE_NEWIPC, "ipc"),
        (CloneFlags::CLONE_NEWCGROUP, "cgroup"),
        (CloneFlags::CLONE_NEWPID, "pid"),
    ];
    entries
        .into_iter()
        .map(|(flag, name)| {
            fs::File::open(format!("/proc/{pid}/ns/{name}"))
                .with_context(|| format!("failed to open task {name} namespace"))
                .map(|file| (flag, file, name))
        })
        .collect()
}

fn enter_and_exec(
    target: &ExecTarget,
    request: &ExecRequest,
    root: fs::File,
    namespaces: Vec<(CloneFlags, fs::File, &'static str)>,
) -> Result<()> {
    for (flag, namespace, name) in namespaces {
        let self_ns = fs::read_link(format!("/proc/self/ns/{name}"));
        let target_ns = fs::read_link(format!("/proc/{}/ns/{name}", target.keeper_pid));
        if let (Ok(current), Ok(target)) = (self_ns, target_ns) {
            if current == target {
                continue;
            }
        }
        setns(namespace, flag).with_context(|| format!("failed to enter task {name} namespace"))?;
    }

    // Entering a mount namespace does not change a process's root directory.
    // Anchor to the keeper's root opened before setns, then chroot explicitly.
    nix::unistd::fchdir(root.as_raw_fd())?;
    nix::unistd::chroot(".")?;
    chdir(request.cwd.as_deref().unwrap_or("/workspace"))?;

    // setns(CLONE_NEWPID) applies only to subsequently created children.
    // Fork once more so the payload is actually born in the task PID ns.
    // SAFETY: the child immediately drops privilege, installs policy, and execs.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            let status = waitpid(child, None)?;
            std::process::exit(exit_code(status));
        }
        ForkResult::Child => {
            landlock::enforce(&target.filesystem_policy)?;
            drop_capabilities(false)?;
            apply_seccomp_filter(false)?;
            let args = request
                .command
                .iter()
                .map(|value| CString::new(value.as_str()))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let env = merge_environment(&target.base_env, &request.env)?;
            let program_path = resolve_program(&request.command[0], &env)?;
            let program = CString::new(program_path)?;
            let env = env
                .iter()
                .map(|value| CString::new(value.as_str()))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            execvpe(&program, &args, &env)?;
            unreachable!()
        }
    }
}

fn validate_request(request: &ExecRequest) -> Result<()> {
    if request.command.is_empty() {
        bail!("command must not be empty");
    }
    if request.timeout_ms == 0 || request.timeout_ms > 3_600_000 {
        bail!("timeout_ms must be between 1 and 3600000");
    }
    if let Some(cwd) = &request.cwd {
        if !cwd.starts_with('/') || cwd.split('/').any(|part| part == "..") {
            bail!("cwd must be a normalized absolute sandbox path");
        }
    }
    for value in &request.env {
        validate_env(value)?;
    }
    Ok(())
}

fn merge_environment(base: &[String], delta: &[String]) -> Result<Vec<String>> {
    let mut values = std::collections::BTreeMap::new();
    for value in base.iter().chain(delta) {
        validate_env(value)?;
        let (name, _) = value.split_once('=').expect("validated environment entry");
        values.insert(name.to_string(), value.clone());
    }
    Ok(values.into_values().collect())
}

fn validate_env(value: &str) -> Result<()> {
    let Some((name, _)) = value.split_once('=') else {
        bail!("environment entries must use NAME=VALUE");
    };
    if name.is_empty()
        || !name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
    {
        bail!("invalid environment variable name: {name}");
    }
    Ok(())
}

fn resolve_program(command: &str, env: &[String]) -> Result<String> {
    if command.contains('/') {
        return Ok(command.to_string());
    }
    let path = env
        .iter()
        .rev()
        .find_map(|value| value.strip_prefix("PATH="))
        .unwrap_or("/usr/bin:/bin");
    for directory in path.split(':').filter(|value| !value.is_empty()) {
        let candidate = Path::new(directory).join(command);
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }
    bail!("command not found in task PATH: {command}")
}

fn collect_helper(
    helper: Pid,
    stdout_fd: i32,
    stderr_fd: i32,
    timeout_ms: u64,
    exec_cgroup: &Path,
) -> Result<ExecResponse> {
    // SAFETY: each raw descriptor is owned here and converted exactly once.
    let mut stdout_file = unsafe { fs::File::from_raw_fd(stdout_fd) };
    let mut stderr_file = unsafe { fs::File::from_raw_fd(stderr_fd) };
    set_nonblocking(stdout_file.as_raw_fd())?;
    set_nonblocking(stderr_file.as_raw_fd())?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut status = None;
    let mut timed_out = false;
    let mut output_truncated = false;

    while status.is_none() || stdout_open || stderr_open {
        if status.is_none() {
            status = match waitpid(helper, Some(WaitPidFlag::WNOHANG))? {
                WaitStatus::StillAlive => None,
                other => Some(other),
            };
        }
        drain(
            &mut stdout_file,
            &mut stdout,
            &mut stdout_open,
            &mut output_truncated,
        )?;
        drain(
            &mut stderr_file,
            &mut stderr,
            &mut stderr_open,
            &mut output_truncated,
        )?;
        if status.is_none() && Instant::now() >= deadline {
            timed_out = true;
            kill_exec_cgroup(exec_cgroup);
            let _ = kill(helper, Signal::SIGKILL);
            status = Some(waitpid(helper, None)?);
        }
        if status.is_some() {
            // A tool call may not leave untracked background processes. Kill
            // the per-exec cgroup so pipe writers cannot outlive the result.
            kill_exec_cgroup(exec_cgroup);
        }
        if status.is_some() && !stdout_open && !stderr_open {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(ExecResponse {
        exit_code: if timed_out {
            124
        } else {
            exit_code(status.expect("helper status collected"))
        },
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
        output_truncated,
    })
}

fn kill_exec_cgroup(path: &Path) {
    if fs::write(path.join("cgroup.kill"), "1").is_ok() {
        return;
    }
    if let Ok(pids) = fs::read_to_string(path.join("cgroup.procs")) {
        for pid in pids.lines().filter_map(|value| value.parse::<i32>().ok()) {
            let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
        }
    }
}

fn cleanup_exec_cgroup(path: &Path) {
    for _ in 0..20 {
        if fs::remove_dir(path).is_ok() || !path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn drain(
    file: &mut fs::File,
    output: &mut Vec<u8>,
    open: &mut bool,
    truncated: &mut bool,
) -> Result<()> {
    if !*open {
        return Ok(());
    }
    let mut buffer = [0u8; 8192];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => {
                *open = false;
                return Ok(());
            }
            Ok(count) => {
                let remaining = MAX_OUTPUT_BYTES.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..count.min(remaining)]);
                if count > remaining {
                    *truncated = true;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error).context("failed to capture task output"),
        }
    }
}

fn set_nonblocking(fd: i32) -> Result<()> {
    // SAFETY: fcntl only reads and updates flags on an owned descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to set nonblocking pipe");
    }
    Ok(())
}

fn exit_code(status: WaitStatus) -> i32 {
    match status {
        WaitStatus::Exited(_, code) => code,
        WaitStatus::Signaled(_, signal, _) => 128 + signal as i32,
        _ => 125,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_process_start_time() {
        assert!(process_start_time(std::process::id() as i32).unwrap() > 0);
    }

    #[test]
    fn rejects_unsafe_request_fields() {
        let mut request = ExecRequest {
            command: vec!["true".into()],
            cwd: Some("../outside".into()),
            env: Vec::new(),
            timeout_ms: 100,
        };
        assert!(validate_request(&request).is_err());
        request.cwd = Some("/workspace".into());
        request.env = vec!["BAD-NAME=value".into()];
        assert!(validate_request(&request).is_err());
        request.env = vec!["GOOD_NAME=value".into()];
        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn environment_delta_overrides_base() {
        let merged = merge_environment(
            &["PATH=/bin".into(), "LANG=C".into()],
            &["LANG=en_US.UTF-8".into()],
        )
        .unwrap();
        assert!(merged.contains(&"PATH=/bin".to_string()));
        assert!(merged.contains(&"LANG=en_US.UTF-8".to_string()));
        assert!(!merged.contains(&"LANG=C".to_string()));
    }

    #[test]
    fn resolves_program_from_task_environment() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("tool"), "fixture").unwrap();
        let env = vec![format!("PATH={}", directory.path().display())];
        assert_eq!(
            resolve_program("tool", &env).unwrap(),
            directory.path().join("tool").to_string_lossy()
        );
        assert!(resolve_program("missing", &env).is_err());
    }
}
