use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;

use agent_engine::{AccountRotatedEngine, EngineInput};
use agent_runtime::ExecutionService;

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

    let execution = match ExecutionService::from_workspace(&state.workspace) {
        Ok(service) => service,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    };
    let engine = Arc::new(AccountRotatedEngine::new(
        state.vault.clone(),
        engine_kind,
        None,
        model_override,
    ));

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
        let overflow = Arc::new(AtomicBool::new(false));
        let callback_overflow = overflow.clone();
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
                if tx_guard.try_send(Ok(Event::default().data(chunk_data.to_string()))).is_err() {
                    callback_overflow.store(true, Ordering::SeqCst);
                }
            } else {
                callback_overflow.store(true, Ordering::SeqCst);
            }
        });

        let mut input = EngineInput::new(prompt, &state.workspace);
        if !system_ctx.is_empty() {
            input.system_context = system_ctx;
        }
        input.temperature = payload.temperature;
        input.max_tokens = payload.max_tokens;
        input.on_stream_chunk = Some(stream_cb);

        tokio::spawn(async move {
            let outcome = {
            let run = execution.execute(engine, &input);
            tokio::pin!(run);
            loop {
                tokio::select! {
                    result = &mut run => break Some(result),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                        if tx.is_closed() { break None; }
                        if overflow.load(Ordering::SeqCst) {
                            break Some(Err(agent_engine::EngineError::ProcessFailed("Stream buffer exhausted; output is incomplete".into())));
                        }
                    }
                }
            }
            };
            let Some(outcome) = outcome else { return; };
            let outcome = if overflow.load(Ordering::SeqCst) {
                Err(agent_engine::EngineError::ProcessFailed("Stream buffer exhausted; output is incomplete".into()))
            } else { outcome };
            match outcome {
                Ok(output) if output.exit_code == 0 => {
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
                result => {
                    let message = match result {
                        Ok(output) => format!("Engine exited with code {}", output.exit_code),
                        Err(err) => err.to_string(),
                    };
                    let err_chunk = serde_json::json!({
                        "id": &completion_id,
                        "object": "chat.completion.chunk",
                        "created": created_ts,
                        "model": &model_name,
                        "error": { "message": message }
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
        match execution.execute(engine, &input).await {
            Ok(output) if output.exit_code == 0 => {
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
                    "usage": null
                });
                Json(resp).into_response()
            }
            result => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": {
                        "message": match result { Ok(output) => format!("Engine exited with code {}", output.exit_code), Err(err) => err.to_string() },
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
