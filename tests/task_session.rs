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
        "env": ["PATH=/usr/bin:/bin", "LANG=C"],
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

    let (_, body) = request(
        "POST",
        &exec_path,
        Some(token),
        Some(&json!({
            "command":["/bin/sh","-c","printf private > \"$HOME/.cache/marker\""],
            "timeout_ms":5000
        })),
    );
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["exit_code"],
        0
    );
    let (_, body) = request(
        "POST",
        &exec_path,
        Some(token),
        Some(&json!({"command":["/bin/cat","/home/agent/.cache/marker"],"timeout_ms":5000})),
    );
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["stdout"],
        "private"
    );
    let manifest_path = format!("/var/lib/tinybox/tasks/{id}/environment.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["source"], "host");
    assert!(manifest["mappings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|mapping| {
            mapping["target"] == "/home/agent" && mapping["mode"] == "private_write"
        }));

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
    assert!(!std::path::Path::new(&format!("/var/lib/tinybox/tasks/{id}")).exists());

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
    wait_gone(std::path::Path::new(&format!(
        "/var/lib/tinybox/tasks/{crashed_id}"
    )));

    let mut invalid_profile = create.clone();
    invalid_profile["environment"] = json!({"source":"profile","name":"missing"});
    let state_entries_before = std::fs::read_dir("/var/lib/tinybox/tasks")
        .map(|entries| entries.count())
        .unwrap_or(0);
    let (status, _) = request("POST", "/api/tasks", None, Some(&invalid_profile));
    assert_eq!(status, 400);
    let state_entries_after = std::fs::read_dir("/var/lib/tinybox/tasks")
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(state_entries_after, state_entries_before);

    std::fs::write(
        workspace.join("smoke.rs"),
        "fn main(){println!(\"rust-ok\");}",
    )
    .unwrap();
    std::fs::write(workspace.join("smoke.js"), "console.log('node-ok')").unwrap();
    std::fs::write(workspace.join("smoke.py"), "print('python-ok')").unwrap();
    let profile_cases = [
        (
            "rust",
            vec!["/bin/sh", "-c", "rustc /workspace/smoke.rs -o /home/agent/.cache/smoke-rust && /home/agent/.cache/smoke-rust"],
            "rust-ok",
        ),
        ("node", vec!["node", "/workspace/smoke.js"], "node-ok"),
        ("python", vec!["python3", "/workspace/smoke.py"], "python-ok"),
    ];
    for (profile, command, expected) in profile_cases {
        let mut profile_create = create.clone();
        profile_create["environment"] = json!({"source":"profile","name":profile});
        let (status, body) = request("POST", "/api/tasks", None, Some(&profile_create));
        assert_eq!(status, 202, "{profile} profile create failed: {body}");
        let task: Value = serde_json::from_str(&body).unwrap();
        let profile_id = task["id"].as_str().unwrap();
        let profile_token = task["token"].as_str().unwrap();
        for _ in 0..100 {
            let (_, body) = request("GET", &format!("/api/tasks/{profile_id}"), None, None);
            if body.contains("\"running\"") {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let (_, body) = request(
            "POST",
            &format!("/api/tasks/{profile_id}/exec"),
            Some(profile_token),
            Some(&json!({"command":command,"timeout_ms":30000})),
        );
        let result: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(result["exit_code"], 0, "{profile} smoke failed: {body}");
        assert!(result["stdout"].as_str().unwrap().contains(expected));
        let (_, body) = request(
            "POST",
            &format!("/api/tasks/{profile_id}/exec"),
            Some(profile_token),
            Some(
                &json!({"command":["/bin/sh","-c","printf reused > \"$XDG_CACHE_HOME/profile-marker\""],"timeout_ms":5000}),
            ),
        );
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap()["exit_code"],
            0
        );
        let (_, body) = request(
            "POST",
            &format!("/api/tasks/{profile_id}/exec"),
            Some(profile_token),
            Some(
                &json!({"command":["/bin/cat","/home/agent/.cache/profile-marker"],"timeout_ms":5000}),
            ),
        );
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap()["stdout"],
            "reused"
        );

        let manifest: Value = serde_json::from_slice(
            &std::fs::read(format!(
                "/var/lib/tinybox/tasks/{profile_id}/environment.json"
            ))
            .unwrap(),
        )
        .unwrap();
        if let Some(mapping) = manifest["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|mapping| mapping["mode"] == "read_only")
        {
            let target = mapping["target"].as_str().unwrap();
            let probe = format!("{target}/.tinybox-write-probe");
            let (_, body) = request(
                "POST",
                &format!("/api/tasks/{profile_id}/exec"),
                Some(profile_token),
                Some(&json!({"command":["/usr/bin/touch",probe],"timeout_ms":5000})),
            );
            assert_ne!(
                serde_json::from_str::<Value>(&body).unwrap()["exit_code"],
                0
            );
        }
        let (status, body) = request(
            "DELETE",
            &format!("/api/tasks/{profile_id}"),
            Some(profile_token),
            None,
        );
        assert_eq!(status, 204, "{profile} destroy failed: {body}");
    }

    let mut rootfs_create = create.clone();
    rootfs_create["environment"] = json!({"source":"rootfs","path":"/"});
    let (status, body) = request("POST", "/api/tasks", None, Some(&rootfs_create));
    assert_eq!(status, 202, "rootfs environment create failed: {body}");
    let rootfs_task: Value = serde_json::from_str(&body).unwrap();
    let rootfs_id = rootfs_task["id"].as_str().unwrap();
    let rootfs_token = rootfs_task["token"].as_str().unwrap();
    for _ in 0..100 {
        let (_, body) = request("GET", &format!("/api/tasks/{rootfs_id}"), None, None);
        if body.contains("\"running\"") {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(format!(
            "/var/lib/tinybox/tasks/{rootfs_id}/environment.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["source"], "rootfs");
    let (status, _) = request(
        "DELETE",
        &format!("/api/tasks/{rootfs_id}"),
        Some(rootfs_token),
        None,
    );
    assert_eq!(status, 204);
    let _ = replacement.kill();
    let _ = replacement.wait();
}
