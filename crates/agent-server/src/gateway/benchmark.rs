use std::sync::Arc;
use std::time::{Duration, Instant};

use super::fault::GatewayFaultPolicy;
use super::resilience::ResilientGatewayService;
use super::sharded::ShardedResilientGatewayService;

#[derive(Debug, Clone, PartialEq)]
pub struct GatewayBenchmarkStats {
    pub total_requests: usize,
    pub successful_resolutions: usize,
    pub fallback_resolutions: usize,
    pub rejected_count: usize,
    pub p50_latency_us: f64,
    pub p90_latency_us: f64,
    pub p99_latency_us: f64,
    pub max_latency_us: f64,
    pub total_duration: Duration,
    pub throughput_ops_per_sec: f64,
    pub memory_allocated_bytes: usize,
}

pub struct GatewayBenchmarkHarness;

impl GatewayBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        fallback: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> GatewayBenchmarkStats {
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
        GatewayBenchmarkStats {
            total_requests,
            successful_resolutions: successful,
            fallback_resolutions: fallback,
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

    pub fn make_benchmark_model(idx: usize, is_invalid: bool) -> (String, bool) {
        if is_invalid {
            (format!("invalid_engine_model_{idx}"), true)
        } else {
            let models = [
                "codex",
                "claude",
                "codex/gpt-4o",
                "claude/claude-3-5-sonnet",
                "codex/o1-preview",
                "claude/claude-3-5-haiku",
            ];
            (models[idx % models.len()].to_string(), false)
        }
    }

    pub fn run_benchmark(
        service: &ResilientGatewayService,
        num_requests: usize,
    ) -> GatewayBenchmarkStats {
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut fallback = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_invalid = i.is_multiple_of(10);
            let (candidate, invalid) = Self::make_benchmark_model(i, is_invalid);
            let t0 = Instant::now();
            let res = service.resolve_model_resilient(&candidate);
            match res {
                Ok(_) if !invalid => successful += 1,
                Ok(_) => fallback += 1,
                Err(_) => rejected += 1,
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
    ) -> GatewayBenchmarkStats {
        let concurrency = concurrency.max(1);
        let num_requests = num_requests.max(1);
        let shared_service = Arc::new(tokio::sync::Mutex::new(
            ResilientGatewayService::new()
                .with_fault_policy(GatewayFaultPolicy::none())
                .with_fallback_model("codex")
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
                    let is_invalid = req_idx.is_multiple_of(10);
                    let (cand, invalid) = Self::make_benchmark_model(req_idx, is_invalid);

                    let t0 = Instant::now();
                    let res = {
                        let guard = shared_clone.lock().await;
                        guard.resolve_model_resilient(&cand)
                    };
                    match res {
                        Ok(_) if !invalid => local_success += 1,
                        Ok(_) => local_fallback += 1,
                        Err(_) => local_rejected += 1,
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (
                    local_latencies,
                    local_success,
                    local_fallback,
                    local_rejected,
                )
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        let mut total_fallback = 0;
        let mut total_rejected = 0;
        for handle in handles {
            if let Ok((latencies, success, fallback, rejected)) = handle.await {
                all_latencies.extend(latencies);
                total_success += success;
                total_fallback += fallback;
                total_rejected += rejected;
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
    ) -> GatewayBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let concurrency = concurrency.max(1);
        let num_requests = num_requests.max(1);
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;

        let sharded_service = Arc::new(
            ShardedResilientGatewayService::new(NUM_SHARDS)
                .with_fault_policy(GatewayFaultPolicy::none())
                .with_fallback_model("codex")
                .with_max_capacity(num_requests + per_shard_cap * NUM_SHARDS),
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
                    let is_invalid = req_idx.is_multiple_of(10);
                    let (cand, invalid) = Self::make_benchmark_model(req_idx, is_invalid);

                    let t0 = Instant::now();
                    let res = service_clone.resolve_model_resilient(&cand).await;
                    match res {
                        Ok(_) if !invalid => local_success += 1,
                        Ok(_) => local_fallback += 1,
                        Err(_) => local_rejected += 1,
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (
                    local_latencies,
                    local_success,
                    local_fallback,
                    local_rejected,
                )
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        let mut total_fallback = 0;
        let mut total_rejected = 0;
        for handle in handles {
            if let Ok((latencies, success, fallback, rejected)) = handle.await {
                all_latencies.extend(latencies);
                total_success += success;
                total_fallback += fallback;
                total_rejected += rejected;
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
    ) -> (GatewayBenchmarkStats, GatewayBenchmarkStats) {
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
