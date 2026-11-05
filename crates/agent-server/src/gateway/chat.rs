use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;

use agent_engine::{AccountRotatedEngine, ComputeEngine, EngineInput};
use agent_runtime::sandbox::{IsolationMode, SandboxEnvironment};

use super::types::{format_prompt_from_messages, parse_model_spec, ChatCompletionRequest};
use crate::AppState;

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
            )
                .into_response();
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
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": {
                        "message": err.to_string(),
                        "type": "engine_error"
                    }
                })),
            )
                .into_response(),
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
