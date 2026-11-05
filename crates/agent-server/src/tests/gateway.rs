use axum::{body::Body, http::Request, http::StatusCode};
use tower::ServiceExt;

use super::common::app;

#[tokio::test]
async fn authenticated_gateway_lists_models_and_manages_accounts() -> Result<(), Box<dyn std::error::Error>> {
    let app = app()?;
    // 1. GET /v1/models
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // 2. POST /api/v1/auth/accounts
    let add_acc = serde_json::json!({
        "engine": "antigravity",
        "id": "acc_test_1",
        "name": "Test Google Account",
        "extra_env": { "TEST_VAR": "1" }
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/accounts")
                .header("authorization", "Bearer test-secret")
                .header("content-type", "application/json")
                .body(Body::from(add_acc.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // 3. GET /api/v1/auth/accounts (verifying secrets are redacted)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/accounts")
                .header("authorization", "Bearer test-secret")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 8192).await?;
    let json_body: serde_json::Value = serde_json::from_slice(&body)?;
    let accounts = json_body["accounts"]
        .as_array()
        .ok_or("Expected accounts array in json body")?;
    assert_eq!(accounts.len(), 1);
    assert!(accounts[0].get("extra_env").is_none());
    assert!(accounts[0].get("extra_env_keys").is_some());

    // 4. Reject account with path traversal ID
    let bad_acc = serde_json::json!({
        "engine": "antigravity",
        "id": "../evil",
        "name": "Bad Traversal Account",
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/accounts")
                .header("authorization", "Bearer test-secret")
                .header("content-type", "application/json")
                .body(Body::from(bad_acc.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // 5. Reject unknown model without spawning process
    let bad_model_req = serde_json::json!({
        "model": "malicious_unrecognized_binary",
        "messages": [{"role": "user", "content": "hi"}],
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer test-secret")
                .header("content-type", "application/json")
                .body(Body::from(bad_model_req.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // 6. Test async task submission returning 202 Accepted
    let submit_req = serde_json::json!({
        "title": "Async Test Task",
        "goal": "Test 202 pattern",
        "priority": "normal"
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/tasks")
                .header("authorization", "Bearer test-secret")
                .header("content-type", "application/json")
                .body(Body::from(submit_req.to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(response.into_body(), 8192).await?;
    let body_json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(body_json["status"], "queued");
    assert!(body_json["task_id"].as_str().is_some());
    Ok(())
}
