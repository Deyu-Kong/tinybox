use std::fs;
use std::process::Command;

#[test]
fn test_oci_bundle_executes_process_and_env() {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let bundle = tempfile::tempdir().unwrap();
    let rootfs = bundle.path().join("rootfs");
    fs::create_dir(&rootfs).unwrap();
    let config = r#"{"process":{"args":["sh","-c","test \"$TINYBOX_PHASE6\" = ok"],"env":["PATH=/usr/bin:/bin","TINYBOX_PHASE6=ok"]},"root":{"path":"rootfs","readonly":true},"linux":{"namespaces":[{"type":"pid"},{"type":"mount"}]}}"#;
    fs::write(bundle.path().join("config.json"), config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tinybox"))
        .args(["run", "--oci", bundle.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_oci_bundle_requires_process_and_root() {
    let bundle = tempfile::tempdir().unwrap();
    fs::write(bundle.path().join("config.json"), "{}").unwrap();
    let output = tinybox_oci(bundle.path()).unwrap();
    assert!(!output.status.success());
}

fn tinybox_oci(bundle: &std::path::Path) -> std::io::Result<std::process::Output> {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
        .args(["run", "--oci", bundle.to_str().unwrap()])
        .output()
}
