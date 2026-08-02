use std::process::Command;

fn tinybox_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn test_memory_oom() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args([
            "run",
            "--memory",
            "64m",
            "--",
            "python3",
            "-c",
            "a = bytearray(200*1024*1024)",
        ])
        .output()
        .expect("failed to execute tinybox");

    assert!(
        !output.status.success(),
        "expected OOM kill, but command succeeded. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let code = output.status.code().unwrap_or(0);
    assert!(
        code == 137 || code == 1,
        "expected exit code 137 (OOM) or 1, got {}",
        code
    );
}

#[test]
fn test_memory_short_flag() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "-m", "64m", "--", "echo", "hello"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}

#[test]
fn test_memory_normal() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--memory", "256m", "--", "echo", "hello"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}

#[test]
fn test_memory_invalid() {
    let output = tinybox_bin()
        .args(["run", "--memory", "invalid", "--", "echo", "test"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());
}

#[test]
fn test_cpus_normal() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--cpus", "0.5", "--", "echo", "hello"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}

#[test]
fn test_cpus_invalid() {
    let output = tinybox_bin()
        .args(["run", "--cpus", "0", "--", "echo", "test"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());

    let output = tinybox_bin()
        .args(["run", "--cpus", "-1", "--", "echo", "test"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());
}

#[test]
fn test_cpu_quota() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args([
            "run",
            "--cpu-quota",
            "50000",
            "--cpu-period",
            "100000",
            "--",
            "echo",
            "hello",
        ])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}

#[test]
fn test_cpu_quota_invalid() {
    let output = tinybox_bin()
        .args(["run", "--cpu-quota", "0", "--", "echo", "test"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());
}

#[test]
fn test_pids_limit() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--pids-limit", "10", "--", "echo", "hello"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
}
