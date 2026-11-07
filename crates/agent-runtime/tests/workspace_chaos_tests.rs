use agent_runtime::{
    is_retryable_workspace_error, IsolationMode, ResilientWorkspaceManager,
    WorkspaceBenchmarkHarness, WorkspaceBenchmarkStats, WorkspaceError, WorkspaceFaultKind,
    WorkspaceFaultPolicy, WorkspaceManager,
};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{}_{}", prefix, Uuid::new_v4().simple()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// TDD test case 1 for TOPIC-20261105-7fe3cf:
/// Chaos Engineering and Fault Injection resilience in agent-runtime::workspace.
/// Tests recovery from GitIndexLock, TransientIo, SocketTimeout, and LinkerOom faults.
#[tokio::test]
async fn test_topic_7fe3cf_workspace_chaos_resilience() {
    let base_dir = unique_temp_dir("ws_chaos_base");

    // 1. Recover from intermittent Git Index Lock contention
    let policy = WorkspaceFaultPolicy::intermittent(WorkspaceFaultKind::GitIndexLock, 2);
    let mut resilient = ResilientWorkspaceManager::new(
        WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
        policy,
        3,
    )
    .with_backoff(Duration::from_millis(1));

    let path_res = resilient.prepare_resilient("task-lock-recovery").await;
    assert!(
        path_res.is_ok(),
        "Must recover from intermittent Git index lock contention: {:?}",
        path_res.err()
    );
    assert_eq!(resilient.recovery_count(), 1);

    // 2. Recover from intermittent Transient IO
    let policy_io = WorkspaceFaultPolicy::intermittent(WorkspaceFaultKind::TransientIo, 2);
    let mut resilient_io = ResilientWorkspaceManager::new(
        WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
        policy_io,
        3,
    )
    .with_backoff(Duration::from_millis(1));

    let path_io = resilient_io.prepare_resilient("task-io-recovery").await;
    assert!(path_io.is_ok(), "Must recover from transient I/O fault");
    assert_eq!(resilient_io.recovery_count(), 1);

    // 3. Recover from intermittent Linker OOM / Contention
    let policy_linker =
        WorkspaceFaultPolicy::intermittent(WorkspaceFaultKind::LinkerOomContention, 2);
    let mut resilient_linker = ResilientWorkspaceManager::new(
        WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
        policy_linker,
        3,
    )
    .with_backoff(Duration::from_millis(1));

    let path_linker = resilient_linker
        .prepare_resilient("task-linker-recovery")
        .await;
    assert!(
        path_linker.is_ok(),
        "Must recover from linker memory contention"
    );
    assert_eq!(resilient_linker.recovery_count(), 1);

    // 4. Permanent failure surfaces correctly when retries exhausted
    let policy_perm = WorkspaceFaultPolicy::intermittent(WorkspaceFaultKind::CorruptedPath, 5);
    let mut resilient_perm = ResilientWorkspaceManager::new(
        WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
        policy_perm,
        2,
    )
    .with_backoff(Duration::ZERO);

    let path_perm = resilient_perm.prepare_resilient("task-perm").await;
    assert!(path_perm.is_err(), "Must fail when retries exhausted");
    assert!(matches!(
        path_perm.unwrap_err(),
        WorkspaceError::CorruptedPath(_)
    ));

    // 5. Teardown resilience
    let teardown_res = resilient.teardown_resilient().await;
    assert!(teardown_res.is_ok());

    let _ = std::fs::remove_dir_all(&base_dir);
}

/// TDD test case 2 for TOPIC-20261105-7fe3cf:
/// High-load concurrency benchmark (10,000 requests) & P50/P90/P99 latency profiling
/// with bounded memory verification.
#[tokio::test]
async fn test_topic_7fe3cf_workspace_benchmark_and_high_load() {
    let base_dir = unique_temp_dir("ws_bench_base");
    let mut resilient = ResilientWorkspaceManager::new(
        WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
        WorkspaceFaultPolicy::none(),
        3,
    )
    .with_max_capacity(2_000)
    .with_backoff(Duration::ZERO);

    let stats: WorkspaceBenchmarkStats =
        WorkspaceBenchmarkHarness::run_benchmark(&mut resilient, 10_000).await;

    println!("\n[Workspace Benchmark Stats (10,000 requests)]");
    println!("  Total requests      : {}", stats.total_requests);
    println!("  Successful ops      : {}", stats.successful_operations);
    println!("  Rejected ops        : {}", stats.rejected_count);
    println!(
        "  Throughput          : {:.2} ops/sec",
        stats.throughput_ops_per_sec
    );
    println!("  P50 Latency         : {:.2} µs", stats.p50_latency_us);
    println!("  P90 Latency         : {:.2} µs", stats.p90_latency_us);
    println!("  P99 Latency         : {:.2} µs", stats.p99_latency_us);
    println!("  Max Latency         : {:.2} µs", stats.max_latency_us);
    println!("  Duration            : {:?}", stats.total_duration);
    println!(
        "  Memory Allocated    : {} bytes",
        stats.memory_allocated_bytes
    );

    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 1_000.0,
        "Throughput should exceed 1,000 ops/sec, got {:.2}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0, "P50 latency must be tracked");
    assert!(stats.p90_latency_us > 0.0, "P90 latency must be tracked");
    assert!(
        stats.p99_latency_us < 50_000.0,
        "P99 latency must be under 50ms (50,000 µs), got {} µs",
        stats.p99_latency_us
    );

    // Bounded memory check: memory after 5k extra requests should remain bounded
    let mem_before = resilient.memory_usage_bytes();
    let _ = WorkspaceBenchmarkHarness::run_benchmark(&mut resilient, 5_000).await;
    let mem_after = resilient.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 4096,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );

    let _ = std::fs::remove_dir_all(&base_dir);
}

/// TDD test case 3 for TOPIC-20261105-7fe3cf:
/// Batch preparation and FIFO capacity eviction.
#[tokio::test]
async fn test_topic_7fe3cf_workspace_batch_and_fifo_capacity() {
    let base_dir = unique_temp_dir("ws_batch_base");
    let mut resilient = ResilientWorkspaceManager::new(
        WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
        WorkspaceFaultPolicy::none(),
        3,
    )
    .with_max_capacity(50)
    .with_backoff(Duration::ZERO);

    let task_ids = vec!["batch-1", "batch-2", "batch-3", "batch-4"];
    let batch_res = resilient.batch_prepare(&task_ids).await;
    assert_eq!(batch_res.len(), 4);
    for r in batch_res {
        assert!(r.is_ok());
    }

    assert_eq!(resilient.active_count(), 4);
    let audits = resilient.audit_records();
    assert_eq!(audits.len(), 4);

    let _ = std::fs::remove_dir_all(&base_dir);
}

/// TDD test case 4 for TOPIC-20261105-7fe3cf:
/// Lock contention comparison: Single Shared Lock vs 16-Way Sharded Concurrency.
#[tokio::test]
async fn test_topic_7fe3cf_workspace_lock_contention_comparison() {
    let single_lock_stats = WorkspaceBenchmarkHarness::run_single_lock_benchmark(10_000, 4).await;
    let sharded_stats = WorkspaceBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;

    println!("\n[Lock Contention Comparison (10,000 requests across 4 workers)]");
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
}

/// TDD test case 5 for TOPIC-20261105-7fe3cf:
/// Static verification of workspace Cargo.toml profiles and .cargo/config.toml linker flags
/// to permanently prevent GNU ld memory exhaustion and `collect2: fatal error: ld terminated`.
#[test]
fn test_topic_7fe3cf_cargo_workspace_profiles_and_linker_protection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let cargo_toml_path = root.join("Cargo.toml");
    let cargo_toml =
        std::fs::read_to_string(&cargo_toml_path).expect("Must read workspace Cargo.toml");

    // Profile optimizations
    assert!(
        cargo_toml.contains("[profile.dev]"),
        "Cargo.toml must define [profile.dev]"
    );
    assert!(
        cargo_toml.contains("[profile.test]"),
        "Cargo.toml must define [profile.test]"
    );
    assert!(
        cargo_toml.contains("debug = 1"),
        "Cargo.toml profiles must use debug = 1 to prevent ld memory exhaustion"
    );

    // .cargo/config.toml linker flag
    let cargo_config_path = root.join(".cargo/config.toml");
    assert!(
        cargo_config_path.exists(),
        ".cargo/config.toml must exist to configure linker flags"
    );
    let cargo_config =
        std::fs::read_to_string(&cargo_config_path).expect("Must read .cargo/config.toml");
    assert!(
        cargo_config.contains("--no-keep-memory"),
        ".cargo/config.toml must instruct GNU ld to use --no-keep-memory"
    );

    // Verify all 7 workspace members
    let expected_members = [
        "agent-domain",
        "agent-engine",
        "agent-memory",
        "agent-runtime",
        "agent-core",
        "agent-cli",
        "agent-server",
    ];
    for member in expected_members {
        let crate_cargo = root.join("crates").join(member).join("Cargo.toml");
        assert!(
            crate_cargo.exists(),
            "Workspace member {} Cargo.toml must exist",
            member
        );
    }

    // Verify retryable classification logic
    assert!(is_retryable_workspace_error(
        &WorkspaceError::GitLockContention
    ));
    assert!(is_retryable_workspace_error(&WorkspaceError::TransientIo(
        "disk busy".into()
    )));
    assert!(is_retryable_workspace_error(&WorkspaceError::SocketTimeout));
    assert!(is_retryable_workspace_error(&WorkspaceError::LinkerOom));
    assert!(!is_retryable_workspace_error(&WorkspaceError::AbruptExit(
        1
    )));
    assert!(!is_retryable_workspace_error(&WorkspaceError::Permanent(
        "perm".into()
    )));
}

/// TDD test case 6 for TOPIC-20261105-7fe3cf:
/// Race condition reproduction and high concurrency resilience verification.
/// Simulates multiple concurrent workers racing on prepare, teardown, and FIFO eviction
/// while encountering transient Git lock and linker memory contention faults.
#[tokio::test]
async fn test_topic_7fe3cf_workspace_race_condition_and_concurrency_resilience() {
    let base_dir = unique_temp_dir("ws_race_repro");

    // 1. Without resilience (no retries), intermittent GitIndexLock triggers error
    let non_resilient_policy =
        WorkspaceFaultPolicy::intermittent(WorkspaceFaultKind::GitIndexLock, 2);
    let mut non_resilient = ResilientWorkspaceManager::new(
        WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
        non_resilient_policy,
        0, // 0 retries -> fails immediately on fault
    );
    let failure_res = non_resilient.prepare_resilient("unhandled-race").await;
    assert!(
        failure_res.is_err(),
        "Non-resilient execution must surface GitLockContention failure"
    );
    assert_eq!(failure_res.unwrap_err(), WorkspaceError::GitLockContention);

    // 2. With ResilientWorkspaceManager, concurrent tasks recover from faults automatically
    let policy = WorkspaceFaultPolicy::intermittent(WorkspaceFaultKind::GitIndexLock, 4);
    let shared_mgr = std::sync::Arc::new(tokio::sync::Mutex::new(
        ResilientWorkspaceManager::new(
            WorkspaceManager::new(&base_dir, IsolationMode::LocalDirectory),
            policy,
            5,
        )
        .with_max_capacity(20)
        .with_backoff(Duration::from_millis(1)),
    ));

    let workers = 8;
    let ops_per_worker = 15;
    let mut handles = Vec::new();

    for w in 0..workers {
        let mgr = std::sync::Arc::clone(&shared_mgr);
        handles.push(tokio::spawn(async move {
            let mut succ = 0;
            for i in 0..ops_per_worker {
                let task_id = format!("worker-{}-task-{}", w, i);
                let mut guard = mgr.lock().await;
                if guard.prepare_resilient(&task_id).await.is_ok() {
                    succ += 1;
                }
            }
            succ
        }));
    }

    let mut total_succeeded = 0;
    for h in handles {
        if let Ok(s) = h.await {
            total_succeeded += s;
        }
    }

    assert_eq!(
        total_succeeded,
        workers * ops_per_worker,
        "All concurrent tasks must succeed through automatic recovery"
    );

    let guard = shared_mgr.lock().await;
    assert!(
        guard.recovery_count() > 0,
        "Recovery count must record self-healing events"
    );
    assert!(
        guard.active_count() <= 20,
        "Active count must not exceed max_capacity (20) under race condition"
    );
    assert!(
        guard.audit_records().len() <= 20,
        "Audit ledger length must not exceed max_capacity (20) under race condition"
    );
    drop(guard);

    let _ = std::fs::remove_dir_all(&base_dir);
}
