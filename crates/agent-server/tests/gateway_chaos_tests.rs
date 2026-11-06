use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

use agent_core::prelude::*;
use agent_engine::AccountEngineKind;
use agent_server::auth::AuthState;
use agent_server::gateway::{
    simulate_gateway_fault, GatewayError, GatewayErrorTier, GatewayFault, GatewayFaultPolicy,
    ResilientGatewayService,
};
use agent_server::{build_app, AppState};

fn create_gateway_test_app() -> Router {
    let bus = InterAgentBus::new(16);
    let temp_vault =
        std::env::temp_dir().join(format!("test_vault_gw_{}", uuid::Uuid::new_v4().simple()));
    let task_store =
        Arc::new(agent_runtime::SqliteTaskStore::memory().expect("sqlite memory store"));
    let task_service =
        Arc::new(agent_runtime::TaskService::new(task_store, "/tmp", 2, None, None));

    build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
            metrics: Arc::new(agent_server::metrics::ResilientMetricsService::default()),
        },
        AuthState::new(Arc::new("test-token".into()), vec![]),
    )
}

async fn spawn_gateway_test_server(app: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

// =========================================================================
// 1. Direct Fault Mapping & Severity Classification
// =========================================================================

#[test]
fn test_gateway_fault_simulation_direct() {
    let test_cases = vec![
        (
            GatewayFault::SocketTimeout,
            GatewayError::SocketTimeout,
            StatusCode::REQUEST_TIMEOUT,
            GatewayErrorTier::TransientNetwork,
            true,
        ),
        (
            GatewayFault::ConnectionReset,
            GatewayError::ConnectionReset,
            StatusCode::BAD_GATEWAY,
            GatewayErrorTier::TransientNetwork,
            true,
        ),
        (
            GatewayFault::NetworkDisconnection,
            GatewayError::NetworkDisconnection(
                "Injected network disconnection: peer disconnected during transfer".to_string(),
            ),
            StatusCode::BAD_GATEWAY,
            GatewayErrorTier::TransientNetwork,
            true,
        ),
        (
            GatewayFault::IoError,
            GatewayError::IoError(
                "Injected I/O error: disk or vault read failed intermittently".to_string(),
            ),
            StatusCode::INTERNAL_SERVER_ERROR,
            GatewayErrorTier::IoFailure,
            true,
        ),
        (
            GatewayFault::StreamInterrupted,
            GatewayError::StreamInterrupted(
                "Injected SSE stream interruption: transmission buffer stalled".to_string(),
            ),
            StatusCode::BAD_GATEWAY,
            GatewayErrorTier::EngineFailure,
            false,
        ),
        (
            GatewayFault::EngineTimeout,
            GatewayError::EngineTimeout(
                "Injected execution timeout: compute engine took too long to complete".to_string(),
            ),
            StatusCode::GATEWAY_TIMEOUT,
            GatewayErrorTier::EngineFailure,
            false,
        ),
        (
            GatewayFault::EngineFailure,
            GatewayError::EngineFailure(
                "Injected engine crash: compute process exited abnormally".to_string(),
            ),
            StatusCode::INTERNAL_SERVER_ERROR,
            GatewayErrorTier::EngineFailure,
            false,
        ),
        (
            GatewayFault::InvalidModel,
            GatewayError::InvalidModel(
                "Injected invalid model specification: model unsupported".to_string(),
            ),
            StatusCode::BAD_REQUEST,
            GatewayErrorTier::ClientProtocol,
            false,
        ),
        (
            GatewayFault::AccountNotFound,
            GatewayError::AccountNotFound(
                "Injected account not found: target account is not registered".to_string(),
            ),
            StatusCode::NOT_FOUND,
            GatewayErrorTier::NotFound,
            false,
        ),
    ];

    for (fault, expected_err, expected_status, expected_tier, expected_retryable) in test_cases {
        assert_eq!(fault.to_gateway_error(), expected_err);
        assert_eq!(simulate_gateway_fault(&fault), Err(expected_err.clone()));
        assert_eq!(expected_err.to_status_code(), expected_status);
        assert_eq!(expected_err.tier(), expected_tier);
        assert_eq!(expected_err.is_retryable(), expected_retryable);
    }

    let exhausted = GatewayError::MaxRetriesExceeded(3);
    assert_eq!(exhausted.tier(), GatewayErrorTier::Exhausted);
    assert_eq!(exhausted.to_status_code(), StatusCode::GATEWAY_TIMEOUT);
    assert!(!exhausted.is_retryable());
}

// =========================================================================
// 2. Gateway Fault Policy Mechanics
// =========================================================================

#[test]
fn test_gateway_fault_policy_behavior() {
    // None policy
    let mut none_policy = GatewayFaultPolicy::none();
    assert_eq!(none_policy.should_inject(), None);
    assert_eq!(none_policy.injected_count(), 0);
    assert_eq!(none_policy.invocation_count(), 0);

    // Always policy
    let mut always_policy = GatewayFaultPolicy::always(GatewayFault::SocketTimeout);
    assert_eq!(
        always_policy.should_inject(),
        Some(GatewayFault::SocketTimeout)
    );
    assert_eq!(
        always_policy.should_inject(),
        Some(GatewayFault::SocketTimeout)
    );
    assert_eq!(always_policy.injected_count(), 2);
    assert_eq!(always_policy.invocation_count(), 2);

    // Intermittent policy (every 3rd invocation: 1st, 4th, 7th...)
    let mut intermittent_policy =
        GatewayFaultPolicy::intermittent(GatewayFault::ConnectionReset, 3);
    assert_eq!(
        intermittent_policy.should_inject(),
        Some(GatewayFault::ConnectionReset)
    ); // #1
    assert_eq!(intermittent_policy.should_inject(), None); // #2
    assert_eq!(intermittent_policy.should_inject(), None); // #3
    assert_eq!(
        intermittent_policy.should_inject(),
        Some(GatewayFault::ConnectionReset)
    ); // #4
    assert_eq!(intermittent_policy.injected_count(), 2);
    assert_eq!(intermittent_policy.invocation_count(), 4);

    // Max injections cap
    let mut capped_policy = GatewayFaultPolicy::always(GatewayFault::IoError).with_max_injections(2);
    assert_eq!(capped_policy.should_inject(), Some(GatewayFault::IoError));
    assert_eq!(capped_policy.should_inject(), Some(GatewayFault::IoError));
    assert_eq!(capped_policy.should_inject(), None);
    assert_eq!(capped_policy.injected_count(), 2);

    // Reset policy
    capped_policy.reset();
    assert_eq!(capped_policy.injected_count(), 0);
    assert_eq!(capped_policy.invocation_count(), 0);
}

// =========================================================================
// 3. Resilient Model Resolution & Fallback
// =========================================================================

#[test]
fn test_resilient_gateway_model_resolution() {
    let service = ResilientGatewayService::new();

    // Standard valid engines
    let (kind, override_model) = service
        .resolve_model_resilient("antigravity")
        .expect("antigravity should resolve");
    assert_eq!(kind, AccountEngineKind::Antigravity);
    assert_eq!(override_model, None);

    let (kind, override_model) = service
        .resolve_model_resilient("gemini/gemini-2.0-flash")
        .expect("gemini path should resolve");
    assert_eq!(kind, AccountEngineKind::Antigravity);
    assert_eq!(override_model.as_deref(), Some("gemini-2.0-flash"));

    let (kind, _) = service
        .resolve_model_resilient("claude")
        .expect("claude should resolve");
    assert_eq!(kind, AccountEngineKind::Claude);

    // Unknown model without fallback
    let err = service.resolve_model_resilient("unsupported-unknown-engine");
    assert!(matches!(err, Err(GatewayError::InvalidModel(_))));

    // Unknown model with valid fallback
    let resilient_with_fallback =
        ResilientGatewayService::new().with_fallback_model("antigravity/gemini-1.5-flash");
    let (kind, override_model) = resilient_with_fallback
        .resolve_model_resilient("unsupported-unknown-engine")
        .expect("should recover via fallback model");
    assert_eq!(kind, AccountEngineKind::Antigravity);
    assert_eq!(override_model.as_deref(), Some("gemini-1.5-flash"));
    assert_eq!(resilient_with_fallback.recovered_count(), 1);

    // Fault injection interrupts model resolution
    let fault_service = ResilientGatewayService::new()
        .with_fault_policy(GatewayFaultPolicy::always(GatewayFault::SocketTimeout));
    let fault_res = fault_service.resolve_model_resilient("antigravity");
    assert_eq!(fault_res, Err(GatewayError::SocketTimeout));
}

// =========================================================================
// 4. Async Retry and Self-Healing
// =========================================================================

#[tokio::test]
async fn test_resilient_gateway_async_retry_and_recovery() {
    // 1. Recover from transient socket timeout after 2 injected attempts
    let policy =
        GatewayFaultPolicy::always(GatewayFault::SocketTimeout).with_max_injections(2);
    let service = ResilientGatewayService::new()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(1))
        .with_fault_policy(policy);

    let mut executed_ops = 0;
    let res = service
        .execute_async_resilient(|| {
            executed_ops += 1;
            async move { Ok("operation success") }
        })
        .await;

    assert_eq!(res, Ok("operation success"));
    assert_eq!(service.recovered_count(), 1);
    assert_eq!(service.retries_attempted(), 2);

    // 2. Non-retryable fault (InvalidModel) returns immediately without retrying
    let non_retryable_policy =
        GatewayFaultPolicy::always(GatewayFault::InvalidModel).with_max_injections(3);
    let service = ResilientGatewayService::new()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(1))
        .with_fault_policy(non_retryable_policy);

    let res = service
        .execute_async_resilient(|| async move { Ok("should not happen") })
        .await;

    assert!(matches!(res, Err(GatewayError::InvalidModel(_))));
    assert_eq!(service.retries_attempted(), 0);
    assert_eq!(service.recovered_count(), 0);

    // 3. Exhaustion of retry budget produces MaxRetriesExceeded
    let exhausting_policy = GatewayFaultPolicy::always(GatewayFault::IoError);
    let service = ResilientGatewayService::new()
        .with_max_retries(2)
        .with_backoff(Duration::from_millis(1))
        .with_fault_policy(exhausting_policy);

    let res = service
        .execute_async_resilient(|| async move { Ok("never succeeds") })
        .await;

    assert_eq!(res, Err(GatewayError::MaxRetriesExceeded(2)));
    assert_eq!(service.retries_attempted(), 2);
}

// =========================================================================
// 5. Synchronous Retry and Self-Healing
// =========================================================================

#[test]
fn test_resilient_gateway_sync_retry_and_recovery() {
    // 1. Recover from transient I/O error
    let policy = GatewayFaultPolicy::always(GatewayFault::IoError).with_max_injections(1);
    let service = ResilientGatewayService::new()
        .with_max_retries(2)
        .with_backoff(Duration::from_millis(1))
        .with_fault_policy(policy);

    let mut counter = 0;
    let res = service.execute_sync_resilient(|| {
        counter += 1;
        Ok(format!("sync account loaded #{counter}"))
    });

    assert_eq!(res, Ok("sync account loaded #1".to_string()));
    assert_eq!(service.recovered_count(), 1);
    assert_eq!(service.retries_attempted(), 1);

    // 2. Non-retryable fault immediately returns
    let fail_policy = GatewayFaultPolicy::always(GatewayFault::AccountNotFound);
    let service = ResilientGatewayService::new()
        .with_max_retries(3)
        .with_fault_policy(fail_policy);

    let res = service.execute_sync_resilient(|| Ok("unreachable"));
    assert!(matches!(res, Err(GatewayError::AccountNotFound(_))));
    assert_eq!(service.retries_attempted(), 0);
}

// =========================================================================
// 6. HTTP Endpoints with Fault Injection & Verification
// =========================================================================

#[tokio::test]
async fn test_gateway_http_models_and_invalid_completions() {
    let app = create_gateway_test_app();

    // 1. GET /v1/models returns 200 with list of models
    let req = Request::builder()
        .uri("/v1/models")
        .method("GET")
        .header("authorization", "Bearer test-token")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 2. POST /v1/chat/completions with unsupported model returns 400 Bad Request
    let req_invalid = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("authorization", "Bearer test-token")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "model": "unsupported-model-x",
                "messages": [{ "role": "user", "content": "hi" }]
            })
            .to_string(),
        ))
        .unwrap();

    let res_invalid = app.oneshot(req_invalid).await.unwrap();
    assert_eq!(res_invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_gateway_live_server_ephemeral_binding() {
    let app = create_gateway_test_app();
    let addr = spawn_gateway_test_server(app).await;
    assert!(addr.port() > 0);

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let req = format!(
        "GET /v1/models HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer test-token\r\nConnection: close\r\n\r\n",
        addr.port()
    );
    stream.write_all(req.as_bytes()).await.expect("write req");

    let mut response_buf = String::new();
    stream
        .read_to_string(&mut response_buf)
        .await
        .expect("read response");

    assert!(response_buf.contains("HTTP/1.1 200 OK"));
    assert!(response_buf.contains("\"object\":\"list\""));
    assert!(response_buf.contains("antigravity"));
}

#[tokio::test]
async fn test_gateway_raw_socket_abrupt_disconnect() {
    let app = create_gateway_test_app();
    let addr = spawn_gateway_test_server(app).await;

    // Client connects and sends partial bytes then aborts abruptly
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .await
        .expect("write partial header");
    drop(stream);

    tokio::time::sleep(Duration::from_millis(20)).await;

    // Next connection succeeds without daemon hang or crash
    let mut stream2 = TcpStream::connect(addr).await.expect("connect stream 2");
    let req2 = format!(
        "GET /v1/models HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer test-token\r\nConnection: close\r\n\r\n",
        addr.port()
    );
    stream2.write_all(req2.as_bytes()).await.expect("write req 2");
    let mut response2 = String::new();
    stream2.read_to_string(&mut response2).await.expect("read 2");
    assert!(response2.contains("HTTP/1.1 200 OK"));
}

// =========================================================================
// 7. Concurrent Chaos Stress
// =========================================================================

#[tokio::test]
async fn test_gateway_concurrent_chaos_stress() {
    // Intermittent fault injected every 3rd call, capped at 10 injections
    let policy =
        GatewayFaultPolicy::intermittent(GatewayFault::SocketTimeout, 3).with_max_injections(10);
    let service = Arc::new(
        ResilientGatewayService::new()
            .with_max_retries(4)
            .with_backoff(Duration::from_millis(1))
            .with_fault_policy(policy),
    );

    let mut tasks = Vec::new();
    for i in 0..25 {
        let svc = service.clone();
        tasks.push(tokio::spawn(async move {
            svc.execute_async_resilient(|| async move { Ok(format!("task_{i}_done")) })
                .await
        }));
    }

    let mut success_count = 0;
    for task in tasks {
        let outcome = task.await.expect("task join");
        assert!(outcome.is_ok(), "Every task should heal and succeed: {outcome:?}");
        success_count += 1;
    }

    assert_eq!(success_count, 25);
    assert!(service.retries_attempted() > 0, "Should have attempted retries");
    assert!(service.recovered_count() > 0, "Should have recovered from faults");
}

// =========================================================================
// 8. Root Cause Elimination Test for TOPIC-20261105-61e62e & TOPIC-20261105-dde2fc
// =========================================================================

#[tokio::test]
async fn test_topic_61e62e_dde2fc_gateway_chaos_resilience() {
    // 1. Verify LOC compliance across all modular gateway components (< 350 lines P90)
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let gateway_dir = manifest_dir.join("src/gateway");
    assert!(
        gateway_dir.is_dir(),
        "src/gateway must exist as a modular directory"
    );

    for entry in std::fs::read_dir(&gateway_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let content = std::fs::read_to_string(&path).unwrap();
            let lines = content.lines().count();
            assert!(
                lines <= 350,
                "Submodule {} has {} lines, which exceeds P90 limit (350 lines)",
                path.file_name().unwrap().to_str().unwrap(),
                lines
            );
        }
    }

    // 2. Fault Policy Parity with Auth & Dashboard_html
    let mut policy = GatewayFaultPolicy::always(GatewayFault::IoError).with_max_injections(1);
    assert_eq!(policy.should_inject(), Some(GatewayFault::IoError));
    assert_eq!(policy.should_inject(), None);
    assert_eq!(policy.injected_count(), 1);
    policy.reset();
    assert_eq!(policy.injected_count(), 0);

    // 3. Resilient Service Self-Healing under Intermittent Faults
    let recovery_policy =
        GatewayFaultPolicy::intermittent(GatewayFault::SocketTimeout, 2).with_max_injections(1);
    let service = ResilientGatewayService::new()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(1))
        .with_fault_policy(recovery_policy)
        .with_fallback_model("antigravity/gemini-2.0-flash");

    let res = service
        .execute_async_resilient(|| async { Ok("resilient response") })
        .await;
    assert!(
        res.is_ok(),
        "Gateway service must recover from transient socket timeout"
    );
    assert_eq!(res.unwrap(), "resilient response");

    // 4. Fallback Model Resolution Resiliency
    let (kind, override_model) = service
        .resolve_model_resilient("non-existent-arbitrary-model")
        .expect("should gracefully fallback to gemini-2.0-flash");
    assert_eq!(kind, AccountEngineKind::Antigravity);
    assert_eq!(override_model.as_deref(), Some("gemini-2.0-flash"));

    // 5. Severity Tiers and Status Codes
    assert_eq!(
        GatewayError::SocketTimeout.tier(),
        GatewayErrorTier::TransientNetwork
    );
    assert_eq!(
        GatewayError::SocketTimeout.to_status_code(),
        StatusCode::REQUEST_TIMEOUT
    );
    assert!(GatewayError::SocketTimeout.is_retryable());

    assert_eq!(
        GatewayError::IoError("test".into()).tier(),
        GatewayErrorTier::IoFailure
    );
    assert_eq!(
        GatewayError::IoError("test".into()).to_status_code(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(GatewayError::IoError("test".into()).is_retryable());

    assert_eq!(
        GatewayError::StreamInterrupted("test".into()).tier(),
        GatewayErrorTier::EngineFailure
    );
    assert_eq!(
        GatewayError::StreamInterrupted("test".into()).to_status_code(),
        StatusCode::BAD_GATEWAY
    );
    assert!(!GatewayError::StreamInterrupted("test".into()).is_retryable());

    assert_eq!(
        GatewayError::InvalidModel("test".into()).tier(),
        GatewayErrorTier::ClientProtocol
    );
    assert_eq!(
        GatewayError::InvalidModel("test".into()).to_status_code(),
        StatusCode::BAD_REQUEST
    );
    assert!(!GatewayError::InvalidModel("test".into()).is_retryable());
}
