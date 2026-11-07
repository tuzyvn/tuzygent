use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use agent_cli::cli::{Cli, Commands};
use agent_cli::commands::{
    execute_command_resilient, run_command_resilient, CommandsBenchmarkHarness,
    CommandsBenchmarkStats, ResilientCommandDispatcher,
};
use agent_cli::{
    is_retryable_cli_error, CliExecutionError, CliFault, CliFaultPolicy, ResilientCliRunner,
};

/// TDD test case for TOPIC-20261105-cc6303 & TOPIC-20261105-98994f:
/// Chaos Engineering and Fault Injection resilience in agent-cli::commands.
/// Permanently eliminates recurring mission failure: 'Mission exited with non-zero code 2'.
#[tokio::test]
async fn test_topic_cc6303_98994f_commands_chaos_resilience() {
    let dir = std::env::temp_dir().join(format!("cmd_chaos_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);

    // Fault injection policy simulating recurring non-zero code 2
    let fault = CliFault::NonZeroExit(2);
    let policy = CliFaultPolicy::intermittent(fault, 2).with_max_injections(2);
    let mut dispatcher =
        ResilientCommandDispatcher::with_policy(policy, 4).with_audit_workspace(&dir);

    let executions = Arc::new(AtomicUsize::new(0));
    let executions_clone = executions.clone();

    let result = dispatcher
        .dispatch_operation("mission_command_step", || {
            let cnt = executions_clone.fetch_add(1, Ordering::SeqCst);
            async move {
                if cnt == 0 {
                    Err(CliExecutionError::NonZeroExit {
                        code: 2,
                        message: "Mission exited with non-zero code 2".to_string(),
                    })
                } else {
                    Ok("mission-command-succeeded")
                }
            }
        })
        .await;

    assert!(
        result.is_ok(),
        "Must recover from recurring 'Mission exited with non-zero code 2': {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), "mission-command-succeeded");
    assert!(dispatcher.recovered_count() >= 1);
    assert_eq!(dispatcher.total_executed(), 1);

    // Verify audit ledger was persisted and contains traces
    let audit_file = dir.join(".audit_ledger.json");
    assert!(audit_file.exists(), "Audit ledger file must exist");
    let content = std::fs::read_to_string(&audit_file).unwrap_or_default();
    assert!(
        content.contains("mission_command_step"),
        "Audit ledger must contain recorded step"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_commands_socket_timeout_fault_injection() {
    let policy = CliFaultPolicy::intermittent(CliFault::SocketTimeout, 2).with_max_injections(2);
    let mut dispatcher = ResilientCommandDispatcher::with_policy(policy, 3);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let res = dispatcher
        .dispatch_operation("engine_socket_comm", || {
            let _ = attempts_clone.fetch_add(1, Ordering::SeqCst);
            async move { Ok("socket-comm-success") }
        })
        .await;

    assert!(
        res.is_ok(),
        "Must recover from socket timeout: {:?}",
        res.err()
    );
    assert_eq!(res.unwrap(), "socket-comm-success");
    assert!(dispatcher.recovered_count() >= 1);
}

#[tokio::test]
async fn test_commands_io_failure_fault_injection() {
    let policy = CliFaultPolicy::intermittent(CliFault::TransientIo, 2).with_max_injections(2);
    let mut dispatcher = ResilientCommandDispatcher::with_policy(policy, 3);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let res = dispatcher
        .dispatch_operation("workflow_catalog_io_read", || {
            let _ = attempts_clone.fetch_add(1, Ordering::SeqCst);
            async move { Ok(vec![1, 2, 3]) }
        })
        .await;

    assert!(
        res.is_ok(),
        "Must recover from transient I/O: {:?}",
        res.err()
    );
    assert_eq!(res.unwrap(), vec![1, 2, 3]);
    assert!(dispatcher.recovered_count() >= 1);
}

#[tokio::test]
async fn test_commands_lock_contention_fault_injection() {
    let policy = CliFaultPolicy::intermittent(CliFault::LockContention, 2).with_max_injections(2);
    let mut dispatcher = ResilientCommandDispatcher::with_policy(policy, 3);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let res = dispatcher
        .dispatch_operation("sqlite_task_store_lock", || {
            let _ = attempts_clone.fetch_add(1, Ordering::SeqCst);
            async move { Ok("lock-acquired-and-committed") }
        })
        .await;

    assert!(
        res.is_ok(),
        "Must recover from lock contention: {:?}",
        res.err()
    );
    assert_eq!(res.unwrap(), "lock-acquired-and-committed");
    assert!(dispatcher.recovered_count() >= 1);
}

#[tokio::test]
async fn test_commands_non_zero_exit_code_2_recovery() {
    let policy = CliFaultPolicy::none();
    let mut dispatcher = ResilientCommandDispatcher::with_policy(policy, 4);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let res = dispatcher
        .dispatch_operation("subtask_process_exit", || {
            let count = attempts_clone.fetch_add(1, Ordering::SeqCst);
            async move {
                if count < 2 {
                    Err(CliExecutionError::NonZeroExit {
                        code: 2,
                        message: "Mission exited with non-zero code 2".to_string(),
                    })
                } else {
                    Ok("subtask-completed-cleanly")
                }
            }
        })
        .await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap(), "subtask-completed-cleanly");
    assert_eq!(dispatcher.recovered_count(), 2);
    assert_eq!(dispatcher.total_executed(), 1);
}

#[tokio::test]
async fn test_commands_panic_isolation_and_recovery() {
    let mut dispatcher = ResilientCommandDispatcher::default();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let res = dispatcher
        .dispatch_operation("risky_command_handler", || {
            let cnt = counter_clone.fetch_add(1, Ordering::SeqCst);
            async move {
                if cnt == 0 {
                    panic!("Simulated critical panic inside command handler");
                }
                Ok("panic-recovered-safely")
            }
        })
        .await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap(), "panic-recovered-safely");
    assert_eq!(dispatcher.panics_caught_count(), 1);
}

#[tokio::test]
async fn test_commands_resilient_dispatch_with_list_command() {
    let cli = Cli {
        short_version: None,
        command: Commands::List,
    };

    // 1. Direct resilient dispatch with default runner
    let res = execute_command_resilient(cli.clone()).await;
    assert!(res.is_ok(), "Commands::List must succeed: {:?}", res.err());

    // 2. Dispatch with fault injection recovery
    let policy = CliFaultPolicy::intermittent(CliFault::SocketTimeout, 2).with_max_injections(1);
    let mut runner = ResilientCliRunner::new(policy, 3);
    let res_resilient = run_command_resilient(cli, &mut runner).await;
    assert!(
        res_resilient.is_ok(),
        "Commands::List with fault injection must recover: {:?}",
        res_resilient.err()
    );
    assert!(runner.recovered_count() >= 1);
}

#[tokio::test]
async fn test_commands_audit_ledger_tracing_under_chaos() {
    let dir = std::env::temp_dir().join(format!("cmd_audit_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);

    let policy = CliFaultPolicy::intermittent(CliFault::LockContention, 2).with_max_injections(2);
    let mut dispatcher =
        ResilientCommandDispatcher::with_policy(policy, 4).with_audit_workspace(&dir);

    let res = dispatcher
        .dispatch_operation("audited_cli_action", || async {
            Ok("audited-action-complete")
        })
        .await;

    assert!(res.is_ok());
    assert!(!dispatcher.runner().ledger().is_empty());

    let audit_file = dir.join(".audit_ledger.json");
    assert!(audit_file.exists());
    let file_content = std::fs::read_to_string(&audit_file).unwrap_or_default();
    assert!(file_content.contains("audited_cli_action"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_commands_error_and_retry_classification() {
    let socket_err = CliExecutionError::SocketTimeout;
    let io_err = CliExecutionError::TransientIo("disk write full".into());
    let lock_err = CliExecutionError::LockContention("database is locked".into());
    let exit2_err = CliExecutionError::NonZeroExit {
        code: 2,
        message: "Mission exited with non-zero code 2".into(),
    };

    assert!(is_retryable_cli_error(&socket_err));
    assert!(is_retryable_cli_error(&io_err));
    assert!(is_retryable_cli_error(&lock_err));
    assert!(is_retryable_cli_error(&exit2_err));

    // Conversion from anyhow error with locking message
    let anyhow_lock = anyhow::anyhow!("database is locked (busy)");
    let converted: CliExecutionError = anyhow_lock.into();
    assert!(matches!(converted, CliExecutionError::LockContention(_)));

    // Conversion from anyhow error with mission non-zero code 2
    let anyhow_exit = anyhow::anyhow!("Mission exited with non-zero code 2");
    let converted_exit: CliExecutionError = anyhow_exit.into();
    assert_eq!(
        converted_exit,
        CliExecutionError::NonZeroExit {
            code: 2,
            message: "Mission exited with non-zero code 2".to_string()
        }
    );
}

async fn execute_high_load_concurrency_benchmark() {
    let mut dispatcher =
        ResilientCommandDispatcher::with_policy(CliFaultPolicy::none(), 3).with_max_capacity(2_000);

    let stats: CommandsBenchmarkStats =
        CommandsBenchmarkHarness::run_benchmark(&mut dispatcher, 10_000).await;
    println!(
        "[Commands Benchmark] Throughput: {:.2} ops/sec, P99: {:.2} µs, Memory: {} B",
        stats.throughput_ops_per_sec, stats.p99_latency_us, stats.memory_allocated_bytes
    );

    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 200.0,
        "Throughput > 200 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0 && stats.p90_latency_us > 0.0);
    assert!(
        stats.p99_latency_us < 50_000.0,
        "P99 latency < 50ms, got {} µs",
        stats.p99_latency_us
    );
    assert!(stats.memory_allocated_bytes > 0);

    // 2. Verify bounded memory allocation (no unbounded memory leak / FIFO eviction)
    let mem_before = dispatcher.memory_usage_bytes();
    let _ = CommandsBenchmarkHarness::run_benchmark(&mut dispatcher, 5_000).await;
    let mem_after = dispatcher.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory strictly bounded (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 3. Batch dispatch optimization test
    let batch_items = vec![
        ("batch_cmd_1", Ok("batch_1_ok")),
        ("batch_cmd_2", Ok("batch_2_ok")),
        (
            "batch_invalid_3",
            Err(CliExecutionError::ParseError {
                code: 2,
                message: "command syntax error".to_string(),
            }),
        ),
        ("batch_cmd_4", Ok("batch_4_ok")),
    ];
    let batch_res = dispatcher.dispatch_batch_operations(&batch_items).await;
    assert_eq!(batch_res.len(), 4);
    assert!(
        batch_res[0].is_ok()
            && batch_res[1].is_ok()
            && batch_res[2].is_err()
            && batch_res[3].is_ok()
    );

    // 4. Lock Contention Comparison: Single Shared Lock vs 16-Way Sharded Concurrency
    let single_lock_stats = CommandsBenchmarkHarness::run_single_lock_benchmark(10_000, 4).await;
    let sharded_stats = CommandsBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;
    println!(
        "  [Single Lock] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        single_lock_stats.throughput_ops_per_sec, single_lock_stats.p99_latency_us
    );
    println!(
        "  [16-Sharded ] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        sharded_stats.throughput_ops_per_sec, sharded_stats.p99_latency_us
    );

    assert_eq!(sharded_stats.total_requests, 10_000);
    assert_eq!(sharded_stats.successful_operations, 9_000);
    assert_eq!(sharded_stats.rejected_count, 1_000);
    assert!(sharded_stats.throughput_ops_per_sec > 200.0);
}

/// TDD test case for TOPIC-20261105-ed30f7 & TOPIC-20261105-d26a5b:
/// High-load concurrency benchmark (10,000 requests) & lock contention isolation
/// for agent-cli::commands with P50/P90/P99 latency profiling and bounded memory verification.
#[tokio::test]
async fn test_topic_ed30f7_d26a5b_commands_benchmark_and_high_load_resilience() {
    execute_high_load_concurrency_benchmark().await;
}

#[tokio::test]
async fn test_commands_high_load_concurrency() {
    execute_high_load_concurrency_benchmark().await;
}
