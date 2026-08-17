use std::{
    io::{Read, Write},
    net::TcpStream,
    process::{Child, Command},
    thread,
    time::Duration,
};

fn request(method: &str, path: &str, body: Option<&str>) -> String {
    let mut stream = TcpStream::connect("127.0.0.1:18082").unwrap();
    let body = body.unwrap_or("");
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

/// Poll a sandbox until it leaves the "running" status (spawn_blocking may not
/// have finished yet). Returns the final status line chunk.
fn wait_for_terminal(id: &str) -> String {
    for _ in 0..40 {
        let resp = request("GET", &format!("/api/sandboxes/{id}"), None);
        if !resp.contains("\"running\"") {
            return resp;
        }
        thread::sleep(Duration::from_millis(50));
    }
    request("GET", &format!("/api/sandboxes/{id}"), None)
}

fn daemon() -> Child {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
        .args(["daemon", "--listen", "127.0.0.1:18082"])
        .spawn()
        .unwrap()
}

#[test]
fn daemon_serves_lifecycle_and_prometheus_metrics() {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let mut child = daemon();
    let mut ready = false;
    for _ in 0..20 {
        if TcpStream::connect("127.0.0.1:18082").is_ok() {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(ready);
    let created = request(
        "POST",
        "/api/sandboxes",
        Some(r#"{"rootfs":"/","command":["/bin/sh","-c","sleep 5"]}"#),
    );
    assert!(created.contains("sb-1"));
    let metrics = request("GET", "/metrics", None);
    assert!(metrics.contains("tinybox_sandboxes_total"));
    let listed = request("GET", "/api/sandboxes", None);
    assert!(listed.contains("running"));

    // P1-3: a failing sandbox must land in "failed" status and surface in
    // /metrics as tinybox_sandboxes_failed (previously miscounted as
    // completed).
    let failed = request(
        "POST",
        "/api/sandboxes",
        Some(r#"{"rootfs":"/nonexistent/rootfs","command":["/bin/sh","-c","true"]}"#),
    );
    assert!(failed.contains("sb-2"), "got: {failed}");
    let sb2 = wait_for_terminal("sb-2");
    assert!(sb2.contains("failed"), "expected failed status, got: {sb2}");
    let metrics_after = request("GET", "/metrics", None);
    assert!(
        metrics_after.contains("tinybox_sandboxes_failed")
            && !metrics_after.contains("tinybox_sandboxes_failed 0"),
        "expected non-zero failed counter: {metrics_after}"
    );

    // P1-4: disabling seccomp/caps remotely must be rejected.
    let danger = request(
        "POST",
        "/api/sandboxes",
        Some(r#"{"rootfs":"/","command":["echo"],"dangerous":true}"#),
    );
    assert!(
        danger.contains("400") || danger.to_lowercase().contains("bad request"),
        "dangerous:true should be rejected: {danger}"
    );

    let _ = request("DELETE", "/api/sandboxes/sb-1", None);
    let _ = child.kill();
    let _ = child.wait();
}
