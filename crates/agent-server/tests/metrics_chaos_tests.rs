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
use agent_server::auth::AuthState;
use agent_server::metrics::{
    simulate_metrics_fault, MetricsBenchmarkHarness, MetricsError, MetricsErrorTier, MetricsFault,
    MetricsFaultPolicy, ResilientMetricsService,
};
use agent_server::{build_app, AppState};

fn create_metrics_test_app(service: Option<ResilientMetricsService>) -> Router {
    let bus = InterAgentBus::new(16);
    let temp_vault =
        std::env::temp_dir().join(format!("test_vault_metrics_{}", uuid::Uuid::new_v4().simple()));
    let task_store =
        Arc::new(agent_runtime::SqliteTaskStore::memory().expect("sqlite memory store"));
    let task_service =
        Arc::new(agent_runtime::TaskService::new(task_store, "/tmp", 2, None, None));
    let metrics = Arc::new(service.unwrap_or_default());

    build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
            metrics,
        },
        AuthState::new(Arc::new("test-token".into()), vec![]),
    )
}

async fn spawn_metrics_test_server(app: Router) -> SocketAddr {
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
fn test_metrics_fault_simulation_direct() {
    let test_cases = vec![
        (
            MetricsFault::Timeout,
            MetricsError::Timeout,
            StatusCode::REQUEST_TIMEOUT,
            MetricsErrorTier::Transient,
            true,
        ),
        (
            MetricsFault::LockContention,
            MetricsError::LockContention,
            StatusCode::REQUEST_TIMEOUT,
            MetricsErrorTier::Transient,
            true,
        ),
        (
            MetricsFault::CorruptedOutput,
            MetricsError::CorruptedOutput,
            StatusCode::INTERNAL_SERVER_ERROR,
            MetricsErrorTier::Permanent,
            false,
        ),
        (
            MetricsFault::IoError("Disk read failure".into()),
            MetricsError::IoError("Disk read failure".into()),
            StatusCode::INTERNAL_SERVER_ERROR,
            MetricsErrorTier::Permanent,
            false,
        ),
    ];

    for (fault, expected_err, expected_status, expected_tier, expected_retryable) in test_cases {
        let err = simulate_metrics_fault(fault);
        assert_eq!(err, expected_err);
        assert_eq!(err.tier(), expected_tier);
        assert_eq!(err.to_status_code(), expected_status);
        assert_eq!(err.is_retryable(), expected_retryable);
    }
}

// =========================================================================
// 2. Metrics Recording & Prometheus Text Format Verification
// =========================================================================

#[tokio::test]
async fn test_metrics_recording_and_formatting() {
    let service = ResilientMetricsService::new();

    // Record various operations
    service.record_task_submitted();
    service.record_task_submitted();
    service.record_task_completed(0.125);
    service.record_task_failed(0.05);

    service.record_ooda_cycle(0.042);
    service.record_ooda_cycle(0.058);

    service.record_remediation();
    service.record_remediation();

    service.record_test_result(true);
    service.record_test_result(true);
    service.record_test_result(false);

    let rendered = service
        .render_prometheus_text()
        .expect("prometheus text should render successfully");

    assert!(rendered.contains("# HELP agent_tasks_total"));
    assert!(rendered.contains("# TYPE agent_tasks_total counter"));
    assert!(rendered.contains("agent_tasks_total{status=\"completed\"} 1"));
    assert!(rendered.contains("agent_tasks_total{status=\"failed\"} 1"));
    assert!(rendered.contains("agent_tasks_total 2"));

    assert!(rendered.contains("# HELP agent_ooda_duration_seconds"));
    assert!(rendered.contains("# TYPE agent_ooda_duration_seconds gauge"));
    assert!(rendered.contains("agent_ooda_duration_seconds 0.058000"));

    assert!(rendered.contains("# HELP agent_ooda_cycles_total"));
    assert!(rendered.contains("agent_ooda_cycles_total 2"));

    assert!(rendered.contains("# HELP agent_remediations_total"));
    assert!(rendered.contains("# TYPE agent_remediations_total counter"));
    assert!(rendered.contains("agent_remediations_total 2"));

    assert!(rendered.contains("# HELP agent_tests_success_ratio"));
    assert!(rendered.contains("agent_tests_success_ratio 0.6667"));
}

// =========================================================================
// 3. Resilient Service Self-Healing under Intermittent Faults
// =========================================================================

#[test]
fn test_metrics_resilience_transient_recovery() {
    let recovery_policy =
        MetricsFaultPolicy::always(MetricsFault::Timeout).with_max_injections(1);

    let service = ResilientMetricsService::new()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(1))
        .with_fault_policy(recovery_policy);

    service.record_task_submitted();
    service.record_task_completed(0.01);

    let res = service.render_prometheus_text();
    assert!(
        res.is_ok(),
        "Metrics service must recover from transient timeout"
    );
    assert_eq!(service.recovered_count(), 1);
    assert_eq!(service.retries_attempted(), 1);
}

#[test]
fn test_metrics_resilience_cached_fallback_on_persistent_failure() {
    let service = ResilientMetricsService::new();
    service.record_task_submitted();
    service.record_task_completed(0.05);

    // Initial valid render populates cache
    let initial = service
        .render_prometheus_text()
        .expect("initial render ok");
    assert!(initial.contains("agent_tasks_total 1"));

    // Inject permanent corrupted output fault
    *service.fault_policy().lock().expect("lock fault policy") =
        MetricsFaultPolicy::always(MetricsFault::CorruptedOutput);

    // Service should fallback to cached snapshot gracefully
    let fallback_res = service.render_prometheus_text();
    assert!(
        fallback_res.is_ok(),
        "Service should return cached snapshot when permanent fault occurs"
    );
    assert_eq!(fallback_res.expect("fallback payload"), initial);
}

// =========================================================================
// 4. HTTP Integration Test on /metrics Endpoint
// =========================================================================

#[tokio::test]
async fn test_http_metrics_endpoint_integration() {
    let service = ResilientMetricsService::new();
    service.record_task_submitted();
    service.record_task_completed(0.250);
    service.record_ooda_cycle(0.085);
    service.record_remediation();
    service.record_test_result(true);

    let app = create_metrics_test_app(Some(service));

    let req = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .expect("valid request");

    let resp = app.oneshot(req).await.expect("app response");
    assert_eq!(resp.status(), StatusCode::OK);

    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("content-type header present")
        .to_str()
        .expect("valid header str");
    assert!(
        content_type.contains("text/plain"),
        "Content-Type must be text/plain for Prometheus scraper"
    );

    let body_bytes = axum::body::to_bytes(resp.into_body(), 16384)
        .await
        .expect("read body");
    let body = String::from_utf8(body_bytes.to_vec()).expect("utf8 string");

    assert!(body.contains("agent_tasks_total"));
    assert!(body.contains("agent_ooda_duration_seconds"));
    assert!(body.contains("agent_remediations_total"));
    assert!(body.contains("agent_tests_success_ratio"));
}

#[tokio::test]
async fn test_raw_tcp_metrics_scraping() {
    let service = ResilientMetricsService::new();
    service.record_task_submitted();
    service.record_ooda_cycle(0.012);

    let app = create_metrics_test_app(Some(service));
    let addr = spawn_metrics_test_server(app).await;

    let mut stream = TcpStream::connect(addr).await.expect("tcp connect");
    let http_req = format!("GET /metrics HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", addr);
    stream
        .write_all(http_req.as_bytes())
        .await
        .expect("write request");

    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await.expect("read response");
    let response_str = String::from_utf8_lossy(&buffer);

    assert!(response_str.contains("HTTP/1.1 200 OK"));
    assert!(response_str.contains("text/plain"));
    assert!(response_str.contains("agent_tasks_total"));
    assert!(response_str.contains("agent_ooda_duration_seconds"));
    assert!(response_str.contains("agent_remediations_total"));
}

// =========================================================================
// 5. Concurrency & Benchmark Test (10,000 Operations)
// =========================================================================

#[test]
fn test_metrics_concurrency_and_benchmark() {
    let (single, sharded) = MetricsBenchmarkHarness::compare_concurrency(10_000);

    assert_eq!(single.total_requests, 10_000);
    assert_eq!(sharded.total_requests, 10_000);
    assert!(single.throughput_ops_per_sec > 1_000.0);
    assert!(sharded.throughput_ops_per_sec > 10_000.0);
    assert!(sharded.p99_latency_micros < 50_000.0); // P99 < 50ms
}

// =========================================================================
// 6. Topic P0 Bugfix Verification (TOPIC-20261105-7b40b9 / 866e83 / 08f162)
// =========================================================================

#[tokio::test]
async fn test_topic_7b40b9_866e83_08f162_metrics_resilience() {
    // 1. Acceptance Criteria: Prometheus Metrics Exporter endpoint (/metrics) in Axum router
    let service = ResilientMetricsService::new();
    service.record_task_submitted();
    service.record_task_completed(0.12);
    service.record_ooda_cycle(0.045);
    service.record_remediation();
    service.record_test_result(true);

    let app = create_metrics_test_app(Some(service.clone()));

    let req = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .expect("valid request");

    let resp = app.oneshot(req).await.expect("app response");
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. Acceptance Criteria: Content-Type text/plain
    let ct = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("content type")
        .to_str()
        .expect("header str");
    assert!(ct.contains("text/plain"));

    // 3. Acceptance Criteria: Export agent_tasks_total, agent_ooda_duration_seconds, agent_remediations_total
    let body_bytes = axum::body::to_bytes(resp.into_body(), 16384)
        .await
        .expect("read body");
    let body = String::from_utf8(body_bytes.to_vec()).expect("utf8 string");

    assert!(body.contains("agent_tasks_total"));
    assert!(body.contains("agent_ooda_duration_seconds"));
    assert!(body.contains("agent_remediations_total"));

    // 4. Fault Tolerance: Transient retry and recovery counter
    let intermittent =
        MetricsFaultPolicy::always(MetricsFault::LockContention).with_max_injections(1);
    let fault_service = ResilientMetricsService::new()
        .with_max_retries(2)
        .with_backoff(Duration::from_millis(1))
        .with_fault_policy(intermittent);

    let res = fault_service.render_prometheus_text();
    assert!(res.is_ok());
    assert_eq!(fault_service.recovered_count(), 1);
}
