use crate::policy::CapabilityDescriptor;
use crate::sandbox::{run_sandbox_with_pid, SandboxConfig, SetupError};
use axum::{
    extract::{Path, State},
    http::{header::CONTENT_TYPE, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone)]
pub struct AppState {
    sandboxes: Arc<Mutex<HashMap<String, Sandbox>>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Clone, Serialize)]
struct Sandbox {
    id: String,
    status: String,
    exit_code: Option<i32>,
    pid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    policy_hash: Option<String>,
}

#[derive(Deserialize)]
struct CreateRequest {
    rootfs: String,
    command: Vec<String>,
    #[serde(default)]
    memory_limit_mb: Option<u64>,
    #[serde(default)]
    cpus: Option<f64>,
    #[serde(default)]
    pids_limit: Option<u64>,
    #[serde(default)]
    volumes: Vec<String>,
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    root_readonly: Option<bool>,
    #[serde(default)]
    proxy: Option<String>,
    // P1-4: `dangerous` is accepted only so it can be explicitly rejected over
    // the API — disabling seccomp/caps remotely is a footgun.
    #[serde(default)]
    dangerous: Option<bool>,
    #[serde(default)]
    policy: Option<CapabilityDescriptor>,
}

pub async fn serve(listen: SocketAddr) -> anyhow::Result<()> {
    let state = AppState {
        sandboxes: Arc::new(Mutex::new(HashMap::new())),
        next_id: Arc::new(AtomicU64::new(1)),
    };
    let app = Router::new()
        .route("/api/sandboxes", post(create).get(list))
        .route("/api/sandboxes/:id", get(get_one).delete(remove))
        .route("/metrics", get(metrics))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateRequest>,
) -> impl IntoResponse {
    if req.command.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"command must not be empty"})),
        );
    }
    // P1-4: never allow remotely disabling seccomp/caps.
    if req.dangerous.unwrap_or(false) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"dangerous mode cannot be enabled over the API"})),
        );
    }
    let loaded_policy = match req.policy {
        Some(policy) => match policy.compile() {
            Ok(policy) => Some(policy),
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("invalid policy: {error:#}")})),
                );
            }
        },
        None => None,
    };
    let policy_hash = loaded_policy.as_ref().map(|policy| policy.hash.clone());
    if let Some(policy) = &loaded_policy {
        let resources = &policy.descriptor.resources;
        let requested_memory = req.memory_limit_mb.map(|value| value * 1024 * 1024);
        if requested_memory.is_some_and(|value| value > resources.memory_bytes)
            || req.cpus.is_some_and(|value| value > resources.cpus)
            || req.pids_limit.is_some_and(|value| value > resources.pids)
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"requested resources exceed policy ceiling"})),
            );
        }
    }
    let id = format!("sb-{}", state.next_id.fetch_add(1, Ordering::Relaxed));
    let entry = Sandbox {
        id: id.clone(),
        status: "running".into(),
        exit_code: None,
        pid: None,
        error: None,
        policy_hash: policy_hash.clone(),
    };
    state.sandboxes.lock().unwrap().insert(id.clone(), entry);
    let state_clone = state.clone();
    let id_clone = id.clone();
    tokio::task::spawn_blocking(move || {
        let policy_resources = loaded_policy
            .as_ref()
            .map(|policy| &policy.descriptor.resources);
        let config = SandboxConfig {
            command: req.command,
            hostname: req.hostname,
            rootfs: Some(PathBuf::from(req.rootfs)),
            root_readonly: req.root_readonly.unwrap_or(false),
            env: req.env,
            proxy: req.proxy,
            volumes: req.volumes,
            memory: policy_resources
                .map(|resources| resources.memory_bytes)
                .or_else(|| req.memory_limit_mb.map(|v| v * 1024 * 1024)),
            cpus: policy_resources
                .map(|resources| resources.cpus)
                .or(req.cpus),
            cpu_quota: None,
            cpu_period: None,
            pids_limit: policy_resources
                .map(|resources| resources.pids)
                .or(req.pids_limit),
            dangerous: false,
            filesystem_policy: loaded_policy
                .as_ref()
                .map(|policy| policy.descriptor.filesystem.clone()),
            network_policy: loaded_policy
                .as_ref()
                .map(|policy| policy.descriptor.network.clone()),
            namespaces: None,
            cwd: None,
            uid: 0,
            gid: 0,
        };
        let state_for_pid = state_clone.clone();
        let id_for_pid = id_clone.clone();
        let result = run_sandbox_with_pid(&config, move |pid| {
            if let Some(sb) = state_for_pid.sandboxes.lock().unwrap().get_mut(&id_for_pid) {
                sb.pid = Some(pid.as_raw());
            }
        });
        // P1-3: distinguish completed vs failed.
        if let Some(sb) = state_clone.sandboxes.lock().unwrap().get_mut(&id_clone) {
            match result {
                Ok(code) => {
                    sb.status = "completed".into();
                    sb.exit_code = Some(code);
                }
                Err(e) => {
                    sb.status = if e.downcast_ref::<SetupError>().is_some() {
                        "setup_failed".into()
                    } else {
                        "failed".into()
                    };
                    sb.error = Some(format!("{e:#}"));
                }
            }
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"id": id, "status": "running", "policy_hash": policy_hash})),
    )
}

async fn list(State(state): State<AppState>) -> Json<Vec<Sandbox>> {
    Json(state.sandboxes.lock().unwrap().values().cloned().collect())
}
async fn get_one(Path(id): Path<String>, State(state): State<AppState>) -> impl IntoResponse {
    match state.sandboxes.lock().unwrap().get(&id).cloned() {
        Some(sb) => (StatusCode::OK, Json(sb)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn remove(Path(id): Path<String>, State(state): State<AppState>) -> StatusCode {
    let mut sandboxes = state.sandboxes.lock().unwrap();
    if let Some(sb) = sandboxes.get(&id) {
        if sb.status == "running" || sb.status == "failed" || sb.status == "setup_failed" {
            if let Some(pid) = sb.pid {
                let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
            }
        }
        sandboxes.remove(&id);
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let sandboxes = state.sandboxes.lock().unwrap();
    let total = sandboxes.len();
    let running = sandboxes.values().filter(|s| s.status == "running").count();
    // P1-3: count completed and failed explicitly — previously failed
    // sandboxes were miscounted as "completed" (total - running).
    let completed = sandboxes
        .values()
        .filter(|s| s.status == "completed")
        .count();
    let failed = sandboxes.values().filter(|s| s.status == "failed").count();
    let setup_failed = sandboxes
        .values()
        .filter(|s| s.status == "setup_failed")
        .count();
    let body = format!(
        "# HELP tinybox_sandboxes_total Total sandboxes created\n# TYPE tinybox_sandboxes_total counter\ntinybox_sandboxes_total {total}\n# HELP tinybox_sandboxes_running Currently running sandboxes\n# TYPE tinybox_sandboxes_running gauge\ntinybox_sandboxes_running {running}\n# HELP tinybox_sandboxes_completed Completed payloads\n# TYPE tinybox_sandboxes_completed gauge\ntinybox_sandboxes_completed {completed}\n# HELP tinybox_sandboxes_failed Runtime failures\n# TYPE tinybox_sandboxes_failed gauge\ntinybox_sandboxes_failed {failed}\n# HELP tinybox_sandboxes_setup_failed Sandbox setup failures\n# TYPE tinybox_sandboxes_setup_failed gauge\ntinybox_sandboxes_setup_failed {setup_failed}\n"
    );
    ([(CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}

pub fn parse_listen(value: &str) -> anyhow::Result<SocketAddr> {
    Ok(value.parse()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_listen() {
        assert_eq!(parse_listen("127.0.0.1:8080").unwrap().port(), 8080);
    }
}
