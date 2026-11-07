use super::InterAgentEvent;
use super::fault::EventFaultPolicy;
use super::resilience::ResilientEventProcessor;
use crate::identity::AgentId;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct EventBenchmarkStats {
    pub total_requests: usize,
    pub successful_processed: usize,
    pub p50_latency_us: f64,
    pub p90_latency_us: f64,
    pub p99_latency_us: f64,
    pub max_latency_us: f64,
    pub total_duration: Duration,
    pub throughput_ops_per_sec: f64,
    pub memory_allocated_bytes: usize,
}

pub struct EventBenchmarkHarness;

impl EventBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        duration: Duration,
        mem: usize,
    ) -> EventBenchmarkStats {
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
        EventBenchmarkStats {
            total_requests,
            successful_processed: successful,
            p50_latency_us: p50,
            p90_latency_us: p90,
            p99_latency_us: p99,
            max_latency_us: max,
            total_duration: duration,
            throughput_ops_per_sec: throughput,
            memory_allocated_bytes: mem,
        }
    }

    pub fn run_benchmark(
        processor: &mut ResilientEventProcessor,
        num_requests: usize,
    ) -> EventBenchmarkStats {
        let sender = AgentId::new("bench-sender");
        let receiver = AgentId::new("bench-receiver");
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;

        for i in 0..num_requests {
            let event = InterAgentEvent::new(
                sender.clone(),
                Some(receiver.clone()),
                "benchmark.event",
                serde_json::json!({ "sequence": i }),
            );
            let t0 = Instant::now();
            if let Ok(Some(_)) = processor.process_event(event) {
                successful += 1;
            }
            latencies_us.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
        }

        Self::compute_stats(
            latencies_us,
            num_requests,
            successful,
            start.elapsed(),
            processor.memory_usage_bytes(),
        )
    }

    pub fn run_concurrent_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> EventBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;
        let shards: Arc<Vec<Mutex<ResilientEventProcessor>>> = Arc::new(
            (0..NUM_SHARDS)
                .map(|_| {
                    Mutex::new(
                        ResilientEventProcessor::new(EventFaultPolicy::none(), 3)
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
                let sender = AgentId::new(&format!("worker-{}", t_idx));
                let receiver = AgentId::new("drain-target");
                let mut local_latencies = Vec::with_capacity(per_thread);
                let mut local_success = 0;
                for i in 0..per_thread {
                    let event = InterAgentEvent::new(
                        sender.clone(),
                        Some(receiver.clone()),
                        "concurrent.event",
                        serde_json::json!({ "thread": t_idx, "idx": i }),
                    );
                    let mut hasher = DefaultHasher::new();
                    event.id.hash(&mut hasher);
                    let shard_idx = (hasher.finish() as usize) % NUM_SHARDS;
                    let t0 = Instant::now();
                    let res = {
                        shards_clone[shard_idx]
                            .lock()
                            .expect("lock acquired")
                            .process_event(event)
                    };
                    if let Ok(Some(_)) = res {
                        local_success += 1;
                    }
                    local_latencies.push(t0.elapsed().as_secs_f64() * 1_000_000.0);
                }
                (local_latencies, local_success)
            }));
        }

        let mut all_latencies = Vec::with_capacity(num_requests);
        let mut total_success = 0;
        for handle in handles {
            if let Ok((latencies, success)) = handle.join() {
                all_latencies.extend(latencies);
                total_success += success;
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
