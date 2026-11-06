use crate::fault::{CliExecutionError, CliFaultPolicy};
use crate::resilience::{ResilientCliRunner, ShardedResilientCliRunner};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct CliBenchmarkStats {
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

pub struct CliBenchmarkHarness;

impl CliBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> CliBenchmarkStats {
        latencies.sort_by(|a, b| a.total_cmp(b));
        let p50 = Self::percentile(&latencies, 0.50);
        let p90 = Self::percentile(&latencies, 0.90);
        let p99 = Self::percentile(&latencies, 0.99);
        let max = match latencies.last() {
            Some(val) => *val,
            None => 0.0,
        };
        let secs = duration.as_secs_f64();
        let throughput = if secs > 0.0 {
            total_requests as f64 / secs
        } else {
            0.0
        };
        CliBenchmarkStats {
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

    pub fn make_benchmark_action(idx: usize, is_invalid: bool) -> (String, bool) {
        if is_invalid {
            (format!("invalid_cli_arg_{}", idx), true)
        } else {
            (format!("valid_cli_action_{}", idx), false)
        }
    }

    pub async fn run_benchmark(
        runner: &mut ResilientCliRunner,
        num_requests: usize,
    ) -> CliBenchmarkStats {
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_invalid = i.is_multiple_of(10);
            let (action_name, invalid) = Self::make_benchmark_action(i, is_invalid);
            let t0 = Instant::now();
            let res = runner
                .run_resilient(&action_name, || async move {
                    if invalid {
                        Err(CliExecutionError::ParseError {
                            code: 2,
                            message: "invalid benchmark cli argument".to_string(),
                        })
                    } else {
                        Ok("cli_bench_ok")
                    }
                })
                .await;

            match res {
                Ok(_) => successful += 1,
                Err(_) => rejected += 1,
            }
            latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
        }

        Self::compute_stats(
            latencies_us,
            num_requests,
            successful,
            rejected,
            start.elapsed(),
            runner.memory_usage_bytes(),
        )
    }

    pub async fn run_single_lock_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> CliBenchmarkStats {
        let concurrency = concurrency.max(1);
        let num_requests = num_requests.max(1);
        let shared_runner = Arc::new(tokio::sync::Mutex::new(
            ResilientCliRunner::new(CliFaultPolicy::none(), 3)
                .with_max_capacity(num_requests + 64),
        ));

        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let shared_clone = Arc::clone(&shared_runner);
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let (action_name, invalid) = Self::make_benchmark_action(req_idx, is_invalid);

                    let t0 = Instant::now();
                    let res = {
                        let mut guard = shared_clone.lock().await;
                        guard
                            .run_resilient(&action_name, || async move {
                                if invalid {
                                    Err(CliExecutionError::ParseError {
                                        code: 2,
                                        message: "invalid cli arg".to_string(),
                                    })
                                } else {
                                    Ok("cli_bench_ok")
                                }
                            })
                            .await
                    };
                    match res {
                        Ok(_) => local_success += 1,
                        Err(_) => local_rejected += 1,
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

        let mem_usage = shared_runner.lock().await.memory_usage_bytes();

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
    ) -> CliBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let concurrency = concurrency.max(1);
        let num_requests = num_requests.max(1);
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;

        let sharded_runner = Arc::new(
            ShardedResilientCliRunner::new(CliFaultPolicy::none(), 3, NUM_SHARDS)
                .with_max_capacity(num_requests + per_shard_cap * NUM_SHARDS),
        );

        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let runner_clone = Arc::clone(&sharded_runner);
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let (action_name, invalid) = Self::make_benchmark_action(req_idx, is_invalid);

                    let t0 = Instant::now();
                    let res = runner_clone
                        .run_resilient(&action_name, || async move {
                            if invalid {
                                Err(CliExecutionError::ParseError {
                                    code: 2,
                                    message: "invalid benchmark cli argument".to_string(),
                                })
                            } else {
                                Ok("cli_bench_ok")
                            }
                        })
                        .await;

                    match res {
                        Ok(_) => local_success += 1,
                        Err(_) => local_rejected += 1,
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

        let mem_usage = sharded_runner.memory_usage_bytes().await;

        Self::compute_stats(
            all_latencies,
            num_requests,
            total_success,
            total_rejected,
            start.elapsed(),
            mem_usage,
        )
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
