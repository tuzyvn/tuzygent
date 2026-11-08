use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use agent_memory::{
    simulate_context_fault, ContextBuilder, ContextBuilderBenchmarkHarness,
    ContextBuilderBenchmarkStats, ContextBuilderError, ContextBuilderFault,
    ContextBuilderFaultPolicy, ResilientContextBuilder,
};

/// TDD test case for TOPIC-20261105-83ce28 & TOPIC-20261105-d71323:
/// Chaos Engineering and Fault Injection resilience in agent-memory::context_builder.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-d71323.
#[test]
fn test_topic_83ce28_d71323_context_builder_chaos_resilience() {
    let builder = ContextBuilder::new("Implement distributed fault-tolerant ledger")
        .with_lessons(vec![
            "Always verify ledger cryptographic hash chain".to_string()
        ])
        .with_recent_logs(vec!["Log: Phase 1 consensus verified".to_string()])
        .add_subtask_step("ST-01".to_string(), "Scaffolded module".to_string())
        .with_max_total_chars(8000);

    // 1. Intermittent socket timeout recovery
    let policy = ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::SocketTimeout, 2);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);
    let res = resilient.build();
    assert!(
        res.is_ok(),
        "ResilientContextBuilder should recover from intermittent socket timeout: {:?}",
        res.err()
    );
    let (sys, work) = res.unwrap();
    assert!(sys.contains("PRIMARY GOAL"));
    assert!(sys.contains("Implement distributed fault-tolerant ledger"));
    assert!(work.contains("ST-01"));
    assert_eq!(resilient.built_count(), 1);
    assert!(resilient.recovered_count() > 0);
}

#[test]
fn test_context_builder_intermittent_connection_reset_recovery() {
    let builder = ContextBuilder::new("Autonomous remediation goal")
        .with_recent_logs(vec!["Step A done".to_string()]);

    let policy = ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::ConnectionReset, 2);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(
        res.is_ok(),
        "Must recover from intermittent connection reset"
    );
    assert_eq!(resilient.built_count(), 1);
    assert!(resilient.recovered_count() > 0);
}

#[test]
fn test_context_builder_transient_io_recovery() {
    let builder = ContextBuilder::new("Process telemetry data")
        .with_lessons(vec!["Handle disk buffers cleanly".to_string()]);

    let policy = ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::TransientIo, 2);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(res.is_ok(), "Must recover from transient IO failure");
    assert_eq!(resilient.built_count(), 1);
    assert!(resilient.recovered_count() > 0);
}

#[test]
fn test_context_builder_mut_updates_builder_used_by_build() {
    let mut resilient = ResilientContextBuilder::new(
        ContextBuilder::new("Original goal"),
        ContextBuilderFaultPolicy::none(),
        3,
    );

    *resilient.builder_mut() =
        ContextBuilder::new("Updated goal").with_recent_logs(vec!["Updated activity".to_string()]);

    let (system_context, working_context) = resilient.build().unwrap();
    assert!(system_context.contains("Updated goal"));
    assert!(!system_context.contains("Original goal"));
    assert!(working_context.contains("Updated activity"));
    assert_eq!(resilient.built_count(), 1);
}

#[test]
fn test_context_builder_policy_mut_changes_fault_behavior() {
    let mut resilient = ResilientContextBuilder::new(
        ContextBuilder::new("Policy mutation goal"),
        ContextBuilderFaultPolicy::none(),
        2,
    );
    *resilient.policy_mut() =
        ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::SocketTimeout, 2);

    assert!(resilient.build().is_ok());
    assert_eq!(resilient.policy().injected_count(), 1);
    assert_eq!(resilient.recovered_count(), 1);
    assert_eq!(resilient.built_count(), 1);
}

#[test]
fn test_context_builder_reset_counts_clears_all_counters() {
    let mut resilient = ResilientContextBuilder::new(
        ContextBuilder::new("Count reset goal"),
        ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::SocketTimeout, 2),
        1,
    );

    assert!(resilient.build().is_ok());
    assert_eq!(resilient.built_count(), 1);
    assert_eq!(resilient.recovered_count(), 1);

    *resilient.policy_mut() =
        ContextBuilderFaultPolicy::always(ContextBuilderFault::CorruptedContext);
    assert!(matches!(
        resilient.build(),
        Err(ContextBuilderError::CorruptedContext(_))
    ));
    assert_eq!(resilient.rejected_count(), 1);

    resilient.reset_counts();

    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.recovered_count(), 0);
    assert_eq!(resilient.rejected_count(), 0);
}

#[test]
fn test_context_builder_persistent_timeout_exceeds_retries() {
    let builder = ContextBuilder::new("Mission impossible");
    let policy = ContextBuilderFaultPolicy::always(ContextBuilderFault::SocketTimeout);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 2);

    let res = resilient.build();
    assert!(matches!(res, Err(ContextBuilderError::SocketTimeout)));
    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.recovered_count(), 2);
}

#[test]
fn test_context_builder_persistent_connection_reset_exceeds_retries() {
    let builder = ContextBuilder::new("Persistent failure goal");
    let policy = ContextBuilderFaultPolicy::always(ContextBuilderFault::ConnectionReset);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 2);

    let res = resilient.build();
    assert!(matches!(res, Err(ContextBuilderError::ConnectionReset)));
    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.recovered_count(), 2);
}

#[test]
fn test_context_builder_corrupted_context_rejection() {
    let builder = ContextBuilder::new("Goal with corrupt incoming fragment");
    let policy = ContextBuilderFaultPolicy::always(ContextBuilderFault::CorruptedContext);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(matches!(res, Err(ContextBuilderError::CorruptedContext(_))));
    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.rejected_count(), 1);
}

#[test]
fn test_context_builder_budget_exceeded_rejection() {
    let builder = ContextBuilder::new("Massive context goal").with_max_total_chars(1000);
    let policy = ContextBuilderFaultPolicy::always(ContextBuilderFault::BudgetExceeded);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(matches!(
        res,
        Err(ContextBuilderError::BudgetExceeded {
            required: _,
            max_budget: _
        })
    ));
    assert_eq!(resilient.built_count(), 0);
    assert_eq!(resilient.rejected_count(), 1);
}

#[test]
fn test_context_builder_empty_goal_validation_failure() {
    let builder = ContextBuilder::new("   ");
    let policy = ContextBuilderFaultPolicy::none();
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build();
    assert!(matches!(res, Err(ContextBuilderError::EmptyGoal)));
}

#[test]
fn test_context_builder_audit_ledger_recording() {
    let mut ledger = AuditLedger::new();
    let builder =
        ContextBuilder::new("Audit tracked goal").with_recent_logs(vec!["Activity 1".to_string()]);

    let policy = ContextBuilderFaultPolicy::none();
    let mut resilient = ResilientContextBuilder::new(builder, policy, 3);

    let res = resilient.build_and_record(&mut ledger);
    assert!(res.is_ok());

    let entries = ledger.entries();
    assert!(!entries.is_empty());
    assert_eq!(entries[0].event_type, AuditEventType::CommandAllowed);
    assert_eq!(entries[0].severity, ActionSeverity::Routine);
    assert!(entries[0].details.contains("Audit tracked goal"));

    // Now test recording a blocked event on validation failure
    let invalid_builder = ContextBuilder::new("");
    let mut resilient_invalid =
        ResilientContextBuilder::new(invalid_builder, ContextBuilderFaultPolicy::none(), 3);
    let err_res = resilient_invalid.build_and_record(&mut ledger);
    assert!(matches!(err_res, Err(ContextBuilderError::EmptyGoal)));

    let updated_entries = ledger.entries();
    assert!(updated_entries.len() >= 2);
    let last = &updated_entries[updated_entries.len() - 1];
    assert_eq!(last.event_type, AuditEventType::CommandBlocked);
    assert_eq!(last.severity, ActionSeverity::Destructive);
}

#[test]
fn test_simulate_context_fault_direct() {
    assert_eq!(
        simulate_context_fault(&ContextBuilderFault::SocketTimeout),
        Err(ContextBuilderError::SocketTimeout)
    );
    assert_eq!(
        simulate_context_fault(&ContextBuilderFault::ConnectionReset),
        Err(ContextBuilderError::ConnectionReset)
    );
    assert!(matches!(
        simulate_context_fault(&ContextBuilderFault::TransientIo),
        Err(ContextBuilderError::TransientIo(_))
    ));
    assert!(matches!(
        simulate_context_fault(&ContextBuilderFault::CorruptedContext),
        Err(ContextBuilderError::CorruptedContext(_))
    ));
    assert!(matches!(
        simulate_context_fault(&ContextBuilderFault::BudgetExceeded),
        Err(ContextBuilderError::BudgetExceeded { .. })
    ));
}

#[test]
fn test_high_volume_context_builder_resilience() {
    let builder = ContextBuilder::new("High volume test goal")
        .with_lessons(vec!["Continuous resilience verification".to_string()]);

    let policy = ContextBuilderFaultPolicy::intermittent(ContextBuilderFault::SocketTimeout, 3)
        .with_max_injections(10);
    let mut resilient = ResilientContextBuilder::new(builder, policy, 5);

    for _ in 0..50 {
        let res = resilient.build();
        assert!(res.is_ok());
    }

    assert_eq!(resilient.built_count(), 50);
    assert_eq!(resilient.recovered_count(), 10);
}

#[test]
fn test_topic_d85ca9_9c90cb_context_builder_benchmark_and_resilience() {
    println!("[Reproduction & Benchmark Test] Executing TOPIC-20261105-d85ca9 / 9c90cb ContextBuilder Benchmark...");

    // 1. Single-threaded benchmark (10,000 requests)
    let mut resilient = ResilientContextBuilder::new(
        ContextBuilder::new("Base Benchmark Goal"),
        ContextBuilderFaultPolicy::none(),
        3,
    )
    .with_max_capacity(2_000);

    let stats: ContextBuilderBenchmarkStats =
        ContextBuilderBenchmarkHarness::run_benchmark(&mut resilient, 10_000);
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_built, 9_000);
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

    // 2. Verify bounded memory allocation (no unbounded leak)
    let mem_before = resilient.memory_usage_bytes();
    // Run another 5,000 requests to trigger FIFO eviction in bounded capacity
    let _ = ContextBuilderBenchmarkHarness::run_benchmark(&mut resilient, 5_000);
    let mem_after = resilient.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 3. Batch resolution optimization test
    let batch_builders = vec![
        ContextBuilderBenchmarkHarness::make_benchmark_builder(1, false),
        ContextBuilderBenchmarkHarness::make_benchmark_builder(2, false),
        ContextBuilderBenchmarkHarness::make_benchmark_builder(3, true), // Invalid: empty goal
        ContextBuilderBenchmarkHarness::make_benchmark_builder(4, false),
    ];
    let batch_res = resilient.build_batch(&batch_builders);
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(matches!(batch_res[2], Err(ContextBuilderError::EmptyGoal)));
    assert!(batch_res[3].is_ok());

    // 4. Concurrent benchmark test (10,000 requests across 4 threads)
    let concurrent_stats = ContextBuilderBenchmarkHarness::run_concurrent_benchmark(10_000, 4);
    assert_eq!(concurrent_stats.total_requests, 10_000);
    assert_eq!(concurrent_stats.successful_built, 9_000);
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
    println!("[Reproduction & Benchmark Test] TOPIC-20261105-d85ca9 / 9c90cb passed successfully!");
}
