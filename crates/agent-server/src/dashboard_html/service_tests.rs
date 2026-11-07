use super::{ResilientDashboardService, ShardedDashboardCache};
use crate::dashboard_html::{
    DashboardContext, DashboardFault, DashboardFaultPolicy, FALLBACK_DASHBOARD_HTML,
};
use axum::http::StatusCode;
use std::sync::Arc;
use std::time::Duration;

fn custom_template_dir() -> std::path::PathBuf {
    static NEXT_DIR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "agent-server-dashboard-test-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("dashboard.html"), "<title>{{title}}</title>").unwrap();
    dir
}

#[test]
fn new_uses_documented_defaults() {
    let service = ResilientDashboardService::new();

    assert_eq!(service.max_retries, 3);
    assert_eq!(service.backoff, Duration::from_millis(5));
    assert!(service.custom_asset_dir.is_none());
    assert!(service.cache().is_none());
    assert_eq!(service.max_capacity(), None);
    assert_eq!(service.rendered_count(), 0);
}

#[test]
fn retry_backoff_and_custom_asset_builders_store_their_values() {
    let directory = std::path::PathBuf::from("dashboard-fixtures");
    let service = ResilientDashboardService::new()
        .with_max_retries(7)
        .with_backoff(Duration::from_millis(23))
        .with_custom_asset_dir(directory.clone());

    assert_eq!(service.max_retries, 7);
    assert_eq!(service.backoff, Duration::from_millis(23));
    assert_eq!(
        service.custom_asset_dir.as_deref(),
        Some(directory.as_path())
    );
}

#[test]
fn with_fault_policy_installs_a_mutable_policy_handle() {
    let service = ResilientDashboardService::new()
        .with_fault_policy(DashboardFaultPolicy::always(DashboardFault::SocketTimeout));
    let policy = service.fault_policy();

    assert_eq!(policy.lock().unwrap().injected_count(), 0);
    assert!(matches!(
        policy.lock().unwrap().should_inject(),
        Some(DashboardFault::SocketTimeout)
    ));
    assert_eq!(policy.lock().unwrap().injected_count(), 1);
}

#[test]
fn with_cache_creates_an_empty_cache_with_requested_capacity() {
    let service = ResilientDashboardService::new().with_cache(8);

    assert_eq!(service.cache_stats().unwrap().total_entries, 0);
    assert!(service.memory_usage_bytes() >= std::mem::size_of_val(&service));
}

#[test]
fn fault_policy_returns_the_shared_policy_handle() {
    let service = ResilientDashboardService::new()
        .with_fault_policy(DashboardFaultPolicy::always(DashboardFault::SocketTimeout));
    let policy = service.fault_policy();

    assert!(matches!(
        policy.lock().unwrap().should_inject(),
        Some(DashboardFault::SocketTimeout)
    ));
    let service_policy = service.fault_policy();
    assert!(Arc::ptr_eq(&policy, &service_policy));
    assert_eq!(service_policy.lock().unwrap().injected_count(), 1);
}

#[test]
fn memory_usage_includes_recorded_history_and_cache() {
    let mut service = ResilientDashboardService::new().with_max_capacity(2);
    let baseline = service.memory_usage_bytes();
    service.record_history("a substantial history entry".into());
    assert!(service.memory_usage_bytes() > baseline);

    let cached = ResilientDashboardService::new().with_cache(4);
    assert!(cached.memory_usage_bytes() > ResilientDashboardService::new().memory_usage_bytes());
}

#[tokio::test]
async fn render_dashboard_substitutes_context_in_custom_template() {
    let dir = custom_template_dir();
    let ctx = DashboardContext::new().with_title("Test dashboard");
    let html = ResilientDashboardService::new()
        .with_custom_asset_dir(&dir)
        .render_dashboard(&ctx)
        .await
        .unwrap();

    assert_eq!(html, "<title>Test dashboard</title>");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn render_dashboard_cached_stores_and_reuses_rendered_html() {
    let dir = custom_template_dir();
    let service = ResilientDashboardService::new()
        .with_cache(8)
        .with_custom_asset_dir(&dir);
    let original = service
        .render_dashboard_cached("dashboard", &DashboardContext::new().with_title("First"))
        .await
        .unwrap();
    let from_cache = service
        .render_dashboard_cached("dashboard", &DashboardContext::new().with_title("Second"))
        .await
        .unwrap();

    assert_eq!(from_cache, original);
    assert_eq!(from_cache, "<title>First</title>");
    assert_eq!(service.cache_stats().unwrap().total_entries, 1);
    assert_eq!(service.cache_stats().unwrap().hits, 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn render_dashboard_with_fallback_returns_html_and_status() {
    let (html, status) = ResilientDashboardService::new()
        .render_dashboard_with_fallback(&DashboardContext::new())
        .await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<!DOCTYPE html>"));

    let failing = ResilientDashboardService::new()
        .with_fault_policy(DashboardFaultPolicy::always(DashboardFault::MissingAsset));
    let (fallback_html, fallback_status) = failing
        .render_dashboard_with_fallback(&DashboardContext::new())
        .await;
    assert_eq!(fallback_html, FALLBACK_DASHBOARD_HTML);
    assert_eq!(fallback_status, StatusCode::NOT_FOUND);
}

#[test]
fn render_batch_renders_each_context_independently() {
    let dir = custom_template_dir();
    let contexts = [
        DashboardContext::new().with_title("Batch one"),
        DashboardContext::new().with_title("Batch two"),
    ];
    let results = ResilientDashboardService::new()
        .with_custom_asset_dir(&dir)
        .render_batch(&contexts);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap(), "<title>Batch one</title>");
    assert_eq!(results[1].as_ref().unwrap(), "<title>Batch two</title>");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn get_asset_serves_embedded_asset_and_reports_missing_asset() {
    let service = ResilientDashboardService::new();
    let (css, content_type, status) = service.get_asset("style.css").await;
    assert_eq!(content_type, "text/css; charset=utf-8");
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(css).unwrap().contains("--bg"));

    let (missing, content_type, status) = service.get_asset("no-such-asset").await;
    assert_eq!(content_type, "text/plain; charset=utf-8");
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(String::from_utf8(missing)
        .unwrap()
        .contains("Missing asset"));
}

#[test]
fn max_capacity_reports_configured_history_limit() {
    assert_eq!(ResilientDashboardService::new().max_capacity(), None);
    assert_eq!(
        ResilientDashboardService::new()
            .with_max_capacity(2)
            .max_capacity(),
        Some(2)
    );
}

#[test]
fn record_history_keeps_only_the_newest_entries_within_capacity() {
    let mut service = ResilientDashboardService::new().with_max_capacity(2);
    service.record_history("first".to_string());
    service.record_history("second".to_string());
    service.record_history("third".to_string());

    assert_eq!(
        service
            .history
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["second", "third"]
    );
}

#[test]
fn reset_counts_clears_all_render_counters() {
    let mut service = ResilientDashboardService::new();
    service.rendered_count = 3;
    service.fallback_count = 2;
    service.error_count = 1;

    assert_eq!(service.rendered_count(), 3);
    assert_eq!(service.fallback_count(), 2);
    assert_eq!(service.error_count(), 1);

    service.reset_counts();

    assert_eq!(service.rendered_count(), 0);
    assert_eq!(service.fallback_count(), 0);
    assert_eq!(service.error_count(), 0);
}

#[test]
fn with_shared_cache_uses_the_supplied_cache() {
    let cache = Arc::new(ShardedDashboardCache::new(2, 8));
    let service = ResilientDashboardService::new().with_shared_cache(cache.clone());

    assert!(Arc::ptr_eq(
        service.cache().expect("cache configured"),
        &cache
    ));
    assert!(service.cache_stats().is_some());
}
