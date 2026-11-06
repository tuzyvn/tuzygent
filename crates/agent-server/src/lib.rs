use axum::{
    http::Method,
    middleware,
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub mod auth;
pub mod dashboard_html;
pub mod gateway;
pub mod handlers;
pub mod metrics;
pub mod state;

pub use auth::{
    extract_bearer_token, extract_ws_token, require_auth, simulate_auth_fault, token_matches,
    AuthBenchmarkHarness, AuthBenchmarkStats, AuthCacheEntry, AuthCacheStats, AuthError,
    AuthErrorTier, AuthFallbackStrategy, AuthFault, AuthFaultPolicy, AuthState,
    ConcurrentAuthCache, ResilientAuthValidator, ShardedAuthCache, ShardedResilientAuthValidator,
};
pub use dashboard_html::{
    simulate_dashboard_fault, ConcurrentDashboardCache, DashboardAsset, DashboardAssetRegistry,
    DashboardBenchmarkHarness, DashboardBenchmarkStats, DashboardCacheEntry, DashboardCacheStats,
    DashboardContext, DashboardError, DashboardErrorTier, DashboardFault, DashboardFaultPolicy,
    DashboardState, ResilientDashboardService, ShardedDashboardCache,
    ShardedResilientDashboardService, DASHBOARD_HTML, FALLBACK_DASHBOARD_HTML,
};
pub use gateway::*;
pub use handlers::*;
pub use metrics::{
    simulate_metrics_fault, MetricType, MetricsBenchmarkHarness, MetricsBenchmarkReport,
    MetricsError, MetricsErrorTier, MetricsFault, MetricsFaultPolicy, MetricsSnapshot, OodaMetrics,
    RemediationMetrics, ResilientMetricsService, TaskMetrics, TestMetrics,
};
pub use state::{AppState, ServerCli};

pub fn build_app(state: AppState, auth: AuthState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(auth.allowed_origins.clone())
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderName::from_static("idempotency-key"),
        ]);

    let gateway = Router::new()
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/models", get(list_models_handler))
        .route(
            "/api/v1/auth/accounts",
            get(list_accounts_handler).post(add_account_handler),
        )
        .route(
            "/api/v1/auth/accounts/{engine}/{id}",
            delete(remove_account_handler),
        )
        .route(
            "/api/v1/auth/accounts/{engine}/{id}/reset",
            post(reset_account_handler),
        )
        .route(
            "/api/v1/auth/accounts/{engine}/{id}/import-local",
            post(import_local_handler),
        )
        .route(
            "/api/v1/auth/accounts/{engine}/{id}/verify",
            post(verify_account_handler),
        );

    let api = Router::new()
        .route("/api/v1/engines", get(list_engines_handler))
        .route(
            "/api/v1/agents",
            get(list_agents_handler).post(create_agent_handler),
        )
        .route("/api/v1/tasks/run", post(run_task_handler))
        .route(
            "/api/v1/tasks",
            post(submit_task_handler).get(list_tasks_handler),
        )
        .route("/api/v1/tasks/{id}", get(get_task_handler))
        .route("/api/v1/tasks/{id}/cancel", post(cancel_task_handler))
        .route("/api/v1/tasks/{id}/approve", post(approve_task_handler))
        .route("/api/v1/tasks/{id}/review", post(review_task_handler))
        .route("/api/v1/ws", get(ws_handler))
        .merge(gateway)
        .route_layer(middleware::from_fn_with_state(auth, require_auth));

    Router::new()
        .route("/", get(dashboard_handler))
        .route("/dashboard", get(dashboard_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics::metrics_handler))
        .merge(api)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
pub mod tests;
