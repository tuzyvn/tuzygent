use crate::bus::fault::BusError;
use crate::bus::policy::BusFaultPolicy;
use crate::bus::resilience::ResilientInterAgentBus;
use agent_domain::{AgentId, InterAgentEvent};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct BusBenchmarkStats {
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

pub struct BusBenchmarkHarness;

impl BusBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> BusBenchmarkStats {
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
        BusBenchmarkStats {
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

    pub fn make_benchmark_event(idx: usize, is_invalid: bool) -> InterAgentEvent {
        if is_invalid {
            let mut event = InterAgentEvent::new(
                AgentId::new("bench-invalid-sender"),
                Some(AgentId::new("coordinator")),
                "bus.benchmark.event",
                serde_json::json!({ "idx": idx }),
            );
            event.id = String::new(); // Empty ID triggers BusError::CorruptedEvent
            event
        } else {
            let sender_name = format!("agent-{:04}", idx % 16);
            let topic_name = format!("bus.benchmark.topic.{}", idx % 8);
            InterAgentEvent::new(
                AgentId::new(&sender_name),
                Some(AgentId::new("coordinator")),
                &topic_name,
                serde_json::json!({
                    "sequence": idx,
                    "payload": "high concurrency bus event",
                }),
            )
        }
    }

    pub fn run_benchmark(
        bus: &mut ResilientInterAgentBus,
        num_requests: usize,
    ) -> BusBenchmarkStats {
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_invalid = i.is_multiple_of(10);
            let event = Self::make_benchmark_event(i, is_invalid);
            let t0 = Instant::now();
            match bus.publish_resilient(event) {
                Ok(_) => successful += 1,
                Err(BusError::CorruptedEvent(_)) => rejected += 1,
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
            bus.memory_usage_bytes(),
        )
    }

    pub fn run_single_lock_benchmark(num_requests: usize, concurrency: usize) -> BusBenchmarkStats {
        let shared_bus = Arc::new(Mutex::new(
            ResilientInterAgentBus::new(num_requests + 64, BusFaultPolicy::none(), 3)
                .with_max_capacity(num_requests + 64),
        ));

        let start = Instant::now();
        let per_thread = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let bus_clone = Arc::clone(&shared_bus);
            handles.push(std::thread::spawn(move || {
                let mut local_latencies = Vec::with_capacity(per_thread);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_thread {
                    let req_idx = t_idx * per_thread + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let event = Self::make_benchmark_event(req_idx, is_invalid);

                    let t0 = Instant::now();
                    let res = {
                        let mut guard = bus_clone.lock().unwrap_or_else(|p| p.into_inner());
                        guard.publish_resilient(event)
                    };
                    match res {
                        Ok(_) => local_success += 1,
                        Err(BusError::CorruptedEvent(_)) => local_rejected += 1,
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

        let mem_usage = shared_bus
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .memory_usage_bytes();

        Self::compute_stats(
            all_latencies,
            num_requests,
            total_success,
            total_rejected,
            start.elapsed(),
            mem_usage,
        )
    }

    pub fn run_concurrent_benchmark(num_requests: usize, concurrency: usize) -> BusBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;
        let shards: Arc<Vec<Mutex<ResilientInterAgentBus>>> = Arc::new(
            (0..NUM_SHARDS)
                .map(|_| {
                    Mutex::new(
                        ResilientInterAgentBus::new(per_shard_cap, BusFaultPolicy::none(), 3)
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
                    let event = Self::make_benchmark_event(req_idx, is_invalid);

                    let mut hasher = DefaultHasher::new();
                    req_idx.hash(&mut hasher);
                    let shard_idx = (hasher.finish() as usize) % NUM_SHARDS;

                    let t0 = Instant::now();
                    let res = {
                        let mut guard = shards_clone[shard_idx]
                            .lock()
                            .unwrap_or_else(|p| p.into_inner());
                        guard.publish_resilient(event)
                    };
                    match res {
                        Ok(_) => local_success += 1,
                        Err(BusError::CorruptedEvent(_)) => local_rejected += 1,
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
            .map(|s| {
                s.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .memory_usage_bytes()
            })
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
