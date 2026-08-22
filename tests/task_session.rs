use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::symlink;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

const ADDRESS: &str = "127.0.0.1:18083";

fn request(method: &str, path: &str, token: Option<&str>, body: Option<&Value>) -> (u16, String) {
    let mut stream = TcpStream::connect(ADDRESS).unwrap();
    let body = body.map(Value::to_string).unwrap_or_default();
    let token = token
        .map(|value| format!("X-Tinybox-Task-Token: {value}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n{token}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

fn daemon() -> Child {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
        .args(["daemon", "--listen", ADDRESS])
        .spawn()
        .unwrap()
}

fn wait_ready() {
    for _ in 0..50 {
        if TcpStream::connect(ADDRESS).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("daemon did not become ready");
}

fn wait_gone(path: &std::path::Path) {
    for _ in 0..100 {
        if !path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("path remained after cleanup: {}", path.display());
}

#[test]
fn persistent_task_exec_is_stateful_and_policy_enforced() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("SKIP: task session acceptance requires root");
        return;
    }
    let fixture = tempfile::Builder::new()
        .prefix("tinybox-task-test.")
        .tempdir_in("/var/tmp")
        .unwrap();
    let workspace = fixture.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let secret = fixture.path().join("synthetic-secret");
    std::fs::write(&secret, "TASK-CANARY").unwrap();
    symlink(&secret, workspace.join("escape")).unwrap();

    let mut daemon_process = daemon();
    wait_ready();
    let create = json!({
        "workspace": workspace,
        "env": ["PATH=/usr/bin:/bin", "HOME=/tmp", "LANG=C"],
        "policy": {
            "version": 1,
            "filesystem": [{"path":"/workspace", "access":"read_write"}],
            "network": [],
            "resources": {"memory_bytes":268435456, "cpus":1.0, "pids":32},
            "phases": []
        }
    });
    let (status, body) = request("POST", "/api/tasks", None, Some(&create));
    assert_eq!(status, 202, "create failed: {body}");
    let created: Value = serde_json::from_str(&body).unwrap();
    let id = created["id"].as_str().unwrap();
    let token = created["token"].as_str().unwrap();

    let (status, _) = request("DELETE", &format!("/api/sandboxes/{id}"), None, None);
    assert_eq!(status, 403, "generic sandbox DELETE bypassed task token");

    for _ in 0..100 {
        let (_, body) = request("GET", &format!("/api/tasks/{id}"), None, None);
        if body.contains("\"running\"") {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let exec_path = format!("/api/tasks/{id}/exec");
    let (status, body) = request(
        "POST",
        &exec_path,
        Some(token),
        Some(&json!({
            "command":["/bin/sh","-c","printf persisted > state; cat /proc/self/cgroup"],
            "cwd":"/workspace",
            "timeout_ms":5000
        })),
    );
    assert_eq!(status, 200, "first exec failed: {body}");
    let first: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(first["exit_code"], 0);
    assert!(first["stdout"].as_str().unwrap().contains("/exec-"));

    let (_, body) = request(
        "POST",
        &exec_path,
        Some(token),
        Some(&json!({
            "command":["/bin/cat","/workspace/state"],
            "cwd":"/workspace",
            "timeout_ms":5000
        })),
    );
    let second: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(second["exit_code"], 0);
    assert_eq!(second["stdout"], "persisted");

    for denied_path in [
        secret.to_string_lossy().into_owned(),
        "/workspace/escape".into(),
    ] {
        let (_, body) = request(
            "POST",
            &exec_path,
            Some(token),
            Some(&json!({
                "command":["/bin/cat",denied_path],
                "cwd":"/workspace",
                "timeout_ms":5000
            })),
        );
        let denied: Value = serde_json::from_str(&body).unwrap();
        assert_ne!(denied["exit_code"], 0, "secret access unexpectedly worked");
        assert!(!denied["stdout"].as_str().unwrap().contains("TASK-CANARY"));
    }

    let (status, _) = request(
        "POST",
        &exec_path,
        Some("wrong-token"),
        Some(&json!({"command":["/bin/true"],"timeout_ms":5000})),
    );
    assert_eq!(status, 401);

    let (_, body) = request(
        "POST",
        &exec_path,
        Some(token),
        Some(&json!({"command":["/bin/sh","-c","sleep 10"],"timeout_ms":100})),
    );
    let timeout: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(timeout["exit_code"], 124);
    assert_eq!(timeout["timed_out"], true);

    let (_, body) = request(
        "POST",
        &exec_path,
        Some(token),
        Some(&json!({
            "command":["/bin/sh","-c","sleep 30 & printf background-started"],
            "timeout_ms":5000
        })),
    );
    let background: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(background["exit_code"], 0);
    assert_eq!(background["stdout"], "background-started");
    let task_cgroup = format!("/sys/fs/cgroup/tinybox-{id}");
    assert!(
        std::fs::read_dir(&task_cgroup)
            .unwrap()
            .all(|entry| !entry.unwrap().path().is_dir()),
        "per-exec cgroup remained after command completion"
    );

    let (status, _) = request("DELETE", &format!("/api/tasks/{id}"), Some(token), None);
    assert_eq!(status, 204);
    wait_gone(std::path::Path::new(&format!(
        "/sys/fs/cgroup/tinybox-{id}"
    )));

    let (status, body) = request("POST", "/api/tasks", None, Some(&create));
    assert_eq!(status, 202, "crash-recovery task create failed: {body}");
    let crashed: Value = serde_json::from_str(&body).unwrap();
    let crashed_id = crashed["id"].as_str().unwrap();
    let crashed_cgroup = format!("/sys/fs/cgroup/tinybox-{crashed_id}");
    for _ in 0..100 {
        let (_, body) = request("GET", &format!("/api/tasks/{crashed_id}"), None, None);
        if body.contains("\"running\"") {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let _ = daemon_process.kill();
    let _ = daemon_process.wait();
    for _ in 0..100 {
        let populated = std::fs::read_to_string(format!("{crashed_cgroup}/cgroup.events"))
            .unwrap_or_default()
            .contains("populated 1");
        if !populated {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let mut replacement = daemon();
    wait_ready();
    wait_gone(std::path::Path::new(&crashed_cgroup));
    let _ = replacement.kill();
    let _ = replacement.wait();
}
