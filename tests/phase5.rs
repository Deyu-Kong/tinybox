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
        .args([
            "run",
            "--dangerous",
            "--",
            "mount",
            "-t",
            "tmpfs",
            "none",
            "/tmp",
        ])
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

#[test]
fn test_capabilities_dropped() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--", "cat", "/proc/self/status"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let cap_eff = stdout
        .lines()
        .find(|line| line.starts_with("CapEff:"))
        .expect("CapEff not found in /proc/self/status");

    let cap_value = u64::from_str_radix(cap_eff.split_whitespace().nth(1).unwrap(), 16)
        .expect("failed to parse CapEff");

    let cap_sys_admin = 1u64 << 21;
    assert_eq!(
        cap_value & cap_sys_admin,
        0,
        "CAP_SYS_ADMIN should be dropped, but CapEff is {:016x}",
        cap_value
    );

    let cap_net_admin = 1u64 << 12;
    assert_eq!(
        cap_value & cap_net_admin,
        0,
        "CAP_NET_ADMIN should be dropped, but CapEff is {:016x}",
        cap_value
    );

    // P0-4 regression: the capability bounding set must also be cleared for
    // the dangerous caps, so a setuid binary exec'd in the sandbox cannot
    // re-acquire them on execve(2).
    let cap_bnd = stdout
        .lines()
        .find(|line| line.starts_with("CapBnd:"))
        .expect("CapBnd not found in /proc/self/status");
    let bnd_value = u64::from_str_radix(cap_bnd.split_whitespace().nth(1).unwrap(), 16)
        .expect("failed to parse CapBnd");
    assert_eq!(
        bnd_value & cap_sys_admin,
        0,
        "CAP_SYS_ADMIN should be absent from the bounding set, but CapBnd is {:016x}",
        bnd_value
    );
    assert_eq!(
        bnd_value & cap_net_admin,
        0,
        "CAP_NET_ADMIN should be absent from the bounding set, but CapBnd is {:016x}",
        bnd_value
    );
}

#[test]
fn test_dangerous_keeps_capabilities() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }

    let output = tinybox_bin()
        .args(["run", "--dangerous", "--", "cat", "/proc/self/status"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let cap_eff = stdout
        .lines()
        .find(|line| line.starts_with("CapEff:"))
        .expect("CapEff not found in /proc/self/status");

    let cap_value = u64::from_str_radix(cap_eff.split_whitespace().nth(1).unwrap(), 16)
        .expect("failed to parse CapEff");

    let cap_sys_admin = 1u64 << 21;
    assert_ne!(
        cap_value & cap_sys_admin,
        0,
        "CAP_SYS_ADMIN should be present with --dangerous, but CapEff is {:016x}",
        cap_value
    );
}
