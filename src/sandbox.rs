use anyhow::{bail, Result};
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{execvp, fork, ForkResult};
use std::ffi::CString;

pub struct SandboxConfig {
    pub command: Vec<String>,
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

    // SAFETY: fork() is safe here as we immediately execvp in the child.
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            let status = waitpid(child, None)?;
            Ok(exit_code_from_status(status))
        }
        ForkResult::Child => {
            execvp(&program, &args)?;
            unreachable!()
        }
    }
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
    fn test_run_true() {
        let config = SandboxConfig {
            command: vec!["true".to_string()],
        };
        let code = run_sandbox(&config).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_false() {
        let config = SandboxConfig {
            command: vec!["false".to_string()],
        };
        let code = run_sandbox(&config).unwrap();
        assert_eq!(code, 1);
    }

    #[test]
    fn test_run_echo() {
        let config = SandboxConfig {
            command: vec!["echo".to_string(), "hello".to_string()],
        };
        let code = run_sandbox(&config).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_exit_code() {
        let config = SandboxConfig {
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "exit 42".to_string(),
            ],
        };
        let code = run_sandbox(&config).unwrap();
        assert_eq!(code, 42);
    }

    #[test]
    fn test_empty_command() {
        let config = SandboxConfig {
            command: vec![],
        };
        assert!(run_sandbox(&config).is_err());
    }
}
