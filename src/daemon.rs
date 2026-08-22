use crate::audit::AuditSink;
use crate::cgroup::Cgroup;
use crate::policy::{CapabilityDescriptor, NetworkRule};
use crate::sandbox::{run_sandbox_with_pid, run_sandbox_with_pids, SandboxConfig, SetupError};
use crate::task::{self, ExecRequest, ExecTarget};
use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::HeaderMap,
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
    kind: String,
    #[serde(skip)]
    audit: AuditSink,
    #[serde(skip)]
    descriptor: Option<CapabilityDescriptor>,
    #[serde(skip)]
    network_policy: Option<Arc<std::sync::RwLock<Vec<NetworkRule>>>>,
    #[serde(skip)]
    cgroup_path: PathBuf,
    #[serde(skip)]
    keeper_pid: Option<i32>,
    #[serde(skip)]
    keeper_start_time: Option<u64>,
    #[serde(skip)]
    task_token: Option<String>,
    #[serde(skip)]
    task_env: Vec<String>,
    #[serde(skip)]
    exec_lock: Arc<Mutex<()>>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskCreateRequest {
    workspace: String,
    policy: CapabilityDescriptor,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    hostname: Option<String>,
}

pub async fn serve(listen: SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    task::cleanup_orphaned_task_cgroups()
        .context("failed to clean task state left by an interrupted daemon")?;
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
        .route("/api/tasks", post(create_task))
        .route("/api/tasks/:id", get(get_one).delete(remove_task))
        .route("/api/tasks/:id/exec", post(exec_task))
        .route("/metrics", get(metrics))
        .with_state(state);
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
        kind: "sandbox".into(),
        audit: audit.clone(),
        descriptor: descriptor.clone(),
        network_policy: network_policy.clone(),
        cgroup_path,
        keeper_pid: None,
        keeper_start_time: None,
        task_token: None,
        task_env: Vec::new(),
        exec_lock: Arc::new(Mutex::new(())),
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

async fn create_task(
    State(state): State<AppState>,
    Json(req): Json<TaskCreateRequest>,
) -> impl IntoResponse {
    let workspace = match std::fs::canonicalize(&req.workspace) {
        Ok(path) if path.is_dir() => path,
        Ok(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"workspace must be a directory"})),
            )
        }
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":format!("invalid workspace: {error}")})),
            )
        }
    };
    let loaded = match req.policy.compile() {
        Ok(policy) => policy,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":format!("invalid policy: {error:#}")})),
            )
        }
    };
    if !loaded.descriptor.phases.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"task API accepts static policies only"})),
        );
    }
    if loaded.descriptor.filesystem.iter().any(|rule| {
        rule.path != std::path::Path::new("/workspace")
            && !rule.path.starts_with("/workspace/")
            && rule.path != std::path::Path::new("/tmp")
    }) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error":"task filesystem rules must stay under /workspace or /tmp"}),
            ),
        );
    }
    if let Err(error) = validate_task_env(&req.env) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":error.to_string()})),
        );
    }

    let id = format!("task-{}", state.next_id.fetch_add(1, Ordering::Relaxed));
    let token = match generate_task_token() {
        Ok(token) => token,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error":format!("failed to create task token: {error:#}")}),
                ),
            )
        }
    };
    let cgroup_name = format!("tinybox-{id}");
    let cgroup_path = PathBuf::from("/sys/fs/cgroup").join(&cgroup_name);
    let policy_hash = loaded.hash.clone();
    let descriptor = loaded.descriptor.clone();
    let network_policy = Arc::new(std::sync::RwLock::new(descriptor.network.clone()));
    let audit = AuditSink::new(id.clone(), Some(policy_hash.clone()));
    audit.record(
        "runtime",
        "allow",
        "task.create",
        "/workspace",
        Some("task:ceiling".into()),
        "persistent Agent task accepted",
    );
    let entry = Sandbox {
        id: id.clone(),
        status: "starting".into(),
        exit_code: None,
        pid: None,
        error: None,
        policy_hash: Some(policy_hash.clone()),
        phase: None,
        generation: 0,
        kind: "task".into(),
        audit: audit.clone(),
        descriptor: Some(descriptor.clone()),
        network_policy: Some(network_policy.clone()),
        cgroup_path: cgroup_path.clone(),
        keeper_pid: None,
        keeper_start_time: None,
        task_token: Some(token.clone()),
        task_env: req.env.clone(),
        exec_lock: Arc::new(Mutex::new(())),
    };
    state.sandboxes.lock().unwrap().insert(id.clone(), entry);

    let state_clone = state.clone();
    let id_clone = id.clone();
    tokio::task::spawn_blocking(move || {
        let resources = &descriptor.resources;
        let config = SandboxConfig {
            cgroup_name: Some(cgroup_name),
            command: vec!["/bin/sleep".into(), "2147483647".into()],
            hostname: req.hostname,
            rootfs: Some(PathBuf::from("/")),
            root_readonly: false,
            env: req.env,
            proxy: None,
            volumes: vec![format!("{}:/workspace", workspace.display())],
            memory: Some(resources.memory_bytes),
            cpus: Some(resources.cpus),
            cpu_quota: None,
            cpu_period: None,
            pids_limit: Some(resources.pids),
            dangerous: false,
            filesystem_policy: Some(descriptor.filesystem.clone()),
            network_policy: Some(network_policy),
            audit: Some(audit),
            namespaces: None,
            cwd: Some("/workspace".into()),
            uid: 0,
            gid: 0,
        };
        let supervisor_state = state_clone.clone();
        let supervisor_id = id_clone.clone();
        let keeper_state = state_clone.clone();
        let keeper_id = id_clone.clone();
        let result = run_sandbox_with_pids(
            &config,
            move |pid| {
                if let Some(task) = supervisor_state
                    .sandboxes
                    .lock()
                    .unwrap()
                    .get_mut(&supervisor_id)
                {
                    task.pid = Some(pid.as_raw());
                }
            },
            move |pid| {
                if let Some(task) = keeper_state.sandboxes.lock().unwrap().get_mut(&keeper_id) {
                    task.keeper_pid = Some(pid.as_raw());
                    task.keeper_start_time = task::process_start_time(pid.as_raw()).ok();
                    if task.keeper_start_time.is_some() {
                        task.status = "running".into();
                    }
                }
            },
        );
        if let Some(task) = state_clone.sandboxes.lock().unwrap().get_mut(&id_clone) {
            match result {
                Ok(code) => {
                    task.status = "completed".into();
                    task.exit_code = Some(code);
                }
                Err(error) => {
                    task.status = if error.downcast_ref::<SetupError>().is_some() {
                        "setup_failed".into()
                    } else {
                        "failed".into()
                    };
                    task.error = Some(format!("{error:#}"));
                }
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "id": id,
            "status": "starting",
            "token": token,
            "policy_hash": policy_hash,
            "workspace": "/workspace"
        })),
    )
}

async fn exec_task(
    Path(id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ExecRequest>,
) -> impl IntoResponse {
    let Some(token) = task_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let task = {
        let sandboxes = state.sandboxes.lock().unwrap();
        let Some(task) = sandboxes.get(&id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if task.kind != "task"
            || !task
                .task_token
                .as_deref()
                .is_some_and(|expected| constant_time_eq(expected, token))
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        if task.status != "running" {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error":format!("task is {}", task.status)})),
            )
                .into_response();
        }
        task.clone()
    };
    let Some(keeper_pid) = task.keeper_pid else {
        return StatusCode::CONFLICT.into_response();
    };
    let Some(keeper_start_time) = task.keeper_start_time else {
        return StatusCode::CONFLICT.into_response();
    };
    let filesystem_policy = task
        .descriptor
        .as_ref()
        .map(|policy| policy.filesystem.clone())
        .unwrap_or_default();
    let target = ExecTarget {
        keeper_pid,
        keeper_start_time,
        cgroup_path: task.cgroup_path.clone(),
        filesystem_policy,
        base_env: task.task_env.clone(),
    };
    let audit = task.audit.clone();
    let exec_lock = task.exec_lock.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _guard = exec_lock.lock().unwrap();
        task::exec_in_task(&target, &request)
    })
    .await;
    match result {
        Ok(Ok(response)) => {
            audit.record(
                "runtime",
                "allow",
                "task.exec",
                format!("exit={}", response.exit_code),
                Some("task:exec".into()),
                if response.timed_out {
                    "tool call timed out"
                } else {
                    "tool call completed"
                },
            );
            (StatusCode::OK, Json(serde_json::json!(response))).into_response()
        }
        Ok(Err(error)) => {
            audit.record(
                "runtime",
                "deny",
                "task.exec",
                "setup",
                Some("task:exec".into()),
                format!("{error:#}"),
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error":format!("task exec failed: {error:#}")})),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":format!("task exec worker failed: {error}")})),
        )
            .into_response(),
    }
}

async fn remove_task(
    Path(id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(token) = task_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let (supervisor_pid, cgroup_path) = {
        let mut sandboxes = state.sandboxes.lock().unwrap();
        let Some(task) = sandboxes.get_mut(&id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if task.kind != "task"
            || !task
                .task_token
                .as_deref()
                .is_some_and(|expected| constant_time_eq(expected, token))
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let Some(supervisor_pid) = task.pid else {
            return StatusCode::CONFLICT.into_response();
        };
        task.status = "stopping".into();
        (supervisor_pid, task.cgroup_path.clone())
    };

    let result =
        tokio::task::spawn_blocking(move || task::destroy_task(supervisor_pid, &cgroup_path)).await;
    let mut sandboxes = state.sandboxes.lock().unwrap();
    let cleanup_error = match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("{error:#}")),
        Err(error) => Some(format!("cleanup worker failed: {error}")),
    };
    if let Some(error) = cleanup_error {
        if let Some(task) = sandboxes.get_mut(&id) {
            task.status = "cleanup_failed".into();
            task.error = Some(format!("task cleanup failed: {error}"));
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":format!("task cleanup failed: {error}")})),
        )
            .into_response();
    }
    sandboxes.remove(&id);
    StatusCode::NO_CONTENT.into_response()
}

fn task_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-tinybox-task-token")
        .and_then(|value| value.to_str().ok())
}

fn constant_time_eq(expected: &str, provided: &str) -> bool {
    if expected.len() != provided.len() {
        return false;
    }
    expected
        .bytes()
        .zip(provided.bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn generate_task_token() -> anyhow::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn validate_task_env(values: &[String]) -> anyhow::Result<()> {
    const ALLOWED: &[&str] = &["PATH", "LANG", "LC_ALL", "TERM", "HOME"];
    for value in values {
        let Some((name, _)) = value.split_once('=') else {
            anyhow::bail!("task env entries must use NAME=VALUE");
        };
        if !ALLOWED.contains(&name) {
            anyhow::bail!("task environment variable is not allowlisted: {name}");
        }
    }
    Ok(())
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
        if sb.kind == "task" {
            return StatusCode::FORBIDDEN;
        }
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

    #[test]
    fn task_tokens_use_exact_constant_time_comparison() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc123", "short"));
    }

    #[test]
    fn task_environment_is_allowlisted() {
        assert!(validate_task_env(&["PATH=/usr/bin:/bin".into(), "LANG=C".into()]).is_ok());
        assert!(validate_task_env(&["AWS_SECRET_ACCESS_KEY=secret".into()]).is_err());
        assert!(validate_task_env(&["MALFORMED".into()]).is_err());
    }
}
