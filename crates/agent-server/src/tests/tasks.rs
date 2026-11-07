#[cfg(unix)]
#[tokio::test]
async fn authenticated_task_delegates_to_codex_via_rest() -> Result<(), Box<dyn std::error::Error>>
{
    use crate::{AppState, AuthState, build_app};
    use agent_core::prelude::*;
    use axum::{body::Body, http::Request, http::StatusCode};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use tower::ServiceExt;

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
    let temp_vault =
        std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4().simple()));
    let task_store = Arc::new(agent_runtime::SqliteTaskStore::memory()?);
    let workspace_str = workspace
        .to_str()
        .ok_or("Failed to convert workspace path to str")?;
    let task_service = Arc::new(agent_runtime::TaskService::new(
        task_store,
        workspace_str,
        2,
        None,
        None,
    ));
    let app = build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: workspace_str.into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
            metrics: Arc::new(crate::metrics::ResilientMetricsService::default()),
        },
        AuthState::new(Arc::new("test-secret".into()), vec![]),
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
#[tokio::test]
async fn invalid_engine_does_not_create_a_queued_task() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let app = super::common::app().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/tasks")
                .header("authorization", "Bearer test-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"title":"test","goal":"test","engine":"unknown-not-configured"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tasks")
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["tasks"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn idempotency_key_replays_and_rejects_conflicting_requests() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let app = super::common::app().unwrap();
    let request = |goal: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/tasks")
            .header("authorization", "Bearer test-secret")
            .header("content-type", "application/json")
            .header("idempotency-key", "test-request-123")
            .body(Body::from(
                serde_json::json!({"title":"task", "goal":goal}).to_string(),
            ))
            .unwrap()
    };
    let first = app.clone().oneshot(request("same goal")).await.unwrap();
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first: serde_json::Value =
        serde_json::from_slice(&axum::body::to_bytes(first.into_body(), 8192).await.unwrap())
            .unwrap();
    let second = app.clone().oneshot(request("same goal")).await.unwrap();
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let second: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(second.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["task_id"], second["task_id"]);
    assert_eq!(second["replayed"], true);
    let conflict = app.oneshot(request("different goal")).await.unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn manual_review_endpoint_requires_auth_and_current_revision() {
    use crate::{AppState, AuthState, build_app};
    use agent_core::prelude::*;
    use agent_runtime::TaskStore;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use std::sync::Arc;
    use tower::ServiceExt;
    let store = Arc::new(agent_runtime::SqliteTaskStore::memory().unwrap());
    let service = Arc::new(agent_runtime::TaskService::new(
        store.clone(),
        "/tmp",
        1,
        None,
        None,
    ));
    let mut task = Task::new("review", "review", Priority::Normal);
    let reason = "Manual verification required".to_string();
    task.status = TaskStatus::WaitingApproval {
        reason: reason.clone(),
    };
    let mut step = SubTask::new("one", "report", "generate", "review");
    step.status = TaskStatus::WaitingApproval { reason };
    step.output_excerpt = Some("report".into());
    task.subtasks.push(step);
    store.save_task(&task).await.unwrap();
    store
        .save_checkpoint(&task.id, &task.subtasks)
        .await
        .unwrap();
    let bus = InterAgentBus::new(16);
    let app = build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(
                std::env::temp_dir().join(format!("vault-{}", uuid::Uuid::new_v4())),
            )),
            task_service: service,
            metrics: Arc::new(crate::metrics::ResilientMetricsService::default()),
        },
        AuthState::new(Arc::new("test-secret".into()), vec![]),
    );
    let path = format!("/api/v1/tasks/{}", task.id);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    let revision = body["review_revision"].as_str().unwrap();
    let review_body =
        serde_json::json!({"revision":revision,"reviewer":"operator","approved":true}).to_string();
    let request = |auth: bool| {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("{path}/review"))
            .header("content-type", "application/json");
        if auth {
            builder = builder.header("authorization", "Bearer test-secret");
        }
        builder.body(Body::from(review_body.clone())).unwrap()
    };
    assert_eq!(
        app.clone().oneshot(request(false)).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.clone().oneshot(request(true)).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(request(true)).await.unwrap().status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn cancel_task_endpoint_cancels_queued_task_and_requires_auth() {
    use crate::{AppState, AuthState, build_app};
    use agent_core::prelude::*;
    use agent_runtime::TaskStore;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    let store = Arc::new(agent_runtime::SqliteTaskStore::memory().unwrap());
    let service = Arc::new(agent_runtime::TaskService::new(
        store.clone(),
        "/tmp",
        1,
        None,
        None,
    ));
    let task = Task::new("cancellable task", "will be canceled", Priority::Normal);
    store.save_task(&task).await.unwrap();

    let bus = InterAgentBus::new(16);
    let app = build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(
                std::env::temp_dir().join(format!("vault-{}", uuid::Uuid::new_v4())),
            )),
            task_service: service,
            metrics: Arc::new(crate::metrics::ResilientMetricsService::default()),
        },
        AuthState::new(Arc::new("test-secret".into()), vec![]),
    );

    let cancel_uri = format!("/api/v1/tasks/{}/cancel", task.id);

    // Unauthenticated request rejected
    let unauth_req = Request::builder()
        .method("POST")
        .uri(&cancel_uri)
        .body(Body::empty())
        .unwrap();
    let unauth_resp = app.clone().oneshot(unauth_req).await.unwrap();
    assert_eq!(unauth_resp.status(), StatusCode::UNAUTHORIZED);

    // Authenticated request cancels task
    let auth_req = Request::builder()
        .method("POST")
        .uri(&cancel_uri)
        .header("authorization", "Bearer test-secret")
        .body(Body::empty())
        .unwrap();
    let auth_resp = app.clone().oneshot(auth_req).await.unwrap();
    assert_eq!(auth_resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(auth_resp.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["canceled"], true);

    // Re-canceling is idempotent and succeeds
    let re_cancel_req = Request::builder()
        .method("POST")
        .uri(&cancel_uri)
        .header("authorization", "Bearer test-secret")
        .body(Body::empty())
        .unwrap();
    let re_cancel_resp = app.clone().oneshot(re_cancel_req).await.unwrap();
    assert_eq!(re_cancel_resp.status(), StatusCode::OK);

    // Canceling a non-existent task returns 404 Not Found
    let not_found_req = Request::builder()
        .method("POST")
        .uri("/api/v1/tasks/non-existent-task-xyz/cancel")
        .header("authorization", "Bearer test-secret")
        .body(Body::empty())
        .unwrap();
    let not_found_resp = app.oneshot(not_found_req).await.unwrap();
    assert_eq!(not_found_resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn run_task_handler_rejects_an_unknown_engine() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let app = super::common::app().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/tasks/run")
                .header("authorization", "Bearer test-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"agent_id":"test","goal":"test","engine":"missing-engine"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn submit_task_handler_rejects_an_invalid_idempotency_key() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let app = super::common::app().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/tasks")
                .header("authorization", "Bearer test-secret")
                .header("content-type", "application/json")
                .header("idempotency-key", "invalid key")
                .body(Body::from(r#"{"title":"test","goal":"test"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_tasks_handler_returns_an_empty_task_list() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let app = super::common::app().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tasks")
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["tasks"], serde_json::json!([]));
}

#[tokio::test]
async fn get_task_handler_returns_not_found_for_an_unknown_task() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let app = super::common::app().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tasks/missing-task")
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn review_task_handler_returns_not_found_for_an_unknown_task() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let app = super::common::app().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/tasks/missing-task/review")
                .header("authorization", "Bearer test-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"revision":"revision","reviewer":"operator","approved":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancel_task_handler_returns_not_found_for_an_unknown_task() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let app = super::common::app().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/tasks/missing-task/cancel")
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn approve_task_handler_returns_not_found_for_an_unknown_task() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let app = super::common::app().unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/tasks/missing-task/approve")
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn approve_task_handler_rejects_a_task_that_is_not_waiting_for_approval() {
    use crate::{AppState, AuthState, build_app};
    use agent_core::prelude::*;
    use agent_runtime::TaskStore;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    let store = Arc::new(agent_runtime::SqliteTaskStore::memory().unwrap());
    let service = Arc::new(agent_runtime::TaskService::new(
        store.clone(),
        "/tmp",
        1,
        None,
        None,
    ));
    let task = Task::new("not awaiting approval", "task goal", Priority::Normal);
    let task_id = task.id.to_string();
    store.save_task(&task).await.unwrap();

    let bus = InterAgentBus::new(16);
    let app = build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(
                std::env::temp_dir().join(format!("vault-{}", uuid::Uuid::new_v4())),
            )),
            task_service: service,
            metrics: Arc::new(crate::metrics::ResilientMetricsService::default()),
        },
        AuthState::new(Arc::new("test-secret".into()), vec![]),
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/tasks/{task_id}/approve"))
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("not in WaitingApproval status")
    );
}
