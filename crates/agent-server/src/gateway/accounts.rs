use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use agent_engine::{AccountEngineKind, ComputeEngine, EngineInput, SafeAccountProfile};
use agent_runtime::ExecutionService;

use super::types::AddAccountReq;
use crate::AppState;

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
    state
        .vault
        .register_account(engine_kind, &payload.id, &payload.name, payload.extra_env)
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
    let imported = state
        .vault
        .import_local_credentials(&engine_kind, &id)
        .unwrap_or(false);
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
        AccountEngineKind::Generic(_) => {
            return Json(serde_json::json!({
                "success": false,
                "error": "Account verification is unavailable for generic engines"
            }))
        }
    };

    let input = EngineInput::new("echo verify", &state.workspace);
    let execution = match ExecutionService::from_workspace(&state.workspace) {
        Ok(service) => service,
        Err(err) => return Json(serde_json::json!({"success": false, "error": err.to_string()})),
    };

    let start = std::time::Instant::now();
    match execution.execute(engine_impl, &input).await {
        Ok(output) if output.exit_code == 0 => {
            let latency = start.elapsed().as_millis();
            state.vault.record_success(&engine_kind, &id);
            Json(serde_json::json!({
                "success": true,
                "latency_ms": latency,
                "output": output.stdout_content.trim()
            }))
        }
        result => {
            let error = match result {
                Ok(output) => format!("Engine exited with code {}", output.exit_code),
                Err(err) => err.to_string(),
            };
            state.vault.mark_cooldown(&engine_kind, &id, 60, &error);
            Json(serde_json::json!({
                "success": false,
                "error": error
            }))
        }
    }
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
