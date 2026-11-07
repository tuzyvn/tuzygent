use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tokio::task::JoinSet;
use tower::ServiceExt;

use agent_core::prelude::*;
use agent_server::dashboard_html::{
    DashboardBenchmarkHarness, DashboardBenchmarkStats, DashboardContext, DashboardFaultPolicy,
    ResilientDashboardService, ShardedResilientDashboardService,
};
use agent_server::{build_app, AppState, AuthState};

fn calculate_percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pct_clamped = pct.clamp(0.0, 1.0);
    let idx = ((sorted.len() as f64) * pct_clamped).floor() as usize;
    sorted[idx.min(sorted.len().saturating_sub(1))]
}

fn create_load_test_app() -> Router {
    let bus = InterAgentBus::new(16);
    let temp_vault =
        std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4().simple()));
    let task_store =
        Arc::new(agent_runtime::SqliteTaskStore::memory().expect("sqlite memory store"));
    let task_service = Arc::new(agent_runtime::TaskService::new(
        task_store, "/tmp", 2, None, None,
    ));
    build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
            metrics: Arc::new(agent_server::metrics::ResilientMetricsService::default()),
        },
        AuthState::new(Arc::new("test_token".into()), vec![]),
    )
}

fn print_stats_table(title: &str, stats: &DashboardBenchmarkStats) {
    println!("\n================================================================================");
    println!(" BENCHMARK REPORT: {title}");
    println!("================================================================================");
    println!(" Total Requests     : {:>10}", stats.total_requests);
    println!(" Successful Renders : {:>10}", stats.successful_renders);
    println!(" Fallback Renders   : {:>10}", stats.fallback_renders);
    println!(" Rejected Count     : {:>10}", stats.rejected_count);
    println!(
        " Total Duration     : {:>10.3} ms",
        stats.total_duration.as_secs_f64() * 1000.0
    );
    println!(
        " Throughput         : {:>10.2} ops/sec",
        stats.throughput_ops_per_sec
    );
    println!(
        " P50 Latency        : {:>10.2} µs ({:.4} ms)",
        stats.p50_latency_us,
        stats.p50_latency_us / 1000.0
    );
    println!(
        " P90 Latency        : {:>10.2} µs ({:.4} ms)",
        stats.p90_latency_us,
        stats.p90_latency_us / 1000.0
    );
    println!(
        " P99 Latency        : {:>10.2} µs ({:.4} ms)",
        stats.p99_latency_us,
        stats.p99_latency_us / 1000.0
    );
    println!(
        " Max Latency        : {:>10.2} µs ({:.4} ms)",
        stats.max_latency_us,
        stats.max_latency_us / 1000.0
    );
    println!(
        " Memory Footprint   : {:>10} bytes ({:.2} KB)",
        stats.memory_allocated_bytes,
        stats.memory_allocated_bytes as f64 / 1024.0
    );
    println!("================================================================================\n");
}

/// TDD test case for TOPIC-20261105-0c42a5 & TOPIC-20261105-42d5da:
/// Benchmark high concurrency (10,000 requests) with 16-way sharded execution service.
#[tokio::test]
async fn test_dashboard_load_10k_concurrent_requests_sharded() {
    const TOTAL_REQUESTS: usize = 10_000;
    const CONCURRENCY: usize = 16;

    println!("\n>>> Running 10,000 concurrent dashboard render requests (16-way sharded)...");
    let stats =
        DashboardBenchmarkHarness::run_concurrent_benchmark(TOTAL_REQUESTS, CONCURRENCY).await;

    print_stats_table("10,000 Concurrent Requests (16-Way Sharded Mutex)", &stats);

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_renders, 9_000);
    assert_eq!(stats.fallback_renders, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 1_000.0,
        "Throughput should exceed 1,000 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(
        stats.p99_latency_us < 50_000.0,
        "P99 latency must be under 50ms, got {} µs",
        stats.p99_latency_us
    );
    assert!(stats.p50_latency_us > 0.0);
    assert!(stats.p90_latency_us >= stats.p50_latency_us);
    assert!(stats.p99_latency_us >= stats.p90_latency_us);
}

/// TDD test case for TOPIC-20261105-0c42a5:
/// Spawns 10,000 simultaneous Tokio tasks inflight.
#[tokio::test]
async fn test_dashboard_load_10k_simultaneous_tasks() {
    const TOTAL_REQUESTS: usize = 10_000;
    const NUM_SHARDS: usize = 16;

    let service = Arc::new(
        ShardedResilientDashboardService::new(NUM_SHARDS)
            .with_max_capacity(TOTAL_REQUESTS + 128)
            .with_cache(TOTAL_REQUESTS + 64),
    );

    println!("\n>>> Spawning 10,000 simultaneous Tokio tasks for dashboard rendering...");
    let start_time = Instant::now();
    let mut join_set = JoinSet::new();

    for req_idx in 0..TOTAL_REQUESTS {
        let s_clone = Arc::clone(&service);
        join_set.spawn(async move {
            let is_err = req_idx.is_multiple_of(10);
            let (ctx, err_flag) =
                DashboardBenchmarkHarness::make_benchmark_context(req_idx, is_err);
            let key = format!("dashboard_task_{req_idx}");
            let req_start = Instant::now();
            let (html, status) = s_clone.render_dashboard_with_fallback(&key, &ctx).await;
            let latency_us = req_start.elapsed().as_secs_f64() * 1_000_000.0;
            let success = status.is_success() && !err_flag && !html.is_empty();
            let fallback = !status.is_success() || err_flag;
            (latency_us, success, fallback)
        });
    }

    let mut latencies_us = Vec::with_capacity(TOTAL_REQUESTS);
    let mut success_count = 0;
    let mut fallback_count = 0;

    while let Some(res) = join_set.join_next().await {
        let (lat, success, fallback) = res.expect("tokio task should not panic");
        latencies_us.push(lat);
        if success {
            success_count += 1;
        } else if fallback {
            fallback_count += 1;
        }
    }

    let total_duration = start_time.elapsed();
    latencies_us.sort_by(|a, b| a.total_cmp(b));

    let p50 = calculate_percentile(&latencies_us, 0.50);
    let p90 = calculate_percentile(&latencies_us, 0.90);
    let p99 = calculate_percentile(&latencies_us, 0.99);
    let max = latencies_us.last().copied().unwrap_or(0.0);
    let secs = total_duration.as_secs_f64();
    let throughput = if secs > 0.0 {
        TOTAL_REQUESTS as f64 / secs
    } else {
        0.0
    };
    let mem_usage = service.memory_usage_bytes().await;

    let stats = DashboardBenchmarkStats {
        total_requests: TOTAL_REQUESTS,
        successful_renders: success_count,
        fallback_renders: fallback_count,
        rejected_count: 0,
        p50_latency_us: p50,
        p90_latency_us: p90,
        p99_latency_us: p99,
        max_latency_us: max,
        total_duration,
        throughput_ops_per_sec: throughput,
        memory_allocated_bytes: mem_usage,
    };

    print_stats_table("10,000 Independent Tokio Tasks Inflight", &stats);

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_renders, 9_000);
    assert_eq!(stats.fallback_renders, 1_000);
    assert!(stats.throughput_ops_per_sec > 1_000.0);
    assert!(stats.p99_latency_us < 50_000.0);
}

/// TDD test case: Lock contention comparison between single lock vs 16-way sharded.
#[tokio::test]
async fn test_dashboard_load_10k_lock_contention_comparison() {
    const REQUEST_COUNT: usize = 10_000;

    println!("\n>>> Running dashboard lock contention comparison under 10,000 requests...");
    let (single, sharded) = DashboardBenchmarkHarness::compare_concurrency(REQUEST_COUNT).await;

    print_stats_table("Single Mutex Lock (Baseline)", &single);
    print_stats_table("16-Way Sharded Mutex (Optimized)", &sharded);

    assert_eq!(single.total_requests, REQUEST_COUNT);
    assert_eq!(sharded.total_requests, REQUEST_COUNT);
    assert_eq!(single.successful_renders, 9_000);
    assert_eq!(sharded.successful_renders, 9_000);
    assert!(single.throughput_ops_per_sec > 1_000.0);
    assert!(sharded.throughput_ops_per_sec > 1_000.0);
    assert!(sharded.p99_latency_us < 50_000.0);

    println!(
        "[Analysis] Sharded Throughput: {:.2} ops/sec vs Single: {:.2} ops/sec",
        sharded.throughput_ops_per_sec, single.throughput_ops_per_sec
    );
    println!(
        "[Analysis] Sharded P99 Latency: {:.2} µs vs Single: {:.2} µs",
        sharded.p99_latency_us, single.p99_latency_us
    );
}

/// TDD test case: 10,000 HTTP requests simulation against `/dashboard`.
#[tokio::test]
async fn test_dashboard_load_10k_http_endpoint_simulation() {
    const TOTAL_REQUESTS: usize = 10_000;
    const CONCURRENCY: usize = 20;
    let app = create_load_test_app();

    println!("\n>>> Simulating 10,000 HTTP requests to /dashboard endpoint...");
    let start_time = Instant::now();
    let per_worker = TOTAL_REQUESTS / CONCURRENCY;
    let mut handles = Vec::with_capacity(CONCURRENCY);

    for _ in 0..CONCURRENCY {
        let app_clone = app.clone();
        handles.push(tokio::spawn(async move {
            let mut local_latencies = Vec::with_capacity(per_worker);
            let mut local_success = 0;

            for _ in 0..per_worker {
                let req = Request::builder()
                    .method("GET")
                    .uri("/dashboard")
                    .body(Body::empty())
                    .expect("valid http request");

                let t0 = Instant::now();
                let resp = app_clone
                    .clone()
                    .oneshot(req)
                    .await
                    .expect("service response");
                let elapsed_us = t0.elapsed().as_secs_f64() * 1_000_000.0;
                local_latencies.push(elapsed_us);

                if resp.status() == StatusCode::OK {
                    local_success += 1;
                }
            }

            (local_latencies, local_success)
        }));
    }

    let mut all_latencies = Vec::with_capacity(TOTAL_REQUESTS);
    let mut total_success = 0;

    for handle in handles {
        let (latencies, success) = handle.await.expect("worker task completed");
        all_latencies.extend(latencies);
        total_success += success;
    }

    let total_duration = start_time.elapsed();
    all_latencies.sort_by(|a, b| a.total_cmp(b));

    let p50 = calculate_percentile(&all_latencies, 0.50);
    let p90 = calculate_percentile(&all_latencies, 0.90);
    let p99 = calculate_percentile(&all_latencies, 0.99);
    let max = all_latencies.last().copied().unwrap_or(0.0);
    let secs = total_duration.as_secs_f64();
    let throughput = if secs > 0.0 {
        TOTAL_REQUESTS as f64 / secs
    } else {
        0.0
    };

    let stats = DashboardBenchmarkStats {
        total_requests: TOTAL_REQUESTS,
        successful_renders: total_success,
        fallback_renders: 0,
        rejected_count: 0,
        p50_latency_us: p50,
        p90_latency_us: p90,
        p99_latency_us: p99,
        max_latency_us: max,
        total_duration,
        throughput_ops_per_sec: throughput,
        memory_allocated_bytes: 0,
    };

    print_stats_table("10,000 HTTP /dashboard Requests", &stats);

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_renders, TOTAL_REQUESTS);
    assert!(stats.throughput_ops_per_sec > 1_000.0);
    assert!(stats.p99_latency_us < 50_000.0);
}

/// TDD test case: Proves zero memory leaks with bounded memory footprint under repeated 10k bursts.
#[tokio::test]
async fn test_dashboard_load_10k_bounded_memory_ring_buffer() {
    const TOTAL_REQUESTS: usize = 10_000;
    const BOUNDED_CAPACITY: usize = 1_000;

    let service = ResilientDashboardService::new()
        .with_fault_policy(DashboardFaultPolicy::none())
        .with_max_capacity(BOUNDED_CAPACITY);

    println!("\n>>> Measuring memory stability under multiple 10,000-request bursts...");

    let stats_1 = DashboardBenchmarkHarness::run_benchmark(&service, TOTAL_REQUESTS).await;
    let mem_burst_1 = service.memory_usage_bytes();

    let stats_2 = DashboardBenchmarkHarness::run_benchmark(&service, TOTAL_REQUESTS).await;
    let mem_burst_2 = service.memory_usage_bytes();

    let stats_3 = DashboardBenchmarkHarness::run_benchmark(&service, TOTAL_REQUESTS).await;
    let mem_burst_3 = service.memory_usage_bytes();

    print_stats_table(
        "10,000 Requests Burst 1 (Bounded Memory 1,000 entries)",
        &stats_1,
    );
    print_stats_table(
        "10,000 Requests Burst 2 (Bounded Memory 1,000 entries)",
        &stats_2,
    );
    print_stats_table(
        "10,000 Requests Burst 3 (Bounded Memory 1,000 entries)",
        &stats_3,
    );

    println!(
        "[Memory Footprint] Burst 1: {} bytes, Burst 2: {} bytes, Burst 3: {} bytes",
        mem_burst_1, mem_burst_2, mem_burst_3
    );

    assert_eq!(
        mem_burst_1, mem_burst_2,
        "Memory must stabilize and remain strictly bounded after reaching capacity"
    );
    assert_eq!(
        mem_burst_2, mem_burst_3,
        "Memory must not grow across successive 10,000-request bursts"
    );
}

/// TDD test case: High-throughput batch rendering and caching efficiency.
#[tokio::test]
async fn test_dashboard_batch_and_cache_performance() {
    const BATCH_SIZE: usize = 1_000;
    let service = ResilientDashboardService::new().with_cache(BATCH_SIZE * 2);

    let contexts: Vec<DashboardContext> = (0..BATCH_SIZE)
        .map(|i| DashboardContext {
            title: Some(format!("Batch Dashboard {i}")),
            version: Some("1.0.0-batch".to_string()),
            ..DashboardContext::default()
        })
        .collect();

    println!("\n>>> Executing batch rendering of 1,000 dashboard contexts...");
    let t0 = Instant::now();
    let results = service.render_batch(&contexts);
    let duration = t0.elapsed();

    assert_eq!(results.len(), BATCH_SIZE);
    for res in results {
        assert!(res.is_ok());
    }

    let throughput = BATCH_SIZE as f64 / duration.as_secs_f64();
    println!(
        "Batch Rendering: 1,000 items in {:.3} ms ({:.2} ops/sec)",
        duration.as_secs_f64() * 1000.0,
        throughput
    );
    assert!(throughput > 1_000.0);

    // Warm cache vs Cold cache
    let cache_key = "cached_test_node";
    let ctx = contexts[0].clone();

    // Cold cache miss
    let cold_t0 = Instant::now();
    let res_cold = service.render_dashboard_cached(cache_key, &ctx).await;
    let cold_dur = cold_t0.elapsed();
    assert!(res_cold.is_ok());

    // Warm cache hit
    let warm_t0 = Instant::now();
    let res_warm = service.render_dashboard_cached(cache_key, &ctx).await;
    let warm_dur = warm_t0.elapsed();
    assert!(res_warm.is_ok());

    assert_eq!(res_cold.expect("rendered"), res_warm.expect("rendered"));
    println!(
        "Cold Render Duration: {:.2} µs vs Warm Cache Render Duration: {:.2} µs",
        cold_dur.as_secs_f64() * 1_000_000.0,
        warm_dur.as_secs_f64() * 1_000_000.0
    );

    let stats = service.cache_stats().expect("cache stats present");
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
}
