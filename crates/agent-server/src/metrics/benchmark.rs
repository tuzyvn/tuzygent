use std::sync::Arc;
use std::time::Instant;

use super::resilience::ResilientMetricsService;

#[derive(Debug, Clone, PartialEq)]
pub struct MetricsBenchmarkReport {
    pub total_requests: usize,
    pub elapsed_micros: u64,
    pub p50_latency_micros: f64,
    pub p90_latency_micros: f64,
    pub p99_latency_micros: f64,
    pub throughput_ops_per_sec: f64,
}

pub struct MetricsBenchmarkHarness;

impl MetricsBenchmarkHarness {
    fn calculate_percentiles(mut latencies: Vec<f64>) -> (f64, f64, f64) {
        if latencies.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p50_idx = ((latencies.len() as f64) * 0.50).min((latencies.len() - 1) as f64) as usize;
        let p90_idx = ((latencies.len() as f64) * 0.90).min((latencies.len() - 1) as f64) as usize;
        let p99_idx = ((latencies.len() as f64) * 0.99).min((latencies.len() - 1) as f64) as usize;
        (latencies[p50_idx], latencies[p90_idx], latencies[p99_idx])
    }

    pub fn run_single_thread(iterations: usize) -> MetricsBenchmarkReport {
        let service = ResilientMetricsService::new();
        let mut latencies = Vec::with_capacity(iterations);
        let start = Instant::now();

        for i in 0..iterations {
            let t0 = Instant::now();
            service.record_task_submitted();
            if i % 2 == 0 {
                service.record_task_completed(0.05);
                service.record_ooda_cycle(0.012);
            } else {
                service.record_task_failed(0.02);
                service.record_remediation();
            }
            if i % 10 == 0 {
                let _ = service.render_prometheus_text();
            }
            latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
        }

        let elapsed = start.elapsed();
        let elapsed_micros = elapsed.as_micros() as u64;
        let throughput = if elapsed.as_secs_f64() > 0.0 {
            (iterations as f64) / elapsed.as_secs_f64()
        } else {
            0.0
        };

        let (p50, p90, p99) = Self::calculate_percentiles(latencies);

        MetricsBenchmarkReport {
            total_requests: iterations,
            elapsed_micros,
            p50_latency_micros: p50,
            p90_latency_micros: p90,
            p99_latency_micros: p99,
            throughput_ops_per_sec: throughput,
        }
    }

    pub fn run_sharded_concurrent(iterations: usize, shards_count: usize) -> MetricsBenchmarkReport {
        let shards: Vec<Arc<ResilientMetricsService>> = (0..shards_count)
            .map(|_| Arc::new(ResilientMetricsService::new()))
            .collect();

        let per_thread = iterations / shards_count;
        let start = Instant::now();
        let mut handles = Vec::with_capacity(shards_count);

        for (shard_idx, shard) in shards.iter().enumerate() {
            let shard = shard.clone();
            let handle = std::thread::spawn(move || {
                let mut local_latencies = Vec::with_capacity(per_thread);
                for i in 0..per_thread {
                    let t0 = Instant::now();
                    shard.record_task_submitted();
                    if (shard_idx + i) % 2 == 0 {
                        shard.record_task_completed(0.03);
                        shard.record_ooda_cycle(0.015);
                    } else {
                        shard.record_task_failed(0.01);
                        shard.record_remediation();
                    }
                    if i % 50 == 0 {
                        let _ = shard.render_prometheus_text();
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                local_latencies
            });
            handles.push(handle);
        }

        let mut all_latencies = Vec::with_capacity(iterations);
        for h in handles {
            if let Ok(lats) = h.join() {
                all_latencies.extend(lats);
            }
        }

        let elapsed = start.elapsed();
        let elapsed_micros = elapsed.as_micros() as u64;
        let actual_total = all_latencies.len();
        let throughput = if elapsed.as_secs_f64() > 0.0 {
            (actual_total as f64) / elapsed.as_secs_f64()
        } else {
            0.0
        };

        let (p50, p90, p99) = Self::calculate_percentiles(all_latencies);

        MetricsBenchmarkReport {
            total_requests: actual_total,
            elapsed_micros,
            p50_latency_micros: p50,
            p90_latency_micros: p90,
            p99_latency_micros: p99,
            throughput_ops_per_sec: throughput,
        }
    }

    pub fn compare_concurrency(iterations: usize) -> (MetricsBenchmarkReport, MetricsBenchmarkReport) {
        let single = Self::run_single_thread(iterations);
        let sharded = Self::run_sharded_concurrent(iterations, 16);
        (single, sharded)
    }
}
