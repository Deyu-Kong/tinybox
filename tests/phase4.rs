use std::process::Command;

fn tinybox_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|v| v.contains("WSL") || v.contains("Microsoft"))
        .unwrap_or(false)
}

#[test]
fn test_mem_limit_oom() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    if is_wsl() {
        eprintln!("skipping: WSL2 cgroup memory limit not functional");
        return;
    }

    let output = tinybox_bin()
        .args([
            "run",
            "--mem-limit",
            "64M",
            "--",
            "sh",
            "-c",
            "dd if=/dev/zero of=/dev/null bs=1M count=200",
        ])
        .output()
        .expect("failed to execute tinybox");

    assert!(
        !output.status.success(),
        "expected OOM kill, but command succeeded"
    );

    let code = output.status.code().unwrap_or(0);
    assert!(
        code == 137 || code == 1,
        "expected exit code 137 (OOM) or 1, got {}",
        code
    );
}

#[test]
fn test_mem_limit_normal() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--mem-limit", "256M", "--", "echo", "hello"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}

#[test]
fn test_mem_limit_invalid() {
    let output = tinybox_bin()
        .args(["run", "--mem-limit", "invalid", "--", "echo", "test"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());
}
