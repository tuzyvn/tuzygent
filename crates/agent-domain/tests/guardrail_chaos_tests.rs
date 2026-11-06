use agent_domain::{
    ActionSeverity, AuditLedger, GuardrailBenchmarkHarness, GuardrailBenchmarkStats,
    GuardrailError, GuardrailFault, GuardrailFaultPolicy, GuardrailPolicy,
    ResilientGuardrailEvaluator,
};

#[test]
fn test_guardrail_validation_and_rule_integrity() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::none();
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    // Normal safe command must pass
    assert!(evaluator.check_command_safety("cargo test --all").is_ok());
    assert_eq!(evaluator.evaluated_count(), 1);

    // Red line violation must be blocked
    let res = evaluator.check_command_safety("rm -rf /var/data");
    assert!(matches!(res, Err(GuardrailError::Blocked(_))));
    assert_eq!(evaluator.blocked_count(), 1);
}

#[test]
fn test_guardrail_intermittent_socket_timeout_recovery() {
    let policy = GuardrailPolicy::default_enterprise();
    // Intermittent socket timeout every 2 invocations, max retries 3
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::SocketTimeout, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("git status");
    assert!(res.is_ok());
    assert!(evaluator.recovered_count() > 0);
    assert_eq!(evaluator.evaluated_count(), 1);
}

#[test]
fn test_guardrail_max_retries_exceeded_on_persistent_timeout() {
    let policy = GuardrailPolicy::default_enterprise();
    // Always inject SocketTimeout, max retries 2
    let fault_policy = GuardrailFaultPolicy::always(GuardrailFault::SocketTimeout);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 2);

    let res = evaluator.check_command_safety("cargo check");
    assert_eq!(res, Err(GuardrailError::SocketTimeout));
    assert_eq!(evaluator.evaluated_count(), 0);
}

#[test]
fn test_guardrail_connection_reset_recovery() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::ConnectionReset, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("echo 'safe command'");
    assert!(res.is_ok());
    assert!(evaluator.recovered_count() > 0);
}

#[test]
fn test_guardrail_transient_io_failure_recovery() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::TransientIo, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("ls -la");
    assert!(res.is_ok());
    assert!(evaluator.recovered_count() > 0);
}

#[test]
fn test_guardrail_corrupted_rule_rejection() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::always(GuardrailFault::CorruptedRule);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("cargo test");
    assert!(matches!(res, Err(GuardrailError::CorruptedPolicy(_))));
    assert_eq!(evaluator.evaluated_count(), 0);
}

#[test]
fn test_guardrail_service_unavailable_recovery() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::ServiceUnavailable, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);

    let res = evaluator.check_command_safety("python3 script.py");
    assert!(res.is_ok());
    assert!(evaluator.recovered_count() > 0);
}

#[test]
fn test_guardrail_audit_ledger_chaos_integration() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::SocketTimeout, 2);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3);
    let mut ledger = AuditLedger::new();

    // 1. Safe command under intermittent chaos
    let res1 = evaluator.evaluate_and_record(&mut ledger, "cargo build --release");
    assert!(res1.is_ok());

    // 2. Blocked command under intermittent chaos
    let res2 = evaluator.evaluate_and_record(&mut ledger, "DROP TABLE orders");
    assert!(matches!(res2, Err(GuardrailError::Blocked(_))));

    // 3. Verify audit ledger chain integrity
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.entries().len(), 2);
}

#[test]
fn test_guardrail_audit_ledger_persistent_failure_records_exception() {
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::always(GuardrailFault::SocketTimeout);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 2);
    let mut ledger = AuditLedger::new();

    let res = evaluator.evaluate_and_record(&mut ledger, "cargo run");
    assert_eq!(res, Err(GuardrailError::SocketTimeout));

    // Ledger should record the exception event
    assert!(ledger.verify_integrity().is_ok());
    assert_eq!(ledger.entries().len(), 1);
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::HighImpact);
}

#[test]
fn test_high_volume_guardrail_chaos_resilience() {
    let policy = GuardrailPolicy::default_enterprise();
    // Intermittent connection reset every 3 invocations with max retries 4
    let fault_policy = GuardrailFaultPolicy::intermittent(GuardrailFault::ConnectionReset, 3);
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 4);
    let mut ledger = AuditLedger::new();

    for i in 0..50 {
        let cmd = if i % 5 == 0 {
            "rm -rf /tmp/target"
        } else {
            "cargo test"
        };
        let _ = evaluator.evaluate_and_record(&mut ledger, cmd);
    }

    assert_eq!(ledger.entries().len(), 50);
    assert!(ledger.verify_integrity().is_ok());
    assert!(evaluator.recovered_count() > 0);
    assert_eq!(evaluator.blocked_count(), 10);
    assert_eq!(evaluator.evaluated_count(), 40);
}

// =========================================================================
// Root Cause Elimination Test for TOPIC-20261105-ddd866 & TOPIC-20261105-b4f9b6
// High Load Benchmark (10,000 requests concurrent), P50/P90/P99 Latency & Memory Resilience
// =========================================================================

#[test]
fn test_topic_ddd866_b4f9b6_guardrail_benchmark_and_resilience() {
    // 1. Verify LOC compliance across all modular guardrail components (< 350 lines P90)
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let guardrail_dir = manifest_dir.join("src/guardrail");
    assert!(
        guardrail_dir.is_dir(),
        "src/guardrail must exist as a modular directory"
    );

    let guardrail_root = manifest_dir.join("src/guardrail.rs");
    if guardrail_root.is_file() {
        let content = std::fs::read_to_string(&guardrail_root).unwrap();
        let lines = content.lines().count();
        assert!(
            lines <= 350,
            "guardrail.rs has {} lines, which exceeds P90 limit (350 lines)",
            lines
        );
    }

    for entry in std::fs::read_dir(&guardrail_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let content = std::fs::read_to_string(&path).unwrap();
            let lines = content.lines().count();
            assert!(
                lines <= 350,
                "Submodule {} has {} lines, which exceeds P90 limit (350 lines)",
                path.file_name().unwrap().to_str().unwrap(),
                lines
            );
        }
    }

    // 2. High-load benchmark: 10,000 requests measuring P50/P90/P99 latency, throughput, and proving bounded memory allocation
    let policy = GuardrailPolicy::default_enterprise();
    let fault_policy = GuardrailFaultPolicy::none();
    let mut evaluator = ResilientGuardrailEvaluator::new(policy, fault_policy, 3)
        .with_max_capacity(10_000);

    let stats: GuardrailBenchmarkStats =
        GuardrailBenchmarkHarness::run_benchmark(&mut evaluator, 10_000);
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_evaluated, 8_000);
    assert_eq!(stats.blocked_count, 2_000);
    assert!(
        stats.throughput_ops_per_sec > 1_000.0,
        "Throughput should exceed 1,000 ops/sec, got {}",
        stats.throughput_ops_per_sec
    );
    assert!(stats.p50_latency_us > 0.0, "P50 latency should be measured");
    assert!(stats.p90_latency_us > 0.0, "P90 latency should be measured");
    assert!(
        stats.p99_latency_us < 5_000.0,
        "P99 latency should be under 5000 us (5ms), got {} us",
        stats.p99_latency_us
    );
    assert!(
        stats.memory_allocated_bytes > 0,
        "Memory allocation must be tracked"
    );

    // Verify bounded memory allocation (no unbounded leak):
    let mem_before = evaluator.memory_usage_bytes();
    // Run another 5,000 requests to trigger eviction in bounded capacity
    let _ = GuardrailBenchmarkHarness::run_benchmark(&mut evaluator, 5_000);
    let mem_after = evaluator.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 1024,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 3. Batch processing optimization test
    let batch_commands = vec![
        "cargo test",
        "git status",
        "rm -rf /opt/data",
        "echo 123",
        "DROP TABLE customers",
    ];
    let batch_res = evaluator.evaluate_batch(&batch_commands);
    assert_eq!(batch_res.len(), 5);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(matches!(batch_res[2], Err(GuardrailError::Blocked(_))));
    assert!(batch_res[3].is_ok());
    assert!(matches!(batch_res[4], Err(GuardrailError::Blocked(_))));

    // 4. Concurrent benchmark test (10,000 requests across 4 threads)
    let concurrent_stats = GuardrailBenchmarkHarness::run_concurrent_benchmark(10_000, 4);
    assert_eq!(concurrent_stats.total_requests, 10_000);
    assert_eq!(concurrent_stats.successful_evaluated, 8_000);
    assert_eq!(concurrent_stats.blocked_count, 2_000);
    assert!(
        concurrent_stats.throughput_ops_per_sec > 500.0,
        "Concurrent throughput should be healthy, got {}",
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
}
