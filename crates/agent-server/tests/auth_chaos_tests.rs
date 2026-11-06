use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode};
use axum::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

use agent_core::prelude::*;
use agent_server::auth::{
    extract_bearer_token, extract_ws_token, simulate_auth_fault, AuthBenchmarkHarness,
    AuthBenchmarkStats, AuthError, AuthFault, AuthFaultPolicy, AuthState, ConcurrentAuthCache,
    ResilientAuthValidator, ShardedAuthCache, ShardedResilientAuthValidator,
};
use agent_server::{build_app, AppState};

fn create_chaos_test_app(token: &str, allowed_origins: Vec<&str>) -> Router {
    let bus = InterAgentBus::new(16);
    let temp_vault =
        std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4().simple()));
    let task_store =
        Arc::new(agent_runtime::SqliteTaskStore::memory().expect("sqlite memory store"));
    let task_service =
        Arc::new(agent_runtime::TaskService::new(task_store, "/tmp", 2, None, None));
    let origins = allowed_origins
        .into_iter()
        .map(|o| o.parse().expect("valid header value"))
        .collect();
    build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
            metrics: Arc::new(agent_server::metrics::ResilientMetricsService::default()),
        },
        AuthState::new(Arc::new(token.into()), origins),
    )
}

fn create_chaos_app_with_policy(
    token: &str,
    allowed_origins: Vec<&str>,
    policy: AuthFaultPolicy,
) -> Router {
    let bus = InterAgentBus::new(16);
    let temp_vault =
        std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4().simple()));
    let task_store =
        Arc::new(agent_runtime::SqliteTaskStore::memory().expect("sqlite memory store"));
    let task_service =
        Arc::new(agent_runtime::TaskService::new(task_store, "/tmp", 2, None, None));
    let origins = allowed_origins
        .into_iter()
        .map(|o| o.parse().expect("valid header value"))
        .collect();
    build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
            metrics: Arc::new(agent_server::metrics::ResilientMetricsService::default()),
        },
        AuthState::new(Arc::new(token.into()), origins).with_fault_policy(policy),
    )
}

// =========================================================================
// 1. Direct fault mapping & simulation tests
// =========================================================================

#[test]
fn test_auth_fault_simulation_direct() {
    let test_cases = vec![
        (AuthFault::SocketTimeout, AuthError::SocketTimeout),
        (AuthFault::ConnectionReset, AuthError::ConnectionReset),
        (
            AuthFault::IntermittentIo,
            AuthError::IntermittentIo("Transient I/O failure during auth stream".to_string()),
        ),
        (
            AuthFault::TokenDecodingFailure,
            AuthError::TokenDecodingFailure("Failed to decode token from request".to_string()),
        ),
        (
            AuthFault::MalformedHeader,
            AuthError::MalformedHeader("Malformed authorization header structure".to_string()),
        ),
    ];

    for (fault, expected_err) in test_cases {
        assert_eq!(fault.to_auth_error(), expected_err);
        assert_eq!(simulate_auth_fault(&fault).unwrap_err(), expected_err);
    }
}

// =========================================================================
// 2. ResilientAuthValidator TDD unit tests
// =========================================================================

#[test]
fn test_auth_resilience_intermittent_socket_timeout_recovery() {
    let policy = AuthFaultPolicy::intermittent(AuthFault::SocketTimeout, 2);
    let mut validator = ResilientAuthValidator::new(
        Arc::new("super-secret-token".into()),
        vec![],
        policy,
        3,
    );

    let res = validator.validate_token_resilient("super-secret-token");
    assert!(res.is_ok(), "Validator must recover from intermittent socket timeout: {:?}", res);
    assert!(res.unwrap());
    assert_eq!(validator.validated_count(), 1);
    assert!(validator.recovered_count() > 0);
}

#[test]
fn test_auth_resilience_intermittent_connection_reset_recovery() {
    let policy = AuthFaultPolicy::intermittent(AuthFault::ConnectionReset, 2);
    let mut validator = ResilientAuthValidator::new(
        Arc::new("super-secret-token".into()),
        vec![],
        policy,
        3,
    );

    let res = validator.validate_token_resilient("super-secret-token");
    assert!(res.is_ok(), "Validator must recover from connection reset: {:?}", res);
    assert!(res.unwrap());
    assert!(validator.recovered_count() > 0);
}

#[test]
fn test_auth_resilience_intermittent_io_recovery() {
    let policy = AuthFaultPolicy::intermittent(AuthFault::IntermittentIo, 2);
    let mut validator = ResilientAuthValidator::new(
        Arc::new("super-secret-token".into()),
        vec![],
        policy,
        3,
    );

    let res = validator.validate_token_resilient("super-secret-token");
    assert!(res.is_ok(), "Validator must recover from intermittent I/O: {:?}", res);
    assert!(res.unwrap());
    assert!(validator.recovered_count() > 0);
}

#[test]
fn test_auth_resilience_persistent_timeout_max_retries_exceeded() {
    let policy = AuthFaultPolicy::always(AuthFault::SocketTimeout);
    let mut validator = ResilientAuthValidator::new(
        Arc::new("super-secret-token".into()),
        vec![],
        policy,
        2,
    );

    let res = validator.validate_token_resilient("super-secret-token");
    assert_eq!(res, Err(AuthError::MaxRetriesExceeded(2)));
    assert_eq!(validator.validated_count(), 0);
    assert_eq!(validator.rejected_count(), 1);
}

// =========================================================================
// 3. Token decoding failure & malformed header tests
// =========================================================================

#[tokio::test]
async fn test_token_decoding_failure_corrupted_utf8_in_bearer() {
    let app = create_chaos_test_app("valid-token", vec![]);

    // Header with invalid non-ASCII byte sequence [0xFF, 0xFE, 0xFD]
    let invalid_bytes = vec![0xFF, 0xFE, 0xFD];
    let header_val = HeaderValue::from_bytes(&invalid_bytes).expect("valid header bytes");

    let req = Request::builder()
        .uri("/api/v1/agents")
        .header("authorization", header_val)
        .body(Body::empty())
        .expect("request build");

    // Direct extraction failure test
    let err = extract_bearer_token(&req).expect_err("should fail decoding invalid UTF-8 bytes");
    assert!(matches!(err, AuthError::TokenDecodingFailure(_)));

    // Axum app endpoint test: should safely return 401 Unauthorized without panicking
    let response = app.oneshot(req).await.expect("app response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_token_decoding_failure_corrupted_utf8_in_websocket() {
    let app = create_chaos_test_app("valid-token", vec![]);

    let invalid_bytes = vec![0xC0, 0xAF]; // Invalid UTF-8 sequence
    let header_val = HeaderValue::from_bytes(&invalid_bytes).expect("valid header bytes");

    let req = Request::builder()
        .uri("/api/v1/ws")
        .header("sec-websocket-protocol", header_val)
        .body(Body::empty())
        .expect("request build");

    let err = extract_ws_token(&req).expect_err("should fail decoding invalid websocket protocol");
    assert!(matches!(err, AuthError::TokenDecodingFailure(_)));

    let response = app.oneshot(req).await.expect("app response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_token_decoding_failure_malformed_schemes_and_empty_tokens() {
    let app = create_chaos_test_app("valid-token", vec![]);

    let malformed_headers = vec![
        ("Bearer ", "empty token after Bearer"),
        ("Basic dXNlcjpwYXNz", "wrong authentication scheme"),
        ("Token valid-token", "non-standard token prefix"),
        ("Bearer  valid-token", "double space after Bearer"),
        ("bearer valid-token", "lowercase bearer"),
        ("BEARER valid-token", "uppercase bearer"),
        ("Bearer\tvalid-token", "tab delimiter instead of space"),
        ("Bearer", "missing space and token"),
        ("", "empty authorization value"),
    ];

    for (header, description) in malformed_headers {
        let req = Request::builder()
            .uri("/api/v1/agents")
            .header("authorization", header)
            .body(Body::empty())
            .expect("request build");

        let err = extract_bearer_token(&req);
        assert!(
            err.is_err(),
            "Expected error for '{}' ({}), got {:?}",
            header,
            description,
            err
        );

        let response = app.clone().oneshot(req).await.expect("app response");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "Failed for '{}' ({})",
            header,
            description
        );
    }
}

#[tokio::test]
async fn test_websocket_subprotocol_edge_cases() {
    let app = create_chaos_test_app("valid-token", vec![]);

    let bad_protocols = vec![
        "tuzygent",                         // missing token
        "tuzygent, ",                       // empty token
        "other, valid-token",               // wrong subprotocol identifier
        "tuzygent, valid-token, extra",     // too many parts
        "valid-token, tuzygent",            // reversed order
        ",,,",                              // commas only
        "tuzygent,valid-token,extra,param", // excessive parts
    ];

    for proto in bad_protocols {
        let req = Request::builder()
            .uri("/api/v1/ws")
            .header("sec-websocket-protocol", proto)
            .body(Body::empty())
            .expect("request build");

        let err = extract_ws_token(&req);
        assert!(err.is_err(), "Expected error for protocol '{}'", proto);

        let response = app.clone().oneshot(req).await.expect("app response");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "Protocol '{}' should be rejected",
            proto
        );
    }
}

// =========================================================================
// 4. Real TCP Socket chaos tests (Timeout, Choppy I/O, Abrupt RST)
// =========================================================================

async fn spawn_test_server(app: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

#[tokio::test]
async fn test_real_socket_timeout_stalled_client() {
    let app = create_chaos_test_app("test-secret", vec![]);
    let addr = spawn_test_server(app).await;

    // Connect raw TCP socket and stall midway through headers
    let mut stream = TcpStream::connect(addr).await.expect("connect to server");
    stream
        .write_all(b"GET /api/v1/agents HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .await
        .expect("write partial header");

    // Client simulated timeout: pause without sending complete header
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Abort stalled client stream
    drop(stream);

    // Verify server remains completely responsive to healthy subsequent requests
    let mut healthy_stream = TcpStream::connect(addr).await.expect("connect healthy");
    let request_data = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    healthy_stream.write_all(request_data).await.expect("write");

    let mut response_buf = Vec::new();
    healthy_stream
        .read_to_end(&mut response_buf)
        .await
        .expect("read response");
    let response_str = String::from_utf8_lossy(&response_buf);
    assert!(
        response_str.starts_with("HTTP/1.1 200 OK"),
        "Server must remain responsive after client timeout: {}",
        response_str
    );
}

#[tokio::test]
async fn test_real_socket_abrupt_disconnect_rst() {
    let app = create_chaos_test_app("test-secret", vec![]);
    let addr = spawn_test_server(app).await;

    // Open connection, send partial auth header, abruptly drop stream
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let _ = stream
        .write_all(b"GET /api/v1/agents HTTP/1.1\r\nAuthorization: Bear")
        .await;
    drop(stream); // Abrupt connection drop mid-header

    tokio::time::sleep(Duration::from_millis(30)).await;

    // Subsequent valid authenticated request must succeed
    let mut valid_stream = TcpStream::connect(addr).await.expect("connect");
    let req = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    valid_stream.write_all(req).await.expect("write");

    let mut res_buf = Vec::new();
    valid_stream.read_to_end(&mut res_buf).await.expect("read");
    let res = String::from_utf8_lossy(&res_buf);
    assert!(res.starts_with("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn test_real_socket_choppy_fragmented_delivery() {
    let app = create_chaos_test_app("secret-stream-token", vec![]);
    let addr = spawn_test_server(app).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request_payload = b"GET /api/v1/engines HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer secret-stream-token\r\nConnection: close\r\n\r\n";

    // Write request in tiny 2-byte fragments with short delays simulating choppy network jitter
    for chunk in request_payload.chunks(2) {
        stream.write_all(chunk).await.expect("write chunk");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    let mut response_buf = Vec::new();
    stream
        .read_to_end(&mut response_buf)
        .await
        .expect("read response");
    let response_str = String::from_utf8_lossy(&response_buf);

    assert!(
        response_str.starts_with("HTTP/1.1 200 OK"),
        "Fragmented stream must be reassembled and authenticated: {}",
        response_str
    );
}

#[tokio::test]
async fn test_real_socket_illegal_raw_bytes() {
    let app = create_chaos_test_app("secret-token", vec![]);
    let addr = spawn_test_server(app).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    // Raw binary byte garbage in authorization header line
    let garbage_request = b"GET /api/v1/agents HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: \xff\xfe\x00\r\n\r\n";
    let _ = stream.write_all(garbage_request).await;

    let mut response_buf = Vec::new();
    let _ = stream.read_to_end(&mut response_buf).await;
    let response_str = String::from_utf8_lossy(&response_buf);

    // Axum/hyper should either send 400 Bad Request or close connection, never panic
    assert!(
        response_str.starts_with("HTTP/1.1 400")
            || response_str.starts_with("HTTP/1.1 401")
            || response_str.is_empty(),
        "Unexpected response: {}",
        response_str
    );
}

// =========================================================================
// 5. Concurrency & middleware fault injection tests
// =========================================================================

#[tokio::test]
async fn test_chaos_concurrent_requests_under_fault_and_valid_load() {
    let app = create_chaos_test_app("shared-concurrent-token", vec![]);
    let addr = spawn_test_server(app).await;

    let mut handles = Vec::new();

    // 10 Valid requests
    for _ in 0..10 {
        handles.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let req = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
            stream.write_all(req).await.unwrap();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            let res = String::from_utf8_lossy(&buf);
            assert!(res.starts_with("HTTP/1.1 200 OK"));
        }));
    }

    // 10 Invalid token requests
    for _ in 0..10 {
        handles.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let req = b"GET /api/v1/agents HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer wrong-token\r\nConnection: close\r\n\r\n";
            stream.write_all(req).await.unwrap();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            let res = String::from_utf8_lossy(&buf);
            assert!(res.starts_with("HTTP/1.1 401"));
        }));
    }

    // 10 Abruptly dropped connections
    for _ in 0..10 {
        handles.push(tokio::spawn(async move {
            if let Ok(mut stream) = TcpStream::connect(addr).await {
                let _ = stream.write_all(b"GET /api/v1/agents HTTP/1.1\r\n").await;
                // Abrupt drop
            }
        }));
    }

    for handle in handles {
        handle.await.expect("task join");
    }

    // Verify system health after concurrent chaos
    let mut check_stream = TcpStream::connect(addr).await.expect("connect");
    check_stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .expect("write");
    let mut check_buf = Vec::new();
    check_stream
        .read_to_end(&mut check_buf)
        .await
        .expect("read");
    assert!(String::from_utf8_lossy(&check_buf).starts_with("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn test_require_auth_middleware_fault_injection_integration() {
    // Intermittent SocketTimeout every 2 requests
    let policy = AuthFaultPolicy::intermittent(AuthFault::SocketTimeout, 2);
    let app = create_chaos_app_with_policy("secret-token", vec![], policy);

    let make_request = || {
        Request::builder()
            .uri("/api/v1/agents")
            .header("authorization", "Bearer secret-token")
            .body(Body::empty())
            .unwrap()
    };

    // Request 1: should inject SocketTimeout (invocation (1 - 1) % 2 == 0) -> 408 REQUEST_TIMEOUT
    let res1 = app.clone().oneshot(make_request()).await.unwrap();
    assert_eq!(res1.status(), StatusCode::REQUEST_TIMEOUT);

    // Request 2: should pass (invocation (2 - 1) % 2 != 0) -> 200 OK
    let res2 = app.clone().oneshot(make_request()).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);

    // Request 3: should inject SocketTimeout (invocation (3 - 1) % 2 == 0) -> 408 REQUEST_TIMEOUT
    let res3 = app.oneshot(make_request()).await.unwrap();
    assert_eq!(res3.status(), StatusCode::REQUEST_TIMEOUT);
}

// =========================================================================
// 6. Tiered error handling, Retry backoff & Safe Fallback tests
// =========================================================================

#[test]
fn test_auth_error_tier_and_status_classification() {
    use agent_server::auth::AuthErrorTier;

    // TransientNetwork tier
    let transient_errors = vec![
        AuthError::SocketTimeout,
        AuthError::ConnectionReset,
        AuthError::IntermittentIo("transient".into()),
    ];
    for err in transient_errors {
        assert_eq!(err.tier(), AuthErrorTier::TransientNetwork);
        assert!(err.is_retryable());
    }

    // ClientProtocol tier
    let client_errors = vec![
        AuthError::TokenDecodingFailure("bad utf8".into()),
        AuthError::MalformedHeader("no Bearer".into()),
    ];
    for err in client_errors {
        assert_eq!(err.tier(), AuthErrorTier::ClientProtocol);
        assert!(!err.is_retryable());
    }

    // SecurityViolation tier
    assert_eq!(AuthError::Unauthorized.tier(), AuthErrorTier::SecurityViolation);
    assert!(!AuthError::Unauthorized.is_retryable());
    assert_eq!(AuthError::Forbidden.tier(), AuthErrorTier::SecurityViolation);
    assert!(!AuthError::Forbidden.is_retryable());

    // Exhausted tier
    assert_eq!(AuthError::MaxRetriesExceeded(3).tier(), AuthErrorTier::Exhausted);
    assert!(!AuthError::MaxRetriesExceeded(3).is_retryable());

    // Status code mapping
    assert_eq!(AuthError::SocketTimeout.to_status_code(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(AuthError::ConnectionReset.to_status_code(), StatusCode::BAD_GATEWAY);
    assert_eq!(AuthError::IntermittentIo("e".into()).to_status_code(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(AuthError::TokenDecodingFailure("e".into()).to_status_code(), StatusCode::UNAUTHORIZED);
    assert_eq!(AuthError::MalformedHeader("e".into()).to_status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(AuthError::Unauthorized.to_status_code(), StatusCode::UNAUTHORIZED);
    assert_eq!(AuthError::Forbidden.to_status_code(), StatusCode::FORBIDDEN);
    assert_eq!(AuthError::MaxRetriesExceeded(3).to_status_code(), StatusCode::GATEWAY_TIMEOUT);
}

#[test]
fn test_resilient_auth_validator_safe_fallback_secondary_token() {
    use agent_server::auth::AuthFallbackStrategy;

    let primary_token = Arc::new("primary-secret-token".to_string());
    let fallback_token = Arc::new("backup-secret-token".to_string());

    let mut validator = ResilientAuthValidator::new(
        primary_token,
        vec![],
        AuthFaultPolicy::none(),
        2,
    ).with_fallback_strategy(AuthFallbackStrategy::SecondaryToken(fallback_token));

    // Primary token matches
    let res_primary = validator.validate_token_resilient("primary-secret-token");
    assert!(res_primary.is_ok());
    assert!(res_primary.unwrap());
    assert_eq!(validator.fallback_count(), 0);

    // Fallback token matches
    let res_fallback = validator.validate_token_resilient("backup-secret-token");
    assert!(res_fallback.is_ok());
    assert!(res_fallback.unwrap());
    assert_eq!(validator.fallback_count(), 1);

    // Invalid token rejected
    let res_invalid = validator.validate_token_resilient("completely-wrong-token");
    assert!(res_invalid.is_ok());
    assert!(!res_invalid.unwrap());
    assert_eq!(validator.fallback_count(), 1);
}

#[tokio::test]
async fn test_require_auth_safe_fallback_token_integration() {
    let auth = AuthState::new(
        Arc::new("primary-token".into()),
        vec![],
    ).with_fallback_token(Arc::new("fallback-backup-token".into()));

    let bus = InterAgentBus::new(16);
    let temp_vault = std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4().simple()));
    let task_store = Arc::new(agent_runtime::SqliteTaskStore::memory().expect("sqlite memory store"));
    let task_service = Arc::new(agent_runtime::TaskService::new(task_store, "/tmp", 2, None, None));
    let app = build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
            metrics: Arc::new(agent_server::metrics::ResilientMetricsService::default()),
        },
        auth,
    );

    // Test request with primary token
    let req_primary = Request::builder()
        .uri("/api/v1/agents")
        .header("authorization", "Bearer primary-token")
        .body(Body::empty())
        .unwrap();
    let res_primary = app.clone().oneshot(req_primary).await.unwrap();
    assert_eq!(res_primary.status(), StatusCode::OK);

    // Test request with fallback token
    let req_fallback = Request::builder()
        .uri("/api/v1/agents")
        .header("authorization", "Bearer fallback-backup-token")
        .body(Body::empty())
        .unwrap();
    let res_fallback = app.clone().oneshot(req_fallback).await.unwrap();
    assert_eq!(res_fallback.status(), StatusCode::OK);

    // Test request with wrong token
    let req_wrong = Request::builder()
        .uri("/api/v1/agents")
        .header("authorization", "Bearer wrong-token")
        .body(Body::empty())
        .unwrap();
    let res_wrong = app.oneshot(req_wrong).await.unwrap();
    assert_eq!(res_wrong.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn test_resilient_auth_validator_backoff_and_non_retryable_rejection() {
    // Non-retryable error (TokenDecodingFailure) should NOT retry
    let policy = AuthFaultPolicy::always(AuthFault::TokenDecodingFailure);
    let mut validator = ResilientAuthValidator::new(
        Arc::new("secret".into()),
        vec![],
        policy,
        3,
    ).with_backoff_ms(1);

    let res = validator.validate_token_resilient("secret");
    assert!(matches!(res, Err(AuthError::TokenDecodingFailure(_))));
    // Recovered count should be 0 because it was not retryable
    assert_eq!(validator.recovered_count(), 0);
    assert_eq!(validator.rejected_count(), 1);
}

// =========================================================================
// 8. High-load concurrency benchmark & bounded memory tests (TOPIC-20261105-7d7878 & 2dc00a)
// =========================================================================

#[tokio::test]
async fn test_topic_7d7878_2dc00a_auth_benchmark_and_high_load_resilience() {
    let valid_token = "bench_secret_key_2dc00a";
    let stats: AuthBenchmarkStats =
        AuthBenchmarkHarness::run_concurrent_benchmark(10_000, 4, valid_token).await;

    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 1_000.0,
        "Throughput should exceed 1,000 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(
        stats.p99_latency_us < 50_000.0,
        "P99 latency must be under 50ms, got {} us",
        stats.p99_latency_us
    );
    assert!(stats.p50_latency_us > 0.0);
    assert!(stats.p90_latency_us > 0.0);
}

#[tokio::test]
async fn test_auth_high_load_concurrency() {
    let valid_token = "high_load_secret";
    let mut validator = ResilientAuthValidator::new(
        Arc::new(valid_token.to_string()),
        vec![],
        AuthFaultPolicy::none(),
        3,
    )
    .with_max_capacity(2_000);

    let stats = AuthBenchmarkHarness::run_benchmark(&mut validator, 10_000, valid_token);
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(stats.throughput_ops_per_sec > 10_000.0);
    assert!(stats.p99_latency_us < 50_000.0);
}

#[tokio::test]
async fn test_auth_bounded_memory_capacity() {
    let valid_token = "bounded_mem_secret";
    let mut validator = ResilientAuthValidator::new(
        Arc::new(valid_token.to_string()),
        vec![],
        AuthFaultPolicy::none(),
        3,
    )
    .with_max_capacity(1_000);

    let _ = AuthBenchmarkHarness::run_benchmark(&mut validator, 10_000, valid_token);
    let mem_before = validator.memory_usage_bytes();

    // Run another 5,000 requests; memory should remain strictly bounded
    let _ = AuthBenchmarkHarness::run_benchmark(&mut validator, 5_000, valid_token);
    let mem_after = validator.memory_usage_bytes();

    assert!(
        mem_after <= mem_before + 1024,
        "Memory must be bounded due to FIFO eviction: before={}, after={}",
        mem_before,
        mem_after
    );
}

#[test]
fn test_auth_batch_token_validation() {
    let primary = Arc::new("batch_primary".to_string());
    let mut validator = ResilientAuthValidator::new(
        primary,
        vec![],
        AuthFaultPolicy::none(),
        3,
    );

    let tokens = vec![
        "batch_primary",
        "wrong_token_1",
        "batch_primary",
        "wrong_token_2",
        "batch_primary",
    ];

    let results = validator.validate_batch_tokens(&tokens);
    assert_eq!(results.len(), 5);
    assert_eq!(results[0], Ok(true));
    assert_eq!(results[1], Ok(false));
    assert_eq!(results[2], Ok(true));
    assert_eq!(results[3], Ok(false));
    assert_eq!(results[4], Ok(true));
}

#[tokio::test]
async fn test_auth_sharded_lock_contention_comparison() {
    let valid_token = "comparison_secret";
    let (single, sharded) =
        AuthBenchmarkHarness::compare_concurrency(10_000, valid_token).await;

    assert_eq!(single.total_requests, 10_000);
    assert_eq!(sharded.total_requests, 10_000);
    assert!(single.throughput_ops_per_sec > 1_000.0);
    assert!(sharded.throughput_ops_per_sec > 1_000.0);
    assert!(sharded.p99_latency_us < 50_000.0);
}

#[tokio::test]
async fn test_sharded_resilient_auth_validator_direct() {
    let token = Arc::new("sharded_direct_token".to_string());
    let validator = ShardedResilientAuthValidator::new(
        token,
        vec![],
        AuthFaultPolicy::none(),
        3,
        8,
    );

    assert_eq!(validator.num_shards(), 8);
    let ok = validator.validate_token_resilient("sharded_direct_token").await;
    assert_eq!(ok, Ok(true));
    let fail = validator.validate_token_resilient("wrong_token").await;
    assert_eq!(fail, Ok(false));
    assert_eq!(validator.validated_count().await, 1);
    assert_eq!(validator.rejected_count().await, 1);
}

#[tokio::test]
async fn test_sharded_auth_cache_concurrency_and_eviction() {
    let cache: Arc<ConcurrentAuthCache> = Arc::new(ShardedAuthCache::new(8, 32));
    let mut handles = Vec::new();

    for t_idx in 0..8 {
        let c = Arc::clone(&cache);
        handles.push(tokio::spawn(async move {
            for i in 0..50 {
                let token = format!("worker_{t_idx}_tok_{i}");
                c.insert(&token, i % 2 == 0);
                let res = c.get(&token);
                assert_eq!(res, Some(i % 2 == 0));
            }
        }));
    }

    for h in handles {
        h.await.expect("task join failed");
    }

    let stats = cache.stats();
    assert!(stats.hits >= 400);
    assert!(stats.total_entries <= 32);
    assert!(cache.memory_usage_bytes() > 0);
}

#[tokio::test]
async fn test_sharded_resilient_auth_validator_with_cache() {
    let token = Arc::new("sharded_cached_token_secret".to_string());
    let validator = ShardedResilientAuthValidator::new(
        token,
        vec![],
        AuthFaultPolicy::none(),
        3,
        8,
    )
    .with_cache(64);

    // Initial validation misses cache, inserts into cache
    let ok = validator.validate_token_resilient("sharded_cached_token_secret").await;
    assert_eq!(ok, Ok(true));
    assert_eq!(validator.validated_count().await, 1);

    // Second validation hits cache
    let ok2 = validator.validate_token_resilient("sharded_cached_token_secret").await;
    assert_eq!(ok2, Ok(true));
    assert_eq!(validator.validated_count().await, 2);

    let stats = validator.cache_stats().expect("cache stats present");
    assert!(stats.hits >= 1);
    assert_eq!(stats.total_entries, 1);

    // Clear cache and verify
    validator.clear_cache().await;
    let stats_after = validator.cache_stats().expect("cache stats present");
    assert_eq!(stats_after.total_entries, 0);
}

#[tokio::test]
async fn test_auth_state_with_cache_axum_integration() {
    let secret = "cached_integration_secret";
    let auth = AuthState::new(Arc::new(secret.to_string()), vec![]).with_cache(32);
    let cache_ref = auth.cache().cloned().expect("cache initialized");

    let app = create_chaos_test_app(secret, vec![]);
    let _ = app;
    // Build router using our cached auth state
    let bus = InterAgentBus::new(16);
    let temp_vault =
        std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4().simple()));
    let task_store =
        Arc::new(agent_runtime::SqliteTaskStore::memory().expect("sqlite memory store"));
    let task_service =
        Arc::new(agent_runtime::TaskService::new(task_store, "/tmp", 2, None, None));
    let custom_app = build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
            metrics: Arc::new(agent_server::metrics::ResilientMetricsService::default()),
        },
        auth,
    );

    let make_req = |token: &str| {
        Request::builder()
            .uri("/v1/models")
            .method("GET")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request build")
    };

    // First request - cache miss
    let res1 = custom_app.clone().oneshot(make_req(secret)).await.expect("oneshot");
    assert_eq!(res1.status(), StatusCode::OK);
    assert_eq!(cache_ref.stats().hits, 0);
    assert_eq!(cache_ref.stats().misses, 1);

    // Second request - cache hit!
    let res2 = custom_app.clone().oneshot(make_req(secret)).await.expect("oneshot");
    assert_eq!(res2.status(), StatusCode::OK);
    assert_eq!(cache_ref.stats().hits, 1);

    // Invalid token request
    let res3 = custom_app.clone().oneshot(make_req("bad_token")).await.expect("oneshot");
    assert_eq!(res3.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(cache_ref.stats().misses, 2);

    // Repeated invalid token request - hits cache!
    let res4 = custom_app.oneshot(make_req("bad_token")).await.expect("oneshot");
    assert_eq!(res4.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(cache_ref.stats().hits, 2);
}
