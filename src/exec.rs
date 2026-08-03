use anyhow::{Context, Result};
use std::process::Command;

pub fn exec_in_container(pid: u32, command: &[String]) -> Result<i32> {
    if command.is_empty() {
        anyhow::bail!("no command specified");
    }

    let mut cmd = Command::new("nsenter");
    cmd.arg("-t").arg(pid.to_string())
        .arg("-m")  // mount namespace
        .arg("-u")  // UTS namespace
        .arg("-n")  // network namespace
        .arg("-p")  // PID namespace
        .arg("--")
        .args(command);

    let status = cmd
        .status()
        .context("failed to execute nsenter")?;

    Ok(status.code().unwrap_or(1))
}
