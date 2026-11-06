use crate::durable::fault::DurableMemoryError;
use crate::durable::resilience::ResilientDurableMemoryStore;
use crate::durable::storage::StructuredLesson;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct DurableBenchmarkStats {
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

pub struct DurableBenchmarkHarness;

impl DurableBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> DurableBenchmarkStats {
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
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
        DurableBenchmarkStats {
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

    pub fn make_benchmark_lesson(idx: usize, is_invalid: bool) -> StructuredLesson {
        if is_invalid {
            StructuredLesson {
                id: String::new(),
                timestamp: String::new(),
                task_title: String::new(),
                trigger_error: None,
                remediation_applied: None,
                general_insight: String::new(),
                success: false,
            }
        } else {
            StructuredLesson {
                id: format!("LES-{:06}", idx),
                timestamp: "2026-11-06T00:00:00Z".to_string(),
                task_title: format!("High load durable benchmark iteration {}", idx),
                trigger_error: None,
                remediation_applied: None,
                general_insight: format!("Optimized memory storage verified for node {}", idx),
                success: true,
            }
        }
    }

    pub fn run_benchmark(
        store: &mut ResilientDurableMemoryStore,
        num_requests: usize,
    ) -> DurableBenchmarkStats {
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_invalid = i.is_multiple_of(10);
            let lesson = Self::make_benchmark_lesson(i, is_invalid);
            let t0 = Instant::now();
            match store.save_structured_lesson_cached(&lesson) {
                Ok(_) => successful += 1,
                Err(DurableMemoryError::EmptyContent) => rejected += 1,
                Err(_) => {}
            }
            latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
        }

        Self::compute_stats(
            latencies_us,
            num_requests,
            successful,
            rejected,
            start.elapsed(),
            store.memory_usage_bytes(),
        )
    }

    pub fn run_concurrent_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> DurableBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;
        let shards: Arc<Vec<Mutex<ResilientDurableMemoryStore>>> = Arc::new(
            (0..NUM_SHARDS)
                .map(|i| {
                    Mutex::new(
                        ResilientDurableMemoryStore::from_workspace(format!(
                            "/tmp/durable_bench_shard_{}",
                            i
                        ))
                        .with_max_capacity(per_shard_cap),
                    )
                })
                .collect(),
        );

        let start = Instant::now();
        let per_thread = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let shards_clone = Arc::clone(&shards);
            handles.push(std::thread::spawn(move || {
                let mut local_latencies = Vec::with_capacity(per_thread);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_thread {
                    let req_idx = t_idx * per_thread + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let lesson = Self::make_benchmark_lesson(req_idx, is_invalid);

                    let mut hasher = DefaultHasher::new();
                    req_idx.hash(&mut hasher);
                    let shard_idx = (hasher.finish() as usize) % NUM_SHARDS;

                    let t0 = Instant::now();
                    let res = {
                        shards_clone[shard_idx]
                            .lock()
                            .expect("lock acquired")
                            .save_structured_lesson_cached(&lesson)
                    };
                    match res {
                        Ok(_) => local_success += 1,
                        Err(DurableMemoryError::EmptyContent) => local_rejected += 1,
                        Err(_) => {}
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
            if let Ok((latencies, success, rejected)) = handle.join() {
                all_latencies.extend(latencies);
                total_success += success;
                total_rejected += rejected;
            }
        }

        let mem_usage: usize = shards
            .iter()
            .map(|s| s.lock().expect("lock acquired").memory_usage_bytes())
            .sum();

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
        let idx = ((sorted.len() as f64) * pct).floor() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }
}
