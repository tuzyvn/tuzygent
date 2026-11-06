use crate::resilience::ResilientCoreAgent;
use crate::sharded::ShardedCoreAgentRunner;
use std::time::{Duration, Instant};

/// Benchmark performance metrics and latency percentiles.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreBenchmarkStats {
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

pub struct CoreBenchmarkHarness;

impl CoreBenchmarkHarness {
    pub fn percentile(sorted: &[f64], pct: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let rank = pct * (sorted.len() - 1) as f64;
        let lower = rank.floor() as usize;
        let upper = rank.ceil() as usize;
        let weight = rank - lower as f64;
        let lower_val = sorted.get(lower).copied().unwrap_or(0.0);
        let upper_val = sorted.get(upper).copied().unwrap_or(lower_val);
        lower_val * (1.0 - weight) + upper_val * weight
    }

    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> CoreBenchmarkStats {
        latencies.sort_by(|a, b| a.total_cmp(b));
        let p50 = Self::percentile(&latencies, 0.50);
        let p90 = Self::percentile(&latencies, 0.90);
        let p99 = Self::percentile(&latencies, 0.99);
        let max = sorted_last(&latencies);
        let secs = duration.as_secs_f64();
        let throughput = if secs > 0.0 {
            total_requests as f64 / secs
        } else {
            0.0
        };

        CoreBenchmarkStats {
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

    /// Runs single-runner benchmark under high concurrency.
    pub fn run_single_benchmark(
        runner: &ResilientCoreAgent,
        num_requests: usize,
    ) -> CoreBenchmarkStats {
        let start = Instant::now();
        let mut latencies = Vec::with_capacity(num_requests);
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let req_start = Instant::now();
            let op_name = format!("core_op_{}", i);
            let payload = format!("payload_data_{}", i);

            match runner.execute_operation(&op_name, &payload) {
                Ok(_) => {
                    successful += 1;
                    latencies.push(req_start.elapsed().as_micros() as f64);
                }
                Err(_) => {
                    rejected += 1;
                }
            }
        }

        let duration = start.elapsed();
        let mem = runner.memory_usage_bytes();
        Self::compute_stats(latencies, num_requests, successful, rejected, duration, mem)
    }

    /// Runs 16-way sharded benchmark under high concurrency.
    pub fn run_sharded_benchmark(
        runner: &ShardedCoreAgentRunner,
        num_requests: usize,
    ) -> CoreBenchmarkStats {
        let start = Instant::now();
        let mut latencies = Vec::with_capacity(num_requests);
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let req_start = Instant::now();
            let op_name = format!("sharded_op_{}", i);
            let payload = format!("sharded_payload_{}", i);

            match runner.execute(&op_name, &payload) {
                Ok(_) => {
                    successful += 1;
                    latencies.push(req_start.elapsed().as_micros() as f64);
                }
                Err(_) => {
                    rejected += 1;
                }
            }
        }

        let duration = start.elapsed();
        let mem = runner.total_memory_usage();
        Self::compute_stats(latencies, num_requests, successful, rejected, duration, mem)
    }

    /// Compares lock contention between Single Lock and 16-Way Sharded Concurrency.
    pub fn compare_lock_contention(
        num_requests: usize,
    ) -> (CoreBenchmarkStats, CoreBenchmarkStats, f64) {
        let single_runner = ResilientCoreAgent::new().with_max_capacity(num_requests);
        let sharded_runner = ShardedCoreAgentRunner::new(num_requests / 16 + 1);

        let single_stats = Self::run_single_benchmark(&single_runner, num_requests);
        let sharded_stats = Self::run_sharded_benchmark(&sharded_runner, num_requests);

        let throughput_ratio = if single_stats.throughput_ops_per_sec > 0.0 {
            sharded_stats.throughput_ops_per_sec / single_stats.throughput_ops_per_sec
        } else {
            1.0
        };

        (single_stats, sharded_stats, throughput_ratio)
    }
}

fn sorted_last(slice: &[f64]) -> f64 {
    match slice.last() {
        Some(val) => *val,
        None => 0.0,
    }
}
