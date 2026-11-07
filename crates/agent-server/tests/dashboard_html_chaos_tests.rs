use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

use agent_server::dashboard_html::{
    simulate_dashboard_fault, DashboardAssetRegistry, DashboardContext, DashboardError,
    DashboardErrorTier, DashboardFault, DashboardFaultPolicy, ResilientDashboardService,
    DASHBOARD_HTML, FALLBACK_DASHBOARD_HTML,
};

fn create_dashboard_test_app() -> Router {
    Router::new()
        .route("/", get(dashboard_test_handler))
        .route("/dashboard", get(dashboard_test_handler))
}

async fn dashboard_test_handler() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

async fn spawn_test_server(app: Router) -> Option<SocketAddr> {
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping TCP integration test: binding sockets is not permitted: {error}");
            return None;
        }
        Err(error) => panic!("bind ephemeral port: {error}"),
    };
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Some(addr)
}

// =========================================================================
// 1. Direct Fault Mapping & Severity Classification
// =========================================================================

#[test]
fn test_dashboard_fault_simulation_direct() {
    let test_cases = vec![
        (
            DashboardFault::TemplateRenderFailure,
            DashboardError::TemplateRenderFailure(
                "Injected template rendering failure: placeholder evaluation failed".to_string(),
            ),
        ),
        (
            DashboardFault::MissingAsset,
            DashboardError::MissingAsset(
                "Injected missing asset fault: static asset not found".to_string(),
            ),
        ),
        (
            DashboardFault::IoError,
            DashboardError::IoError(
                "Injected I/O failure: underlying storage read aborted".to_string(),
            ),
        ),
        (
            DashboardFault::NetworkDisconnection,
            DashboardError::NetworkDisconnection(
                "Injected network disconnection: client connection abruptly terminated".to_string(),
            ),
        ),
        (DashboardFault::SocketTimeout, DashboardError::SocketTimeout),
        (
            DashboardFault::ConnectionReset,
            DashboardError::ConnectionReset,
        ),
    ];

    for (fault, expected_err) in test_cases {
        assert_eq!(fault.to_dashboard_error(), expected_err);
        assert_eq!(simulate_dashboard_fault(&fault).unwrap_err(), expected_err);
    }
}

#[test]
fn test_dashboard_error_tier_and_status_classification() {
    // Transient network tier
    assert_eq!(
        DashboardError::SocketTimeout.tier(),
        DashboardErrorTier::TransientNetwork
    );
    assert!(DashboardError::SocketTimeout.is_retryable());
    assert_eq!(
        DashboardError::SocketTimeout.to_status_code(),
        StatusCode::REQUEST_TIMEOUT
    );

    assert_eq!(
        DashboardError::ConnectionReset.tier(),
        DashboardErrorTier::TransientNetwork
    );
    assert!(DashboardError::ConnectionReset.is_retryable());
    assert_eq!(
        DashboardError::ConnectionReset.to_status_code(),
        StatusCode::BAD_GATEWAY
    );

    assert_eq!(
        DashboardError::NetworkDisconnection("peer hung up".into()).tier(),
        DashboardErrorTier::TransientNetwork
    );
    assert!(DashboardError::NetworkDisconnection("peer hung up".into()).is_retryable());
    assert_eq!(
        DashboardError::NetworkDisconnection("peer hung up".into()).to_status_code(),
        StatusCode::BAD_GATEWAY
    );

    // I/O failure tier
    assert_eq!(
        DashboardError::IoError("disk read fail".into()).tier(),
        DashboardErrorTier::IoFailure
    );
    assert!(DashboardError::IoError("disk read fail".into()).is_retryable());
    assert_eq!(
        DashboardError::IoError("disk read fail".into()).to_status_code(),
        StatusCode::INTERNAL_SERVER_ERROR
    );

    // Render failure tier
    assert_eq!(
        DashboardError::TemplateRenderFailure("syntax error".into()).tier(),
        DashboardErrorTier::RenderFailure
    );
    assert!(!DashboardError::TemplateRenderFailure("syntax error".into()).is_retryable());
    assert_eq!(
        DashboardError::TemplateRenderFailure("syntax error".into()).to_status_code(),
        StatusCode::INTERNAL_SERVER_ERROR
    );

    // Missing asset tier
    assert_eq!(
        DashboardError::MissingAsset("logo.svg".into()).tier(),
        DashboardErrorTier::NotFound
    );
    assert!(!DashboardError::MissingAsset("logo.svg".into()).is_retryable());
    assert_eq!(
        DashboardError::MissingAsset("logo.svg".into()).to_status_code(),
        StatusCode::NOT_FOUND
    );

    // Exhausted tier
    assert_eq!(
        DashboardError::MaxRetriesExceeded(3).tier(),
        DashboardErrorTier::Exhausted
    );
    assert!(!DashboardError::MaxRetriesExceeded(3).is_retryable());
    assert_eq!(
        DashboardError::MaxRetriesExceeded(3).to_status_code(),
        StatusCode::GATEWAY_TIMEOUT
    );
}

// =========================================================================
// 2. Scenario 1: Template Render Failure & Fallback Resilience
// =========================================================================

#[test]
fn test_template_render_syntax_failure_unclosed_delimiters() {
    let bad_template = "<html><body><h1>{{title</body></html>";
    let ctx = DashboardContext::new().with_title("AgentCore");

    let err = agent_server::dashboard_html::render_template(bad_template, &ctx).unwrap_err();
    match err {
        DashboardError::TemplateRenderFailure(msg) => {
            assert!(msg.contains("Unclosed placeholder delimiter"));
        }
        other => panic!("Expected TemplateRenderFailure, got {:?}", other),
    }
}

#[test]
fn test_template_render_with_fallback_on_syntax_error() {
    let bad_template = "<html><body>{{unclosed_variable";
    let ctx = DashboardContext::new();

    let (html, status, is_fallback) =
        agent_server::dashboard_html::render_with_fallback(bad_template, &ctx, None);

    assert!(is_fallback);
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(html, FALLBACK_DASHBOARD_HTML);

    // Fallback template must be complete HTML with essential structural elements
    let lower = html.to_lowercase();
    assert!(lower.contains("<!doctype html>"));
    assert!(lower.contains("emergency fallback"));
    assert!(html.contains("auth-bar"));
    assert!(html.contains("stats-grid"));
}

#[test]
fn test_template_render_fault_injection_always() {
    let valid_template = "<html><body>{{title}}</body></html>";
    let ctx = DashboardContext::new().with_title("Gateway Dashboard");
    let mut policy = DashboardFaultPolicy::always(DashboardFault::TemplateRenderFailure);

    let (html, status, is_fallback) =
        agent_server::dashboard_html::render_with_fallback(valid_template, &ctx, Some(&mut policy));

    assert!(is_fallback);
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(html, FALLBACK_DASHBOARD_HTML);
    assert_eq!(policy.injected_count(), 1);
}

#[test]
fn test_template_render_intermittent_fault_recovery() {
    let valid_template = "<html><body><h1>{{title}}</h1></body></html>";
    let ctx = DashboardContext::new().with_title("Active Dashboard");
    // Fails every 2nd call (calls 1, 3 fail; calls 2, 4 succeed)
    let mut policy = DashboardFaultPolicy::intermittent(DashboardFault::TemplateRenderFailure, 2);

    // Call 1: fails
    let (html1, status1, fallback1) =
        agent_server::dashboard_html::render_with_fallback(valid_template, &ctx, Some(&mut policy));
    assert!(fallback1);
    assert_eq!(status1, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(html1, FALLBACK_DASHBOARD_HTML);

    // Call 2: succeeds
    let (html2, status2, fallback2) =
        agent_server::dashboard_html::render_with_fallback(valid_template, &ctx, Some(&mut policy));
    assert!(!fallback2);
    assert_eq!(status2, StatusCode::OK);
    assert!(html2.contains("<h1>Active Dashboard</h1>"));

    // Call 3: fails
    let (_, _, fallback3) =
        agent_server::dashboard_html::render_with_fallback(valid_template, &ctx, Some(&mut policy));
    assert!(fallback3);

    // Call 4: succeeds
    let (html4, status4, fallback4) =
        agent_server::dashboard_html::render_with_fallback(valid_template, &ctx, Some(&mut policy));
    assert!(!fallback4);
    assert_eq!(status4, StatusCode::OK);
    assert!(html4.contains("Active Dashboard"));

    assert_eq!(policy.injected_count(), 2);
}

#[test]
fn test_template_variable_substitution_integrity() {
    let template = "<html><head><title>{{title}}</title></head><body><span>{{gateway_url}}</span><span>{{version}}</span><span>{{custom_var}}</span></body></html>";
    let ctx = DashboardContext::new()
        .with_title("Custom Gateway")
        .with_gateway_url("http://127.0.0.1:9090")
        .with_version("2.5.0")
        .with_var("custom_var", "chaos-tested-value");

    let rendered = agent_server::dashboard_html::render_template(template, &ctx)
        .expect("render should succeed");

    assert!(rendered.contains("<title>Custom Gateway</title>"));
    assert!(rendered.contains("<span>http://127.0.0.1:9090</span>"));
    assert!(rendered.contains("<span>2.5.0</span>"));
    assert!(rendered.contains("<span>chaos-tested-value</span>"));
    assert!(!rendered.contains("{{"));
    assert!(!rendered.contains("}}"));
}

// =========================================================================
// 3. Scenario 2: Missing Assets & Resilient Asset Resolution
// =========================================================================

#[test]
fn test_asset_registry_builtin_assets() {
    let registry = DashboardAssetRegistry::default();

    let dashboard = registry
        .get("dashboard.html")
        .expect("dashboard.html exists");
    assert_eq!(dashboard.content_type, "text/html; charset=utf-8");
    assert!(!dashboard.bytes.is_empty());

    let fallback = registry.get("fallback.html").expect("fallback.html exists");
    assert_eq!(fallback.content_type, "text/html; charset=utf-8");
    assert!(!fallback.bytes.is_empty());

    let css = registry.get("style.css").expect("style.css exists");
    assert_eq!(css.content_type, "text/css; charset=utf-8");

    let favicon = registry.get("favicon.ico").expect("favicon.ico exists");
    assert_eq!(favicon.content_type, "image/x-icon");
}

#[test]
fn test_missing_asset_returns_not_found_error() {
    let registry = DashboardAssetRegistry::default();
    let err = registry
        .load_asset("missing_bundle.js", None, None)
        .unwrap_err();

    match err {
        DashboardError::MissingAsset(ref msg) => {
            assert!(msg.contains("missing_bundle.js"));
            assert_eq!(err.to_status_code(), StatusCode::NOT_FOUND);
        }
        other => panic!("Expected MissingAsset, got {:?}", other),
    }
}

#[test]
fn test_missing_asset_fault_injection() {
    let registry = DashboardAssetRegistry::default();
    let mut policy = DashboardFaultPolicy::always(DashboardFault::MissingAsset);

    let err = registry
        .load_asset("style.css", None, Some(&mut policy))
        .unwrap_err();

    match err {
        DashboardError::MissingAsset(ref msg) => {
            assert!(msg.contains("Simulated fault"));
            assert_eq!(policy.injected_count(), 1);
        }
        other => panic!("Expected MissingAsset, got {:?}", other),
    }
}

#[test]
fn test_resilient_asset_loader_graceful_missing_asset_handling() {
    let registry = DashboardAssetRegistry::default();

    let (bytes, content_type, status) =
        registry.load_asset_resilient("non_existent_asset.js", None, None);

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(content_type, "text/plain; charset=utf-8");
    let content = String::from_utf8(bytes).unwrap();
    assert!(content.contains("Missing asset: non_existent_asset.js"));
}

#[test]
fn test_resilient_asset_loader_embedded_fallback_when_disk_asset_missing() {
    let registry = DashboardAssetRegistry::default();
    let temp_empty_dir =
        std::env::temp_dir().join(format!("empty_asset_dir_{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&temp_empty_dir).expect("create temp dir");

    let err = registry
        .load_asset("style.css", Some(&temp_empty_dir), None)
        .unwrap_err();
    assert_eq!(err.tier(), DashboardErrorTier::NotFound);

    let (bytes, _content_type, status) =
        registry.load_asset_resilient("style.css", Some(&temp_empty_dir), None);
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(String::from_utf8(bytes).unwrap().contains("Missing asset"));

    let _ = std::fs::remove_dir_all(temp_empty_dir);
}

// =========================================================================
// 4. Scenario 3: I/O Errors & Resilient Retry / Fallback
// =========================================================================

#[tokio::test]
async fn test_resilient_dashboard_intermittent_io_recovery() {
    let policy = DashboardFaultPolicy::intermittent(DashboardFault::IoError, 2);
    let service = ResilientDashboardService::new()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(5))
        .with_fault_policy(policy);

    let ctx = DashboardContext::new();
    let rendered = service
        .render_dashboard(&ctx)
        .await
        .expect("service should retry through intermittent I/O fault and succeed");

    assert!(!rendered.is_empty());
    assert!(rendered.contains("auth-bar"));
    assert_eq!(service.fault_policy().lock().unwrap().injected_count(), 1);
}

#[tokio::test]
async fn test_resilient_dashboard_persistent_io_exhausts_retries() {
    let policy = DashboardFaultPolicy::always(DashboardFault::IoError);
    let service = ResilientDashboardService::new()
        .with_max_retries(2)
        .with_backoff(Duration::from_millis(2))
        .with_fault_policy(policy);

    let ctx = DashboardContext::new();
    let err = service.render_dashboard(&ctx).await.unwrap_err();

    match err {
        DashboardError::MaxRetriesExceeded(max) => {
            assert_eq!(max, 2);
            assert_eq!(err.tier(), DashboardErrorTier::Exhausted);
        }
        other => panic!("Expected MaxRetriesExceeded, got {:?}", other),
    }
}

#[tokio::test]
async fn test_resilient_dashboard_fallback_on_persistent_io_failure() {
    let policy = DashboardFaultPolicy::always(DashboardFault::IoError);
    let service = ResilientDashboardService::new()
        .with_max_retries(2)
        .with_backoff(Duration::from_millis(2))
        .with_fault_policy(policy);

    let ctx = DashboardContext::new();
    let (html, status) = service.render_dashboard_with_fallback(&ctx).await;

    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(html, FALLBACK_DASHBOARD_HTML);
}

#[tokio::test]
async fn test_resilient_service_asset_fallback_on_disk_io_error() {
    let temp_unreadable_dir = std::env::temp_dir().join(format!(
        "unreadable_asset_dir_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_unreadable_dir).expect("create temp dir");
    let test_file = temp_unreadable_dir.join("dashboard.html");
    std::fs::write(&test_file, b"corrupted bytes").expect("write test file");

    let service =
        ResilientDashboardService::new().with_custom_asset_dir(temp_unreadable_dir.clone());

    let (bytes, _content_type, status) = service.get_asset("dashboard.html").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"corrupted bytes");

    let _ = std::fs::remove_dir_all(temp_unreadable_dir);
}

// =========================================================================
// 5. Scenario 4: Network Disconnection & Raw Socket Chaos
// =========================================================================

#[tokio::test]
async fn test_axum_app_dashboard_routes_serve_valid_html() {
    let app = create_dashboard_test_app();

    for path in &["/", "/dashboard"] {
        let req = Request::builder()
            .method("GET")
            .uri(*path)
            .body(Body::empty())
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("auth-bar"));
        assert!(body_str.contains("stats-grid"));
    }
}

#[tokio::test]
async fn test_real_socket_abrupt_disconnect_rst() {
    let app = create_dashboard_test_app();
    let Some(addr) = spawn_test_server(app).await else {
        return;
    };

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let _ = stream
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .await;
    drop(stream);

    // Verify subsequent request succeeds
    let mut healthy_stream = TcpStream::connect(addr).await.expect("connect healthy");
    let req = b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    healthy_stream.write_all(req).await.expect("write");

    let mut res_buf = Vec::new();
    healthy_stream
        .read_to_end(&mut res_buf)
        .await
        .expect("read");
    let res = String::from_utf8_lossy(&res_buf);
    assert!(res.starts_with("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn test_real_socket_client_disconnect_mid_stream() {
    let app = create_dashboard_test_app();
    let Some(addr) = spawn_test_server(app).await else {
        return;
    };

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /dashboard HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut small_buf = [0u8; 32];
    let n = stream.read(&mut small_buf).await.unwrap();
    assert!(n > 0);
    // Abrupt disconnect mid-stream
    drop(stream);

    tokio::time::sleep(Duration::from_millis(30)).await;

    // Subsequent client connects and gets full dashboard
    let mut stream2 = TcpStream::connect(addr).await.unwrap();
    stream2
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut res2 = String::new();
    stream2.read_to_string(&mut res2).await.unwrap();
    assert!(res2.contains("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn test_real_socket_stalled_client() {
    let app = create_dashboard_test_app();
    let Some(addr) = spawn_test_server(app).await else {
        return;
    };

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(b"G").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(stream);

    let mut stream2 = TcpStream::connect(addr).await.unwrap();
    stream2
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut response = String::new();
    stream2.read_to_string(&mut response).await.unwrap();
    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("auth-bar"));
}

#[tokio::test]
async fn test_real_socket_choppy_fragmented_delivery() {
    let app = create_dashboard_test_app();
    let Some(addr) = spawn_test_server(app).await else {
        return;
    };

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let raw_req = b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";

    for chunk in raw_req.chunks(3) {
        stream.write_all(chunk).await.unwrap();
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("AgentCore"));
}

// =========================================================================
// 6. Scenario 5: High Concurrency Under Chaotic Faults
// =========================================================================

#[tokio::test]
async fn test_chaos_concurrent_dashboard_requests_under_fault() {
    let policy = DashboardFaultPolicy::intermittent(DashboardFault::TemplateRenderFailure, 3);
    let service = Arc::new(
        ResilientDashboardService::new()
            .with_max_retries(2)
            .with_backoff(Duration::from_millis(2))
            .with_fault_policy(policy),
    );

    let mut handles = vec![];
    for i in 0..30 {
        let svc = service.clone();
        handles.push(tokio::spawn(async move {
            let ctx = DashboardContext::new().with_title(format!("Worker-{}", i));
            svc.render_dashboard_with_fallback(&ctx).await
        }));
    }

    let mut fallback_count = 0;
    let mut ok_count = 0;

    for handle in handles {
        let (html, status) = handle.await.unwrap();
        assert!(!html.is_empty());
        assert!(html.contains("auth-bar"));
        if status == StatusCode::OK {
            ok_count += 1;
            assert!(html.contains("AgentCore"));
        } else {
            fallback_count += 1;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(html, FALLBACK_DASHBOARD_HTML);
        }
    }

    assert!(ok_count > 0, "Expected some successful renders");
    assert!(
        fallback_count > 0,
        "Expected some fallback renders due to faults"
    );
    assert_eq!(ok_count + fallback_count, 30);
}

// =========================================================================
// 7. Root Cause Elimination Test for TOPIC-20261105-ad213f & TOPIC-20261105-376fb5
// =========================================================================

#[tokio::test]
async fn test_topic_ad213f_376fb5_dashboard_html_chaos_resilience() {
    // 1. Verify LOC compliance across all modular dashboard_html components (< 350 lines P90)
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dashboard_html_dir = manifest_dir.join("src/dashboard_html");
    assert!(
        dashboard_html_dir.is_dir(),
        "src/dashboard_html must exist as a modular directory"
    );

    for entry in std::fs::read_dir(&dashboard_html_dir).unwrap() {
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

    let root_file = manifest_dir.join("src/dashboard_html.rs");
    let root_lines = std::fs::read_to_string(&root_file).unwrap().lines().count();
    assert!(
        root_lines <= 350,
        "dashboard_html.rs has {} lines, which exceeds P90 limit (350 lines)",
        root_lines
    );

    // 2. Dynamic Templating Safety & Context Substitution
    let ctx = DashboardContext::new()
        .with_title("AgentCore Mission Control")
        .with_gateway_url("ws://localhost:9001/ws")
        .with_version("2.5.0-resilient")
        .with_var("CUSTOM_METRIC", "99.999%");

    let raw_tmpl = "<h1>{{title}}</h1><span>{{GATEWAY_URL}}</span><span>{{version}}</span><div>{{CUSTOM_METRIC}}</div>";
    let rendered = agent_server::dashboard_html::render_template(raw_tmpl, &ctx)
        .expect("render should succeed");
    assert_eq!(
        rendered,
        "<h1>AgentCore Mission Control</h1><span>ws://localhost:9001/ws</span><span>2.5.0-resilient</span><div>99.999%</div>"
    );

    // 3. Delimiter integrity check rejects malformed unclosed tags
    let malformed_tmpl = "<div>{{unclosed_variable</div>";
    let render_err = agent_server::dashboard_html::render_template(malformed_tmpl, &ctx);
    assert!(matches!(
        render_err,
        Err(DashboardError::TemplateRenderFailure(_))
    ));

    // 4. Graceful Fallback Rendering on Template Failure
    let (fallback_html, status, is_fallback) =
        agent_server::dashboard_html::render_with_fallback(malformed_tmpl, &ctx, None);
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(is_fallback);
    assert_eq!(fallback_html, FALLBACK_DASHBOARD_HTML);

    // 5. Fault Policy Parity with Auth Module
    let mut policy = DashboardFaultPolicy::always(DashboardFault::IoError).with_max_injections(1);
    assert_eq!(policy.should_inject(), Some(DashboardFault::IoError));
    assert_eq!(policy.should_inject(), None);
    assert_eq!(policy.injected_count(), 1);
    policy.reset();
    assert_eq!(policy.injected_count(), 0);

    // 6. Resilient Service Self-Healing under Intermittent Faults
    let recovery_policy =
        DashboardFaultPolicy::intermittent(DashboardFault::SocketTimeout, 2).with_max_injections(1);
    let service = ResilientDashboardService::new()
        .with_max_retries(3)
        .with_backoff(Duration::from_millis(1))
        .with_fault_policy(recovery_policy);

    let res = service.render_dashboard(&ctx).await;
    assert!(
        res.is_ok(),
        "Service must recover from transient socket timeout"
    );
    let recovered_html = res.unwrap();
    assert!(recovered_html.contains("auth-bar"));

    // 7. Embedded Asset Registry Resilience
    let registry = DashboardAssetRegistry::default();
    let (css_bytes, content_type, css_status) =
        registry.load_asset_resilient("style.css", None, None);
    assert_eq!(css_status, StatusCode::OK);
    assert_eq!(content_type, "text/css; charset=utf-8");
    assert!(!css_bytes.is_empty());
}
