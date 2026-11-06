use std::sync::Arc;
use std::time::{Duration, Instant};

use super::sharded::ShardedResilientDashboardService;
use super::service::ResilientDashboardService;
use super::types::{DashboardContext, DashboardFaultPolicy};

#[derive(Debug, Clone, PartialEq)]
pub struct DashboardBenchmarkStats {
    pub total_requests: usize,
    pub successful_renders: usize,
    pub fallback_renders: usize,
    pub rejected_count: usize,
    pub p50_latency_us: f64,
    pub p90_latency_us: f64,
    pub p99_latency_us: f64,
    pub max_latency_us: f64,
    pub total_duration: Duration,
    pub throughput_ops_per_sec: f64,
    pub memory_allocated_bytes: usize,
}

pub struct DashboardBenchmarkHarness;

impl DashboardBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        fallback: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> DashboardBenchmarkStats {
        latencies.sort_by(|a, b| a.total_cmp(b));
        let p50 = Self::percentile(&latencies, 0.50);
        let p90 = Self::percentile(&latencies, 0.90);
        let p99 = Self::percentile(&latencies, 0.99);
        let max = latencies.last().copied().unwrap_or(0.0);
        let secs = duration.as_secs_f64();
        let throughput = if secs > 0.0 {
            total_requests as f64 / secs
        } else {
            0.0
        };
        DashboardBenchmarkStats {
            total_requests,
            successful_renders: successful,
            fallback_renders: fallback,
            rejected_count: rejected,
            p50_latency_us: p50,
            p90_latency_us: p90,
            p99_latency_us: p99,
            max_latency_us: max,
            total_duration: duration,
            throughput_ops_per_sec: throughput,
            memory_allocated_bytes: mem,
        }
    }

    pub fn make_benchmark_context(idx: usize, is_fallback: bool) -> (DashboardContext, bool) {
        if is_fallback {
            let ctx = DashboardContext {
                title: Some(format!("Fault Dashboard {idx} {{unclosed_tag")),
                ..DashboardContext::default()
            };
            (ctx, true)
        } else {
            let ctx = DashboardContext {
                title: Some(format!("AgentCore Dashboard Node {idx}")),
                gateway_url: Some(format!("http://localhost:8080/node_{idx}")),
                version: Some("1.0.0-bench".to_string()),
                ..DashboardContext::default()
            };
            (ctx, false)
        }
    }

    pub async fn run_benchmark(
        service: &ResilientDashboardService,
        num_requests: usize,
    ) -> DashboardBenchmarkStats {
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut fallback = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_err = i.is_multiple_of(10);
            let (ctx, err_flag) = Self::make_benchmark_context(i, is_err);
            let t0 = Instant::now();
            let (html, status) = service.render_dashboard_with_fallback(&ctx).await;
            if status.is_success() && !err_flag && !html.is_empty() {
                successful += 1;
            } else if !status.is_success() || err_flag {
                fallback += 1;
            } else {
                rejected += 1;
            }
            latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
        }

        Self::compute_stats(
            latencies_us,
            num_requests,
            successful,
            fallback,
            rejected,
            start.elapsed(),
            service.memory_usage_bytes(),
        )
    }

    pub async fn run_single_lock_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> DashboardBenchmarkStats {
        let concurrency = concurrency.max(1);
        let num_requests = num_requests.max(1);
        let shared_service = Arc::new(tokio::sync::Mutex::new(
            ResilientDashboardService::new()
                .with_fault_policy(DashboardFaultPolicy::none())
                .with_max_capacity(num_requests + 64),
        ));

        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let shared_clone = Arc::clone(&shared_service);
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_fallback = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_err = req_idx.is_multiple_of(10);
                    let (ctx, err_flag) = Self::make_benchmark_context(req_idx, is_err);

                    let t0 = Instant::now();
                    let (html, status) = {
                        let guard = shared_clone.lock().await;
                        guard.render_dashboard_with_fallback(&ctx).await
                    };
                    if status.is_success() && !err_flag && !html.is_empty() {
                        local_success += 1;
                    } else if !status.is_success() || err_flag {
                        local_fallback += 1;
                    } else {
                        local_rejected += 1;
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (local_latencies, local_success, local_fallback, local_rejected)
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        let mut total_fallback = 0;
        let mut total_rejected = 0;
        for handle in handles {
            if let Ok((latencies, success, fall, rej)) = handle.await {
                all_latencies.extend(latencies);
                total_success += success;
                total_fallback += fall;
                total_rejected += rej;
            }
        }

        let mem_usage = shared_service.lock().await.memory_usage_bytes();

        Self::compute_stats(
            all_latencies,
            num_requests,
            total_success,
            total_fallback,
            total_rejected,
            start.elapsed(),
            mem_usage,
        )
    }

    pub async fn run_concurrent_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> DashboardBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let concurrency = concurrency.max(1);
        let num_requests = num_requests.max(1);
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;

        let sharded_service = Arc::new(
            ShardedResilientDashboardService::new(NUM_SHARDS)
                .with_fault_policy(DashboardFaultPolicy::none())
                .with_max_capacity(num_requests + per_shard_cap * NUM_SHARDS)
                .with_cache(num_requests + 64),
        );

        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let service_clone = Arc::clone(&sharded_service);
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_fallback = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_err = req_idx.is_multiple_of(10);
                    let (ctx, err_flag) = Self::make_benchmark_context(req_idx, is_err);
                    let key = format!("dashboard_key_{req_idx}");

                    let t0 = Instant::now();
                    let (html, status) = service_clone.render_dashboard_with_fallback(&key, &ctx).await;
                    if status.is_success() && !err_flag && !html.is_empty() {
                        local_success += 1;
                    } else if !status.is_success() || err_flag {
                        local_fallback += 1;
                    } else {
                        local_rejected += 1;
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (local_latencies, local_success, local_fallback, local_rejected)
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        let mut total_fallback = 0;
        let mut total_rejected = 0;
        for handle in handles {
            if let Ok((latencies, success, fall, rej)) = handle.await {
                all_latencies.extend(latencies);
                total_success += success;
                total_fallback += fall;
                total_rejected += rej;
            }
        }

        let mem_usage = sharded_service.memory_usage_bytes().await;

        Self::compute_stats(
            all_latencies,
            num_requests,
            total_success,
            total_fallback,
            total_rejected,
            start.elapsed(),
            mem_usage,
        )
    }

    pub async fn compare_concurrency(
        num_requests: usize,
    ) -> (DashboardBenchmarkStats, DashboardBenchmarkStats) {
        let single = Self::run_single_lock_benchmark(num_requests, 4).await;
        let sharded = Self::run_concurrent_benchmark(num_requests, 4).await;
        (single, sharded)
    }

    fn percentile(sorted: &[f64], pct: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let pct_clamped = pct.clamp(0.0, 1.0);
        let idx = ((sorted.len() as f64) * pct_clamped).floor() as usize;
        sorted[idx.min(sorted.len().saturating_sub(1))]
    }
}
