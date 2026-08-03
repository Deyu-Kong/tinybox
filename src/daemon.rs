use crate::sandbox::{run_sandbox, SandboxConfig};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
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
}

#[derive(Deserialize)]
struct CreateRequest {
    rootfs: String,
    command: Vec<String>,
    memory_limit_mb: Option<u64>,
    proxy: Option<String>,
}

#[derive(Serialize)]
struct Metrics {
    sandboxes_total: usize,
    sandboxes_running: usize,
    sandboxes_completed: usize,
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
            memory: req.memory_limit_mb.map(|v| v * 1024 * 1024),
            cpus: None,
            cpu_quota: None,
            cpu_period: None,
            pids_limit: None,
            dangerous: false,
        };
        let result = run_sandbox(&config);
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
    if state.sandboxes.lock().unwrap().remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
async fn metrics(State(state): State<AppState>) -> Json<Metrics> {
    let sandboxes = state.sandboxes.lock().unwrap();
    let total = sandboxes.len();
    let running = sandboxes.values().filter(|s| s.status == "running").count();
    Json(Metrics {
        sandboxes_total: total,
        sandboxes_running: running,
        sandboxes_completed: total - running,
    })
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
