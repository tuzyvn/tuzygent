use crate::execution::fault::ExecutionFaultPolicy;
use crate::execution::resilience::ResilientExecutionService;
use crate::execution::ExecutionService;
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionBenchmarkStats {
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

#[derive(Clone, Default)]
pub struct BenchmarkMockEngine;

#[async_trait]
impl ComputeEngine for BenchmarkMockEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        if input.prompt_or_command.contains("invalid") || input.prompt_or_command.contains("rm -rf") {
            Err(EngineError::PolicyRejected(
                "Command rejected by security guardrail".to_string(),
            ))
        } else {
            Ok(EngineOutput {
                stdout_content: format!("Benchmark execution output for: {}", input.prompt_or_command),
                exit_code: 0,
                execution_duration_ms: 1,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

pub struct ExecutionBenchmarkHarness;

impl ExecutionBenchmarkHarness {
    fn compute_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> ExecutionBenchmarkStats {
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
        ExecutionBenchmarkStats {
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

    pub fn make_benchmark_input(idx: usize, is_invalid: bool) -> EngineInput {
        let prompt = if is_invalid {
            format!("rm -rf /bench/invalid_{}", idx)
        } else {
            format!("echo bench_cmd_{}", idx)
        };
        EngineInput {
            prompt_or_command: prompt,
            system_context: "You are a benchmark execution engine".to_string(),
            working_dir: "/tmp".to_string(),
            timeout_seconds: 5,
            on_event: None,
            command_sandbox: None,
            on_stream_chunk: None,
            temperature: None,
            max_tokens: None,
            max_output_bytes: None,
        }
    }

    pub async fn run_benchmark(
        resilient: &mut ResilientExecutionService,
        num_requests: usize,
    ) -> ExecutionBenchmarkStats {
        let engine = Arc::new(BenchmarkMockEngine);
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_invalid = i.is_multiple_of(10);
            let input = Self::make_benchmark_input(i, is_invalid);
            let t0 = Instant::now();
            match resilient.execute_resilient(engine.clone(), &input).await {
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
            resilient.memory_usage_bytes(),
        )
    }

    pub async fn run_single_lock_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> ExecutionBenchmarkStats {
        let service = ExecutionService::new("/tmp", None, None);
        let shared_resilient = Arc::new(tokio::sync::Mutex::new(
            ResilientExecutionService::new(service, ExecutionFaultPolicy::none(), 3)
                .with_max_capacity(num_requests + 64),
        ));

        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);
        let engine = Arc::new(BenchmarkMockEngine);

        for t_idx in 0..concurrency {
            let shared_clone = Arc::clone(&shared_resilient);
            let eng = Arc::clone(&engine);
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let input = Self::make_benchmark_input(req_idx, is_invalid);

                    let t0 = Instant::now();
                    let res = {
                        let mut guard = shared_clone.lock().await;
                        guard.execute_resilient(eng.clone(), &input).await
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

        let mem_usage = shared_resilient.lock().await.memory_usage_bytes();

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
    ) -> ExecutionBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;

        let shards: Arc<Vec<tokio::sync::Mutex<ResilientExecutionService>>> = Arc::new(
            (0..NUM_SHARDS)
                .map(|_| {
                    let service = ExecutionService::new("/tmp", None, None);
                    tokio::sync::Mutex::new(
                        ResilientExecutionService::new(service, ExecutionFaultPolicy::none(), 3)
                            .with_max_capacity(per_shard_cap),
                    )
                })
                .collect(),
        );

        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);
        let engine = Arc::new(BenchmarkMockEngine);

        for t_idx in 0..concurrency {
            let shards_clone = Arc::clone(&shards);
            let eng = Arc::clone(&engine);
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let input = Self::make_benchmark_input(req_idx, is_invalid);

                    let mut hasher = DefaultHasher::new();
                    req_idx.hash(&mut hasher);
                    let shard_idx = (hasher.finish() as usize) % NUM_SHARDS;

                    let t0 = Instant::now();
                    let res = {
                        let mut guard = shards_clone[shard_idx].lock().await;
                        guard.execute_resilient(eng.clone(), &input).await
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

        let mut mem_usage = 0;
        for shard in shards.iter() {
            mem_usage += shard.lock().await.memory_usage_bytes();
        }

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
