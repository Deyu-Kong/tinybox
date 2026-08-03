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
    let _ = request("DELETE", "/api/sandboxes/sb-1", None);
    let _ = child.kill();
    let _ = child.wait();
}
