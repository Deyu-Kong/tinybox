use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Session {
    id: String,
    token: String,
    daemon: String,
    workspace: PathBuf,
}

pub struct RunOptions {
    pub daemon: String,
    pub workspace: PathBuf,
    pub profile: Option<String>,
    pub rootfs: Option<PathBuf>,
    pub detach: bool,
    pub command: Vec<String>,
}

pub fn run(options: RunOptions) -> Result<i32> {
    if options.profile.is_some() && options.rootfs.is_some() {
        bail!("--profile and --root are mutually exclusive");
    }
    if options.detach && !options.command.is_empty() {
        bail!("detached task creation does not accept a command; use `agent exec`");
    }
    if !options.detach && options.command.is_empty() {
        bail!("foreground agent run requires a command after --");
    }
    let workspace = fs::canonicalize(&options.workspace).context("invalid workspace")?;
    let environment = if let Some(profile) = options.profile {
        json!({"source":"profile","name":profile})
    } else if let Some(path) = options.rootfs {
        json!({"source":"rootfs","path":path})
    } else {
        json!({"source":"host"})
    };
    let body = json!({
        "workspace": workspace,
        "environment": environment,
        "policy": default_policy(),
        "env": ["TERM=xterm-256color"]
    });
    let client = client()?;
    let created = checked(
        client
            .post(url(&options.daemon, "/api/tasks"))
            .json(&body)
            .send(),
    )?;
    let value: Value = created.json()?;
    let session = Session {
        id: required_string(&value, "id")?,
        token: required_string(&value, "token")?,
        daemon: options.daemon,
        workspace,
    };
    save_session(&session)?;
    wait_running(&client, &session)?;
    if options.detach {
        println!("{}", session.id);
        return Ok(0);
    }
    let result = exec_session(&client, &session, options.command, "/workspace", 3_600_000);
    let destroy_result = destroy_session(&client, &session);
    remove_session(&session.id)?;
    let response = result?;
    destroy_result?;
    print_exec_output(&response)?;
    Ok(response["exit_code"].as_i64().unwrap_or(125) as i32)
}

pub fn exec(id: &str, command: Vec<String>, cwd: String, timeout_ms: u64) -> Result<i32> {
    if command.is_empty() {
        bail!("agent exec requires a command after --");
    }
    let session = load_session(id)?;
    let response = exec_session(&client()?, &session, command, &cwd, timeout_ms)?;
    print_exec_output(&response)?;
    Ok(response["exit_code"].as_i64().unwrap_or(125) as i32)
}

pub fn list() -> Result<()> {
    let root = session_root();
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&root)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            let session: Session = serde_json::from_slice(&fs::read(&path)?)?;
            println!("{}\t{}", session.id, session.workspace.display());
        }
    }
    Ok(())
}

pub fn stop(id: &str) -> Result<()> {
    let session = load_session(id)?;
    checked(
        client()?
            .post(url(&session.daemon, &format!("/api/tasks/{id}/stop")))
            .header("X-Tinybox-Task-Token", &session.token)
            .send(),
    )?;
    Ok(())
}

pub fn destroy(id: &str) -> Result<()> {
    let session = load_session(id)?;
    destroy_session(&client()?, &session)?;
    remove_session(id)
}

fn exec_session(
    client: &Client,
    session: &Session,
    command: Vec<String>,
    cwd: &str,
    timeout_ms: u64,
) -> Result<Value> {
    let response = checked(
        client
            .post(url(
                &session.daemon,
                &format!("/api/tasks/{}/exec", session.id),
            ))
            .header("X-Tinybox-Task-Token", &session.token)
            .json(&json!({"command":command,"cwd":cwd,"timeout_ms":timeout_ms}))
            .send(),
    )?;
    response.json().context("invalid task exec response")
}

fn destroy_session(client: &Client, session: &Session) -> Result<()> {
    checked(
        client
            .delete(url(&session.daemon, &format!("/api/tasks/{}", session.id)))
            .header("X-Tinybox-Task-Token", &session.token)
            .send(),
    )?;
    Ok(())
}

fn wait_running(client: &Client, session: &Session) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let value: Value = checked(
            client
                .get(url(&session.daemon, &format!("/api/tasks/{}", session.id)))
                .send(),
        )?
        .json()?;
        match value["status"].as_str() {
            Some("running") => return Ok(()),
            Some("failed" | "setup_failed" | "completed") => bail!("task failed to start: {value}"),
            _ if Instant::now() >= deadline => bail!("timed out waiting for task startup"),
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn checked(response: reqwest::Result<Response>) -> Result<Response> {
    let response =
        response.context("cannot reach tinybox daemon; host execution was not attempted")?;
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().unwrap_or_default();
    bail!("tinybox daemon returned {status}: {body}")
}

fn client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(3700))
        .build()
        .map_err(Into::into)
}

fn url(daemon: &str, path: &str) -> String {
    format!("http://{daemon}{path}")
}

fn default_policy() -> Value {
    json!({"version":1,"filesystem":[{"path":"/workspace","access":"read_write"}],"network":[],"resources":{"memory_bytes":1073741824u64,"cpus":2.0,"pids":256},"phases":[]})
}

fn required_string(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("task response missing {key}"))
}

fn save_session(session: &Session) -> Result<()> {
    let root = session_root();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root)?;
    let path = root.join(format!("{}.json", session.id));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(&serde_json::to_vec(session)?)?;
    Ok(())
}

fn load_session(id: &str) -> Result<Session> {
    if !id.starts_with("task-")
        || !id[5..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        bail!("invalid task id");
    }
    serde_json::from_slice(
        &fs::read(session_root().join(format!("{id}.json"))).context("unknown local Agent task")?,
    )
    .map_err(Into::into)
}

fn remove_session(id: &str) -> Result<()> {
    let path = session_root().join(format!("{id}.json"));
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn session_root() -> PathBuf {
    let uid = unsafe { libc::geteuid() };
    if uid == 0 {
        return PathBuf::from("/run/tinybox/agents");
    }
    if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        if fs::metadata(&path)
            .map(|metadata| {
                use std::os::unix::fs::MetadataExt;
                metadata.is_dir() && metadata.uid() == uid
            })
            .unwrap_or(false)
        {
            return path.join("tinybox/agents");
        }
    }
    PathBuf::from(format!("/run/user/{uid}/tinybox/agents"))
}

fn print_exec_output(value: &Value) -> Result<()> {
    io::stdout().write_all(value["stdout"].as_str().unwrap_or_default().as_bytes())?;
    io::stderr().write_all(value["stderr"].as_str().unwrap_or_default().as_bytes())?;
    Ok(())
}
