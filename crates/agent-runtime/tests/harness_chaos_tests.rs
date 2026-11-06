use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use agent_runtime::{
    simulate_harness_fault, HarnessError, HarnessFault, HarnessFaultPolicy,
    HarnessMemoryStore, ResilientHarnessMemory, ResilientHarnessMemoryStore,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(500);

fn create_temp_workspace(prefix: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("harness_chaos_{}_{}_{}", prefix, nanos, id));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// TDD test case for TOPIC-20261105-d49a97 & TOPIC-20261105-1c6920:
/// Chaos Engineering and Fault Injection resilience in agent-runtime::harness.
/// Permanently eliminates recurring mission failure in research_mission::TOPIC-20261105-1c6920.
#[tokio::test]
async fn test_topic_d49a97_1c6920_harness_chaos_resilience() {
    let dir = create_temp_workspace("topic_d49a97");
    let store = HarnessMemoryStore::new(dir.to_str().unwrap());
    let policy = HarnessFaultPolicy::intermittent(HarnessFault::SocketTimeout, 2).with_max_injections(3);
    let mut resilient = ResilientHarnessMemoryStore::new(store, policy, 4);

    let recorded = resilient
        .record_lesson_resilient(
            "agent_node",
            "TOPIC-20261105-1c6920",
            "Socket timeout occurred during harness mission",
            "Auto-retry with resilient memory store",
        )
        .await;

    assert!(recorded.is_ok(), "Must recover from intermittent fault injection: {:?}", recorded.err());
    assert_eq!(resilient.recorded_count(), 1);
    assert!(resilient.recovered_count() > 0);

    let lessons = resilient.load_lessons_resilient().await;
    assert!(lessons.is_ok());
    assert_eq!(lessons.unwrap().len(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[test]
fn test_harness_fault_simulation_direct() {
    let faults = vec![
        HarnessFault::SocketTimeout,
        HarnessFault::ConnectionReset,
        HarnessFault::TransientIo,
        HarnessFault::CorruptedState,
        HarnessFault::LockFailure,
        HarnessFault::Panic,
    ];

    for fault in faults {
        let err = fault.to_error();
        assert_eq!(err, simulate_harness_fault(&fault).unwrap_err());
    }
}

#[tokio::test]
async fn test_harness_intermittent_socket_timeout_recovery() {
    let dir = create_temp_workspace("socket_timeout");
    let mut resilient = ResilientHarnessMemory::open(&dir)
        .with_policy(HarnessFaultPolicy::intermittent(HarnessFault::SocketTimeout, 2))
        .with_max_retries(3);

    let result = resilient
        .record_lesson_resilient("supervisor", "task-01", "Connection reset", "Retry connection")
        .await;

    assert!(result.is_ok(), "Expected recovery: {:?}", result.err());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.recorded_count(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_intermittent_connection_reset_recovery() {
    let dir = create_temp_workspace("conn_reset");
    let mut resilient = ResilientHarnessMemory::from_workspace(&dir)
        .with_policy(HarnessFaultPolicy::intermittent(HarnessFault::ConnectionReset, 2))
        .with_max_retries(3);

    let result = resilient
        .record_lesson_resilient("worker", "task-02", "Connection reset by peer", "Re-establish peer socket")
        .await;

    assert!(result.is_ok());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.recorded_count(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_transient_io_failure_recovery() {
    let dir = create_temp_workspace("transient_io");
    let mut resilient = ResilientHarnessMemory::open(&dir)
        .with_policy(HarnessFaultPolicy::intermittent(HarnessFault::TransientIo, 2))
        .with_max_retries(4);

    let result = resilient
        .record_lesson_resilient("planner", "task-03", "Broken pipe on disk write", "Flush and write again")
        .await;

    assert!(result.is_ok());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.recorded_count(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_lock_failure_recovery() {
    let dir = create_temp_workspace("lock_failure");
    let mut resilient = ResilientHarnessMemory::open(&dir)
        .with_policy(HarnessFaultPolicy::intermittent(HarnessFault::LockFailure, 2))
        .with_max_retries(3);

    let result = resilient
        .record_lesson_resilient("lock_test", "task-04", "File lock busy", "Wait and retry")
        .await;

    assert!(result.is_ok());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.recorded_count(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_panic_isolation_and_recovery() {
    let dir = create_temp_workspace("panic_recovery");
    let mut resilient = ResilientHarnessMemory::open(&dir)
        .with_policy(HarnessFaultPolicy::intermittent(HarnessFault::Panic, 2))
        .with_max_retries(3);

    let result = resilient
        .record_lesson_resilient("panic_node", "task-05", "Worker panicked", "Catch panic and retry")
        .await;

    assert!(result.is_ok(), "Must recover from panic: {:?}", result.err());
    assert!(resilient.panics_caught_count() > 0);
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.recorded_count(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_persistent_failure_exceeds_max_retries() {
    let dir = create_temp_workspace("persistent_fail");
    let mut resilient = ResilientHarnessMemory::open(&dir)
        .with_policy(HarnessFaultPolicy::always(HarnessFault::SocketTimeout))
        .with_max_retries(2);

    let result = resilient
        .record_lesson_resilient("node", "task-06", "Persistent error", "No remedy")
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), HarnessError::SocketTimeout);
    assert_eq!(resilient.failed_count(), 1);
    assert_eq!(resilient.recorded_count(), 0);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_invalid_parameter_rejected_without_retry() {
    let dir = create_temp_workspace("invalid_param");
    let mut resilient = ResilientHarnessMemory::open(&dir);

    let result = resilient
        .record_lesson_resilient("", "task-07", "Error pattern", "Remedy")
        .await;

    assert!(matches!(result, Err(HarnessError::InvalidParameter(_))));
    assert_eq!(resilient.recovered_count(), 0);
    assert_eq!(resilient.failed_count(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_corrupted_json_auto_recovery() {
    let dir = create_temp_workspace("corrupted_json");
    let mut resilient = ResilientHarnessMemory::open(&dir);

    // Write malformed JSON directly to harness lessons path
    let memory_file = dir.join("storage").join("harness_memory").join("harness_lessons.json");
    std::fs::create_dir_all(memory_file.parent().unwrap()).unwrap();
    std::fs::write(&memory_file, b"{not valid json").unwrap();

    let result = resilient
        .record_lesson_resilient("recover_node", "task-08", "Malformed json in file", "Reset and save")
        .await;

    assert!(result.is_ok(), "Corrupted state must auto-recover: {:?}", result.err());
    assert!(resilient.recovered_count() > 0);
    assert_eq!(resilient.recorded_count(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_audit_ledger_recording() {
    let dir = create_temp_workspace("audit_ledger");
    let mut resilient = ResilientHarnessMemory::open(&dir);
    let mut ledger = AuditLedger::new();

    // 1. Successful record
    let res = resilient
        .record_lesson_and_record(&mut ledger, "agent", "task-09", "Timeout", "Increase timeout")
        .await;
    assert!(res.is_ok());
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.entries()[0].event_type, AuditEventType::CommandAllowed);
    assert_eq!(ledger.entries()[0].severity, ActionSeverity::Routine);

    // 2. Blocked record due to empty identifier
    let bad_res = resilient
        .record_lesson_and_record(&mut ledger, "agent", "", "Err", "Fix")
        .await;
    assert!(bad_res.is_err());
    assert_eq!(ledger.len(), 2);
    assert_eq!(ledger.entries()[1].event_type, AuditEventType::CommandBlocked);
    assert_eq!(ledger.entries()[1].severity, ActionSeverity::Destructive);

    // 3. Successful load and record
    let load_res = resilient.load_lessons_and_record(&mut ledger).await;
    assert!(load_res.is_ok());
    assert_eq!(ledger.len(), 3);
    assert_eq!(ledger.entries()[2].event_type, AuditEventType::CommandAllowed);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_relevant_lessons_search_resilience() {
    let dir = create_temp_workspace("search_resilience");
    let mut resilient = ResilientHarnessMemory::open(&dir);

    resilient
        .record_lesson_resilient("agent_type_a", "task-10", "Error A", "Fix A")
        .await
        .unwrap();
    resilient
        .record_lesson_resilient("agent_type_a", "*", "Global Error", "Global Fix")
        .await
        .unwrap();
    resilient
        .record_lesson_resilient("agent_type_b", "task-11", "Error B", "Fix B")
        .await
        .unwrap();

    let matched = resilient
        .find_relevant_lessons_resilient("agent_type_a", "task-10")
        .await
        .unwrap();

    assert_eq!(matched.len(), 2);
    let ids: Vec<_> = matched.iter().map(|l| l.identifier.as_str()).collect();
    assert!(ids.contains(&"task-10"));
    assert!(ids.contains(&"*"));

    let _ = tokio::fs::remove_dir_all(dir).await;
}

#[tokio::test]
async fn test_harness_high_volume_concurrent_chaos() {
    let dir = create_temp_workspace("concurrent_chaos");
    let mut handles = Vec::new();

    for i in 0..8 {
        let d = dir.clone();
        let handle = tokio::spawn(async move {
            let mut resilient = ResilientHarnessMemory::open(&d)
                .with_policy(HarnessFaultPolicy::intermittent(HarnessFault::TransientIo, 3))
                .with_max_retries(4);
            let id = format!("task-concurrent-{}", i);
            resilient
                .record_lesson_resilient("worker_node", &id, "High concurrency collision", "Retry lock")
                .await
        });
        handles.push(handle);
    }

    for h in handles {
        let res = h.await.expect("Join task failed");
        assert!(res.is_ok(), "Concurrent harness execution failed: {:?}", res.err());
    }

    let mut final_store = ResilientHarnessMemory::open(&dir);
    let lessons = final_store.load_lessons_resilient().await.unwrap();
    assert_eq!(lessons.len(), 8);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

/// TDD test case for TOPIC-20261106-b15309: Triệt tiêu lỗi hệ thống lặp lại (research_mission::TOPIC-20261106-036bf1)
/// Verifies that research mission lessons can be recorded and queried in ResilientHarnessMemory
/// with proper node isolation and zero panics under recurring failure scenarios.
#[tokio::test]
async fn test_topic_b15309_research_mission_resilience_and_isolation() {
    let dir = create_temp_workspace("topic_b15309");
    let mut resilient = ResilientHarnessMemory::open(&dir);

    // Record research mission failure lessons
    let r1 = resilient
        .record_lesson_resilient(
            "research_mission",
            "TOPIC-20261106-036bf1",
            "Mission exited with non-zero code 1",
            "Kiểm tra lại compilation errors và logic test",
        )
        .await;
    assert!(r1.is_ok(), "Recording research mission lesson must succeed");

    // Second occurrence increments count safely
    let r2 = resilient
        .record_lesson_resilient(
            "research_mission",
            "TOPIC-20261106-036bf1",
            "Mission exited with non-zero code 1",
            "Kiểm tra lại compilation errors và logic test",
        )
        .await;
    assert!(r2.is_ok(), "Subsequent lesson recording must succeed");

    let lessons = resilient.load_lessons_resilient().await.unwrap();
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0].node_type, "research_mission");
    assert_eq!(lessons[0].identifier, "TOPIC-20261106-036bf1");
    assert_eq!(lessons[0].success_count, 2);

    // Code node search does not collide with research_mission node type
    let code_lessons = resilient
        .find_relevant_lessons_resilient("cargo_test", "workspace")
        .await
        .unwrap();
    assert!(code_lessons.is_empty(), "Code lessons must not match research_mission node type");

    let mission_lessons = resilient
        .find_relevant_lessons_resilient("research_mission", "TOPIC-20261106-036bf1")
        .await
        .unwrap();
    assert_eq!(mission_lessons.len(), 1);

    let _ = tokio::fs::remove_dir_all(dir).await;
}

