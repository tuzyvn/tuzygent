use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use agent_memory::{
    simulate_durable_fault, DurableMemoryError, DurableMemoryFault, DurableMemoryFaultPolicy,
    DurableMemoryStore, ResilientDurableMemoryStore, StructuredLesson,
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
    assert!(loaded.unwrap().contains("Strict sandbox policy enforcement"));

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
    assert!(res.is_ok(), "Save should recover from corrupt file: {:?}", res.err());

    let loaded = resilient.load_structured_lessons().await;
    assert!(loaded.is_ok());
    let lessons = loaded.unwrap();
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0].id, "les-recover-01");

    let _ = tokio::fs::remove_dir_all(dir).await;
}
