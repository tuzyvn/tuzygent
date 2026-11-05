use agent_core::prelude::*;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct RunTaskReq {
    pub agent_id: String,
    pub goal: String,
    pub engine: Option<String>,
    pub sandbox: Option<String>,
    pub persona_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitTaskReq {
    pub title: String,
    pub goal: String,
    pub priority: Option<String>,
    pub engine: Option<String>,
}

pub async fn run_task_handler(
    State(state): State<AppState>,
    Json(payload): Json<RunTaskReq>,
) -> impl IntoResponse {
    let ws_path = std::path::Path::new(&state.workspace);
    let config = EnginesConfig::load_hierarchical(Some(ws_path));
    let engine_name = payload.engine.as_deref().unwrap_or(&config.defaults.engine);

    let compute_engine = match agent_engine::configured_engine(&config, engine_name, None) {
        Ok(engine) => engine,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":error.to_string()})),
            )
        }
    };

    let scratchpad = ScratchpadManager::new(&state.workspace);
    let durable = DurableMemoryStore::new(&state.workspace);
    let guardrails = GuardrailPolicy::default_enterprise();

    let sandbox_cfg = (|| -> anyhow::Result<SandboxConfig> {
        let mut config = SandboxConfig::from_engines_config(&config)?;
        if let Some(driver) = &payload.sandbox {
            config.driver = SandboxDriverKind::parse(driver)?;
        }
        Ok(config)
    })();
    let sandbox_cfg = match sandbox_cfg {
        Ok(config) => config,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": error.to_string()})),
            )
        }
    };

    let ooda = AutonomousOodaLoop::new(
        compute_engine,
        guardrails,
        scratchpad,
        durable,
        &state.workspace,
    )
    .with_timeout_seconds(config.defaults.timeout_seconds)
    .with_sandbox_config(sandbox_cfg)
    .with_bus(state.bus.clone());

    let ooda = if let Some(path) = &payload.persona_path {
        match PersonaLoader::load_from_dir(path).await {
            Ok(profile) => ooda.with_persona(profile),
            Err(error) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":error.to_string()}))),
        }
    } else { ooda };

    let mut task = Task::new(&payload.goal, &payload.goal, Priority::High);
    task.assigned_agent = Some(AgentId::from_str(&payload.agent_id));

    match ooda.run_goal(&mut task).await {
        Ok(final_status) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": format!("{:?}", final_status),
                "task_id": task.id.to_string(),
                "goal": payload.goal,
                "agent_id": payload.agent_id,
                "engine_used": engine_name,
                "subtasks_count": task.subtasks.len(),
            })),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "failed",
                "error": err.to_string(),
            })),
        ),
    }
}

pub async fn submit_task_handler(
    State(state): State<AppState>,
    Json(payload): Json<SubmitTaskReq>,
) -> impl IntoResponse {
    let priority = match payload.priority.as_deref().unwrap_or("normal") {
        "low" => Priority::Low,
        "high" => Priority::High,
        "critical" => Priority::Critical,
        _ => Priority::Normal,
    };

    match state.task_service.submit_task(&payload.title, &payload.goal, priority).await {
        Ok(task_id) => {
            let ws_path = std::path::Path::new(&state.workspace);
            let config = EnginesConfig::load_hierarchical(Some(ws_path));
            let engine_name = payload.engine.as_deref().unwrap_or(&config.defaults.engine);
            if let Ok(engine) = agent_engine::configured_engine(&config, engine_name, None) {
                state.task_service.dispatch(task_id.clone(), engine);
            }

            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "task_id": task_id.to_string(),
                    "status": "queued",
                    "tracking_url": format!("/api/v1/tasks/{}", task_id),
                })),
            ).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        ).into_response(),
    }
}

pub async fn list_tasks_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.task_service.list_tasks(50, 0).await {
        Ok(tasks) => Json(serde_json::json!({ "tasks": tasks })),
        Err(err) => Json(serde_json::json!({ "error": err.to_string(), "tasks": [] })),
    }
}

pub async fn get_task_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = TaskId(id);
    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            let result = state.task_service.get_result(&task_id).await.unwrap_or(None);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "task": task,
                    "result": result,
                })),
            ).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Task not found" }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn cancel_task_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = TaskId(id);
    match state.task_service.cancel_task(&task_id).await {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({ "canceled": true }))).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Task not found" }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn approve_task_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = TaskId(id);
    let ws_path = std::path::Path::new(&state.workspace);
    let config = EnginesConfig::load_hierarchical(Some(ws_path));
    let engine = match agent_engine::configured_engine(&config, &config.defaults.engine, None) {
        Ok(e) => e,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err.to_string() }))).into_response(),
    };

    match state.task_service.approve_task(&task_id, engine).await {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({ "resumed": true }))).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Task not found" }))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}
