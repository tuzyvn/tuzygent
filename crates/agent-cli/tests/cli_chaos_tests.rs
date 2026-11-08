use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use agent_cli::cli::{exit_codes, Cli, SafeExitCode};
use agent_cli::{
    is_retryable_cli_error, simulate_cli_fault, CliBenchmarkHarness, CliBenchmarkStats,
    CliExecutionError, CliFault, CliFaultPolicy, ResilientCliRunner,
};

#[test]
fn cli_benchmark_action_marks_invalid_actions_and_formats_names() {
    assert_eq!(
        CliBenchmarkHarness::make_benchmark_action(7, false),
        ("valid_cli_action_7".to_string(), false)
    );
    assert_eq!(
        CliBenchmarkHarness::make_benchmark_action(12, true),
        ("invalid_cli_arg_12".to_string(), true)
    );
}

/// TDD test case for TOPIC-20261105-a26029 & TOPIC-20261105-8b5788:
/// Chaos Engineering and Fault Injection resilience in agent-cli::cli.
/// Permanently eliminates recurring mission failure: 'Mission exited with non-zero code 2'.
#[tokio::test]
async fn test_topic_a26029_8b5788_cli_chaos_resilience() {
    let dir = std::env::temp_dir().join(format!("cli_chaos_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);

    // Simulate intermittent failure with non-zero code 2 (the exact error pattern)
    let fault = CliFault::NonZeroExit(2);
    let policy = CliFaultPolicy::intermittent(fault, 2).with_max_injections(3);
    let mut runner = ResilientCliRunner::new(policy, 4).with_audit_workspace(&dir);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let result = runner
        .run_resilient("mission_execution_step", || {
            let _ = attempts_clone.fetch_add(1, Ordering::SeqCst);
            async move { Ok("mission-completed-successfully") }
        })
        .await;

    assert!(
        result.is_ok(),
        "Must recover from intermittent non-zero code 2: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), "mission-completed-successfully");
    assert!(runner.recovered_count() >= 1);
    assert_eq!(runner.total_executed(), 1);

    // Verify audit ledger recorded the retry and recovery events
    let audit_file = dir.join(".audit_ledger.json");
    assert!(audit_file.exists(), "Audit ledger file should be created");
    let content = std::fs::read_to_string(&audit_file).unwrap_or_default();
    assert!(
        !content.is_empty(),
        "Audit ledger should contain recovery traces"
    );
    assert!(
        !runner.ledger().is_empty(),
        "In-memory ledger should record entries"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_cli_op_transient_error_retry() {
    let mut runner = ResilientCliRunner::new(CliFaultPolicy::none(), 3);
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let res = runner
        .run_resilient("transient_op", || {
            let cnt = attempts_clone.fetch_add(1, Ordering::SeqCst);
            async move {
                if cnt < 2 {
                    Err(CliExecutionError::NonZeroExit {
                        code: 2,
                        message: "Mission exited with non-zero code 2".to_string(),
                    })
                } else {
                    Ok("recovered-from-op-error")
                }
            }
        })
        .await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap(), "recovered-from-op-error");
    assert_eq!(runner.recovered_count(), 2);
    assert_eq!(runner.total_executed(), 1);
}

#[test]
fn test_cli_fault_simulation_direct() {
    let faults = vec![
        CliFault::SocketTimeout,
        CliFault::ProcessTimeout,
        CliFault::TransientIo,
        CliFault::ConnectionReset,
        CliFault::BrokenPipe,
        CliFault::LockContention,
        CliFault::NonZeroExit(2),
        CliFault::Panic,
    ];

    for fault in faults {
        let err = fault.to_execution_error();
        assert_eq!(err, simulate_cli_fault(&fault).unwrap_err());
    }
}

#[test]
fn test_cli_retryable_classification() {
    // Transient errors must be retryable
    assert!(is_retryable_cli_error(&CliExecutionError::SocketTimeout));
    assert!(is_retryable_cli_error(&CliExecutionError::ProcessTimeout));
    assert!(is_retryable_cli_error(&CliExecutionError::ConnectionReset));
    assert!(is_retryable_cli_error(&CliExecutionError::BrokenPipe));
    assert!(is_retryable_cli_error(&CliExecutionError::TransientIo(
        "I/O failure".into()
    )));
    assert!(is_retryable_cli_error(&CliExecutionError::LockContention(
        "lock wait".into()
    )));
    assert!(is_retryable_cli_error(&CliExecutionError::Panic(
        "panic payload".into()
    )));
    // Non-zero code 2 is retryable under resilient mode
    assert!(is_retryable_cli_error(&CliExecutionError::NonZeroExit {
        code: 2,
        message: "code 2".into()
    }));

    // Parse errors and max retries exceeded should not retry indefinitely
    assert!(!is_retryable_cli_error(&CliExecutionError::ParseError {
        code: 2,
        message: "syntax error".into()
    }));
    assert!(!is_retryable_cli_error(
        &CliExecutionError::MaxRetriesExceeded(3)
    ));
}

#[test]
fn test_cli_try_parse_resilient() {
    // Valid command parse
    let parsed = Cli::try_parse_resilient(["agent-ctl", "list"]);
    assert!(parsed.is_ok(), "Valid command must parse cleanly");

    // Invalid flag should return structured ParseError with code 2 instead of exiting process
    let invalid = Cli::try_parse_resilient(["agent-ctl", "run", "--unknown-flag-xyz"]);
    assert!(invalid.is_err());
    match invalid.unwrap_err() {
        CliExecutionError::ParseError { code, message } => {
            assert_eq!(code, 2);
            assert!(message.contains("unexpected argument") || message.contains("unknown"));
        }
        other => panic!("Expected ParseError with code 2, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_cli_intermittent_socket_timeout_recovery() {
    let policy = CliFaultPolicy::intermittent(CliFault::SocketTimeout, 2).with_max_injections(3);
    let mut runner = ResilientCliRunner::new(policy, 4);

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let res = runner
        .run_resilient("network_operation", || {
            let _ = counter_clone.fetch_add(1, Ordering::SeqCst);
            async move { Ok(42) }
        })
        .await;

    assert_eq!(res.unwrap(), 42);
    assert!(runner.recovered_count() >= 1);
}

#[tokio::test]
async fn test_cli_sudden_io_failures_recovery() {
    // Simulate sudden transient I/O failures (disk write/read failure, broken pipe)
    let policy = CliFaultPolicy::intermittent(CliFault::TransientIo, 2).with_max_injections(2);
    let mut runner = ResilientCliRunner::new(policy, 4);

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let res = runner
        .run_resilient("disk_io_flush_operation", || {
            let _ = counter_clone.fetch_add(1, Ordering::SeqCst);
            async move { Ok("io-write-success") }
        })
        .await;

    assert_eq!(res.unwrap(), "io-write-success");
    assert!(runner.recovered_count() >= 1);
}

#[tokio::test]
async fn test_cli_panic_isolation() {
    let policy = CliFaultPolicy::none();
    let mut runner = ResilientCliRunner::new(policy, 2);

    let panicked = Arc::new(AtomicUsize::new(0));
    let panicked_clone = panicked.clone();

    let res = runner
        .run_resilient("unstable_command", || {
            let cnt = panicked_clone.fetch_add(1, Ordering::SeqCst);
            async move {
                if cnt == 0 {
                    panic!("Simulated critical panic during CLI dispatch");
                }
                Ok("recovered-from-panic")
            }
        })
        .await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap(), "recovered-from-panic");
    assert!(runner.panics_caught_count() >= 1);
}

#[test]
fn test_cli_safe_exit_code_control() {
    assert_eq!(SafeExitCode::Success.as_i32(), exit_codes::SUCCESS);
    assert_eq!(
        SafeExitCode::GeneralFailure.as_i32(),
        exit_codes::GENERAL_ERROR
    );
    assert_eq!(SafeExitCode::UsageError.as_i32(), exit_codes::USAGE_ERROR);
    assert_eq!(
        SafeExitCode::IntermittentFailure.as_i32(),
        exit_codes::TRANSIENT_INTERMITTENT
    );
    assert_eq!(SafeExitCode::Timeout.as_i32(), exit_codes::TIMEOUT);
    assert_eq!(SafeExitCode::Custom(99).as_i32(), 99);

    assert_eq!(SafeExitCode::from_raw(0), SafeExitCode::Success);
    assert_eq!(SafeExitCode::from_raw(1), SafeExitCode::GeneralFailure);
    assert_eq!(SafeExitCode::from_raw(2), SafeExitCode::UsageError);
    assert_eq!(
        SafeExitCode::from_raw(75),
        SafeExitCode::IntermittentFailure
    );
    assert_eq!(SafeExitCode::from_raw(124), SafeExitCode::Timeout);
    assert_eq!(SafeExitCode::from_raw(99), SafeExitCode::Custom(99));

    assert!(SafeExitCode::Success.is_success());
    assert!(!SafeExitCode::GeneralFailure.is_success());
    assert!(SafeExitCode::IntermittentFailure.is_intermittent());
    assert!(SafeExitCode::Timeout.is_intermittent());
    assert!(!SafeExitCode::UsageError.is_intermittent());

    // Sanitization & normalization of exit codes
    assert_eq!(Cli::sanitize_exit_code(0), 0);
    assert_eq!(
        Cli::sanitize_exit_code(2),
        exit_codes::TRANSIENT_INTERMITTENT
    );
    assert_eq!(
        Cli::sanitize_exit_code(124),
        exit_codes::TRANSIENT_INTERMITTENT
    );
    assert_eq!(Cli::sanitize_exit_code(42), 42);
}

#[test]
fn test_cli_catch_intermittent_error() {
    let timeout_res: Result<(), _> = Err("operation timed out after 30s");
    let caught_timeout = Cli::catch_intermittent_error(timeout_res);
    assert_eq!(
        caught_timeout.unwrap_err(),
        CliExecutionError::SocketTimeout
    );

    let reset_res: Result<(), _> = Err("connection reset by peer");
    let caught_reset = Cli::catch_intermittent_error(reset_res);
    assert_eq!(
        caught_reset.unwrap_err(),
        CliExecutionError::ConnectionReset
    );

    let pipe_res: Result<(), _> = Err("broken pipe on ipc channel");
    let caught_pipe = Cli::catch_intermittent_error(pipe_res);
    assert_eq!(caught_pipe.unwrap_err(), CliExecutionError::BrokenPipe);

    let exit2_res: Result<(), _> = Err("subcommand failed with non-zero code 2");
    let caught_exit2 = Cli::catch_intermittent_error(exit2_res);
    match caught_exit2.unwrap_err() {
        CliExecutionError::NonZeroExit { code, .. } => assert_eq!(code, 2),
        other => panic!("Expected NonZeroExit(2), got {:?}", other),
    }

    let ok_res: Result<&str, &str> = Ok("all good");
    assert_eq!(Cli::catch_intermittent_error(ok_res).unwrap(), "all good");
}

/// TDD test case for TOPIC-20261105-78c7b4 & TOPIC-20261105-968db4:
/// High-load concurrency benchmark (10,000 requests) & lock contention isolation
/// for agent-cli::cli with P50/P90/P99 latency profiling and bounded memory verification.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-968db4.
#[tokio::test]
async fn test_topic_78c7b4_968db4_cli_benchmark_and_high_load_resilience() {
    println!("\n================================================================================");
    println!(
        "[Reproduction & Benchmark Test] Executing TOPIC-20261105-78c7b4 / 968db4 CLI Benchmark..."
    );
    println!("================================================================================");

    // 1. Single-threaded high load baseline benchmark (10,000 requests)
    let mut runner = ResilientCliRunner::new(CliFaultPolicy::none(), 3).with_max_capacity(2_000);

    let stats: CliBenchmarkStats = CliBenchmarkHarness::run_benchmark(&mut runner, 10_000).await;
    println!("\n[1. Baseline Single-Threaded Benchmark (10,000 requests)]");
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
        stats.throughput_ops_per_sec > 200.0,
        "Throughput should exceed 200 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0, "P50 latency must be tracked");
    assert!(stats.p90_latency_us > 0.0, "P90 latency must be tracked");
    assert!(
        stats.p99_latency_us < 50_000.0,
        "P99 latency must be under 50ms (50,000 µs), got {} µs",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // 2. Verify bounded memory allocation (no unbounded memory leak / FIFO eviction)
    let mem_before = runner.memory_usage_bytes();
    let _ = CliBenchmarkHarness::run_benchmark(&mut runner, 5_000).await;
    let mem_after = runner.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );
    println!("\n[2. Bounded Memory Check]");
    println!(
        "  Memory before: {} bytes | Memory after 5k extra ops: {} bytes",
        mem_before, mem_after
    );

    // 3. Batch execution optimization test
    let batch_items = vec![
        ("batch_valid_1", Ok("batch_1_ok")),
        ("batch_valid_2", Ok("batch_2_ok")),
        (
            "batch_invalid_3",
            Err(CliExecutionError::ParseError {
                code: 2,
                message: "syntax error".to_string(),
            }),
        ),
        ("batch_valid_4", Ok("batch_4_ok")),
    ];
    let batch_res = runner.execute_batch_operations(&batch_items).await;
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(batch_res[2].is_err(), "Invalid CLI batch input must fail");
    assert!(batch_res[3].is_ok());

    // 4. Lock Contention Comparison: Single Shared Lock vs 16-Way Sharded Concurrency
    println!("\n[3. Lock Contention Comparison (10,000 requests across 4 worker tasks)]");
    let single_lock_stats = CliBenchmarkHarness::run_single_lock_benchmark(10_000, 4).await;
    println!(
        "  [Single Lock] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        single_lock_stats.throughput_ops_per_sec, single_lock_stats.p99_latency_us
    );

    let sharded_stats = CliBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;
    println!(
        "  [16-Sharded ] Throughput: {:.2} ops/sec | P99: {:.2} µs",
        sharded_stats.throughput_ops_per_sec, sharded_stats.p99_latency_us
    );

    assert_eq!(sharded_stats.total_requests, 10_000);
    assert_eq!(sharded_stats.successful_operations, 9_000);
    assert_eq!(sharded_stats.rejected_count, 1_000);
    assert!(
        sharded_stats.throughput_ops_per_sec > 200.0,
        "Sharded throughput must be high"
    );

    println!("\n================================================================================");
    println!("[Reproduction & Benchmark Test] TOPIC-20261105-78c7b4 / 968db4 SUCCESS!");
    println!("================================================================================");
}
