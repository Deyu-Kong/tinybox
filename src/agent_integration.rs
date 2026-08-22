use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};

const OPENCODE_TOOL: &str = include_str!("../adapters/opencode/bash.ts");
const PI_EXTENSION: &str = include_str!("../adapters/pi/tinybox.ts");
const SHARED_RUNTIME: &str = include_str!("../adapters/shared/runtime.js");
static AGENT_PID: AtomicI32 = AtomicI32::new(0);

pub fn install(agent: &str, project: Option<&Path>, force: bool) -> Result<PathBuf> {
    let (directory, entry) = match (agent, project) {
        ("opencode", Some(root)) => (root.join(".opencode/tools"), ("bash.ts", OPENCODE_TOOL)),
        ("opencode", None) => (
            config_home()?.join("opencode/tools"),
            ("bash.ts", OPENCODE_TOOL),
        ),
        ("pi", Some(root)) => (
            root.join(".pi/extensions/tinybox"),
            ("index.ts", PI_EXTENSION),
        ),
        ("pi", None) => (
            home_dir()?.join(".pi/agent/extensions/tinybox"),
            ("index.ts", PI_EXTENSION),
        ),
        _ => bail!("unsupported tool adapter {agent:?}; expected opencode or pi"),
    };
    fs::create_dir_all(&directory)
        .with_context(|| format!("create adapter directory {}", directory.display()))?;
    install_file(&directory.join(entry.0), entry.1, force)?;
    install_file(&directory.join("runtime.js"), SHARED_RUNTIME, force)?;
    Ok(directory)
}

pub struct LaunchOptions {
    pub agent: String,
    pub workspace: PathBuf,
    pub profile: Option<String>,
    pub rootfs: Option<PathBuf>,
    pub daemon: String,
    pub arguments: Vec<String>,
}

pub fn launch(options: LaunchOptions) -> Result<i32> {
    if options.profile.is_some() && options.rootfs.is_some() {
        bail!("--profile and --root are mutually exclusive");
    }
    if !matches!(options.agent.as_str(), "opencode" | "pi") {
        bail!("unsupported tool adapter {:?}", options.agent);
    }
    let workspace = fs::canonicalize(&options.workspace).context("invalid workspace")?;
    let tinybox = std::env::current_exe().context("locate tinybox executable")?;
    let adapter_root = verified_user_adapter(&options.agent)?;
    let handlers = SignalHandlers::install()?;
    let mut create = Command::new(&tinybox);
    create
        .args(["agent", "run"])
        .arg(&workspace)
        .args(["--daemon", &options.daemon])
        .arg("--detach");
    if let Some(profile) = &options.profile {
        create.args(["--profile", profile]);
    }
    if let Some(rootfs) = &options.rootfs {
        create.arg("--root").arg(rootfs);
    }
    let output = create.output().context("create tinybox Agent task")?;
    if !output.status.success() {
        bail!(
            "create tinybox Agent task failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let task = String::from_utf8(output.stdout)
        .context("task id was not UTF-8")?
        .trim()
        .to_owned();
    if !task.starts_with("task-") {
        bail!("invalid task id returned by tinybox: {task:?}");
    }

    let mut agent = Command::new(&options.agent);
    if options.agent == "pi" {
        agent.arg("--extension").arg(adapter_root.join("index.ts"));
    }
    agent.args(&options.arguments);
    if options.agent == "opencode" {
        agent.env("OPENCODE_CONFIG_DIR", &adapter_root);
    }
    agent
        .current_dir(&workspace)
        .env("TINYBOX_TASK_ID", &task)
        .env("TINYBOX_BIN", &tinybox)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    reset_child_signal_handlers(&mut agent);
    let status = agent.spawn().and_then(|mut child| {
        AGENT_PID.store(child.id() as i32, Ordering::SeqCst);
        let result = child.wait();
        AGENT_PID.store(0, Ordering::SeqCst);
        result
    });
    drop(handlers);
    let cleanup = Command::new(&tinybox)
        .args(["agent", "destroy", &task])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let status = status.with_context(|| format!("launch {}", options.agent))?;
    if !cleanup.map(|value| value.success()).unwrap_or(false) {
        bail!(
            "{} exited, but tinybox task {task} cleanup failed",
            options.agent
        );
    }
    Ok(exit_code(status))
}

extern "C" fn forward_signal(signal: libc::c_int) {
    let pid = AGENT_PID.load(Ordering::Relaxed);
    if pid > 0 {
        // SAFETY: kill is async-signal-safe, and pid is the live child PID published atomically.
        unsafe { libc::kill(pid, signal) };
    }
}

struct SignalHandlers {
    old_interrupt: libc::sigaction,
    old_terminate: libc::sigaction,
}

impl SignalHandlers {
    fn install() -> Result<Self> {
        // SAFETY: sigaction structures are fully initialized before use; the handler only calls
        // async-signal-safe kill and reads a lock-free atomic.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = forward_signal as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            let mut old_interrupt = std::mem::zeroed();
            let mut old_terminate = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &action, &mut old_interrupt) != 0 {
                return Err(std::io::Error::last_os_error()).context("install SIGINT handler");
            }
            if libc::sigaction(libc::SIGTERM, &action, &mut old_terminate) != 0 {
                libc::sigaction(libc::SIGINT, &old_interrupt, std::ptr::null_mut());
                return Err(std::io::Error::last_os_error()).context("install SIGTERM handler");
            }
            Ok(Self {
                old_interrupt,
                old_terminate,
            })
        }
    }
}

impl Drop for SignalHandlers {
    fn drop(&mut self) {
        // SAFETY: these are the exact prior handlers returned by successful sigaction calls.
        unsafe {
            libc::sigaction(libc::SIGINT, &self.old_interrupt, std::ptr::null_mut());
            libc::sigaction(libc::SIGTERM, &self.old_terminate, std::ptr::null_mut());
        }
    }
}

fn reset_child_signal_handlers(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: pre_exec only invokes async-signal-safe signal(), restoring defaults before exec.
    unsafe {
        command.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            Ok(())
        });
    }
}

fn verified_user_adapter(agent: &str) -> Result<PathBuf> {
    let (directory, entry, expected) = match agent {
        "opencode" => (
            config_home()?.join("opencode"),
            "tools/bash.ts",
            OPENCODE_TOOL,
        ),
        "pi" => (
            home_dir()?.join(".pi/agent/extensions/tinybox"),
            "index.ts",
            PI_EXTENSION,
        ),
        _ => bail!("unsupported tool adapter {agent:?}"),
    };
    let entry_path = directory.join(entry);
    let runtime_path = entry_path.parent().unwrap_or(&directory).join("runtime.js");
    let entry_ok = fs::read_to_string(&entry_path)
        .map(|content| content == expected)
        .unwrap_or(false);
    let runtime_ok = fs::read_to_string(&runtime_path)
        .map(|content| content == SHARED_RUNTIME)
        .unwrap_or(false);
    if !entry_ok || !runtime_ok {
        bail!("missing or modified {agent} adapter; run `tinybox agent integrate {agent}`");
    }
    Ok(if agent == "opencode" {
        directory
    } else {
        entry_path.parent().unwrap().to_path_buf()
    })
}

#[cfg(unix)]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

fn install_file(path: &Path, content: &str, force: bool) -> Result<()> {
    if path.exists() {
        let current = fs::read_to_string(path)
            .with_context(|| format!("read existing adapter {}", path.display()))?;
        if current == content {
            return Ok(());
        }
        if !force {
            bail!(
                "refusing to overwrite {}; rerun with --force after reviewing it",
                path.display()
            );
        }
    }
    fs::write(path, content).with_context(|| format!("write adapter {}", path.display()))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set; cannot locate the user-level adapter directory")
}

fn config_home() -> Result<PathBuf> {
    Ok(std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".config")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_agent() {
        assert!(install("unknown", Some(Path::new("/tmp")), false).is_err());
    }

    #[test]
    fn installs_project_adapters_without_overwriting_conflicts() {
        let root = tempfile::tempdir().unwrap();
        let installed = install("opencode", Some(root.path()), false).unwrap();
        assert!(installed.join("bash.ts").exists());
        assert!(installed.join("runtime.js").exists());
        fs::write(installed.join("bash.ts"), "user tool").unwrap();
        assert!(install("opencode", Some(root.path()), false).is_err());
        install("opencode", Some(root.path()), true).unwrap();
        assert_eq!(
            fs::read_to_string(installed.join("bash.ts")).unwrap(),
            OPENCODE_TOOL
        );
    }

    #[test]
    fn installs_pi_as_a_same_name_bash_override() {
        let root = tempfile::tempdir().unwrap();
        let installed = install("pi", Some(root.path()), false).unwrap();
        let extension = fs::read_to_string(installed.join("index.ts")).unwrap();
        assert!(extension.contains("name: \"bash\""));
        assert!(installed.join("runtime.js").exists());
    }
}
