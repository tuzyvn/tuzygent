use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

use agent_core::prelude::*;
use agent_server::auth::AuthState;
use agent_server::metrics::{
    MetricsFault, MetricsFaultPolicy, ResilientMetricsService,
};
use agent_server::{build_app, AppState};

fn create_test_server_app(service: Option<ResilientMetricsService>) -> Router {
    let bus = InterAgentBus::new(16);
    let temp_vault =
        std::env::temp_dir().join(format!("test_vault_integration_{}", uuid::Uuid::new_v4().simple()));
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

#[tokio::test]
async fn test_metrics_endpoint_failure_reproduction() {
    // -------------------------------------------------------------------------
    // Phase 1: Establish baseline with recorded operational metrics
    // -------------------------------------------------------------------------
    let service = ResilientMetricsService::new()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(1));

    service.record_task_submitted();
    service.record_task_completed(0.085);
    service.record_ooda_cycle(0.042);
    service.record_remediation();
    service.record_test_result(true);

    // Initial render must succeed and prime the cached snapshot
    let baseline_render = service
        .render_prometheus_text()
        .expect("initial render should succeed and prime cache");
    assert!(baseline_render.contains("agent_tasks_total 1"));
    assert!(baseline_render.contains("agent_ooda_duration_seconds 0.042000"));
    assert!(baseline_render.contains("agent_remediations_total 1"));

    // -------------------------------------------------------------------------
    // Phase 2: Reproduce transient failure (Timeout / LockContention)
    // Verify self-healing via automated retry and exponential backoff
    // -------------------------------------------------------------------------
    let transient_policy =
        MetricsFaultPolicy::always(MetricsFault::Timeout).with_max_injections(1);
    *service.fault_policy().lock().expect("lock fault policy") = transient_policy;

    let recovered_render = service
        .render_prometheus_text()
        .expect("service should self-heal and recover from transient timeout");
    assert_eq!(service.recovered_count(), 1, "recovery count should increment");
    assert_eq!(service.retries_attempted(), 1, "retry count should increment");
    assert!(recovered_render.contains("agent_tasks_total 1"));

    // -------------------------------------------------------------------------
    // Phase 3: Reproduce permanent failure (CorruptedOutput / IoError)
    // Verify graceful fallback to the last valid cached snapshot (0% downtime)
    // -------------------------------------------------------------------------
    let permanent_policy = MetricsFaultPolicy::always(MetricsFault::CorruptedOutput);
    *service.fault_policy().lock().expect("lock fault policy") = permanent_policy;

    let fallback_render = service
        .render_prometheus_text()
        .expect("service must fallback to cached snapshot on permanent error");
    assert_eq!(
        fallback_render, baseline_render,
        "fallback output must match cached valid snapshot"
    );

    // -------------------------------------------------------------------------
    // Phase 4: End-to-end Axum HTTP endpoint verification under active fault injection
    // -------------------------------------------------------------------------
    let app = create_test_server_app(Some(service));

    let req = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .expect("valid http request");

    let resp = app.oneshot(req).await.expect("app response");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "HTTP /metrics endpoint must always return 200 OK under fault conditions"
    );

    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("content-type header present")
        .to_str()
        .expect("valid content-type str");
    assert!(
        content_type.contains("text/plain"),
        "Content-Type must be text/plain for Prometheus scrapers"
    );

    let body_bytes = axum::body::to_bytes(resp.into_body(), 16384)
        .await
        .expect("read response body");
    let body_str = String::from_utf8(body_bytes.to_vec()).expect("valid utf8 response");

    assert!(
        body_str.contains("agent_tasks_total"),
        "body must include agent_tasks_total metric"
    );
    assert!(
        body_str.contains("agent_ooda_duration_seconds"),
        "body must include agent_ooda_duration_seconds metric"
    );
    assert!(
        body_str.contains("agent_remediations_total"),
        "body must include agent_remediations_total metric"
    );
}
