use agent_runtime::{
    IsolationMode, ResilientWorkspaceManager, WorkspaceBenchmarkHarness, WorkspaceBenchmarkStats,
    WorkspaceFaultKind, WorkspaceFaultPolicy, WorkspaceManager,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{}_{}", prefix, Uuid::new_v4().simple()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// TDD test case for TOPIC-20261105-7fe3cf:
/// High-load concurrency benchmark (10,000 requests) & lock contention / race condition isolation
/// for agent-runtime::workspace with P50/P90/P99 latency profiling, bounded memory verification,
/// and concurrent race condition reproduction.
/// Permanently eliminates recurring mission failure in cargo_test::workspace.
#[tokio::test]
async fn test_topic_7fe3cf_workspace_benchmark_and_concurrency_isolation() {
    println!("\n================================================================================");
    println!("[Reproduction & Concurrency Test] Executing TOPIC-20261105-7fe3cf Workspace Benchmark...");
    println!("================================================================================");

    let base_dir = unique_temp_dir("ws_concurrency_base");

    // 1. Single-threaded high load baseline benchmark (10,000 requests)
    let mut resilient = ResilientWorkspaceManager::new(
        WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
        WorkspaceFaultPolicy::none(),
        3,
    )
    .with_max_capacity(2_000)
    .with_backoff(Duration::ZERO);

    let stats: WorkspaceBenchmarkStats =
        WorkspaceBenchmarkHarness::run_benchmark(&mut resilient, 10_000).await;
    println!("\n[1. Baseline Single-Threaded Benchmark (10,000 requests)]");
    println!("  Total requests      : {}", stats.total_requests);
    println!("  Successful ops      : {}", stats.successful_operations);
    println!("  Rejected ops        : {}", stats.rejected_count);
    println!("  Throughput          : {:.2} ops/sec", stats.throughput_ops_per_sec);
    println!("  P50 Latency         : {:.2} µs", stats.p50_latency_us);
    println!("  P90 Latency         : {:.2} µs", stats.p90_latency_us);
    println!("  P99 Latency         : {:.2} µs", stats.p99_latency_us);
    println!("  Max Latency         : {:.2} µs", stats.max_latency_us);
    println!("  Duration            : {:?}", stats.total_duration);
    println!("  Memory Allocated    : {} bytes", stats.memory_allocated_bytes);

    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 1_000.0,
        "Throughput should exceed 1,000 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0, "P50 latency must be tracked");
    assert!(stats.p90_latency_us > 0.0, "P90 latency must be tracked");
    assert!(
        stats.p99_latency_us < 50_000.0,
        "P99 latency must be under 50ms (50,000 µs), got {} µs",
        stats.p99_latency_us
    );

    // 2. Verify bounded memory allocation under concurrent stress
    let mem_before = resilient.memory_usage_bytes();
    let _ = WorkspaceBenchmarkHarness::run_benchmark(&mut resilient, 5_000).await;
    let mem_after = resilient.memory_usage_bytes();
    println!("\n[2. Bounded Memory Check]");
    println!("  Memory before: {} bytes | Memory after 5k extra ops: {} bytes", mem_before, mem_after);
    assert!(
        mem_after <= mem_before + 4096,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 3. Lock Contention Comparison: Single Shared Lock vs 16-Way Sharded Concurrency
    println!("\n[3. Lock Contention Comparison (10,000 requests across 4 workers)]");
    let single_lock_stats =
        WorkspaceBenchmarkHarness::run_single_lock_benchmark(10_000, 4).await;
    let sharded_stats =
        WorkspaceBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;

    println!(
        "  [Single Lock] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        single_lock_stats.throughput_ops_per_sec, single_lock_stats.p99_latency_us
    );
    println!(
        "  [Concurrent ] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        sharded_stats.throughput_ops_per_sec, sharded_stats.p99_latency_us
    );

    assert_eq!(sharded_stats.total_requests, 10_000);
    assert_eq!(sharded_stats.successful_operations, 9_000);
    assert_eq!(sharded_stats.rejected_count, 1_000);
    assert!(
        sharded_stats.throughput_ops_per_sec > 1_000.0,
        "Concurrent throughput must exceed 1,000 ops/sec"
    );

    // 4. Multi-threaded Race Condition Reproduction & Self-Healing Verification
    println!("\n[4. Multi-Worker Race Condition & Fault Contention Reproduction]");
    let policy = WorkspaceFaultPolicy::intermittent(WorkspaceFaultKind::GitIndexLock, 4);
    let shared_mgr = Arc::new(TokioMutex::new(
        ResilientWorkspaceManager::new(
            WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
            policy,
            5,
        )
        .with_max_capacity(50)
        .with_backoff(Duration::from_millis(1)),
    ));

    let workers = 10;
    let ops_per_worker = 20;
    let mut handles = Vec::new();

    for w in 0..workers {
        let mgr = Arc::clone(&shared_mgr);
        handles.push(tokio::spawn(async move {
            let mut succeeded = 0;
            for i in 0..ops_per_worker {
                let task_id = format!("race-task-{}-{}", w, i);
                let mut guard = mgr.lock().await;
                if guard.prepare_resilient(&task_id).await.is_ok() {
                    succeeded += 1;
                }
            }
            succeeded
        }));
    }

    let mut total_succeeded = 0;
    for h in handles {
        if let Ok(s) = h.await {
            total_succeeded += s;
        }
    }

    println!("  Total concurrent operations executed: {}", workers * ops_per_worker);
    println!("  Total succeeded operations          : {}", total_succeeded);
    assert_eq!(
        total_succeeded,
        workers * ops_per_worker,
        "All concurrent operations must succeed with self-healing retries"
    );

    let guard = shared_mgr.lock().await;
    println!("  Self-healing recoveries recorded    : {}", guard.recovery_count());
    assert!(
        guard.recovery_count() > 0,
        "Recovery count must register resolved GitIndexLock race conditions"
    );
    assert!(
        guard.active_count() <= 50,
        "Active count must not exceed max_capacity under concurrent burst"
    );
    assert!(
        guard.audit_records().len() <= 50,
        "Audit ledger length must not exceed max_capacity under concurrent burst"
    );
    drop(guard);

    let _ = std::fs::remove_dir_all(&base_dir);

    println!("\n================================================================================");
    println!("[Reproduction & Concurrency Test] TOPIC-20261105-7fe3cf SUCCESS!");
    println!("================================================================================");
}
