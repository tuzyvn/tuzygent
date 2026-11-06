use std::sync::Arc;
use std::time::{Duration, Instant};

use super::fault::AuthFaultPolicy;
use super::sharded::ShardedResilientAuthValidator;
use super::validator::ResilientAuthValidator;

#[derive(Debug, Clone, PartialEq)]
pub struct AuthBenchmarkStats {
    pub total_requests: usize,
    pub successful_operations: usize,
    pub rejected_count: usize,
    pub p50_latency_us: f64,
    pub p90_latency_us: f64,
    pub p99_latency_us: f64,
    pub max_latency_us: f64,
    pub total_duration: Duration,
    pub throughput_ops_per_sec: f64,
    pub memory_allocated_bytes: usize,
}

pub struct AuthBenchmarkHarness;

impl AuthBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> AuthBenchmarkStats {
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
        AuthBenchmarkStats {
            total_requests,
            successful_operations: successful,
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

    pub fn make_benchmark_token(idx: usize, is_invalid: bool, valid_token: &str) -> (String, bool) {
        if is_invalid {
            (format!("invalid_auth_token_{idx}"), true)
        } else {
            (valid_token.to_string(), false)
        }
    }

    pub fn run_benchmark(
        validator: &mut ResilientAuthValidator,
        num_requests: usize,
        valid_token: &str,
    ) -> AuthBenchmarkStats {
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_invalid = i.is_multiple_of(10);
            let (candidate, invalid) = Self::make_benchmark_token(i, is_invalid, valid_token);
            let t0 = Instant::now();
            let res = validator.validate_token_resilient(&candidate);
            match res {
                Ok(true) if !invalid => successful += 1,
                _ => rejected += 1,
            }
            latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
        }

        Self::compute_stats(
            latencies_us,
            num_requests,
            successful,
            rejected,
            start.elapsed(),
            validator.memory_usage_bytes(),
        )
    }

    pub async fn run_single_lock_benchmark(
        num_requests: usize,
        concurrency: usize,
        valid_token: &str,
    ) -> AuthBenchmarkStats {
        let concurrency = concurrency.max(1);
        let num_requests = num_requests.max(1);
        let shared_validator = Arc::new(tokio::sync::Mutex::new(
            ResilientAuthValidator::new(
                Arc::new(valid_token.to_string()),
                vec![],
                AuthFaultPolicy::none(),
                3,
            )
            .with_max_capacity(num_requests + 64),
        ));

        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let shared_clone = Arc::clone(&shared_validator);
            let token_str = valid_token.to_string();
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let (cand, invalid) = Self::make_benchmark_token(req_idx, is_invalid, &token_str);

                    let t0 = Instant::now();
                    let res = {
                        let mut guard = shared_clone.lock().await;
                        guard.validate_token_resilient(&cand)
                    };
                    match res {
                        Ok(true) if !invalid => local_success += 1,
                        _ => local_rejected += 1,
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (local_latencies, local_success, local_rejected)
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        let mut total_rejected = 0;
        for handle in handles {
            if let Ok((latencies, success, rejected)) = handle.await {
                all_latencies.extend(latencies);
                total_success += success;
                total_rejected += rejected;
            }
        }

        let mem_usage = shared_validator.lock().await.memory_usage_bytes();

        Self::compute_stats(
            all_latencies,
            num_requests,
            total_success,
            total_rejected,
            start.elapsed(),
            mem_usage,
        )
    }

    pub async fn run_concurrent_benchmark(
        num_requests: usize,
        concurrency: usize,
        valid_token: &str,
    ) -> AuthBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let concurrency = concurrency.max(1);
        let num_requests = num_requests.max(1);
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;

        let sharded_validator = Arc::new(
            ShardedResilientAuthValidator::new(
                Arc::new(valid_token.to_string()),
                vec![],
                AuthFaultPolicy::none(),
                3,
                NUM_SHARDS,
            )
            .with_max_capacity(num_requests + per_shard_cap * NUM_SHARDS),
        );

        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let validator_clone = Arc::clone(&sharded_validator);
            let token_str = valid_token.to_string();
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let (cand, invalid) = Self::make_benchmark_token(req_idx, is_invalid, &token_str);

                    let t0 = Instant::now();
                    let res = validator_clone.validate_token_resilient(&cand).await;
                    match res {
                        Ok(true) if !invalid => local_success += 1,
                        _ => local_rejected += 1,
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (local_latencies, local_success, local_rejected)
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        let mut total_rejected = 0;
        for handle in handles {
            if let Ok((latencies, success, rejected)) = handle.await {
                all_latencies.extend(latencies);
                total_success += success;
                total_rejected += rejected;
            }
        }

        let mem_usage = sharded_validator.memory_usage_bytes().await;

        Self::compute_stats(
            all_latencies,
            num_requests,
            total_success,
            total_rejected,
            start.elapsed(),
            mem_usage,
        )
    }

    pub async fn compare_concurrency(
        num_requests: usize,
        valid_token: &str,
    ) -> (AuthBenchmarkStats, AuthBenchmarkStats) {
        let single = Self::run_single_lock_benchmark(num_requests, 4, valid_token).await;
        let sharded = Self::run_concurrent_benchmark(num_requests, 4, valid_token).await;
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
