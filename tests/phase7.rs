use std::process::Command;

fn tinybox() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
}

#[test]
fn test_network_namespace_has_no_default_route() {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let output = tinybox()
        .args(["run", "--", "sh", "-c", "test ! -s /proc/net/route"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_proxy_environment_is_injected() {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let output = tinybox()
        .args([
            "run",
            "--proxy",
            "http://127.0.0.1:8080",
            "--",
            "sh",
            "-c",
            "test \"$HTTP_PROXY\" = http://127.0.0.1:8080 && test \"$HTTPS_PROXY\" = http://127.0.0.1:8080",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_proxy_mode_is_network_isolated() {
    // P0-2 regression: --proxy must unshare CLONE_NEWNET, so the sandbox has
    // no default route even when a proxy is configured. Previously --proxy
    // only set env vars and shared the host netns.
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let output = tinybox()
        .args([
            "run",
            "--proxy",
            "http://127.0.0.1:8080",
            "--",
            "sh",
            "-c",
            "test ! -s /proc/net/route",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "--proxy must isolate the netns (no default route): {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
