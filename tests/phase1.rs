use std::process::Command;

fn tinybox_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
}

#[test]
fn test_echo_hello() {
    let output = tinybox_bin()
        .args(["run", "--", "echo", "hello"])
        .output()
        .expect("failed to execute tinybox");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello"));
}

#[test]
fn test_exit_code_42() {
    let output = tinybox_bin()
        .args(["run", "--", "sh", "-c", "exit 42"])
        .output()
        .expect("failed to execute tinybox");

    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn test_true_exits_zero() {
    let output = tinybox_bin()
        .args(["run", "--", "true"])
        .output()
        .expect("failed to execute tinybox");

    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_false_exits_one() {
    let output = tinybox_bin()
        .args(["run", "--", "false"])
        .output()
        .expect("failed to execute tinybox");

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn test_no_command_errors() {
    let output = tinybox_bin()
        .args(["run"])
        .output()
        .expect("failed to execute tinybox");

    assert!(!output.status.success());
}
