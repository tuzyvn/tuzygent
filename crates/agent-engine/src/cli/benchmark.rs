use async_trait::async_trait;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cli::fault::CliFaultPolicy;
use crate::cli::resilience::ResilientCliEngine;
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};

#[derive(Debug, Clone, PartialEq)]
pub struct CliBenchmarkStats {
    pub total_requests: usize,
    pub successful_resolved: usize,
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
pub struct BenchmarkMockCliEngine;

#[async_trait]
impl ComputeEngine for BenchmarkMockCliEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        if input.prompt_or_command.contains("invalid") || input.prompt_or_command.contains("fail") {
            Err(EngineError::ProcessFailed(
                "CLI process exit status 1: invalid command".to_string(),
            ))
        } else {
            Ok(EngineOutput {
                stdout_content: format!("Mock CLI output for: {}", input.prompt_or_command),
                exit_code: 0,
                execution_duration_ms: 1,
                engine_used: EngineKind::Generic("benchmark-mock-cli".to_string()),
                session_id: None,
            })
        }
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Generic("benchmark-mock-cli".to_string())
    }
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
        CliBenchmarkStats {
            total_requests,
            successful_resolved: successful,
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

    pub async fn run_benchmark<E: ComputeEngine + Send + Sync + 'static>(
        engine: &ResilientCliEngine<E>,
        num_requests: usize,
    ) -> CliBenchmarkStats {
        let mut latencies_us = Vec::with_capacity(num_requests);
        let start = Instant::now();
        let mut successful = 0;
        let mut rejected = 0;

        for i in 0..num_requests {
            let is_invalid = i.is_multiple_of(10);
            let prompt = if is_invalid {
                format!("invalid_prompt_{}", i)
            } else {
                format!("benchmark_prompt_{}", i)
            };
            let input = EngineInput {
                prompt_or_command: prompt,
                system_context: "You are a benchmark CLI agent".to_string(),
                working_dir: ".".to_string(),
                timeout_seconds: 5,
                on_event: None,
                command_sandbox: None,
                on_stream_chunk: None,
                temperature: None,
                max_tokens: None,
                max_output_bytes: None,
            };

            let t0 = Instant::now();
            match engine.execute(&input).await {
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
            engine.memory_usage_bytes(),
        )
    }

    pub async fn run_concurrent_benchmark(
        num_requests: usize,
        concurrency: usize,
    ) -> CliBenchmarkStats {
        const NUM_SHARDS: usize = 16;
        let per_shard_cap = (num_requests / NUM_SHARDS) + 64;

        let mut shards_vec = Vec::with_capacity(NUM_SHARDS);
        for _ in 0..NUM_SHARDS {
            let engine = BenchmarkMockCliEngine;
            let resilient = ResilientCliEngine::new(engine, CliFaultPolicy::none(), 3)
                .with_max_capacity(per_shard_cap);
            shards_vec.push(resilient);
        }

        let shards = Arc::new(shards_vec);
        let start = Instant::now();
        let per_task = num_requests / concurrency;
        let mut handles = Vec::with_capacity(concurrency);

        for t_idx in 0..concurrency {
            let shards_clone = Arc::clone(&shards);
            handles.push(tokio::spawn(async move {
                let mut local_latencies = Vec::with_capacity(per_task);
                let mut local_success = 0;
                let mut local_rejected = 0;

                for i in 0..per_task {
                    let req_idx = t_idx * per_task + i;
                    let is_invalid = req_idx.is_multiple_of(10);
                    let prompt = if is_invalid {
                        format!("invalid_prompt_{}_{}", t_idx, i)
                    } else {
                        format!("concurrent_prompt_{}_{}", t_idx, i)
                    };

                    let mut hasher = DefaultHasher::new();
                    prompt.hash(&mut hasher);
                    req_idx.hash(&mut hasher);
                    let shard_idx = (hasher.finish() as usize) % NUM_SHARDS;

                    let input = EngineInput {
                        prompt_or_command: prompt,
                        system_context: "You are a concurrent benchmark CLI agent".to_string(),
                        working_dir: ".".to_string(),
                        timeout_seconds: 5,
                        on_event: None,
                        command_sandbox: None,
                        on_stream_chunk: None,
                        temperature: None,
                        max_tokens: None,
                        max_output_bytes: None,
                    };

                    let t0 = Instant::now();
                    match shards_clone[shard_idx].execute(&input).await {
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

        let mem_usage: usize = shards.iter().map(|s| s.memory_usage_bytes()).sum();

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
