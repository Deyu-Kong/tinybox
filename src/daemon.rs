use crate::audit::AuditSink;
use crate::cgroup::Cgroup;
use crate::policy::{CapabilityDescriptor, NetworkRule};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<String>,
    generation: u64,
    #[serde(skip)]
    audit: AuditSink,
    #[serde(skip)]
    descriptor: Option<CapabilityDescriptor>,
    #[serde(skip)]
    network_policy: Option<Arc<std::sync::RwLock<Vec<NetworkRule>>>>,
    #[serde(skip)]
    cgroup_path: PathBuf,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseRequest {
    phase: String,
    expected_generation: u64,
}

pub async fn serve(listen: SocketAddr) -> anyhow::Result<()> {
    let state = AppState {
        sandboxes: Arc::new(Mutex::new(HashMap::new())),
        next_id: Arc::new(AtomicU64::new(1)),
    };
    let app = Router::new()
        .route("/api/sandboxes", post(create).get(list))
        .route("/api/sandboxes/:id", get(get_one).delete(remove))
        .route("/api/sandboxes/:id/audit", get(audit_events))
        .route("/api/sandboxes/:id/audit/summary", get(audit_summary))
        .route("/api/sandboxes/:id/phase", post(change_phase))
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
    let descriptor = loaded_policy
        .as_ref()
        .map(|policy| policy.descriptor.clone());
    let initial_phase = descriptor
        .as_ref()
        .and_then(|policy| policy.phases.first())
        .cloned();
    let active_network = initial_phase
        .as_ref()
        .map(|phase| phase.network.clone())
        .or_else(|| descriptor.as_ref().map(|policy| policy.network.clone()));
    let network_policy = active_network.map(|rules| Arc::new(std::sync::RwLock::new(rules)));
    let cgroup_name = format!("tinybox-{id}");
    let cgroup_path = PathBuf::from("/sys/fs/cgroup").join(&cgroup_name);
    let audit = AuditSink::new(id.clone(), policy_hash.clone());
    if let Some(phase) = &initial_phase {
        audit.set_phase(&phase.name);
    }
    audit.record(
        "runtime",
        "allow",
        "policy.load",
        policy_hash.clone().unwrap_or_else(|| "none".into()),
        None,
        "request accepted by control plane",
    );
    let entry = Sandbox {
        id: id.clone(),
        status: "running".into(),
        exit_code: None,
        pid: None,
        error: None,
        policy_hash: policy_hash.clone(),
        phase: initial_phase.as_ref().map(|phase| phase.name.clone()),
        generation: 0,
        audit: audit.clone(),
        descriptor: descriptor.clone(),
        network_policy: network_policy.clone(),
        cgroup_path,
    };
    state.sandboxes.lock().unwrap().insert(id.clone(), entry);
    let initial_phase_name = initial_phase.as_ref().map(|phase| phase.name.clone());
    let state_clone = state.clone();
    let id_clone = id.clone();
    tokio::task::spawn_blocking(move || {
        let policy_resources = initial_phase
            .as_ref()
            .map(|phase| &phase.resources)
            .or_else(|| descriptor.as_ref().map(|policy| &policy.resources));
        let config = SandboxConfig {
            cgroup_name: Some(cgroup_name),
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
            network_policy,
            audit: Some(audit),
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
        Json(
            serde_json::json!({"id": id, "status": "running", "policy_hash": policy_hash, "phase": initial_phase_name, "generation": 0}),
        ),
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
async fn audit_events(Path(id): Path<String>, State(state): State<AppState>) -> impl IntoResponse {
    let audit = state
        .sandboxes
        .lock()
        .unwrap()
        .get(&id)
        .map(|sandbox| sandbox.audit.clone());
    match audit {
        Some(audit) => (StatusCode::OK, Json(audit.snapshot())).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn audit_summary(Path(id): Path<String>, State(state): State<AppState>) -> impl IntoResponse {
    let audit = state
        .sandboxes
        .lock()
        .unwrap()
        .get(&id)
        .map(|sandbox| sandbox.audit.clone());
    match audit {
        Some(audit) => (StatusCode::OK, Json(audit.summary())).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn change_phase(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<PhaseRequest>,
) -> impl IntoResponse {
    let mut sandboxes = state.sandboxes.lock().unwrap();
    let Some(sandbox) = sandboxes.get_mut(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let deny = |reason: String, audit: &AuditSink| {
        audit.record(
            "control",
            "deny",
            "phase.transition",
            request.phase.clone(),
            None,
            reason.clone(),
        );
        (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": reason})),
        )
            .into_response()
    };
    if sandbox.status != "running" {
        return deny("sandbox is not running".into(), &sandbox.audit);
    }
    if request.expected_generation != sandbox.generation {
        return deny("phase generation mismatch".into(), &sandbox.audit);
    }
    let Some(descriptor) = &sandbox.descriptor else {
        return deny("sandbox has no phase policy".into(), &sandbox.audit);
    };
    let Some(current_name) = &sandbox.phase else {
        return deny("sandbox has no active phase".into(), &sandbox.audit);
    };
    let Some(current) = descriptor
        .phases
        .iter()
        .find(|phase| &phase.name == current_name)
    else {
        return deny("active phase is invalid".into(), &sandbox.audit);
    };
    if !current.next.contains(&request.phase) {
        return deny("phase transition is not allowed".into(), &sandbox.audit);
    }
    let next = descriptor
        .phases
        .iter()
        .find(|phase| phase.name == request.phase)
        .expect("validated phase graph");
    if let Err(error) = Cgroup::update_resources(
        &sandbox.cgroup_path,
        next.resources.memory_bytes,
        next.resources.cpus,
        next.resources.pids,
    ) {
        return deny(format!("resource update failed: {error:#}"), &sandbox.audit);
    }
    if let Some(network) = &sandbox.network_policy {
        *network.write().unwrap() = next.network.clone();
    }
    sandbox.phase = Some(next.name.clone());
    sandbox.generation += 1;
    sandbox.audit.set_phase(&next.name);
    sandbox.audit.record(
        "control",
        "allow",
        "phase.transition",
        next.name.clone(),
        Some(format!("phase:{}", next.name)),
        "phase and resource policy updated",
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "phase": sandbox.phase,
            "generation": sandbox.generation
        })),
    )
        .into_response()
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
