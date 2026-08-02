use std::process::Command;

fn tinybox_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn test_seccomp_blocks_reboot() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--", "reboot"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());
    let code = output.status.code().unwrap_or(0);
    assert!(
        code == 159 || code == 1,
        "expected exit code 159 (SIGSYS) or 1, got {}",
        code
    );
}

#[test]
fn test_seccomp_blocks_mount() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--", "mount", "-t", "tmpfs", "none", "/tmp"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());
}

#[test]
fn test_dangerous_allows_mount() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--dangerous", "--", "mount", "-t", "tmpfs", "none", "/tmp"])
        .output()
        .expect("failed to execute tinybox");

    assert!(
        output.status.success(),
        "expected success with --dangerous, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_seccomp_allows_echo() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--", "echo", "hello"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}
