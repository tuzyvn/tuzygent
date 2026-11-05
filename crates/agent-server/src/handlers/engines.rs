use agent_core::prelude::*;
use axum::{extract::State, response::IntoResponse, Json};

use crate::state::AppState;

pub async fn list_engines_handler(State(state): State<AppState>) -> impl IntoResponse {
    let ws_path = std::path::Path::new(&state.workspace);
    let config = EnginesConfig::load_hierarchical(Some(ws_path));
    let detected_driver = SandboxConfig::detect_best_driver();

    Json(serde_json::json!({
        "default_engine": config.defaults.engine,
        "default_timeout_seconds": config.defaults.timeout_seconds,
        "sandbox_driver_configured": config.defaults.sandbox_driver,
        "sandbox_driver_detected": detected_driver.to_string(),
        "engines": config.engines,
        "security": {
            "allowed_env_vars": config.security.allowed_env_vars,
            "mount_credentials": config.security.mount_credentials,
        }
    }))
}
