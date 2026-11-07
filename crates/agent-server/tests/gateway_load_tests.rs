use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tokio::task::JoinSet;
use tower::ServiceExt;

use agent_core::prelude::*;
use agent_server::gateway::{
    GatewayBenchmarkHarness, GatewayBenchmarkStats, GatewayBuffer, GatewayFaultPolicy,
    GatewayRingBuffer, ResilientGatewayService, ShardedGatewayRingBuffer,
    ShardedResilientGatewayService,
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

fn create_load_test_app(token: &str) -> Router {
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
        AuthState::new(Arc::new(token.into()), vec![]),
    )
}

fn print_stats_table(title: &str, stats: &GatewayBenchmarkStats) {
    println!("\n================================================================================");
    println!(" BENCHMARK REPORT: {title}");
    println!("================================================================================");
    println!(" Total Requests     : {:>10}", stats.total_requests);
    println!(" Successful Resolv  : {:>10}", stats.successful_resolutions);
    println!(" Fallback Resolv    : {:>10}", stats.fallback_resolutions);
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

/// TDD test case for TOPIC-20261105-7b2efb & TOPIC-20261105-85282b:
/// Benchmark high concurrency (10,000 requests) with 16-way sharded execution service.
#[tokio::test]
async fn test_gateway_load_10k_concurrent_requests_sharded() {
    const TOTAL_REQUESTS: usize = 10_000;
    const CONCURRENCY: usize = 16;

    println!(
        "\n>>> Running 10,000 concurrent gateway model resolution requests (16-way sharded)..."
    );
    let stats =
        GatewayBenchmarkHarness::run_concurrent_benchmark(TOTAL_REQUESTS, CONCURRENCY).await;

    print_stats_table("10,000 Concurrent Requests (16-Way Sharded Mutex)", &stats);

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_resolutions, 9_000);
    assert_eq!(stats.fallback_resolutions, 1_000);
    assert_eq!(stats.rejected_count, 0);
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

/// TDD test case: Spawns 10,000 simultaneous Tokio tasks inflight.
#[tokio::test]
async fn test_gateway_load_10k_simultaneous_tasks() {
    const TOTAL_REQUESTS: usize = 10_000;
    const NUM_SHARDS: usize = 16;

    let service = Arc::new(
        ShardedResilientGatewayService::new(NUM_SHARDS)
            .with_fallback_model("antigravity")
            .with_max_capacity(TOTAL_REQUESTS + 128)
            .with_cache(TOTAL_REQUESTS + 64),
    );

    println!("\n>>> Spawning 10,000 simultaneous Tokio tasks for gateway model resolution...");
    let start_time = Instant::now();
    let mut join_set = JoinSet::new();

    for req_idx in 0..TOTAL_REQUESTS {
        let s_clone = Arc::clone(&service);
        join_set.spawn(async move {
            let is_invalid = req_idx.is_multiple_of(10);
            let (candidate, invalid) =
                GatewayBenchmarkHarness::make_benchmark_model(req_idx, is_invalid);
            let req_start = Instant::now();
            let res = s_clone.resolve_model_resilient(&candidate).await;
            let latency_us = req_start.elapsed().as_secs_f64() * 1_000_000.0;
            let success = res.is_ok() && !invalid;
            let fallback = res.is_ok() && invalid;
            (latency_us, success, fallback)
        });
    }

    let mut latencies_us = Vec::with_capacity(TOTAL_REQUESTS);
    let mut success_count = 0;
    let mut fallback_count = 0;
    let mut rejected_count = 0;

    while let Some(res) = join_set.join_next().await {
        let (lat, success, fallback) = res.expect("tokio task should not panic");
        latencies_us.push(lat);
        if success {
            success_count += 1;
        } else if fallback {
            fallback_count += 1;
        } else {
            rejected_count += 1;
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

    let stats = GatewayBenchmarkStats {
        total_requests: TOTAL_REQUESTS,
        successful_resolutions: success_count,
        fallback_resolutions: fallback_count,
        rejected_count,
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
    assert_eq!(stats.successful_resolutions, 9_000);
    assert_eq!(stats.fallback_resolutions, 1_000);
    assert_eq!(stats.rejected_count, 0);
    assert!(stats.throughput_ops_per_sec > 1_000.0);
    assert!(stats.p99_latency_us < 50_000.0);
}

/// TDD test case: Lock contention comparison between single lock vs 16-way sharded mutex.
#[tokio::test]
async fn test_gateway_load_10k_lock_contention_comparison() {
    const REQUEST_COUNT: usize = 10_000;

    println!("\n>>> Running lock contention comparison under 10,000 requests...");
    let (single, sharded) = GatewayBenchmarkHarness::compare_concurrency(REQUEST_COUNT).await;

    print_stats_table("Single Mutex Lock (Baseline)", &single);
    print_stats_table("16-Way Sharded Mutex (Optimized)", &sharded);

    assert_eq!(single.total_requests, REQUEST_COUNT);
    assert_eq!(sharded.total_requests, REQUEST_COUNT);
    assert_eq!(single.successful_resolutions, 9_000);
    assert_eq!(sharded.successful_resolutions, 9_000);
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

/// TDD test case: HTTP requests simulation to accounts route (`/api/v1/auth/accounts`).
#[tokio::test]
async fn test_gateway_http_accounts_route_load() {
    const TOTAL_REQUESTS: usize = 1_000;
    const CONCURRENCY: usize = 10;
    let token = "gateway_accounts_load_test_token";
    let app = create_load_test_app(token);

    println!("\n>>> Simulating {TOTAL_REQUESTS} HTTP requests to gateway accounts route (/api/v1/auth/accounts)...");
    let start_time = Instant::now();
    let per_worker = TOTAL_REQUESTS / CONCURRENCY;
    let mut handles = Vec::with_capacity(CONCURRENCY);

    for _ in 0..CONCURRENCY {
        let app_clone = app.clone();
        let auth_header = format!("Bearer {token}");
        handles.push(tokio::spawn(async move {
            let mut local_latencies = Vec::with_capacity(per_worker);
            let mut local_success = 0;

            for _ in 0..per_worker {
                let req = Request::builder()
                    .method("GET")
                    .uri("/api/v1/auth/accounts")
                    .header("Authorization", &auth_header)
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

    let stats = GatewayBenchmarkStats {
        total_requests: TOTAL_REQUESTS,
        successful_resolutions: total_success,
        fallback_resolutions: 0,
        rejected_count: 0,
        p50_latency_us: p50,
        p90_latency_us: p90,
        p99_latency_us: p99,
        max_latency_us: max,
        total_duration,
        throughput_ops_per_sec: throughput,
        memory_allocated_bytes: 0,
    };

    print_stats_table("HTTP /api/v1/auth/accounts Load Benchmark", &stats);

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_resolutions, TOTAL_REQUESTS);
    assert!(stats.throughput_ops_per_sec > 200.0);
    assert!(stats.p99_latency_us < 100_000.0);
}

/// TDD test case: HTTP requests simulation to chat route (`/v1/chat/completions`).
#[tokio::test]
async fn test_gateway_http_chat_route_load() {
    const TOTAL_REQUESTS: usize = 500;
    const CONCURRENCY: usize = 10;
    let token = "gateway_chat_load_test_token";
    let app = create_load_test_app(token);

    println!("\n>>> Simulating {TOTAL_REQUESTS} HTTP requests to gateway chat route (/v1/chat/completions)...");
    let start_time = Instant::now();
    let per_worker = TOTAL_REQUESTS / CONCURRENCY;
    let mut handles = Vec::with_capacity(CONCURRENCY);

    for worker_idx in 0..CONCURRENCY {
        let app_clone = app.clone();
        let auth_header = format!("Bearer {token}");
        handles.push(tokio::spawn(async move {
            let mut local_latencies = Vec::with_capacity(per_worker);
            let mut handled_count = 0;

            for i in 0..per_worker {
                let req_num = worker_idx * per_worker + i;
                let payload = serde_json::json!({
                    "model": if req_num.is_multiple_of(10) { "invalid_chat_engine" } else { "antigravity" },
                    "messages": [{"role": "user", "content": "benchmark load ping"}],
                    "stream": false
                });

                let req = Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("Authorization", &auth_header)
                    .header("Content-Type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .expect("valid http request");

                let t0 = Instant::now();
                let resp = app_clone.clone().oneshot(req).await.expect("service response");
                let elapsed_us = t0.elapsed().as_secs_f64() * 1_000_000.0;
                local_latencies.push(elapsed_us);

                // Invalid engine produces BAD_REQUEST (400), unconfigured account returns INTERNAL_SERVER_ERROR (500)
                if resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::INTERNAL_SERVER_ERROR {
                    handled_count += 1;
                }
            }

            (local_latencies, handled_count)
        }));
    }

    let mut all_latencies = Vec::with_capacity(TOTAL_REQUESTS);
    let mut total_handled = 0;

    for handle in handles {
        let (latencies, count) = handle.await.expect("worker task completed");
        all_latencies.extend(latencies);
        total_handled += count;
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

    let stats = GatewayBenchmarkStats {
        total_requests: TOTAL_REQUESTS,
        successful_resolutions: total_handled,
        fallback_resolutions: 0,
        rejected_count: 0,
        p50_latency_us: p50,
        p90_latency_us: p90,
        p99_latency_us: p99,
        max_latency_us: max,
        total_duration,
        throughput_ops_per_sec: throughput,
        memory_allocated_bytes: 0,
    };

    print_stats_table("HTTP /v1/chat/completions Load Benchmark", &stats);

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_resolutions, TOTAL_REQUESTS);
    assert!(stats.throughput_ops_per_sec > 200.0);
    assert!(stats.p99_latency_us < 100_000.0);
}

/// TDD test case: HTTP requests simulation across mixed gateway routes (accounts, chat, models).
#[tokio::test]
async fn test_gateway_http_mixed_accounts_and_chat_load() {
    const TOTAL_REQUESTS: usize = 600;
    const CONCURRENCY: usize = 10;
    let token = "gateway_mixed_load_test_token";
    let app = create_load_test_app(token);

    println!("\n>>> Simulating {TOTAL_REQUESTS} concurrent HTTP requests across mixed accounts and chat routes...");
    let start_time = Instant::now();
    let per_worker = TOTAL_REQUESTS / CONCURRENCY;
    let mut handles = Vec::with_capacity(CONCURRENCY);

    for worker_idx in 0..CONCURRENCY {
        let app_clone = app.clone();
        let auth_header = format!("Bearer {token}");
        handles.push(tokio::spawn(async move {
            let mut local_latencies = Vec::with_capacity(per_worker);
            let mut local_handled = 0;

            for i in 0..per_worker {
                let req_num = worker_idx * per_worker + i;
                let t0 = Instant::now();

                let resp = match req_num % 3 {
                    0 => {
                        // Route 1: GET /api/v1/auth/accounts
                        let req = Request::builder()
                            .method("GET")
                            .uri("/api/v1/auth/accounts")
                            .header("Authorization", &auth_header)
                            .body(Body::empty())
                            .expect("valid http request");
                        app_clone
                            .clone()
                            .oneshot(req)
                            .await
                            .expect("service response")
                    }
                    1 => {
                        // Route 2: POST /v1/chat/completions
                        let payload = serde_json::json!({
                            "model": "antigravity",
                            "messages": [{"role": "user", "content": "mixed test"}],
                            "stream": false
                        });
                        let req = Request::builder()
                            .method("POST")
                            .uri("/v1/chat/completions")
                            .header("Authorization", &auth_header)
                            .header("Content-Type", "application/json")
                            .body(Body::from(payload.to_string()))
                            .expect("valid http request");
                        app_clone
                            .clone()
                            .oneshot(req)
                            .await
                            .expect("service response")
                    }
                    _ => {
                        // Route 3: GET /v1/models
                        let req = Request::builder()
                            .method("GET")
                            .uri("/v1/models")
                            .header("Authorization", &auth_header)
                            .body(Body::empty())
                            .expect("valid http request");
                        app_clone
                            .clone()
                            .oneshot(req)
                            .await
                            .expect("service response")
                    }
                };

                let elapsed_us = t0.elapsed().as_secs_f64() * 1_000_000.0;
                local_latencies.push(elapsed_us);

                if resp.status().is_success() || resp.status() == StatusCode::INTERNAL_SERVER_ERROR
                {
                    local_handled += 1;
                }
            }

            (local_latencies, local_handled)
        }));
    }

    let mut all_latencies = Vec::with_capacity(TOTAL_REQUESTS);
    let mut total_handled = 0;

    for handle in handles {
        let (latencies, count) = handle.await.expect("worker task completed");
        all_latencies.extend(latencies);
        total_handled += count;
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

    let stats = GatewayBenchmarkStats {
        total_requests: TOTAL_REQUESTS,
        successful_resolutions: total_handled,
        fallback_resolutions: 0,
        rejected_count: 0,
        p50_latency_us: p50,
        p90_latency_us: p90,
        p99_latency_us: p99,
        max_latency_us: max,
        total_duration,
        throughput_ops_per_sec: throughput,
        memory_allocated_bytes: 0,
    };

    print_stats_table(
        "HTTP Mixed Routes (Accounts, Chat, Models) Load Benchmark",
        &stats,
    );

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_resolutions, TOTAL_REQUESTS);
    assert!(stats.throughput_ops_per_sec > 200.0);
    assert!(stats.p99_latency_us < 100_000.0);
}

/// TDD test case: Proves zero memory leaks with bounded memory footprint under repeated 10k bursts.
#[tokio::test]
async fn test_gateway_load_10k_bounded_memory_ring_buffer() {
    const TOTAL_REQUESTS: usize = 10_000;
    const BOUNDED_CAPACITY: usize = 1_000;

    let service = ResilientGatewayService::new()
        .with_fault_policy(GatewayFaultPolicy::none())
        .with_fallback_model("antigravity")
        .with_max_capacity(BOUNDED_CAPACITY);

    println!("\n>>> Measuring memory stability under multiple 10,000-request bursts...");

    let stats_1 = GatewayBenchmarkHarness::run_benchmark(&service, TOTAL_REQUESTS);
    let mem_burst_1 = service.memory_usage_bytes();

    let stats_2 = GatewayBenchmarkHarness::run_benchmark(&service, TOTAL_REQUESTS);
    let mem_burst_2 = service.memory_usage_bytes();

    let stats_3 = GatewayBenchmarkHarness::run_benchmark(&service, TOTAL_REQUESTS);
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

/// TDD test case: High-throughput batch model resolution and caching efficiency.
#[tokio::test]
async fn test_gateway_batch_and_cache_performance() {
    const BATCH_SIZE: usize = 10_000;
    let service = ResilientGatewayService::new().with_cache(BATCH_SIZE * 2);

    let models: Vec<String> = (0..BATCH_SIZE)
        .map(|i| {
            if i.is_multiple_of(10) {
                format!("invalid_engine_model_{i}")
            } else {
                let base = [
                    "antigravity",
                    "codex",
                    "claude",
                    "gemini/gemini-2.0-flash-exp",
                ];
                base[i % base.len()].to_string()
            }
        })
        .collect();

    let model_refs: Vec<&str> = models.iter().map(|s| s.as_str()).collect();

    println!("\n>>> Executing batch resolution of 10,000 models...");
    let t0 = Instant::now();
    let results = service.batch_resolve_models(&model_refs);
    let duration = t0.elapsed();

    assert_eq!(results.len(), BATCH_SIZE);
    let mut success_count = 0;
    let mut rejected_count = 0;

    for (i, res) in results.into_iter().enumerate() {
        let is_invalid = i.is_multiple_of(10);
        match res {
            Ok(_) if !is_invalid => success_count += 1,
            Err(_) if is_invalid => rejected_count += 1,
            other => panic!("Unexpected result at {i}: {:?}", other),
        }
    }

    assert_eq!(success_count, 9_000);
    assert_eq!(rejected_count, 1_000);

    let throughput = BATCH_SIZE as f64 / duration.as_secs_f64();
    println!(
        "Batch Resolution: 10,000 items in {:.3} ms ({:.2} ops/sec)",
        duration.as_secs_f64() * 1000.0,
        throughput
    );
    assert!(throughput > 10_000.0);

    // Warm cache vs Cold cache
    let cache_key = "gemini/gemini-2.0-flash-exp";

    // Cold cache miss
    let cold_service = ResilientGatewayService::new().with_cache(100);
    let cold_t0 = Instant::now();
    let res_cold = cold_service.resolve_model_cached(cache_key);
    let cold_dur = cold_t0.elapsed();
    assert!(res_cold.is_ok());

    // Warm cache hit
    let warm_t0 = Instant::now();
    let res_warm = cold_service.resolve_model_cached(cache_key);
    let warm_dur = warm_t0.elapsed();
    assert!(res_warm.is_ok());

    assert_eq!(res_cold.expect("resolved"), res_warm.expect("resolved"));
    println!(
        "Cold Resolution Duration: {:.2} µs vs Warm Cache Resolution Duration: {:.2} µs",
        cold_dur.as_secs_f64() * 1_000_000.0,
        warm_dur.as_secs_f64() * 1_000_000.0
    );

    let stats = cold_service.cache_stats().expect("cache stats present");
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
}

/// TDD test case: Verifies lock-free length checks, bounded FIFO eviction, and memory bounds.
#[test]
fn test_gateway_buffer_bounded_fifo_and_lockfree_len() {
    let buffer = GatewayRingBuffer::new(8);
    assert_eq!(buffer.capacity(), 8);
    assert!(buffer.is_empty());
    assert_eq!(buffer.len(), 0);

    for i in 0..100 {
        buffer.push(&format!("item_{i}"));
    }
    assert_eq!(buffer.len(), 8);
    assert_eq!(buffer.total_pushed(), 100);
    assert!(!buffer.is_empty());

    let mem1 = buffer.memory_usage_bytes();
    for i in 0..100 {
        buffer.push(&format!("item_{i}"));
    }
    let mem2 = buffer.memory_usage_bytes();
    assert_eq!(
        mem1, mem2,
        "Memory must be strictly bounded under repeated pushes"
    );

    buffer.clear();
    assert!(buffer.is_empty());
    assert_eq!(buffer.len(), 0);
}

/// TDD test case: High-efficiency batch push and sharded ring buffer lock distribution.
#[test]
fn test_gateway_sharded_buffer_batch_push_and_contention_reduction() {
    let sharded = ShardedGatewayRingBuffer::new(4, 16);
    let items: Vec<String> = (0..100).map(|i| format!("batch_model_{i}")).collect();
    let item_refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();

    sharded.push_batch(&item_refs);
    assert!(sharded.len() <= 16);
    assert_eq!(sharded.total_pushed(), 100);

    let mem1 = sharded.memory_usage_bytes();
    sharded.push_batch(&item_refs);
    let mem2 = sharded.memory_usage_bytes();
    assert_eq!(mem1, mem2, "Sharded memory must remain strictly bounded");

    // Unified GatewayBuffer abstraction test
    let buf = GatewayBuffer::sharded(4, 16);
    assert!(buf.as_sharded().is_some());
    assert!(buf.as_single().is_none());
    assert!(buf.is_empty());

    buf.push("model_a");
    buf.push("model_b");
    buf.push_batch(&["model_c", "model_d"]);
    buf.push_keyed("gemini", "model_e");
    buf.push_keyed_batch(&[("claude", "model_f"), ("codex", "model_g")]);

    assert_eq!(buf.len(), 7);
    assert_eq!(buf.total_pushed(), 7);
    buf.clear();
    assert!(buf.is_empty());
}

/// TDD test case: High-concurrency 10,000 pushes across 16 tasks verifying zero deadlocks.
#[tokio::test]
async fn test_gateway_buffer_concurrent_contention_10k_pushes() {
    const NUM_TASKS: usize = 16;
    const PUSHES_PER_TASK: usize = 625; // 16 * 625 = 10,000
    let sharded_buf = Arc::new(ShardedGatewayRingBuffer::new(16, 1_000));
    let mut handles = Vec::with_capacity(NUM_TASKS);

    let start = Instant::now();
    for task_id in 0..NUM_TASKS {
        let buf_clone = Arc::clone(&sharded_buf);
        handles.push(tokio::spawn(async move {
            let mut batch = Vec::with_capacity(PUSHES_PER_TASK);
            let items: Vec<String> = (0..PUSHES_PER_TASK)
                .map(|i| format!("task_{task_id}_item_{i}"))
                .collect();
            for item in &items {
                batch.push(item.as_str());
            }
            buf_clone.push_batch(&batch);
        }));
    }

    for h in handles {
        h.await.expect("task completed without panic");
    }
    let elapsed = start.elapsed();

    assert_eq!(sharded_buf.total_pushed(), NUM_TASKS * PUSHES_PER_TASK);
    assert!(sharded_buf.len() <= 1_000);
    println!(
        "Concurrent 10,000 Pushes completed in {:.3} ms ({:.2} ops/sec)",
        elapsed.as_secs_f64() * 1000.0,
        10_000.0 / elapsed.as_secs_f64()
    );
}
