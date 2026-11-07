use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use agent_memory::{
    simulate_durable_fault, DurableBenchmarkHarness, DurableBenchmarkStats, DurableMemoryError,
    DurableMemoryFault, DurableMemoryFaultPolicy, DurableMemoryStore, ResilientDurableMemoryStore,
    StructuredLesson,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(100);

fn create_temp_dir(prefix: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("durable_chaos_test_{}_{}_{}", prefix, nanos, id));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// TDD test case for TOPIC-20261105-d1ef61 & TOPIC-20261105-c109f7:
/// Chaos Engineering and Fault Injection resilience in agent-memory::durable.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-c109f7.
#[tokio::test]
async fn test_topic_d1ef61_c109f7_durable_memory_chaos_resilience() {
    let dir = create_temp_dir("chaos_resilience");
    let store = DurableMemoryStore::new(&dir);

    // 1. Intermittent socket timeout recovery during save_lesson
    let policy = DurableMemoryFaultPolicy::intermittent(DurableMemoryFault::SocketTimeout, 2);
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 3);

    let res = resilient
        .save_lesson("Always verify crypto signatures on distributed ledger nodes")
        .await;
    assert!(
        res.is_ok(),
        "ResilientDurableMemoryStore should recover from intermittent socket timeout: {:?}",
        res.err()
    );
    assert_eq!(resilient.saved_count(), 1);
    assert!(resilient.recovered_count() > 0);

    // Verify content was actually persisted
    let loaded = resilient.load_relevant_context("crypto signatures").await;
    assert!(loaded.is_ok());
    assert!(loaded.unwrap().contains("crypto signatures"));

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_durable_intermittent_connection_reset_recovery() {
    let dir = create_temp_dir("conn_reset");
    let store = DurableMemoryStore::new(&dir);

    let policy = DurableMemoryFaultPolicy::intermittent(DurableMemoryFault::ConnectionReset, 2);
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 3);

    let lesson = StructuredLesson {
        id: "les-chaos-01".to_string(),
        timestamp: "2026-11-06T00:00:00Z".to_string(),
        task_title: "Connection Reset Recovery".to_string(),
        trigger_error: Some("ECONNRESET".to_string()),
        remediation_applied: Some("Retry with exponential backoff".to_string()),
        general_insight: "Socket resets must be retried gracefully".to_string(),
        success: true,
    };

    let res = resilient.save_structured_lesson(&lesson).await;
    assert!(
        res.is_ok(),
        "Must recover from intermittent connection reset"
    );
    assert_eq!(resilient.saved_count(), 1);
    assert!(resilient.recovered_count() > 0);

    let loaded = resilient.load_structured_lessons().await;
    assert!(loaded.is_ok());
    let lessons = loaded.unwrap();
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0].id, "les-chaos-01");

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_durable_transient_io_recovery() {
    let dir = create_temp_dir("transient_io");
    let store = DurableMemoryStore::new(&dir);

    let policy = DurableMemoryFaultPolicy::intermittent(DurableMemoryFault::TransientIo, 2);
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 3);

    let res = resilient
        .save_user_directive("Strict sandbox policy enforcement: active")
        .await;
    assert!(res.is_ok(), "Must recover from transient IO failure");
    assert_eq!(resilient.saved_count(), 1);
    assert!(resilient.recovered_count() > 0);

    let loaded = resilient.load_relevant_context("sandbox").await;
    assert!(loaded.is_ok());
    assert!(loaded
        .unwrap()
        .contains("Strict sandbox policy enforcement"));

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_durable_persistent_timeout_exceeds_retries() {
    let dir = create_temp_dir("persistent_timeout");
    let store = DurableMemoryStore::new(&dir);

    let policy = DurableMemoryFaultPolicy::always(DurableMemoryFault::SocketTimeout);
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 2);

    let res = resilient.save_lesson("Unreachable memory write").await;
    assert!(matches!(res, Err(DurableMemoryError::SocketTimeout)));
    assert_eq!(resilient.saved_count(), 0);
    assert_eq!(resilient.recovered_count(), 2);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_durable_persistent_connection_reset_exceeds_retries() {
    let dir = create_temp_dir("persistent_conn_reset");
    let store = DurableMemoryStore::new(&dir);

    let policy = DurableMemoryFaultPolicy::always(DurableMemoryFault::ConnectionReset);
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 2);

    let res = resilient.load_relevant_context("any goal").await;
    assert!(matches!(res, Err(DurableMemoryError::ConnectionReset)));
    assert_eq!(resilient.loaded_count(), 0);
    assert_eq!(resilient.recovered_count(), 2);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_durable_corrupted_state_rejection() {
    let dir = create_temp_dir("corrupt_rejection");
    let store = DurableMemoryStore::new(&dir);

    let policy = DurableMemoryFaultPolicy::always(DurableMemoryFault::CorruptedState);
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 3);

    let res = resilient.save_lesson("Should reject corrupt state").await;
    assert!(matches!(res, Err(DurableMemoryError::CorruptedState(_))));
    assert_eq!(resilient.saved_count(), 0);
    assert_eq!(resilient.rejected_count(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_durable_empty_content_validation_failure() {
    let dir = create_temp_dir("empty_content");
    let store = DurableMemoryStore::new(&dir);

    let policy = DurableMemoryFaultPolicy::none();
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 3);

    let res1 = resilient.save_lesson("   ").await;
    assert!(matches!(res1, Err(DurableMemoryError::EmptyContent)));

    let res2 = resilient.save_user_directive("").await;
    assert!(matches!(res2, Err(DurableMemoryError::EmptyContent)));
    assert_eq!(resilient.rejected_count(), 2);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_durable_lock_failure_recovery() {
    let dir = create_temp_dir("lock_failure");
    let store = DurableMemoryStore::new(&dir);

    let policy = DurableMemoryFaultPolicy::intermittent(DurableMemoryFault::LockFailure, 2);
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 3);

    let lesson = StructuredLesson {
        id: "les-lock-01".to_string(),
        timestamp: "2026-11-06T00:00:00Z".to_string(),
        task_title: "Lock contention resolution".to_string(),
        trigger_error: None,
        remediation_applied: None,
        general_insight: "Lock acquisition retries gracefully".to_string(),
        success: true,
    };

    let res = resilient.save_structured_lesson(&lesson).await;
    assert!(
        res.is_ok(),
        "Must recover from intermittent lock acquisition failure"
    );
    assert_eq!(resilient.saved_count(), 1);
    assert!(resilient.recovered_count() > 0);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_durable_audit_ledger_recording() {
    let dir = create_temp_dir("audit_ledger");
    let store = DurableMemoryStore::new(&dir);
    let mut ledger = AuditLedger::new();

    let policy = DurableMemoryFaultPolicy::none();
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 3);

    let res = resilient
        .save_lesson_and_record(&mut ledger, "Audit recorded operational lesson")
        .await;
    assert!(res.is_ok());

    let entries = ledger.entries();
    assert!(!entries.is_empty());
    assert_eq!(entries[0].event_type, AuditEventType::CommandAllowed);
    assert_eq!(entries[0].severity, ActionSeverity::Routine);
    assert!(entries[0]
        .details
        .contains("Audit recorded operational lesson"));

    // Now test recording a blocked event on validation failure
    let err_res = resilient.save_lesson_and_record(&mut ledger, "").await;
    assert!(matches!(err_res, Err(DurableMemoryError::EmptyContent)));

    let updated_entries = ledger.entries();
    assert!(updated_entries.len() >= 2);
    let last = &updated_entries[updated_entries.len() - 1];
    assert_eq!(last.event_type, AuditEventType::CommandBlocked);
    assert_eq!(last.severity, ActionSeverity::Destructive);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_save_structured_lesson_and_record_success_and_validation_failure() {
    let dir = create_temp_dir("audit_structured_lesson");
    let store = DurableMemoryStore::new(&dir);
    let mut resilient =
        ResilientDurableMemoryStore::new(store, DurableMemoryFaultPolicy::none(), 3);
    let mut ledger = AuditLedger::new();
    let lesson = StructuredLesson {
        id: "audit-lesson-01".to_string(),
        timestamp: "2026-11-07T00:00:00Z".to_string(),
        task_title: "Record structured lesson".to_string(),
        trigger_error: None,
        remediation_applied: Some("Validate the persisted result".to_string()),
        general_insight: "Structured audit writes preserve lesson fields".to_string(),
        success: true,
    };

    resilient
        .save_structured_lesson_and_record(&mut ledger, &lesson)
        .await
        .unwrap();

    let persisted = resilient.load_structured_lessons().await.unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].id, lesson.id);
    assert_eq!(persisted[0].task_title, lesson.task_title);
    assert_eq!(persisted[0].general_insight, lesson.general_insight);
    assert_eq!(resilient.saved_count(), 1);
    let allowed = &ledger.entries()[0];
    assert_eq!(allowed.event_type, AuditEventType::CommandAllowed);
    assert_eq!(allowed.severity, ActionSeverity::Routine);
    assert!(allowed.details.contains("audit-lesson-01"));

    let invalid_lesson = StructuredLesson {
        id: "   ".to_string(),
        ..lesson
    };
    let result = resilient
        .save_structured_lesson_and_record(&mut ledger, &invalid_lesson)
        .await;
    assert!(matches!(result, Err(DurableMemoryError::EmptyContent)));
    assert_eq!(resilient.rejected_count(), 1);
    let blocked = ledger.entries().last().unwrap();
    assert_eq!(blocked.event_type, AuditEventType::CommandBlocked);
    assert_eq!(blocked.severity, ActionSeverity::Destructive);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_load_relevant_context_and_record_success_and_store_failure() {
    let dir = create_temp_dir("audit_relevant_context");
    let store = DurableMemoryStore::new(&dir);
    let mut resilient =
        ResilientDurableMemoryStore::new(store, DurableMemoryFaultPolicy::none(), 3);
    let mut ledger = AuditLedger::new();
    resilient
        .save_lesson("Persisted context for the audit loading scenario")
        .await
        .unwrap();

    let context = resilient
        .load_relevant_context_and_record(&mut ledger, "audit loading scenario")
        .await
        .unwrap();
    assert!(context.contains("Persisted context for the audit loading scenario"));
    assert_eq!(resilient.loaded_count(), 1);
    let allowed = &ledger.entries()[0];
    assert_eq!(allowed.event_type, AuditEventType::CommandAllowed);
    assert_eq!(allowed.severity, ActionSeverity::Routine);
    assert!(allowed.details.contains("audit loading scenario"));

    let failing_dir = create_temp_dir("audit_relevant_context_failure");
    let failing_store = DurableMemoryStore::new(&failing_dir);
    let policy = DurableMemoryFaultPolicy::always(DurableMemoryFault::ConnectionReset);
    let mut failing_resilient = ResilientDurableMemoryStore::new(failing_store, policy, 1);
    let result = failing_resilient
        .load_relevant_context_and_record(&mut ledger, "unavailable context")
        .await;
    assert!(matches!(result, Err(DurableMemoryError::ConnectionReset)));
    let exception = ledger.entries().last().unwrap();
    assert_eq!(exception.event_type, AuditEventType::CommandException);
    assert_eq!(exception.severity, ActionSeverity::HighImpact);
    assert!(exception.details.contains("Connection reset"));

    let _ = tokio::fs::remove_dir_all(dir).await;
    let _ = tokio::fs::remove_dir_all(failing_dir).await;
}

#[tokio::test]
async fn test_save_user_directive_and_record_success_and_validation_failure() {
    let dir = create_temp_dir("audit_user_directive");
    let store = DurableMemoryStore::new(&dir);
    let mut resilient =
        ResilientDurableMemoryStore::new(store, DurableMemoryFaultPolicy::none(), 3);
    let mut ledger = AuditLedger::new();

    resilient
        .save_user_directive_and_record(&mut ledger, "  Keep audit records immutable.  ")
        .await
        .unwrap();

    let context = resilient
        .load_relevant_context("audit records")
        .await
        .unwrap();
    assert!(context.contains("Keep audit records immutable."));
    assert_eq!(resilient.saved_count(), 1);
    let allowed = &ledger.entries()[0];
    assert_eq!(allowed.event_type, AuditEventType::CommandAllowed);
    assert_eq!(allowed.severity, ActionSeverity::Routine);
    assert!(allowed.details.contains("Keep audit records immutable."));

    let result = resilient
        .save_user_directive_and_record(&mut ledger, " \n ")
        .await;
    assert!(matches!(result, Err(DurableMemoryError::EmptyContent)));
    assert_eq!(resilient.rejected_count(), 1);
    let blocked = ledger.entries().last().unwrap();
    assert_eq!(blocked.event_type, AuditEventType::CommandBlocked);
    assert_eq!(blocked.severity, ActionSeverity::Destructive);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[test]
fn test_simulate_durable_fault_direct() {
    assert_eq!(
        simulate_durable_fault(&DurableMemoryFault::SocketTimeout),
        Err(DurableMemoryError::SocketTimeout)
    );
    assert_eq!(
        simulate_durable_fault(&DurableMemoryFault::ConnectionReset),
        Err(DurableMemoryError::ConnectionReset)
    );
    assert!(matches!(
        simulate_durable_fault(&DurableMemoryFault::TransientIo),
        Err(DurableMemoryError::TransientIo(_))
    ));
    assert!(matches!(
        simulate_durable_fault(&DurableMemoryFault::CorruptedState),
        Err(DurableMemoryError::CorruptedState(_))
    ));
    assert!(matches!(
        simulate_durable_fault(&DurableMemoryFault::LockFailure),
        Err(DurableMemoryError::LockFailure(_))
    ));
}

#[test]
fn test_bounded_lesson_cache_evicts_oldest_and_tracks_save_counts() {
    let dir = create_temp_dir("bounded_lesson_cache");
    let store = DurableMemoryStore::new(&dir);
    let mut resilient =
        ResilientDurableMemoryStore::new(store, DurableMemoryFaultPolicy::none(), 3)
            .with_max_capacity(2);

    let make_lesson = |id: &str| StructuredLesson {
        id: id.to_string(),
        timestamp: "2026-11-08T00:00:00Z".to_string(),
        task_title: format!("Lesson {id}"),
        trigger_error: None,
        remediation_applied: None,
        general_insight: format!("Insight for {id}"),
        success: true,
    };

    for id in ["oldest", "middle", "newest"] {
        resilient
            .save_structured_lesson_cached(&make_lesson(id))
            .expect("valid lessons should be saved");
    }

    let cached = resilient.cached_lessons();
    assert_eq!(cached.len(), 2);
    assert_eq!(cached[0].id, "middle");
    assert_eq!(cached[1].id, "newest");
    assert!(!resilient.has_lesson("oldest"));
    assert!(resilient.has_lesson("middle"));
    assert!(resilient.has_lesson("newest"));
    assert_eq!(resilient.saved_count(), 3);
    assert_eq!(resilient.recovered_count(), 0);
    assert_eq!(resilient.rejected_count(), 0);

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn test_high_volume_durable_resilience() {
    let dir = create_temp_dir("high_volume");
    let store = DurableMemoryStore::new(&dir);

    let policy = DurableMemoryFaultPolicy::intermittent(DurableMemoryFault::SocketTimeout, 3)
        .with_max_injections(10);
    let mut resilient = ResilientDurableMemoryStore::new(store, policy, 5);

    for i in 0..25 {
        let res = resilient
            .save_lesson(&format!("High volume lesson entry #{}", i))
            .await;
        assert!(res.is_ok(), "Operation #{} should succeed", i);
    }

    assert_eq!(resilient.saved_count(), 25);
    assert_eq!(resilient.recovered_count(), 10);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_corrupted_json_does_not_brick_subsequent_saves() {
    let dir = create_temp_dir("corrupt_auto_recover");
    let store = DurableMemoryStore::new(&dir);

    // Write corrupted JSON into structured_lessons.json
    let storage_dir = dir.join("storage").join("harness_memory");
    tokio::fs::create_dir_all(&storage_dir)
        .await
        .expect("create dir");
    tokio::fs::write(
        storage_dir.join("structured_lessons.json"),
        b"{ invalid json syntax ",
    )
    .await
    .expect("write corrupt json");

    let mut resilient = ResilientDurableMemoryStore::with_store(store);

    let lesson = StructuredLesson {
        id: "les-recover-01".to_string(),
        timestamp: "2026-11-06T00:00:00Z".to_string(),
        task_title: "Auto-recover from corrupted file".to_string(),
        trigger_error: None,
        remediation_applied: None,
        general_insight: "Corrupted store auto-recovers without crash".to_string(),
        success: true,
    };

    // Resilient store recovers gracefully from corrupt file
    let res = resilient.save_structured_lesson(&lesson).await;
    assert!(
        res.is_ok(),
        "Save should recover from corrupt file: {:?}",
        res.err()
    );

    let loaded = resilient.load_structured_lessons().await;
    assert!(loaded.is_ok());
    let lessons = loaded.unwrap();
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0].id, "les-recover-01");

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[test]
fn test_topic_b493ca_b86f27_durable_benchmark_and_resilience() {
    println!("[Reproduction & Benchmark Test] Executing TOPIC-20261105-b493ca / b86f27 Durable Benchmark...");

    // 1. Single-threaded benchmark (10,000 requests)
    let mut resilient = ResilientDurableMemoryStore::from_workspace("/tmp/durable_test_bench")
        .with_max_capacity(2_000);

    let stats: DurableBenchmarkStats =
        DurableBenchmarkHarness::run_benchmark(&mut resilient, 10_000);
    assert_eq!(stats.total_requests, 10_000);
    assert_eq!(stats.successful_operations, 9_000);
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
    let _ = DurableBenchmarkHarness::run_benchmark(&mut resilient, 5_000);
    let mem_after = resilient.memory_usage_bytes();
    assert!(
        mem_after <= mem_before + 2048,
        "Memory usage must be strictly bounded under high load (before: {}, after: {})",
        mem_before,
        mem_after
    );

    // 3. Batch resolution optimization test
    let batch_lessons = vec![
        DurableBenchmarkHarness::make_benchmark_lesson(1, false),
        DurableBenchmarkHarness::make_benchmark_lesson(2, false),
        DurableBenchmarkHarness::make_benchmark_lesson(3, true), // Invalid: empty content
        DurableBenchmarkHarness::make_benchmark_lesson(4, false),
    ];
    let batch_res = resilient.save_batch(&batch_lessons);
    assert_eq!(batch_res.len(), 4);
    assert!(batch_res[0].is_ok());
    assert!(batch_res[1].is_ok());
    assert!(matches!(
        batch_res[2],
        Err(DurableMemoryError::EmptyContent)
    ));
    assert!(batch_res[3].is_ok());

    // 4. Concurrent benchmark test (10,000 requests across 4 threads)
    let concurrent_stats = DurableBenchmarkHarness::run_concurrent_benchmark(10_000, 4);
    assert_eq!(concurrent_stats.total_requests, 10_000);
    assert_eq!(concurrent_stats.successful_operations, 9_000);
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
    println!("[Reproduction & Benchmark Test] TOPIC-20261105-b493ca / b86f27 passed successfully!");
}
