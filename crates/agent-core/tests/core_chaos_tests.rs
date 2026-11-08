use agent_core::benchmark::CoreBenchmarkHarness;
use agent_core::fault::{CircuitState, CoreExecutionError, CoreFaultKind, CoreFaultPolicy};
use agent_core::resilience::{CoreExecutionResult, ResilientCoreAgent, ResilientCoreAgentBuilder};
use agent_core::safe::{SafeGuard, SafeUnwrapExt};
use agent_core::sharded::ShardedCoreAgentRunner;
use std::sync::Arc;

#[test]
fn test_core_execution_result_serialization_compatibility() {
    let result = CoreExecutionResult {
        operation_id: "op-123".to_string(),
        name: "summarize".to_string(),
        output: "summary ready".to_string(),
        duration_us: 42,
        retries_used: 2,
        recovered_from_fault: true,
    };

    let serialized = serde_json::to_value(&result).expect("serialize execution result");
    assert_eq!(
        serialized,
        serde_json::json!({
            "operation_id": "op-123",
            "name": "summarize",
            "output": "summary ready",
            "duration_us": 42,
            "retries_used": 2,
            "recovered_from_fault": true,
        })
    );

    let deserialized: CoreExecutionResult =
        serde_json::from_value(serialized).expect("deserialize execution result");
    assert_eq!(deserialized, result);
}

#[test]
fn test_safe_unwrap_eliminates_panics_on_option_and_result() {
    // 1. None Option safety
    let none_val: Option<String> = None;
    assert_eq!(
        none_val.clone().safe_unwrap_or("fallback".to_string()),
        "fallback"
    );
    assert_eq!(
        none_val
            .clone()
            .safe_unwrap_or_else(|| "computed".to_string()),
        "computed"
    );
    assert_eq!(none_val.clone().safe_unwrap_or_default(), "");

    let resolve_res = none_val.safe_resolve("agent_core_lookup");
    assert!(resolve_res.is_err());
    assert!(matches!(
        resolve_res.unwrap_err(),
        CoreExecutionError::UnwrapSafetyViolation(_)
    ));

    // 2. Err Result safety
    let err_val: Result<i32, String> = Err("network failure".to_string());
    assert_eq!(err_val.clone().safe_unwrap_or(42), 42);
    assert_eq!(err_val.clone().safe_unwrap_or_else(|| 100), 100);
    assert_eq!(err_val.clone().safe_unwrap_or_default(), 0);

    let resolve_res = err_val.safe_resolve("task_result");
    assert!(resolve_res.is_err());
    assert!(matches!(
        resolve_res.unwrap_err(),
        CoreExecutionError::UnwrapSafetyViolation(_)
    ));

    // 3. Some / Ok values unpack cleanly
    let some_val: Option<String> = Some("active-agent".to_string());
    assert_eq!(
        some_val.safe_unwrap_or("fallback".to_string()),
        "active-agent"
    );

    let ok_val: Result<i32, &str> = Ok(99);
    assert_eq!(ok_val.safe_unwrap_or(0), 99);
}

#[test]
fn test_safe_guard_catches_panics() {
    let safe_outcome = SafeGuard::catch_unwind_safe(
        || {
            panic!("simulated unexpected panic in worker");
        },
        "recovered_cleanly",
    );
    assert_eq!(safe_outcome, "recovered_cleanly");

    let result_outcome = SafeGuard::catch_unwind_result(|| {
        panic!("critical internal thread failure");
    });
    assert!(result_outcome.is_err());
    assert!(matches!(
        result_outcome.unwrap_err(),
        CoreExecutionError::Panic(_)
    ));
}

#[test]
fn test_resilient_core_agent_fault_injection_and_circuit_breaker() {
    let policy = Arc::new(CoreFaultPolicy::new(3));
    let agent = ResilientCoreAgentBuilder::new()
        .max_capacity(100)
        .fault_policy(Arc::clone(&policy))
        .build();

    // Normal execution
    let res = agent.execute_operation("check_status", "param=1");
    assert!(res.is_ok());
    assert_eq!(policy.circuit_state(), CircuitState::Closed);

    // Inject transient failure
    policy.set_fault(CoreFaultKind::SimulateTransientFailure);
    let fail1 = agent.execute_operation("check_status", "param=2");
    assert!(matches!(
        fail1.unwrap_err(),
        CoreExecutionError::TransientEngineFailure(_)
    ));
    assert_eq!(policy.circuit_state(), CircuitState::HalfOpen);

    let _ = agent.execute_operation("check_status", "param=3");
    let _ = agent.execute_operation("check_status", "param=4");
    assert_eq!(policy.circuit_state(), CircuitState::Open);

    // When Circuit is open, requests are fast-rejected
    let open_res = agent.execute_operation("check_status", "param=5");
    assert!(matches!(
        open_res.unwrap_err(),
        CoreExecutionError::CircuitOpen(_)
    ));

    // Clear fault & recover
    policy.clear_fault();
    policy.record_success();
    assert_eq!(policy.circuit_state(), CircuitState::Closed);
    let rec_res = agent.execute_operation("check_status", "param=6");
    assert!(rec_res.is_ok());
}

#[test]
fn test_fault_policy_injected_count_tracks_injected_faults() {
    let policy = CoreFaultPolicy::new(3);

    assert_eq!(policy.injected_count(), 0);
    policy.set_fault(CoreFaultKind::SimulateTransientFailure);
    assert!(matches!(
        policy.check_and_inject(),
        Err(CoreExecutionError::TransientEngineFailure(_))
    ));
    assert_eq!(policy.injected_count(), 1);
}

#[test]
fn test_fault_policy_record_failure_updates_circuit_state() {
    let policy = CoreFaultPolicy::new(2);

    policy.record_failure();
    assert_eq!(policy.circuit_state(), CircuitState::HalfOpen);
    policy.record_failure();
    assert_eq!(policy.circuit_state(), CircuitState::Open);
}

#[test]
fn test_fault_policy_get_fault_returns_configured_fault() {
    let policy = CoreFaultPolicy::new(3);

    assert_eq!(policy.get_fault(), CoreFaultKind::None);
    policy.set_fault(CoreFaultKind::SimulateTimeout);
    assert_eq!(policy.get_fault(), CoreFaultKind::SimulateTimeout);
    policy.clear_fault();
    assert_eq!(policy.get_fault(), CoreFaultKind::None);
}

#[test]
fn test_resilient_core_agent_retry_with_transient_failure() {
    let agent = ResilientCoreAgent::new();
    let res = agent.execute_with_retry("retry_op", "payload", 3);
    assert!(res.is_ok());
    let unwrapped = res.unwrap();
    assert_eq!(unwrapped.retries_used, 0);
    assert!(!unwrapped.recovered_from_fault);
}

#[test]
fn test_resilient_core_agent_bounded_memory_capacity_and_eviction() {
    let agent = ResilientCoreAgent::new().with_max_capacity(50);
    for i in 0..100 {
        let _ = agent.execute_operation(&format!("op_{}", i), "data");
    }

    assert_eq!(agent.record_count(), 50);
    assert!(agent.memory_usage_bytes() > 0);

    agent.clear();
    assert_eq!(agent.record_count(), 0);
    assert_eq!(agent.memory_usage_bytes(), 0);
}

#[test]
fn test_sharded_core_agent_runner_distribution() {
    let runner = ShardedCoreAgentRunner::new(100);
    for i in 0..300 {
        let res = runner.execute(&format!("agent_subtask_{}", i), "data");
        assert!(res.is_ok());
    }

    assert_eq!(runner.total_record_count(), 300);
    assert!(runner.total_memory_usage() > 0);

    let batch = vec![("batch_1", "p1"), ("batch_2", "p2"), ("batch_3", "p3")];
    let batch_res = runner.execute_batch(batch);
    assert_eq!(batch_res.len(), 3);
    assert!(batch_res.iter().all(|r| r.is_ok()));

    runner.clear_all();
    assert_eq!(runner.total_record_count(), 0);
}

#[test]
fn test_core_benchmark_harness_stats_and_percentiles() {
    let runner = ResilientCoreAgent::new().with_max_capacity(200);
    let stats = CoreBenchmarkHarness::run_single_benchmark(&runner, 100);

    assert_eq!(stats.total_requests, 100);
    assert_eq!(stats.successful_operations, 100);
    assert_eq!(stats.rejected_count, 0);
    assert!(stats.throughput_ops_per_sec > 1000.0);
    assert!(stats.p50_latency_us > 0.0);
    assert!(stats.p99_latency_us >= stats.p50_latency_us);
    assert!(stats.p99_latency_us < 50_000.0, "P99 must be under 50ms");
}

#[test]
fn test_benchmark_percentile_empty_boundaries_and_interpolation() {
    assert_eq!(CoreBenchmarkHarness::percentile(&[], 0.50), 0.0);

    let sorted = [10.0, 20.0, 40.0, 80.0];
    assert_eq!(CoreBenchmarkHarness::percentile(&sorted, 0.0), 10.0);
    assert_eq!(CoreBenchmarkHarness::percentile(&sorted, 1.0), 80.0);
    assert_eq!(CoreBenchmarkHarness::percentile(&sorted, 0.5), 30.0);
}

#[test]
fn test_sharded_benchmark_reports_successes_rejections_and_empty_run() {
    let policy = Arc::new(CoreFaultPolicy::new(100));
    let runner = ShardedCoreAgentRunner::with_shared_fault_policy(Arc::clone(&policy), 20);

    let stats = CoreBenchmarkHarness::run_sharded_benchmark(&runner, 4);
    assert_eq!(stats.total_requests, 4);
    assert_eq!(stats.successful_operations, 4);
    assert_eq!(stats.rejected_count, 0);
    assert!(stats.p50_latency_us >= 0.0);
    assert!(stats.p90_latency_us >= stats.p50_latency_us);
    assert!(stats.p99_latency_us >= stats.p90_latency_us);
    assert!(stats.max_latency_us >= stats.p99_latency_us);
    assert!(stats.total_duration.as_nanos() > 0);
    assert!(stats.throughput_ops_per_sec > 0.0);
    assert!(stats.memory_allocated_bytes > 0);

    policy.set_fault(CoreFaultKind::SimulateTimeout);
    let rejected = CoreBenchmarkHarness::run_sharded_benchmark(&runner, 3);
    assert_eq!(rejected.total_requests, 3);
    assert_eq!(rejected.successful_operations, 0);
    assert_eq!(rejected.rejected_count, 3);
    assert_eq!(rejected.p50_latency_us, 0.0);
    assert_eq!(rejected.p99_latency_us, 0.0);
    assert_eq!(rejected.max_latency_us, 0.0);
    assert!(rejected.total_duration.as_nanos() > 0);
    assert!(rejected.throughput_ops_per_sec > 0.0);
    assert!(rejected.memory_allocated_bytes > 0);

    let empty = CoreBenchmarkHarness::run_sharded_benchmark(&runner, 0);
    assert_eq!(empty.total_requests, 0);
    assert_eq!(empty.successful_operations, 0);
    assert_eq!(empty.rejected_count, 0);
    assert_eq!(empty.p50_latency_us, 0.0);
    assert_eq!(empty.p90_latency_us, 0.0);
    assert_eq!(empty.p99_latency_us, 0.0);
    assert_eq!(empty.max_latency_us, 0.0);
    assert_eq!(empty.throughput_ops_per_sec, 0.0);
    assert_eq!(empty.memory_allocated_bytes, rejected.memory_allocated_bytes);
}

#[test]
fn test_core_benchmark_compare_lock_contention() {
    let (single, sharded, ratio) = CoreBenchmarkHarness::compare_lock_contention(200);
    assert_eq!(single.total_requests, 200);
    assert_eq!(sharded.total_requests, 200);
    assert!(ratio > 0.0);
    assert!(sharded.p99_latency_us < 50_000.0);
}

#[test]
fn test_high_concurrency_10k_operations() {
    let runner = Arc::new(ShardedCoreAgentRunner::new(1000));
    let mut handles = Vec::new();

    for t in 0..10 {
        let runner_clone = Arc::clone(&runner);
        handles.push(std::thread::spawn(move || {
            for i in 0..1000 {
                let op = format!("worker_{}_task_{}", t, i);
                let res = runner_clone.execute(&op, "concurrency_test");
                assert!(res.is_ok());
            }
        }));
    }

    for h in handles {
        assert!(h.join().is_ok());
    }

    assert_eq!(runner.total_record_count(), 10000);
}

#[test]
fn test_unambiguous_public_exports_and_prelude() {
    use agent_core::prelude::*;

    let _agent = ResilientCoreAgent::new();
    let _sharded = ShardedCoreAgentRunner::new(10);
    let _policy = CoreFaultPolicy::default();
    let opt: Option<i32> = None;
    assert_eq!(opt.safe_unwrap_or(77), 77);
}

/// TDD test case for TOPIC-20261105-a60060 in agent-core:
/// Verifies that agent-core exports clean prelude and unified namespace with zero ambiguous glob re-exports.
#[test]
fn test_topic_a60060_core_unambiguous_reexports() {
    mod strict_import_scope {
        #![deny(ambiguous_glob_reexports)]
        pub use agent_core::prelude::*;
    }

    use strict_import_scope::*;
    let _priority = Priority::Normal;
    let _task = Task::new("T1", "desc", _priority);
    let _guardrail = GuardrailPolicy::default_enterprise();
    let _agent = ResilientCoreAgent::new();
    let _runner = ShardedCoreAgentRunner::new(4);
    let _harness = CoreBenchmarkHarness;
    assert_eq!(_runner.total_record_count(), 0);
}

/// TDD test case for TOPIC-20261106-efe10f in agent-core:
/// Verifies that safe.rs has zero unwrap calls or smells, and safe error handling mechanics operate correctly.
#[test]
fn test_topic_efe10f_safe_unwrap_zero_smell_and_safe_fallback() {
    let safe_source = include_str!("../src/safe.rs");
    // Ensure 0 calls or smells of .unwrap() in safe.rs
    assert!(
        !safe_source.contains(".unwrap()"),
        "crates/agent-core/src/safe.rs must not contain any .unwrap() calls or smells"
    );

    #[derive(Debug, PartialEq, Eq)]
    struct CustomSafeVal(i32);
    impl Default for CustomSafeVal {
        fn default() -> Self {
            CustomSafeVal(999)
        }
    }

    // Test safe_unwrap_or_default on Option
    let none_opt: Option<CustomSafeVal> = None;
    assert_eq!(none_opt.safe_unwrap_or_default(), CustomSafeVal(999));

    let some_opt: Option<CustomSafeVal> = Some(CustomSafeVal(123));
    assert_eq!(some_opt.safe_unwrap_or_default(), CustomSafeVal(123));

    // Test safe_unwrap_or_default on Result
    let err_res: Result<CustomSafeVal, &str> = Err("transient failure");
    assert_eq!(err_res.safe_unwrap_or_default(), CustomSafeVal(999));

    let ok_res: Result<CustomSafeVal, &str> = Ok(CustomSafeVal(456));
    assert_eq!(ok_res.safe_unwrap_or_default(), CustomSafeVal(456));

    // Test safe_resolve error preservation
    let err_context: Result<i32, &str> = Err("disk full");
    let resolved = err_context.safe_resolve("write_checkpoint");
    assert!(resolved.is_err());
    let err_msg = format!("{:?}", resolved.unwrap_err());
    assert!(err_msg.contains("disk full"));
    assert!(err_msg.contains("write_checkpoint"));
}
