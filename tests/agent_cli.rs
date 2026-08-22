use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

const ADDRESS: &str = "127.0.0.1:18084";

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn daemon() -> Daemon {
    let daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_tinybox"))
            .args(["daemon", "--listen", ADDRESS])
            .spawn()
            .unwrap(),
    );
    for _ in 0..100 {
        if TcpStream::connect(ADDRESS).is_ok() {
            return daemon;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon did not become ready");
}

fn tinybox(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tinybox"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn agent_cli_foreground_and_detached_lifecycle() {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("SKIP: Agent CLI acceptance requires root");
        return;
    }
    let fixture = tempfile::Builder::new()
        .prefix("tinybox-agent-cli.")
        .tempdir_in("/var/tmp")
        .unwrap();
    let workspace = fixture.path().to_str().unwrap();
    let _daemon = daemon();

    let foreground = tinybox(&[
        "agent",
        "run",
        workspace,
        "--daemon",
        ADDRESS,
        "--",
        "/bin/sh",
        "-c",
        "printf agent-ok",
    ]);
    assert!(
        foreground.status.success(),
        "{}",
        String::from_utf8_lossy(&foreground.stderr)
    );
    assert_eq!(foreground.stdout, b"agent-ok");

    let detached = tinybox(&["agent", "run", workspace, "--daemon", ADDRESS, "--detach"]);
    assert!(
        detached.status.success(),
        "{}",
        String::from_utf8_lossy(&detached.stderr)
    );
    let id = String::from_utf8(detached.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert!(id.starts_with("task-"));
    assert!(std::path::Path::new(&format!("/run/tinybox/agents/{id}.json")).exists());
    assert_eq!(
        std::fs::metadata(format!("/run/tinybox/agents/{id}.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let listing = tinybox(&["agent", "list"]);
    assert!(listing.status.success());
    assert!(String::from_utf8_lossy(&listing.stdout).contains(&id));

    let first = tinybox(&[
        "agent",
        "exec",
        &id,
        "--",
        "/bin/sh",
        "-c",
        "printf persisted > cli-state",
    ]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = tinybox(&[
        "agent",
        "exec",
        &id,
        "--",
        "/bin/cat",
        "/workspace/cli-state",
    ]);
    assert!(second.status.success());
    assert_eq!(second.stdout, b"persisted");

    let stopped = tinybox(&["agent", "stop", &id]);
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert!(!std::path::Path::new(&format!("/sys/fs/cgroup/tinybox-{id}")).exists());
    assert!(std::path::Path::new(&format!("/var/lib/tinybox/tasks/{id}")).exists());
    let after_stop = tinybox(&["agent", "exec", &id, "--", "/bin/true"]);
    assert!(!after_stop.status.success());

    let destroyed = tinybox(&["agent", "destroy", &id]);
    assert!(
        destroyed.status.success(),
        "{}",
        String::from_utf8_lossy(&destroyed.stderr)
    );
    assert!(!std::path::Path::new(&format!("/var/lib/tinybox/tasks/{id}")).exists());
    assert!(!std::path::Path::new(&format!("/run/tinybox/agents/{id}.json")).exists());

    let marker = fixture.path().join("host-fallback-marker");
    let failed = tinybox(&[
        "agent",
        "run",
        workspace,
        "--daemon",
        "127.0.0.1:1",
        "--",
        "/usr/bin/touch",
        marker.to_str().unwrap(),
    ]);
    assert!(!failed.status.success());
    assert!(
        !marker.exists(),
        "Agent CLI silently fell back to host execution"
    );
}
