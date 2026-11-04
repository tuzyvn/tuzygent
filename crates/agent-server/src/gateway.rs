use agent_engine::{
    AccountEngineKind, AccountRotatedEngine, ComputeEngine, EngineInput,
    SafeAccountProfile,
};
use agent_runtime::sandbox::{IsolationMode, SandboxEnvironment};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct AddAccountReq {
    pub engine: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub extra_env: HashMap<String, String>,
}

pub fn parse_model_spec(model_str: &str) -> Result<(AccountEngineKind, Option<String>), String> {
    let lower = model_str.to_lowercase();
    if lower.starts_with("antigravity") || lower.starts_with("agy") || lower.starts_with("gemini") {
        let specific_model = model_str.split_once('/').map(|(_, m)| m.to_string());
        Ok((AccountEngineKind::Antigravity, specific_model))
    } else if lower.starts_with("codex") || lower.starts_with("gpt") || lower.starts_with("o1") || lower.starts_with("o3") {
        let specific_model = model_str.split_once('/').map(|(_, m)| m.to_string());
        Ok((AccountEngineKind::Codex, specific_model))
    } else if lower.starts_with("claude") {
        let specific_model = model_str.split_once('/').map(|(_, m)| m.to_string());
        Ok((AccountEngineKind::Claude, specific_model))
    } else {
        Err(format!("Model '{}' is not recognized or permitted. Supported engines: antigravity, codex, claude.", model_str))
    }
}

pub fn format_prompt_from_messages(messages: &[ChatMessage]) -> (String, String) {
    let mut system_context = String::new();
    let mut prompt = String::new();

    for msg in messages {
        match msg.role.to_lowercase().as_str() {
            "system" => {
                if !system_context.is_empty() {
                    system_context.push('\n');
                }
                system_context.push_str(&msg.content);
            }
            "user" => {
                if !prompt.is_empty() {
                    prompt.push_str("\n\n[USER]:\n");
                }
                prompt.push_str(&msg.content);
            }
            "assistant" => {
                prompt.push_str("\n\n[ASSISTANT]:\n");
                prompt.push_str(&msg.content);
            }
            _ => {
                prompt.push_str("\n\n");
                prompt.push_str(&msg.content);
            }
        }
    }

    (system_context, prompt)
}

/// Endpoint chuẩn OpenAI POST /v1/chat/completions
pub async fn chat_completions_handler(
    State(state): State<AppState>,
    Json(payload): Json<ChatCompletionRequest>,
) -> Response {
    let (engine_kind, model_override) = match parse_model_spec(&payload.model) {
        Ok(res) => res,
        Err(err_msg) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": {
                        "message": err_msg,
                        "type": "invalid_request_error"
                    }
                })),
            ).into_response();
        }
    };
    let (system_ctx, prompt) = format_prompt_from_messages(&payload.messages);

    let engine = AccountRotatedEngine::new(
        state.vault.clone(),
        engine_kind,
        None,
        model_override,
    );

    let completion_id = format!("chatcmpl-{}", Uuid::new_v4().simple());
    let created_ts = chrono::Utc::now().timestamp();
    let model_name = payload.model.clone();

    if payload.stream {
        use futures::sink::SinkExt;
        let (mut tx, rx) = futures::channel::mpsc::channel::<Result<Event, Infallible>>(128);

        let initial_chunk = serde_json::json!({
            "id": &completion_id,
            "object": "chat.completion.chunk",
            "created": created_ts,
            "model": &model_name,
            "choices": [{
                "index": 0,
                "delta": { "role": "assistant", "content": "" },
                "finish_reason": null
            }]
        });
        let _ = tx.try_send(Ok(Event::default().data(initial_chunk.to_string())));

        let tx_mutex = Arc::new(std::sync::Mutex::new(tx.clone()));
        let cid = completion_id.clone();
        let mname = model_name.clone();

        let stream_cb = Arc::new(move |chunk: &str| {
            let chunk_data = serde_json::json!({
                "id": &cid,
                "object": "chat.completion.chunk",
                "created": created_ts,
                "model": &mname,
                "choices": [{
                    "index": 0,
                    "delta": { "content": chunk },
                    "finish_reason": null
                }]
            });
            if let Ok(mut tx_guard) = tx_mutex.lock() {
                let _ = tx_guard.try_send(Ok(Event::default().data(chunk_data.to_string())));
            }
        });

        let mut input = EngineInput::new(prompt, &state.workspace);
        if !system_ctx.is_empty() {
            input.system_context = system_ctx;
        }
        input.temperature = payload.temperature;
        input.max_tokens = payload.max_tokens;
        let sandbox = SandboxEnvironment::new(&state.workspace, IsolationMode::LocalDirectory);
        input.command_sandbox = sandbox.command_sandbox(Path::new(&input.working_dir));
        input.on_stream_chunk = Some(stream_cb);

        tokio::spawn(async move {
            match engine.execute(&input).await {
                Ok(_output) => {
                    let stop_chunk = serde_json::json!({
                        "id": &completion_id,
                        "object": "chat.completion.chunk",
                        "created": created_ts,
                        "model": &model_name,
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop"
                        }]
                    });
                    let _ = tx.send(Ok(Event::default().data(stop_chunk.to_string()))).await;
                    let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
                }
                Err(err) => {
                    let err_chunk = serde_json::json!({
                        "id": &completion_id,
                        "object": "chat.completion.chunk",
                        "created": created_ts,
                        "model": &model_name,
                        "error": { "message": err.to_string() }
                    });
                    let _ = tx.send(Ok(Event::default().data(err_chunk.to_string()))).await;
                    let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
                }
            }
        });

        Sse::new(rx).into_response()
    } else {
        let mut input = EngineInput::new(prompt, &state.workspace);
        if !system_ctx.is_empty() {
            input.system_context = system_ctx;
        }
        input.temperature = payload.temperature;
        input.max_tokens = payload.max_tokens;
        let sandbox = SandboxEnvironment::new(&state.workspace, IsolationMode::LocalDirectory);
        input.command_sandbox = sandbox.command_sandbox(Path::new(&input.working_dir));

        match engine.execute(&input).await {
            Ok(output) => {
                let resp = serde_json::json!({
                    "id": completion_id,
                    "object": "chat.completion",
                    "created": created_ts,
                    "model": model_name,
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": output.stdout_content
                        },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 0,
                        "completion_tokens": 0,
                        "total_tokens": 0
                    }
                });
                Json(resp).into_response()
            }
            Err(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                    "error": {
                        "message": err.to_string(),
                        "type": "engine_error"
                    }
                }))).into_response()
            }
        }
    }
}

/// Endpoint chuẩn OpenAI GET /v1/models
pub async fn list_models_handler() -> Json<serde_json::Value> {
    let now = chrono::Utc::now().timestamp();
    Json(serde_json::json!({
        "object": "list",
        "data": [
            { "id": "antigravity", "object": "model", "created": now, "owned_by": "agentcore" },
            { "id": "antigravity/gemini-3.5-pro", "object": "model", "created": now, "owned_by": "agentcore" },
            { "id": "antigravity/gemini-1.5-pro", "object": "model", "created": now, "owned_by": "agentcore" },
            { "id": "antigravity/gemini-1.5-flash", "object": "model", "created": now, "owned_by": "agentcore" },
            { "id": "codex", "object": "model", "created": now, "owned_by": "agentcore" },
            { "id": "codex/gpt-4o", "object": "model", "created": now, "owned_by": "agentcore" },
            { "id": "claude", "object": "model", "created": now, "owned_by": "agentcore" },
            { "id": "claude/claude-sonnet-5", "object": "model", "created": now, "owned_by": "agentcore" },
        ]
    }))
}

/// Endpoint danh sách tài khoản quản lý trong vault (redacted secrets)
pub async fn list_accounts_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let accounts = state.vault.list_accounts(None);
    let safe_accounts: Vec<SafeAccountProfile> = accounts.iter().map(|a| a.to_safe()).collect();
    Json(serde_json::json!({
        "accounts": safe_accounts
    }))
}

/// Endpoint đăng ký tài khoản mới (redacted secret return)
pub async fn add_account_handler(
    State(state): State<AppState>,
    Json(payload): Json<AddAccountReq>,
) -> Result<Json<SafeAccountProfile>, (StatusCode, String)> {
    let engine_kind = AccountEngineKind::from_str_name(&payload.engine);
    state.vault.register_account(engine_kind, &payload.id, &payload.name, payload.extra_env)
        .map(|p| Json(p.to_safe()))
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

/// Endpoint xóa tài khoản khỏi pool
pub async fn remove_account_handler(
    State(state): State<AppState>,
    axum::extract::Path((engine, id)): axum::extract::Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let engine_kind = AccountEngineKind::from_str_name(&engine);
    match state.vault.remove_account(&engine_kind, &id) {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err((StatusCode::NOT_FOUND, "Account not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// Reset trạng thái Cooldown về Ready
pub async fn reset_account_handler(
    State(state): State<AppState>,
    axum::extract::Path((engine, id)): axum::extract::Path<(String, String)>,
) -> Json<serde_json::Value> {
    let engine_kind = AccountEngineKind::from_str_name(&engine);
    let ok = state.vault.reset_cooldown(&engine_kind, &id);
    Json(serde_json::json!({ "reset": ok }))
}

/// Sao chép token CLI từ máy host (~/.gemini, ~/.codex) vào sandbox của tài khoản
pub async fn import_local_handler(
    State(state): State<AppState>,
    axum::extract::Path((engine, id)): axum::extract::Path<(String, String)>,
) -> Json<serde_json::Value> {
    let engine_kind = AccountEngineKind::from_str_name(&engine);
    let imported = state.vault.import_local_credentials(&engine_kind, &id).unwrap_or(false);
    Json(serde_json::json!({ "imported": imported }))
}

/// Chạy thử nghiệm lệnh test trên CLI với môi trường cách ly của tài khoản
pub async fn verify_account_handler(
    State(state): State<AppState>,
    axum::extract::Path((engine, id)): axum::extract::Path<(String, String)>,
) -> Json<serde_json::Value> {
    let engine_kind = AccountEngineKind::from_str_name(&engine);
    let account = match state.vault.get_account(&engine_kind, &id) {
        Some(acc) => acc,
        None => {
            return Json(serde_json::json!({
                "success": false,
                "error": "Account not found"
            }));
        }
    };

    let env = state.vault.build_env_for_account(&account);
    let engine_impl: Arc<dyn ComputeEngine> = match engine_kind {
        AccountEngineKind::Antigravity => {
            Arc::new(agent_engine::AntigravityEngine::new(None, None).with_env(env))
        }
        AccountEngineKind::Codex => {
            Arc::new(agent_engine::CodexEngine::new(None, None).with_env(env))
        }
        AccountEngineKind::Claude => {
            Arc::new(agent_engine::ClaudeCodeEngine::new(None, None).with_env(env))
        }
        AccountEngineKind::Generic(ref name) => {
            Arc::new(agent_engine::GenericCliEngine::new(
                name.clone(),
                name.clone(),
                vec![],
                env,
                None,
            ))
        }
    };

    let mut input = EngineInput::new("echo verify", &state.workspace);
    let sandbox = SandboxEnvironment::new(&state.workspace, IsolationMode::LocalDirectory);
    input.command_sandbox = sandbox.command_sandbox(Path::new(&input.working_dir));

    let start = std::time::Instant::now();
    match engine_impl.execute(&input).await {
        Ok(output) => {
            let latency = start.elapsed().as_millis();
            state.vault.record_success(&engine_kind, &id);
            Json(serde_json::json!({
                "success": true,
                "latency_ms": latency,
                "output": output.stdout_content.trim()
            }))
        }
        Err(err) => {
            state.vault.mark_cooldown(&engine_kind, &id, 60, &err.to_string());
            Json(serde_json::json!({
                "success": false,
                "error": err.to_string()
            }))
        }
    }
}

/// Trả về toàn bộ giao diện HTML Web Dashboard quản lý tài khoản và Gateway
pub async fn dashboard_handler() -> Html<&'static str> {
    Html(crate::dashboard_html::DASHBOARD_HTML)
}
