use std::process::Command;

fn tinybox_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn test_pid_namespace_ps() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }
    let output = tinybox_bin()
        .args(["run", "--", "ps", "aux"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.len() <= 5, "expected few processes, got:\n{}", stdout);
}

#[test]
fn test_pid_namespace_id() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }
    let output = tinybox_bin()
        .args(["run", "--", "id"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("uid=0"), "expected uid=0, got: {}", stdout);
}

#[test]
fn test_uts_namespace_hostname() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }
    let output = tinybox_bin()
        .args(["run", "--hostname", "sbox1", "--", "hostname"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim() == "sbox1", "expected 'sbox1', got: {}", stdout);
}

#[test]
fn test_exit_code_passthrough() {
    if !is_root() {
        eprintln!("skipping: requires root");
        return;
    }
    let output = tinybox_bin()
        .args(["run", "--", "sh", "-c", "exit 42"])
        .output()
        .expect("failed to execute tinybox");

    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn test_echo_hello() {
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
