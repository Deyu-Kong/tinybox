use crate::sandbox::{run_sandbox_with_pid, SandboxConfig};
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
}

#[derive(Deserialize)]
struct CreateRequest {
    rootfs: String,
    command: Vec<String>,
    memory_limit_mb: Option<u64>,
    proxy: Option<String>,
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
    let id = format!("sb-{}", state.next_id.fetch_add(1, Ordering::Relaxed));
    let entry = Sandbox {
        id: id.clone(),
        status: "running".into(),
        exit_code: None,
        pid: None,
    };
    state.sandboxes.lock().unwrap().insert(id.clone(), entry);
    let state_clone = state.clone();
    let id_clone = id.clone();
    tokio::task::spawn_blocking(move || {
        let config = SandboxConfig {
            command: req.command,
            hostname: None,
            rootfs: Some(PathBuf::from(req.rootfs)),
            env: Vec::new(),
            proxy: req.proxy,
            network: None,
            ports: Vec::new(),
            memory: req.memory_limit_mb.map(|v| v * 1024 * 1024),
            cpus: None,
            cpu_quota: None,
            cpu_period: None,
            pids_limit: None,
            dangerous: false,
        };
        let state_for_pid = state_clone.clone();
        let id_for_pid = id_clone.clone();
        let result = run_sandbox_with_pid(&config, move |pid| {
            if let Some(sb) = state_for_pid.sandboxes.lock().unwrap().get_mut(&id_for_pid) {
                sb.pid = Some(pid.as_raw());
            }
        });
        if let Some(sb) = state_clone.sandboxes.lock().unwrap().get_mut(&id_clone) {
            sb.status = "completed".into();
            sb.exit_code = result.ok();
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"id": id, "status": "running"})),
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
        if sb.status == "running" {
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
    let body = format!(
        "# HELP tinybox_sandboxes_total Total sandboxes created\n# TYPE tinybox_sandboxes_total counter\ntinybox_sandboxes_total {total}\n# HELP tinybox_sandboxes_running Currently running sandboxes\n# TYPE tinybox_sandboxes_running gauge\ntinybox_sandboxes_running {running}\n# HELP tinybox_sandboxes_completed Completed sandboxes\n# TYPE tinybox_sandboxes_completed gauge\ntinybox_sandboxes_completed {}\n",
        total - running
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
