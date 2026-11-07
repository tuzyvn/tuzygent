use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use agent_domain::{AuditEventType, AuditLedger};
use agent_engine::{
    is_retryable_cli_error_message, simulate_cli_fault, BackoffPolicy, BenchmarkMockCliEngine,
    CliBenchmarkHarness, CliBenchmarkStats, CliError, CliFault, CliFaultPolicy, ComputeEngine,
    EngineError, EngineInput, EngineKind, EngineOutput, GenericCliEngine, ResilientCliEngine,
    ResilientCliWrapper,
};
use std::time::{Duration, Instant};

fn create_test_input(prompt: &str, timeout_seconds: u64) -> EngineInput {
    EngineInput {
        on_event: None,
        command_sandbox: None,
        prompt_or_command: prompt.to_string(),
        system_context: "You are a test CLI agent".to_string(),
        working_dir: ".".to_string(),
        timeout_seconds,
        on_stream_chunk: None,
        temperature: None,
        max_tokens: None,
        max_output_bytes: None,
    }
}

fn create_test_output(stdout_content: &str) -> EngineOutput {
    EngineOutput {
        stdout_content: stdout_content.to_string(),
        exit_code: 0,
        execution_duration_ms: 10,
        engine_used: EngineKind::Generic("test-output".to_string()),
        session_id: None,
    }
}

struct MockFailingCliEngine {
    failures_remaining: Arc<AtomicUsize>,
    fail_error: EngineError,
    success_output: String,
}

#[test]
fn test_resilient_cli_with_engine_uses_default_policy_and_retries() {
    let engine = ResilientCliEngine::with_engine(BenchmarkMockCliEngine);

    assert_eq!(engine.max_retries(), 3);
    assert!(engine.backoff_policy().is_none());
    assert_eq!(engine.fault_policy().lock().unwrap().injected_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_with_fault_policy_applies_injected_fault() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(0)),
        fail_error: EngineError::Timeout,
        success_output: "fault-policy-recovered".to_string(),
    };
    let policy = CliFaultPolicy::always(CliFault::ProcessTimeout).with_max_injections(1);
    let engine = ResilientCliEngine::with_fault_policy(mock, policy);

    let output = engine
        .execute(&create_test_input("fault-policy", 5))
        .await
        .expect("the default retry count should recover from one injected fault");
    assert_eq!(output.stdout_content, "fault-policy-recovered");
    assert_eq!(engine.max_retries(), 3);
    assert_eq!(engine.recovered_count(), 1);
}

#[tokio::test]
async fn test_resilient_cli_with_retry_delay_waits_between_attempts() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(1)),
        fail_error: EngineError::Timeout,
        success_output: "delayed-retry".to_string(),
    };
    let engine = ResilientCliEngine::with_engine(mock).with_retry_delay_ms(25);
    let start = Instant::now();

    let output = engine
        .execute(&create_test_input("retry-delay", 5))
        .await
        .expect("retry should succeed after the configured delay");

    assert_eq!(output.stdout_content, "delayed-retry");
    assert!(start.elapsed() >= Duration::from_millis(20));
    assert_eq!(engine.recovered_count(), 1);
}

#[test]
fn test_resilient_cli_with_backoff_policy_stores_policy() {
    let policy = BackoffPolicy::fixed(Duration::from_millis(17));
    let engine =
        ResilientCliEngine::with_engine(BenchmarkMockCliEngine).with_backoff_policy(policy.clone());

    assert_eq!(engine.backoff_policy(), Some(&policy));
}

#[test]
fn test_resilient_cli_backoff_policy_is_none_by_default() {
    let engine = ResilientCliEngine::with_engine(BenchmarkMockCliEngine);

    assert_eq!(engine.backoff_policy(), None);
}

#[test]
fn test_resilient_cli_with_capacity_reserves_cache_storage() {
    let without_reservation = ResilientCliEngine::with_engine(BenchmarkMockCliEngine);
    let with_reservation = ResilientCliEngine::with_engine(BenchmarkMockCliEngine).with_capacity(8);

    assert!(with_reservation.memory_usage_bytes() > without_reservation.memory_usage_bytes());
    // Capacity only reserves storage; caching remains disabled until bounded capacity is set.
    with_reservation.cache_response("reserved-only", &create_test_output("unused"));
    assert!(with_reservation.get_cached("reserved-only").is_none());
}

#[test]
fn test_resilient_cli_cache_evicts_oldest_response_at_capacity() {
    let engine = ResilientCliEngine::with_engine(BenchmarkMockCliEngine).with_max_capacity(2);

    engine.cache_response("first", &create_test_output("first-response"));
    engine.cache_response("second", &create_test_output("second-response"));
    engine.cache_response("third", &create_test_output("third-response"));

    assert!(engine.get_cached("first").is_none());
    assert_eq!(
        engine.get_cached("second").unwrap().stdout_content,
        "second-response"
    );
    assert_eq!(
        engine.get_cached("third").unwrap().stdout_content,
        "third-response"
    );
}

#[async_trait::async_trait]
impl ComputeEngine for MockFailingCliEngine {
    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let current = self.failures_remaining.load(Ordering::SeqCst);
        if current > 0 {
            self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
            Err(match &self.fail_error {
                EngineError::Timeout => EngineError::Timeout,
                EngineError::ProcessFailed(msg) => EngineError::ProcessFailed(msg.clone()),
                EngineError::CliNotFound(msg) => EngineError::CliNotFound(msg.clone()),
                EngineError::ApiRequestFailed(msg) => EngineError::ApiRequestFailed(msg.clone()),
                EngineError::PolicyRejected(msg) => EngineError::PolicyRejected(msg.clone()),
            })
        } else {
            Ok(EngineOutput {
                stdout_content: self.success_output.clone(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Generic("mock-failing-cli".to_string()),
                session_id: None,
            })
        }
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Generic("mock-failing-cli".to_string())
    }
}

#[test]
fn test_cli_fault_simulation_and_taxonomy() {
    assert_eq!(
        simulate_cli_fault(&CliFault::SocketTimeout),
        Err(CliError::SocketTimeout)
    );
    assert_eq!(
        simulate_cli_fault(&CliFault::ProcessTimeout),
        Err(CliError::ProcessTimeout)
    );
    assert_eq!(
        simulate_cli_fault(&CliFault::BrokenPipe),
        Err(CliError::BrokenPipe)
    );
    assert!(matches!(
        simulate_cli_fault(&CliFault::TransientIo),
        Err(CliError::TransientIo(_))
    ));
    assert!(matches!(
        simulate_cli_fault(&CliFault::ProcessFailed),
        Err(CliError::ProcessFailed(_))
    ));
    assert!(matches!(
        simulate_cli_fault(&CliFault::CorruptedOutput),
        Err(CliError::CorruptedOutput(_))
    ));
    assert!(matches!(
        simulate_cli_fault(&CliFault::CliNotFound),
        Err(CliError::CliNotFound(_))
    ));

    // Verify mapping to EngineError
    assert!(matches!(
        CliFault::SocketTimeout.to_engine_error(),
        EngineError::Timeout
    ));
    assert!(matches!(
        CliFault::ProcessTimeout.to_engine_error(),
        EngineError::Timeout
    ));
    assert!(matches!(
        CliFault::BrokenPipe.to_engine_error(),
        EngineError::ProcessFailed(_)
    ));
    assert!(matches!(
        CliFault::TransientIo.to_engine_error(),
        EngineError::ProcessFailed(_)
    ));
    assert!(matches!(
        CliFault::CorruptedOutput.to_engine_error(),
        EngineError::ProcessFailed(_)
    ));
    assert!(matches!(
        CliFault::CliNotFound.to_engine_error(),
        EngineError::CliNotFound(_)
    ));
}

#[test]
fn test_cli_fault_simulate_fault_alias_matches_fault_method() {
    for (fault, expected) in [
        (CliFault::SocketTimeout, CliError::SocketTimeout),
        (CliFault::ProcessTimeout, CliError::ProcessTimeout),
        (CliFault::BrokenPipe, CliError::BrokenPipe),
        (
            CliFault::TransientIo,
            CliError::TransientIo(
                "Simulated transient I/O failure during CLI process execution".to_string(),
            ),
        ),
        (
            CliFault::ProcessFailed,
            CliError::ProcessFailed(
                "Simulated CLI process failure with non-zero exit code".to_string(),
            ),
        ),
        (
            CliFault::CorruptedOutput,
            CliError::CorruptedOutput("Corrupted or malformed CLI stream output".to_string()),
        ),
        (
            CliFault::CliNotFound,
            CliError::CliNotFound("Simulated CLI binary not found in PATH".to_string()),
        ),
    ] {
        assert_eq!(
            agent_engine::cli::simulate_fault(&fault),
            Err(expected.clone()),
            "free function should map {:?} to its documented CLI error",
            fault
        );
        assert_eq!(
            fault.simulate_fault(),
            Err(expected),
            "method should map {:?} to its documented CLI error",
            fault
        );
    }
}

#[test]
fn test_cli_retryable_error_message_classification() {
    for message in [
        "Socket TIMEOUT",
        "process timed out",
        "broken pipe",
        "connection reset by peer",
        "connection closed unexpectedly",
        "resource temporarily unavailable",
        "OS error 32",
        "OS error 11",
        "OS error 104",
        "interrupted system call",
        "I/O error while reading",
        "io error while writing",
        "transient CLI failure",
    ] {
        assert!(
            is_retryable_cli_error_message(message),
            "expected retryable classification for {message:?}"
        );
    }

    for message in [
        "",
        "invalid arguments",
        "permission denied",
        "malformed response",
    ] {
        assert!(
            !is_retryable_cli_error_message(message),
            "expected non-retryable classification for {message:?}"
        );
    }
}

#[test]
fn test_cli_retryable_engine_error_classification() {
    assert!(agent_engine::cli::is_retryable_engine_error(
        &EngineError::Timeout
    ));
    assert!(agent_engine::cli::is_retryable_engine_error(
        &EngineError::ProcessFailed("broken pipe".to_string())
    ));
    assert!(!agent_engine::cli::is_retryable_engine_error(
        &EngineError::ProcessFailed("invalid arguments".to_string())
    ));
    assert!(!agent_engine::cli::is_retryable_engine_error(
        &EngineError::CliNotFound("missing binary".to_string())
    ));
    assert!(!agent_engine::cli::is_retryable_engine_error(
        &EngineError::ApiRequestFailed("timeout".to_string())
    ));
    assert!(!agent_engine::cli::is_retryable_engine_error(
        &EngineError::ProcessFailed(String::new())
    ));
}

#[test]
fn test_cli_fault_policy_intermittent_and_max_injections() {
    let mut policy =
        CliFaultPolicy::intermittent(CliFault::ProcessTimeout, 2).with_max_injections(2);

    assert_eq!(policy.should_inject(), Some(CliFault::ProcessTimeout)); // invocation 1
    assert_eq!(policy.should_inject(), None); // invocation 2
    assert_eq!(policy.should_inject(), Some(CliFault::ProcessTimeout)); // invocation 3
    assert_eq!(policy.should_inject(), None); // max injections (2) reached
    assert_eq!(policy.injected_count(), 2);
    assert_eq!(policy.invocation_count(), 3);

    policy.reset();
    assert_eq!(policy.invocation_count(), 0);
    assert_eq!(policy.injected_count(), 0);
}

#[tokio::test]
async fn test_generic_cli_baseline_success() {
    let engine = GenericCliEngine::new(
        "test_echo",
        "echo",
        vec!["{prompt}".to_string()],
        HashMap::new(),
        None,
    );
    let input = create_test_input("hello-chaos-cli", 5);
    let output = engine.execute(&input).await.expect("echo should succeed");

    assert_eq!(output.exit_code, 0);
    assert!(output.stdout_content.contains("hello-chaos-cli"));
}

#[tokio::test]
async fn test_resilient_cli_engine_recovers_intermittent_process_timeout() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(2)),
        fail_error: EngineError::Timeout,
        success_output: "recovered-after-timeout".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok(), "Expected recovery after transient timeouts");
    let output = res.unwrap();
    assert_eq!(output.stdout_content, "recovered-after-timeout");
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);
    assert_eq!(resilient.rejected_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_engine_recovers_broken_pipe() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(1)),
        fail_error: EngineError::ProcessFailed("Broken pipe (os error 32)".to_string()),
        success_output: "recovered-after-broken-pipe".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok());
    assert_eq!(res.unwrap().stdout_content, "recovered-after-broken-pipe");
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
}

#[tokio::test]
async fn test_resilient_cli_engine_recovers_transient_io_failure() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(2)),
        fail_error: EngineError::ProcessFailed(
            "Transient I/O error: Resource temporarily unavailable".to_string(),
        ),
        success_output: "recovered-after-transient-io".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);
}

#[tokio::test]
async fn test_resilient_cli_engine_chaos_injection_recovery() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(0)),
        fail_error: EngineError::Timeout,
        success_output: "injected-chaos-recovered".to_string(),
    };

    // Policy injects 2 transient faults: ProcessTimeout and BrokenPipe
    let policy = CliFaultPolicy::always(CliFault::ProcessTimeout).with_max_injections(2);
    let resilient = ResilientCliEngine::new(mock, policy, 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.resolved_count(), 1);
}

#[tokio::test]
async fn test_resilient_cli_engine_fails_when_exceeding_max_retries() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(5)),
        fail_error: EngineError::Timeout,
        success_output: "should-not-reach".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 2);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_err());
    assert_eq!(resilient.recovered_count(), 2);
    assert_eq!(resilient.rejected_count(), 1);
    assert_eq!(resilient.resolved_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_engine_corrupted_output_not_retried() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(0)),
        fail_error: EngineError::Timeout,
        success_output: "should-not-reach".to_string(),
    };

    // Corrupted output is non-transient, reject immediately
    let policy = CliFaultPolicy::always(CliFault::CorruptedOutput);
    let resilient = ResilientCliEngine::new(mock, policy, 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_err());
    assert_eq!(resilient.rejected_count(), 1);
    assert_eq!(resilient.recovered_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_engine_cli_not_found_not_retried() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(0)),
        fail_error: EngineError::Timeout,
        success_output: "should-not-reach".to_string(),
    };

    // Missing binary should fail fast without retry
    let policy = CliFaultPolicy::always(CliFault::CliNotFound);
    let resilient = ResilientCliEngine::new(mock, policy, 3);
    let input = create_test_input("ping", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_err());
    assert!(matches!(res.unwrap_err(), EngineError::CliNotFound(_)));
    assert_eq!(resilient.rejected_count(), 1);
    assert_eq!(resilient.recovered_count(), 0);
}

#[tokio::test]
async fn test_resilient_cli_audit_ledger_recorded_flow() {
    let mock = MockFailingCliEngine {
        failures_remaining: Arc::new(AtomicUsize::new(1)),
        fail_error: EngineError::ProcessFailed("Broken pipe".to_string()),
        success_output: "ledger-verified-output".to_string(),
    };

    let resilient = ResilientCliEngine::new(mock, CliFaultPolicy::none(), 3);
    let mut ledger = AuditLedger::new();
    let input = create_test_input("audit-test", 5);

    let res = resilient.execute_and_record(&mut ledger, &input).await;
    assert!(res.is_ok());
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);

    // Audit ledger should have recorded 1 retry attempt + 1 final success
    assert_eq!(ledger.len(), 2);
    let entries = ledger.entries();
    assert_eq!(entries[0].event_type, AuditEventType::CommandAllowed);
    assert!(entries[0].details.contains("retrying attempt 1/3"));
    assert_eq!(entries[1].event_type, AuditEventType::CommandAllowed);
    assert!(entries[1].details.contains("CLI execution succeeded"));
}

#[tokio::test]
async fn test_resilient_cli_with_generic_engine_live_echo() {
    let generic = GenericCliEngine::new(
        "live_echo",
        "echo",
        vec!["{prompt}".to_string()],
        HashMap::new(),
        None,
    );

    // Inject 1 fault via policy, resilient wrapper should retry and succeed
    let policy = CliFaultPolicy::always(CliFault::ProcessTimeout).with_max_injections(1);
    let resilient = generic.with_chaos(policy, 3);

    let input = create_test_input("live-echo-resilience-test", 5);
    let res = resilient.execute(&input).await;

    assert!(res.is_ok());
    let output = res.unwrap();
    assert!(output.stdout_content.contains("live-echo-resilience-test"));
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.resolved_count(), 1);
}

#[tokio::test]
async fn test_topic_1823d4_bec5af_cli_benchmark_and_resilience() {
    println!(
        "[Reproduction & Benchmark Test] Executing TOPIC-20261105-1823d4 / bec5af CLI Benchmark..."
    );

    // 1. Verify LOC compliance across all modular CLI components (< 350 lines P90)
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cli_dir = manifest_dir.join("src/cli");
    assert!(
        cli_dir.is_dir(),
        "src/cli must exist as a modular directory"
    );

    let mut checked_files = 0;
    for entry in std::fs::read_dir(&cli_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let content = std::fs::read_to_string(&path).unwrap();
            let lines = content.lines().count();
            assert!(
                lines <= 350,
                "CLI submodule {} has {} lines, which exceeds P90 limit (350 lines)",
                path.file_name().unwrap().to_str().unwrap(),
                lines
            );
            checked_files += 1;
        }
    }
    assert!(
        checked_files >= 4,
        "Expected at least 4 modular CLI source files, found {}",
        checked_files
    );

    // 2. High-load benchmark: 10,000 requests measuring P50/P90/P99 latency, throughput, and proving bounded memory allocation
    let inner_mock = BenchmarkMockCliEngine;
    let resilient =
        ResilientCliEngine::new(inner_mock, CliFaultPolicy::none(), 3).with_max_capacity(2_000);

    let stats: CliBenchmarkStats = CliBenchmarkHarness::run_benchmark(&resilient, 10_000).await;
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_resolved, 9_000);
    assert_eq!(stats.rejected_count, 1_000);
    assert!(
        stats.throughput_ops_per_sec > 500.0,
        "Throughput should exceed 500 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0, "P50 latency must be tracked");
    assert!(stats.p90_latency_us > 0.0, "P90 latency must be tracked");
    assert!(
        stats.p99_latency_us < 20_000.0,
        "P99 latency must be under 20ms, got {} us",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // 3. Verify bounded memory allocation (no unbounded leak / FIFO eviction)
    let mem_before = resilient.memory_usage_bytes();
    // Run another 5,000 requests to trigger FIFO eviction in bounded capacity
    let _ = CliBenchmarkHarness::run_benchmark(&resilient, 5_000).await;
    let mem_after = resilient.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 4. Batch execution optimization test
    let batch_inputs = vec![
        create_test_input("valid_cmd_1", 5),
        create_test_input("valid_cmd_2", 5),
        create_test_input("invalid_batch_cmd", 5),
        create_test_input("valid_cmd_3", 5),
    ];
    let batch_res = resilient.execute_batch(&batch_inputs).await;
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(matches!(batch_res[2], Err(EngineError::ProcessFailed(_))));
    assert!(batch_res[3].is_ok());

    // 5. Concurrent benchmark test (10,000 requests across 4 workers, measuring lock contention)
    let concurrent_stats = CliBenchmarkHarness::run_concurrent_benchmark(10_000, 4).await;
    assert_eq!(concurrent_stats.total_requests, 10_000);
    assert_eq!(concurrent_stats.successful_resolved, 9_000);
    assert_eq!(concurrent_stats.rejected_count, 1_000);
    assert!(
        concurrent_stats.throughput_ops_per_sec > 500.0,
        "Concurrent throughput should exceed 500 ops/sec, got {}",
        concurrent_stats.throughput_ops_per_sec
    );
    assert!(
        concurrent_stats.p50_latency_us > 0.0,
        "Concurrent P50 latency should be measured"
    );
    assert!(
        concurrent_stats.p90_latency_us > 0.0,
        "Concurrent P90 latency should be measured"
    );
    assert!(
        concurrent_stats.p99_latency_us < 20_000.0,
        "Concurrent P99 latency should be well under 20ms, got {} us",
        concurrent_stats.p99_latency_us
    );
    assert!(
        concurrent_stats.memory_allocated_bytes > 0,
        "Concurrent memory usage should be tracked"
    );

    println!("[Reproduction & Benchmark Test] TOPIC-20261105-1823d4 / bec5af passed successfully!");
}
