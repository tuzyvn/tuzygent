#[cfg(unix)]
#[tokio::test]
async fn authenticated_task_delegates_to_codex_via_rest() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use agent_core::prelude::*;
    use axum::{body::Body, http::Request, http::StatusCode};
    use tower::ServiceExt;
    use crate::{build_app, AppState, AuthState};

    let workspace =
        std::env::temp_dir().join(format!("tuzygent-rest-codex-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace)?;
    let binary = workspace.join("codex");
    std::fs::write(
        &binary,
        include_str!("../../../agent-engine/tests/fixtures/codex.sh"),
    )?;
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    let binary_str = binary
        .to_str()
        .ok_or("Failed to convert binary path to str")?;
    std::fs::write(
        workspace.join("tuzygent.toml"),
        format!(
            r#"
        [defaults]
        sandbox_driver = "host"
        timeout_seconds = 5
        [engines.codex]
        kind = "codex"
        binary_path = {:?}
        [engines.codex.env]
        CODEX_TEST_CASE = "ooda"
    "#,
            binary_str
        ),
    )?;
    let bus = InterAgentBus::new(32);
    let temp_vault = std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4().simple()));
    let task_store = Arc::new(agent_runtime::SqliteTaskStore::memory()?);
    let workspace_str = workspace
        .to_str()
        .ok_or("Failed to convert workspace path to str")?;
    let task_service = Arc::new(agent_runtime::TaskService::new(task_store, workspace_str, 2, None, None));
    let app = build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: workspace_str.into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
        },
        AuthState {
            token: Arc::new("test-secret".into()),
            allowed_origins: vec![],
        },
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/tasks/run")
        .header("authorization", "Bearer test-secret")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"agent_id":"dev","goal":"delegate task","engine":"codex"}"#,
        ))?;
    let response = app.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 8192).await?;
    let result: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(result["engine_used"], "codex");
    assert_eq!(result["status"], "Completed");
    assert_eq!(result["subtasks_count"], 1);
    std::fs::remove_dir_all(workspace)?;
    Ok(())
}
