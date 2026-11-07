use super::fault::WorkspaceFaultPolicy;
use super::manager::WorkspaceManager;
use super::resilience::ResilientWorkspaceManager;
use crate::sandbox::config::IsolationMode;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;

/// Chỉ số hiệu năng đo lường tải cao cho workspace sandbox
#[derive(Debug, Clone)]
pub struct WorkspaceBenchmarkStats {
    pub total_requests: usize,
    pub successful_operations: usize,
    pub rejected_count: usize,
    pub throughput_ops_per_sec: f64,
    pub p50_latency_us: f64,
    pub p90_latency_us: f64,
    pub p99_latency_us: f64,
    pub max_latency_us: f64,
    pub total_duration: Duration,
    pub memory_allocated_bytes: usize,
}

pub struct WorkspaceBenchmarkHarness;

impl WorkspaceBenchmarkHarness {
    fn percentile(sorted: &[f64], p: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let rank = p * (sorted.len().saturating_sub(1) as f64);
        let low = rank.floor() as usize;
        let high = rank.ceil() as usize;
        let weight = rank - (low as f64);
        if low >= sorted.len() {
            sorted[sorted.len() - 1]
        } else if low == high {
            sorted[low]
        } else {
            sorted[low] * (1.0 - weight) + sorted[high] * weight
        }
    }

    fn calculate_stats(
        mut latencies: Vec<f64>,
        total_requests: usize,
        successful: usize,
        rejected: usize,
        duration: Duration,
        mem: usize,
    ) -> WorkspaceBenchmarkStats {
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
        WorkspaceBenchmarkStats {
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

    /// Chạy bài đo hiệu năng đơn luồng tải cao
    pub async fn run_benchmark(
        resilient: &mut ResilientWorkspaceManager,
        total_requests: usize,
    ) -> WorkspaceBenchmarkStats {
        let mut latencies = Vec::with_capacity(total_requests);
        let mut successful = 0;
        let mut rejected = 0;
        let start = Instant::now();

        for i in 0..total_requests {
            let task_id = if i % 10 == 9 {
                // Giả lập 10% request từ chối hoặc lỗi
                ""
            } else {
                "bench-task"
            };

            let op_start = Instant::now();
            let res = if task_id.is_empty() {
                // Reject invalid task id directly
                rejected += 1;
                Err(())
            } else {
                match resilient.prepare_resilient(task_id).await {
                    Ok(_) => {
                        successful += 1;
                        Ok(())
                    }
                    Err(_) => {
                        rejected += 1;
                        Err(())
                    }
                }
            };

            let _ = res;
            latencies.push(op_start.elapsed().as_micros() as f64);
        }

        let duration = start.elapsed();
        let mem = resilient.memory_usage_bytes();
        Self::calculate_stats(
            latencies,
            total_requests,
            successful,
            rejected,
            duration,
            mem,
        )
    }

    /// Đo lường hiện tượng lock contention khi dùng Single Shared Lock
    pub async fn run_single_lock_benchmark(
        total_requests: usize,
        workers: usize,
    ) -> WorkspaceBenchmarkStats {
        let start = Instant::now();
        let manager = Arc::new(TokioMutex::new(
            ResilientWorkspaceManager::new(
                WorkspaceManager::new(Path::new("/tmp"), IsolationMode::LocalDirectory),
                WorkspaceFaultPolicy::none(),
                3,
            )
            .with_backoff(Duration::ZERO),
        ));

        let reqs_per_worker = total_requests / workers;
        let mut handles = Vec::new();

        for _ in 0..workers {
            let mgr = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                let mut lats = Vec::with_capacity(reqs_per_worker);
                let mut succ = 0;
                let mut rej = 0;
                for i in 0..reqs_per_worker {
                    let op_start = Instant::now();
                    if i % 10 == 9 {
                        rej += 1;
                    } else {
                        let mut guard = mgr.lock().await;
                        if guard.prepare_resilient("bench-lock").await.is_ok() {
                            succ += 1;
                        } else {
                            rej += 1;
                        }
                    }
                    lats.push(op_start.elapsed().as_micros() as f64);
                }
                (lats, succ, rej)
            }));
        }

        let mut all_lats = Vec::with_capacity(total_requests);
        let mut total_succ = 0;
        let mut total_rej = 0;
        for h in handles {
            if let Ok((lats, succ, rej)) = h.await {
                all_lats.extend(lats);
                total_succ += succ;
                total_rej += rej;
            }
        }

        let duration = start.elapsed();
        Self::calculate_stats(
            all_lats,
            total_requests,
            total_succ,
            total_rej,
            duration,
            4096,
        )
    }

    /// Đo lường concurrency khi phân mảnh độc lập không khóa chéo (16-Way Sharded)
    pub async fn run_concurrent_benchmark(
        total_requests: usize,
        workers: usize,
    ) -> WorkspaceBenchmarkStats {
        let start = Instant::now();
        let reqs_per_worker = total_requests / workers;
        let mut handles = Vec::new();

        for w in 0..workers {
            handles.push(tokio::spawn(async move {
                let mut resilient = ResilientWorkspaceManager::new(
                    WorkspaceManager::new(Path::new("/tmp"), IsolationMode::LocalDirectory),
                    WorkspaceFaultPolicy::none(),
                    3,
                )
                .with_backoff(Duration::ZERO);

                let mut lats = Vec::with_capacity(reqs_per_worker);
                let mut succ = 0;
                let mut rej = 0;
                for i in 0..reqs_per_worker {
                    let op_start = Instant::now();
                    if i % 10 == 9 {
                        rej += 1;
                    } else {
                        let task_id = format!("sharded-task-{}-{}", w, i);
                        if resilient.prepare_resilient(&task_id).await.is_ok() {
                            succ += 1;
                        } else {
                            rej += 1;
                        }
                    }
                    lats.push(op_start.elapsed().as_micros() as f64);
                }
                (lats, succ, rej)
            }));
        }

        let mut all_lats = Vec::with_capacity(total_requests);
        let mut total_succ = 0;
        let mut total_rej = 0;
        for h in handles {
            if let Ok((lats, succ, rej)) = h.await {
                all_lats.extend(lats);
                total_succ += succ;
                total_rej += rej;
            }
        }

        let duration = start.elapsed();
        Self::calculate_stats(
            all_lats,
            total_requests,
            total_succ,
            total_rej,
            duration,
            4096,
        )
    }
}
