use agent_domain::{Priority, TaskStatus};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_runtime::{SqliteTaskStore, TaskService};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn unique_temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dlq_test_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

struct MockSuccessEngine;

#[async_trait]
impl ComputeEngine for MockSuccessEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let stdout = if input.prompt_or_command.contains("Architect") || input.prompt_or_command.contains("JSON") {
            r#"[{"id":"ST-1","title":"Execution step","command_or_prompt":"echo ok","expected_verification":"cmd: true"}]"#.to_string()
        } else {
            "Operation completed successfully.".to_string()
        };
        Ok(EngineOutput {
            stdout_content: stdout,
            exit_code: 0,
            execution_duration_ms: 5,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

struct PanickingEngine {
    panic_count: Arc<AtomicUsize>,
}

#[async_trait]
impl ComputeEngine for PanickingEngine {
    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        self.panic_count.fetch_add(1, Ordering::SeqCst);
        panic!("Simulated critical poison task panic!");
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

/// TDD Test 1: Verify poison task is isolated to Dead-Letter Queue after reaching poison threshold
#[tokio::test]
async fn test_poison_task_isolated_to_dead_letter_queue_after_repeated_failures() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let panic_count = Arc::new(AtomicUsize::new(0));
    let engine = Arc::new(PanickingEngine {
        panic_count: panic_count.clone(),
    });

    // Configure service with poison threshold = 2
    let service = TaskService::new(store.clone(), &ws, 2, None, None).with_poison_threshold(2);
    assert_eq!(service.poison_threshold(), 2);

    let task_id = service
        .submit_task("Poison Job", "Will fail repeatedly", Priority::Normal)
        .await
        .unwrap();

    // 1st dispatch -> fails once (failure_count = 1 < 2)
    service.dispatch(task_id.clone(), engine.clone());
    tokio::time::sleep(Duration::from_millis(300)).await;

    let task_1 = service.get_task(&task_id).await.unwrap().expect("Task exists");
    assert!(
        matches!(task_1.status, TaskStatus::Failed { .. }),
        "First failure should result in Failed status, got: {:?}",
        task_1.status
    );
    assert_eq!(task_1.failure_count, 1);
    assert!(!task_1.is_quarantined());
    assert!(!service.is_quarantined(&task_id).await.unwrap());
    assert_eq!(service.count_quarantined_tasks().await.unwrap(), 0);

    // Retry task -> 2nd dispatch -> reaches threshold 2 -> isolated to Dead-Letter Queue
    let retried = service.retry_task(&task_id, engine.clone()).await.unwrap();
    assert!(retried, "Retry should succeed for Failed task");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let task_2 = service.get_task(&task_id).await.unwrap().expect("Task exists");
    assert!(
        matches!(task_2.status, TaskStatus::Quarantined { .. }),
        "Second failure should isolate task into Quarantined (Dead-Letter Queue), got: {:?}",
        task_2.status
    );
    assert!(task_2.is_quarantined());
    assert_eq!(task_2.failure_count, 2);
    assert!(service.is_quarantined(&task_id).await.unwrap());
    assert_eq!(service.count_quarantined_tasks().await.unwrap(), 1);

    // Standard retry_task must NOT allow retrying a Quarantined task directly
    let retried_quarantined = service.retry_task(&task_id, engine.clone()).await.unwrap();
    assert!(
        !retried_quarantined,
        "retry_task must refuse to re-dispatch a quarantined poison task"
    );

    let _ = std::fs::remove_dir_all(&ws);
}

/// TDD Test 2: Verify Dead-Letter Queue isolation unblocks processing of subsequent tasks
#[tokio::test]
async fn test_dead_letter_queue_isolation_unblocks_remaining_queue() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let panic_count = Arc::new(AtomicUsize::new(0));
    let poison_engine = Arc::new(PanickingEngine {
        panic_count: panic_count.clone(),
    });
    let success_engine = Arc::new(MockSuccessEngine);

    // Single concurrency queue with poison_threshold = 1 (immediate quarantine on poison failure)
    let service = TaskService::new(store.clone(), &ws, 1, None, None).with_poison_threshold(1);

    let poison_id = service
        .submit_task("Poison Pill", "Panics immediately", Priority::High)
        .await
        .unwrap();

    let normal_id = service
        .submit_task("Healthy Task", "Processes normally", Priority::High)
        .await
        .unwrap();

    // Dispatch poison task
    service.dispatch(poison_id.clone(), poison_engine);
    // Dispatch normal task
    service.dispatch(normal_id.clone(), success_engine);

    // Wait for both to be processed through the queue
    tokio::time::sleep(Duration::from_millis(600)).await;

    let poison_task = service.get_task(&poison_id).await.unwrap().expect("poison task exists");
    let normal_task = service.get_task(&normal_id).await.unwrap().expect("normal task exists");

    // Poison task must be quarantined in Dead-Letter Queue
    assert!(
        matches!(poison_task.status, TaskStatus::Quarantined { .. }),
        "Poison task should be quarantined, got: {:?}",
        poison_task.status
    );

    // Normal task MUST NOT be blocked, and must reach Completed!
    assert_eq!(
        normal_task.status,
        TaskStatus::Completed,
        "Subsequent healthy task must successfully complete without queue starvation"
    );

    let _ = std::fs::remove_dir_all(&ws);
}

/// TDD Test 3: Verify Dead-Letter Queue query APIs and manual/re-queue release
#[tokio::test]
async fn test_dead_letter_queue_query_and_release_lifecycle() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 2, None, None);

    let t1 = service.submit_task("Task 1", "Goal 1", Priority::Normal).await.unwrap();
    let t2 = service.submit_task("Task 2", "Goal 2", Priority::Normal).await.unwrap();

    // Explicitly quarantine t1
    let q_res = service.quarantine_task(&t1, "Manual quarantine: suspicious behavior").await.unwrap();
    assert!(q_res);

    // Query APIs
    assert!(service.is_quarantined(&t1).await.unwrap());
    assert!(!service.is_quarantined(&t2).await.unwrap());
    assert_eq!(service.count_quarantined_tasks().await.unwrap(), 1);

    let quarantined_list = service.list_quarantined_tasks(10, 0).await.unwrap();
    assert_eq!(quarantined_list.len(), 1);
    assert_eq!(quarantined_list[0].id, t1);

    let fetched_q = service.get_quarantined_task(&t1).await.unwrap();
    assert!(fetched_q.is_some());
    assert_eq!(fetched_q.unwrap().id, t1);

    let summary = service.status_summary().await.unwrap();
    assert_eq!(summary.get("quarantined"), Some(&1));

    // Batch retry failed tasks must ignore quarantined tasks
    let retried = service.retry_failed_tasks(Arc::new(MockSuccessEngine)).await.unwrap();
    assert_eq!(retried, 0, "Quarantined tasks must not be retried by retry_failed_tasks");

    // Release quarantined task with new engine
    let released = service
        .release_quarantined_task(&t1, Some(Arc::new(MockSuccessEngine)))
        .await
        .unwrap();
    assert!(released);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // t1 should now be completed
    let t1_after = service.get_task(&t1).await.unwrap().expect("t1 exists");
    assert_eq!(t1_after.status, TaskStatus::Completed);
    assert_eq!(t1_after.failure_count, 0);
    assert_eq!(service.count_quarantined_tasks().await.unwrap(), 0);

    let _ = std::fs::remove_dir_all(&ws);
}

/// TDD Test 4: Verify release_all_quarantined
#[tokio::test]
async fn test_batch_release_all_quarantined() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 2, None, None);

    let t1 = service.submit_task("Task 1", "Goal 1", Priority::Low).await.unwrap();
    let t2 = service.submit_task("Task 2", "Goal 2", Priority::Low).await.unwrap();
    let t3 = service.submit_task("Task 3", "Goal 3", Priority::Low).await.unwrap();

    service.quarantine_task(&t1, "Reason 1").await.unwrap();
    service.quarantine_task(&t2, "Reason 2").await.unwrap();
    service.quarantine_task(&t3, "Reason 3").await.unwrap();

    assert_eq!(service.count_quarantined_tasks().await.unwrap(), 3);

    let released_count = service.release_all_quarantined(None).await.unwrap();
    assert_eq!(released_count, 3);
    assert_eq!(service.count_quarantined_tasks().await.unwrap(), 0);

    for id in &[t1, t2, t3] {
        let task = service.get_task(id).await.unwrap().expect("task exists");
        assert_eq!(task.status, TaskStatus::Queued);
        assert_eq!(task.failure_count, 0);
    }

    let _ = std::fs::remove_dir_all(&ws);
}

/// TDD Test 5: Verify TOPIC-20261106-013a3a acceptance criteria end-to-end
#[tokio::test]
async fn test_topic_20261106_013a3a_dead_letter_queue_and_poison_task_isolation() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let panic_count = Arc::new(AtomicUsize::new(0));
    let poison_engine = Arc::new(PanickingEngine {
        panic_count: panic_count.clone(),
    });
    let healthy_engine = Arc::new(MockSuccessEngine);

    // Concurrency = 1: Poison task must not starve healthy tasks once isolated
    let service = TaskService::new(store.clone(), &ws, 1, None, None).with_poison_threshold(2);
    assert_eq!(service.poison_threshold(), 2);

    let poison_id = service
        .submit_task("Poison Job", "Repeatedly triggers fatal faults", Priority::Critical)
        .await
        .unwrap();
    let healthy_id = service
        .submit_task("Healthy Worker", "Executes cleanly", Priority::Normal)
        .await
        .unwrap();

    // 1st dispatch for poison task -> failure_count = 1
    service.dispatch(poison_id.clone(), poison_engine.clone());
    tokio::time::sleep(Duration::from_millis(300)).await;

    let p1 = service.get_task(&poison_id).await.unwrap().expect("p1 exists");
    assert_eq!(p1.failure_count, 1);
    assert!(matches!(p1.status, TaskStatus::Failed { .. }));
    assert!(!service.is_quarantined(&poison_id).await.unwrap());

    // Retry poison task -> failure_count = 2 >= poison_threshold -> Quarantined
    let retried = service.retry_task(&poison_id, poison_engine.clone()).await.unwrap();
    assert!(retried);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let p2 = service.get_task(&poison_id).await.unwrap().expect("p2 exists");
    assert_eq!(p2.failure_count, 2);
    assert!(p2.is_quarantined());
    assert!(service.is_quarantined(&poison_id).await.unwrap());
    assert_eq!(service.count_quarantined_tasks().await.unwrap(), 1);

    // Verify healthy task now dispatches and completes smoothly
    service.dispatch(healthy_id.clone(), healthy_engine.clone());
    tokio::time::sleep(Duration::from_millis(300)).await;

    let healthy_task = service.get_task(&healthy_id).await.unwrap().expect("healthy exists");
    assert_eq!(healthy_task.status, TaskStatus::Completed);

    // Operator inspects DLQ
    let dlq_items = service.list_quarantined_tasks(10, 0).await.unwrap();
    assert_eq!(dlq_items.len(), 1);
    assert_eq!(dlq_items[0].id, poison_id);

    // Operator fixes the engine and releases poison task from DLQ
    let released = service
        .release_quarantined_task(&poison_id, Some(healthy_engine.clone()))
        .await
        .unwrap();
    assert!(released);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let p_fixed = service.get_task(&poison_id).await.unwrap().expect("p_fixed exists");
    assert_eq!(p_fixed.status, TaskStatus::Completed);
    assert_eq!(p_fixed.failure_count, 0);
    assert_eq!(service.count_quarantined_tasks().await.unwrap(), 0);

    let _ = std::fs::remove_dir_all(&ws);
}
