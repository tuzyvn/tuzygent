use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tokio::task::JoinSet;
use tower::ServiceExt;

use agent_core::prelude::*;
use agent_server::auth::{
    AuthBenchmarkHarness, AuthBenchmarkStats, AuthFaultPolicy, AuthState, ResilientAuthValidator,
    ShardedResilientAuthValidator,
};
use agent_server::{build_app, AppState};

fn calculate_percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pct_clamped = pct.clamp(0.0, 1.0);
    let idx = ((sorted.len() as f64) * pct_clamped).floor() as usize;
    sorted[idx.min(sorted.len().saturating_sub(1))]
}

fn create_load_test_app(token: &str, allowed_origins: Vec<&str>) -> Router {
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

fn print_stats_table(title: &str, stats: &AuthBenchmarkStats) {
    println!("\n================================================================================");
    println!(" BENCHMARK REPORT: {title}");
    println!("================================================================================");
    println!(" Total Requests     : {:>10}", stats.total_requests);
    println!(" Successful Ops     : {:>10}", stats.successful_operations);
    println!(" Rejected Ops       : {:>10}", stats.rejected_count);
    println!(" Total Duration     : {:>10.3} ms", stats.total_duration.as_secs_f64() * 1000.0);
    println!(" Throughput         : {:>10.2} ops/sec", stats.throughput_ops_per_sec);
    println!(" P50 Latency        : {:>10.2} µs ({:.4} ms)", stats.p50_latency_us, stats.p50_latency_us / 1000.0);
    println!(" P90 Latency        : {:>10.2} µs ({:.4} ms)", stats.p90_latency_us, stats.p90_latency_us / 1000.0);
    println!(" P99 Latency        : {:>10.2} µs ({:.4} ms)", stats.p99_latency_us, stats.p99_latency_us / 1000.0);
    println!(" Max Latency        : {:>10.2} µs ({:.4} ms)", stats.max_latency_us, stats.max_latency_us / 1000.0);
    println!(" Memory Footprint   : {:>10} bytes ({:.2} KB)", stats.memory_allocated_bytes, stats.memory_allocated_bytes as f64 / 1024.0);
    println!("================================================================================\n");
}

#[tokio::test]
async fn test_auth_load_10k_concurrent_requests_sharded() {
    const TOTAL_REQUESTS: usize = 10_000;
    const CONCURRENCY: usize = 16;
    let valid_token = "secret_auth_token_sharded_10k";

    println!("\n>>> Running 10,000 concurrent requests test with 16 worker tasks (16-way sharded)...");
    let stats = AuthBenchmarkHarness::run_concurrent_benchmark(TOTAL_REQUESTS, CONCURRENCY, valid_token).await;

    print_stats_table("10,000 Concurrent Requests (16-Way Sharded Mutex)", &stats);

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
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

#[tokio::test]
async fn test_auth_load_10k_simultaneous_tasks() {
    const TOTAL_REQUESTS: usize = 10_000;
    const NUM_SHARDS: usize = 16;
    let valid_token = "simultaneous_10k_token";

    let validator = Arc::new(
        ShardedResilientAuthValidator::new(
            Arc::new(valid_token.to_string()),
            vec![],
            AuthFaultPolicy::none(),
            3,
            NUM_SHARDS,
        )
        .with_max_capacity(TOTAL_REQUESTS + 128),
    );

    println!("\n>>> Spawning 10,000 simultaneous Tokio tasks for auth validation...");
    let start_time = Instant::now();
    let mut join_set = JoinSet::new();

    for req_idx in 0..TOTAL_REQUESTS {
        let v_clone = Arc::clone(&validator);
        let t_str = valid_token.to_string();
        join_set.spawn(async move {
            let is_invalid = req_idx.is_multiple_of(10);
            let (candidate, invalid) = AuthBenchmarkHarness::make_benchmark_token(req_idx, is_invalid, &t_str);
            let req_start = Instant::now();
            let res = v_clone.validate_token_resilient(&candidate).await;
            let latency_us = req_start.elapsed().as_secs_f64() * 1_000_000.0;
            let success = matches!(res, Ok(true) if !invalid);
            (latency_us, success)
        });
    }

    let mut latencies_us = Vec::with_capacity(TOTAL_REQUESTS);
    let mut success_count = 0;
    let mut rejected_count = 0;

    while let Some(res) = join_set.join_next().await {
        let (lat, success) = res.expect("tokio task should not panic");
        latencies_us.push(lat);
        if success {
            success_count += 1;
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
    let throughput = if secs > 0.0 { TOTAL_REQUESTS as f64 / secs } else { 0.0 };
    let mem_usage = validator.memory_usage_bytes().await;

    let stats = AuthBenchmarkStats {
        total_requests: TOTAL_REQUESTS,
        successful_operations: success_count,
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
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(stats.throughput_ops_per_sec > 1_000.0);
    assert!(stats.p99_latency_us < 50_000.0);
}

#[tokio::test]
async fn test_auth_load_10k_lock_contention_comparison() {
    const REQUEST_COUNT: usize = 10_000;
    let valid_token = "contention_test_secret";

    println!("\n>>> Running lock contention comparison under 10,000 requests...");
    let (single, sharded) =
        AuthBenchmarkHarness::compare_concurrency(REQUEST_COUNT, valid_token).await;

    print_stats_table("Single Mutex Lock (Baseline)", &single);
    print_stats_table("16-Way Sharded Mutex (Optimized)", &sharded);

    assert_eq!(single.total_requests, REQUEST_COUNT);
    assert_eq!(sharded.total_requests, REQUEST_COUNT);
    assert_eq!(single.successful_operations, 9_000);
    assert_eq!(sharded.successful_operations, 9_000);
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

#[tokio::test]
async fn test_auth_load_10k_http_endpoint_simulation() {
    const TOTAL_REQUESTS: usize = 10_000;
    const CONCURRENCY: usize = 20;
    let valid_token = "http_auth_test_key_10k";
    let app = create_load_test_app(valid_token, vec!["http://localhost:3000"]);

    println!("\n>>> Simulating 10,000 HTTP requests to authenticated API endpoint (/v1/models)...");
    let start_time = Instant::now();
    let per_worker = TOTAL_REQUESTS / CONCURRENCY;
    let mut handles = Vec::with_capacity(CONCURRENCY);

    for worker_idx in 0..CONCURRENCY {
        let app_clone = app.clone();
        let token_str = valid_token.to_string();
        handles.push(tokio::spawn(async move {
            let mut local_latencies = Vec::with_capacity(per_worker);
            let mut local_success = 0;
            let mut local_rejected = 0;

            for i in 0..per_worker {
                let req_idx = worker_idx * per_worker + i;
                let is_invalid = req_idx.is_multiple_of(10);
                let auth_header = if is_invalid {
                    format!("Bearer invalid_token_{req_idx}")
                } else {
                    format!("Bearer {token_str}")
                };

                let req = Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("Authorization", auth_header)
                    .body(Body::empty())
                    .expect("valid http request");

                let t0 = Instant::now();
                let resp = app_clone.clone().oneshot(req).await.expect("service response");
                let elapsed_us = t0.elapsed().as_secs_f64() * 1_000_000.0;
                local_latencies.push(elapsed_us);

                if resp.status() == StatusCode::OK {
                    local_success += 1;
                } else if resp.status() == StatusCode::UNAUTHORIZED {
                    local_rejected += 1;
                }
            }

            (local_latencies, local_success, local_rejected)
        }));
    }

    let mut all_latencies = Vec::with_capacity(TOTAL_REQUESTS);
    let mut total_success = 0;
    let mut total_rejected = 0;

    for handle in handles {
        let (latencies, success, rejected) = handle.await.expect("worker task completed");
        all_latencies.extend(latencies);
        total_success += success;
        total_rejected += rejected;
    }

    let total_duration = start_time.elapsed();
    all_latencies.sort_by(|a, b| a.total_cmp(b));

    let p50 = calculate_percentile(&all_latencies, 0.50);
    let p90 = calculate_percentile(&all_latencies, 0.90);
    let p99 = calculate_percentile(&all_latencies, 0.99);
    let max = all_latencies.last().copied().unwrap_or(0.0);
    let secs = total_duration.as_secs_f64();
    let throughput = if secs > 0.0 { TOTAL_REQUESTS as f64 / secs } else { 0.0 };

    let stats = AuthBenchmarkStats {
        total_requests: TOTAL_REQUESTS,
        successful_operations: total_success,
        rejected_count: total_rejected,
        p50_latency_us: p50,
        p90_latency_us: p90,
        p99_latency_us: p99,
        max_latency_us: max,
        total_duration,
        throughput_ops_per_sec: throughput,
        memory_allocated_bytes: 0,
    };

    print_stats_table("10,000 HTTP /v1/models Requests with require_auth", &stats);

    assert_eq!(stats.total_requests, TOTAL_REQUESTS);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(stats.throughput_ops_per_sec > 500.0);
    assert!(stats.p99_latency_us < 100_000.0);
}

#[tokio::test]
async fn test_auth_load_10k_bounded_memory_ring_buffer() {
    const TOTAL_REQUESTS: usize = 10_000;
    const BOUNDED_CAPACITY: usize = 1_000;
    let valid_token = "bounded_mem_10k_token";

    let mut validator = ResilientAuthValidator::new(
        Arc::new(valid_token.to_string()),
        vec![],
        AuthFaultPolicy::none(),
        3,
    )
    .with_max_capacity(BOUNDED_CAPACITY);

    println!("\n>>> Measuring memory stability under multiple 10,000-request bursts...");

    // Burst 1: 10,000 requests
    let stats_1 = AuthBenchmarkHarness::run_benchmark(&mut validator, TOTAL_REQUESTS, valid_token);
    let mem_burst_1 = validator.memory_usage_bytes();

    // Burst 2: Another 10,000 requests
    let stats_2 = AuthBenchmarkHarness::run_benchmark(&mut validator, TOTAL_REQUESTS, valid_token);
    let mem_burst_2 = validator.memory_usage_bytes();

    // Burst 3: Another 10,000 requests
    let stats_3 = AuthBenchmarkHarness::run_benchmark(&mut validator, TOTAL_REQUESTS, valid_token);
    let mem_burst_3 = validator.memory_usage_bytes();

    print_stats_table("10,000 Requests Burst 1 (Bounded Memory 1,000 entries)", &stats_1);
    print_stats_table("10,000 Requests Burst 2 (Bounded Memory 1,000 entries)", &stats_2);
    print_stats_table("10,000 Requests Burst 3 (Bounded Memory 1,000 entries)", &stats_3);

    println!(
        "[Memory Footprint] Burst 1: {} bytes, Burst 2: {} bytes, Burst 3: {} bytes",
        mem_burst_1, mem_burst_2, mem_burst_3
    );

    // Memory footprint must be strictly bounded due to ring buffer FIFO eviction
    assert_eq!(
        mem_burst_1, mem_burst_2,
        "Memory must stabilize and remain strictly bounded after reaching capacity"
    );
    assert_eq!(
        mem_burst_2, mem_burst_3,
        "Memory must not grow across successive 10,000-request bursts"
    );
}

#[tokio::test]
async fn test_auth_load_10k_batch_validation() {
    const BATCH_SIZE: usize = 10_000;
    let valid_token = "batch_10k_secret_key";

    let mut validator = ResilientAuthValidator::new(
        Arc::new(valid_token.to_string()),
        vec![],
        AuthFaultPolicy::none(),
        3,
    );

    let tokens: Vec<String> = (0..BATCH_SIZE)
        .map(|i| {
            if i.is_multiple_of(10) {
                format!("invalid_token_{i}")
            } else {
                valid_token.to_string()
            }
        })
        .collect();

    let token_refs: Vec<&str> = tokens.iter().map(|s| s.as_str()).collect();

    let start = Instant::now();
    let results = validator.validate_batch_tokens(&token_refs);
    let duration = start.elapsed();

    assert_eq!(results.len(), BATCH_SIZE);
    let mut success_count = 0;
    let mut rejected_count = 0;

    for (i, res) in results.into_iter().enumerate() {
        let is_invalid = i.is_multiple_of(10);
        match res {
            Ok(true) if !is_invalid => success_count += 1,
            Ok(false) if is_invalid => rejected_count += 1,
            other => panic!("Unexpected result at {i}: {:?}", other),
        }
    }

    assert_eq!(success_count, 9_000);
    assert_eq!(rejected_count, 1_000);

    let throughput = BATCH_SIZE as f64 / duration.as_secs_f64();
    println!("\n================================================================================");
    println!(" BATCH VALIDATION BENCHMARK (10,000 tokens in a single batch)");
    println!("================================================================================");
    println!(" Batch Size         : {:>10}", BATCH_SIZE);
    println!(" Validated Valid    : {:>10}", success_count);
    println!(" Rejected Invalid   : {:>10}", rejected_count);
    println!(" Batch Duration     : {:>10.3} ms", duration.as_secs_f64() * 1000.0);
    println!(" Throughput         : {:>10.2} ops/sec", throughput);
    println!(" Avg Latency/Item   : {:>10.2} ns", (duration.as_nanos() as f64) / (BATCH_SIZE as f64));
    println!("================================================================================\n");

    assert!(throughput > 100_000.0, "Batch throughput should exceed 100,000 ops/sec");
}
