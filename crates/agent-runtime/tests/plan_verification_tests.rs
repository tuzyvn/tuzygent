use agent_domain::{
    GuardrailPolicy, Priority, SubTask, Task, TaskResult, TaskStatus, VerificationSpec,
};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{AutonomousOodaLoop, SqliteTaskStore, TaskService, TaskStore, TaskVerifier};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static COUNTER: AtomicU64 = AtomicU64::new(500);

fn unique_temp_dir() -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("agentcore_verif_test_{}_{}", ts, id));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

struct MockSuccessEngine;

struct TwoStepEngine {
    second_started: tokio::sync::Notify,
}
#[async_trait]
impl ComputeEngine for TwoStepEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        if input.prompt_or_command == "second step" {
            self.second_started.notify_one();
            std::future::pending().await
        } else {
            std::fs::write(
                std::path::Path::new(&input.working_dir).join("first-artifact"),
                "ok",
            )
            .unwrap();
            Ok(EngineOutput {
                stdout_content: "first done".into(),
                exit_code: 0,
                execution_duration_ms: 0,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

#[tokio::test]
async fn checkpoint_records_completed_and_inflight_steps() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let engine = Arc::new(TwoStepEngine {
        second_started: tokio::sync::Notify::new(),
    });
    let ooda = AutonomousOodaLoop::new(
        engine.clone(),
        GuardrailPolicy::default_enterprise(),
        ScratchpadManager::new(&ws),
        DurableMemoryStore::new(&ws),
        ws.to_str().unwrap(),
    )
    .with_checkpoint_store(store.clone());
    let mut task = Task::new("checkpoint", "two steps", Priority::Normal);
    task.subtasks.push(SubTask::new(
        "first",
        "first",
        "first step",
        "file: first-artifact",
    ));
    task.subtasks.push(SubTask::new(
        "second",
        "second",
        "second step",
        "file: first-artifact",
    ));
    let id = task.id.clone();
    let handle = tokio::spawn(async move { ooda.run_goal_detailed(&mut task).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        engine.second_started.notified(),
    )
    .await
    .unwrap();
    let checkpoint = store.get_checkpoint(&id).await.unwrap().unwrap();
    assert_eq!(checkpoint[0].status, TaskStatus::Completed);
    assert_eq!(checkpoint[1].status, TaskStatus::Executing);
    assert_eq!(checkpoint[0].output_excerpt.as_deref(), Some("first done"));
    handle.abort();
    let _ = handle.await;
    drop(store);
    let reopened = SqliteTaskStore::new(&ws).unwrap();
    let checkpoint = reopened.get_checkpoint(&id).await.unwrap().unwrap();
    assert_eq!(checkpoint[0].status, TaskStatus::Completed);
    assert_eq!(checkpoint[1].status, TaskStatus::Executing);
    drop(reopened);
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn corrupt_checkpoint_stops_dispatch_before_engine_runs() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 1, None, None);
    let id = service
        .submit_task("broken", "do not execute", Priority::Normal)
        .await
        .unwrap();
    let conn = rusqlite::Connection::open(store.db_path()).unwrap();
    conn.execute(
        "INSERT INTO subtask_checkpoints (task_id, subtasks_json, updated_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![id.0, "invalid json", "now"],
    )
    .unwrap();
    service.dispatch(id.clone(), Arc::new(PanickingEngine));
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some(result) = service.get_result(&id).await.unwrap() {
                break result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(&result.status, TaskStatus::Failed { error } if error.contains("checkpoint")));
    let retained: String = conn
        .query_row(
            "SELECT subtasks_json FROM subtask_checkpoints WHERE task_id=?1",
            rusqlite::params![id.0],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, "invalid json");
    drop(conn);
    drop(service);
    drop(store);
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn finishing_task_is_atomic_and_does_not_reverse_cancellation() {
    let ws = unique_temp_dir();
    let store = SqliteTaskStore::new(&ws).unwrap();
    let task = Task::new("atomic", "test", Priority::Normal);
    store.save_task(&task).await.unwrap();
    let mut running = store.claim_queued(&task.id).await.unwrap().unwrap();
    running
        .subtasks
        .push(SubTask::new("one", "one", "work", "manual"));
    running.status = TaskStatus::Completed;
    let result = TaskResult {
        task_id: task.id.clone(),
        status: TaskStatus::Completed,
        summary: "done".into(),
        artifacts: vec![],
        verifications: vec![],
        duration_ms: 1,
        engine_used: None,
    };
    let conn = rusqlite::Connection::open(store.db_path()).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_result BEFORE INSERT ON task_results BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    assert!(store.finish_task(&running, &result, false).await.is_err());
    assert_eq!(
        store.get_task(&task.id).await.unwrap().unwrap().status,
        TaskStatus::Executing
    );
    assert!(store.get_checkpoint(&task.id).await.unwrap().is_none());
    conn.execute_batch("DROP TRIGGER fail_result;").unwrap();
    assert!(store.cancel_task(&task.id).await.unwrap());
    store.finish_task(&running, &result, false).await.unwrap();
    assert!(matches!(
        store.get_task(&task.id).await.unwrap().unwrap().status,
        TaskStatus::Canceled { .. }
    ));
    assert!(matches!(
        store.get_result(&task.id).await.unwrap().unwrap().status,
        TaskStatus::Canceled { .. }
    ));
    let (snapshot_task, snapshot_result) =
        store.get_task_and_result(&task.id).await.unwrap().unwrap();
    assert_eq!(snapshot_task.status, snapshot_result.unwrap().status);
    assert_eq!(
        store.get_checkpoint(&task.id).await.unwrap().unwrap().len(),
        1
    );
    drop(conn);
    drop(store);
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn unverified_output_requires_review_without_reexecution() {
    let ws = unique_temp_dir();
    let ooda = AutonomousOodaLoop::new(
        Arc::new(MockSuccessEngine),
        GuardrailPolicy::default_enterprise(),
        ScratchpadManager::new(&ws),
        DurableMemoryStore::new(&ws),
        ws.to_str().unwrap(),
    );
    let mut task = Task::new("review", "produce a report", Priority::Normal);
    task.subtasks.push(SubTask::new(
        "one",
        "report",
        "make report",
        "looks correct",
    ));
    let result = ooda.run_goal_detailed(&mut task).await.unwrap();
    assert!(matches!(result.status, TaskStatus::WaitingApproval { .. }));
    assert!(matches!(
        task.subtasks[0].status,
        TaskStatus::WaitingApproval { .. }
    ));
    assert_eq!(
        task.subtasks[0].output_excerpt.as_deref(),
        Some("Operation completed successfully.")
    );
    assert!(!result.verifications[0].passed);
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn approve_does_not_rerun_a_task_awaiting_manual_verification() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 1, None, None);
    let mut task = Task::new("review", "review", Priority::Normal);
    let reason = "Manual verification required".to_string();
    task.status = TaskStatus::WaitingApproval {
        reason: reason.clone(),
    };
    let mut subtask = SubTask::new("one", "work", "do work", "review");
    subtask.status = TaskStatus::WaitingApproval { reason };
    task.subtasks.push(subtask);
    store.save_task(&task).await.unwrap();
    assert!(service
        .approve_task(&task.id, Arc::new(PanickingEngine))
        .await
        .is_err());
    assert!(matches!(
        service.get_task(&task.id).await.unwrap().unwrap().status,
        TaskStatus::WaitingApproval { .. }
    ));
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn manual_review_is_versioned_atomic_and_does_not_rerun_step() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 1, None, None);
    let mut task = Task::new("review", "review", Priority::Normal);
    let reason = "Manual verification required".to_string();
    task.status = TaskStatus::WaitingApproval {
        reason: reason.clone(),
    };
    let mut step = SubTask::new("one", "report", "make report", "review output");
    step.status = TaskStatus::WaitingApproval { reason };
    step.output_excerpt = Some("produced report".into());
    task.subtasks.push(step);
    store.save_task(&task).await.unwrap();
    store
        .save_checkpoint(&task.id, &task.subtasks)
        .await
        .unwrap();
    let revision = service.review_revision(&task).unwrap();
    assert!(service
        .review_task(&task.id, "stale", "reviewer", true)
        .await
        .is_err());
    assert!(service
        .review_task(&task.id, &revision, "reviewer", true)
        .await
        .unwrap());
    let (stored, result) = service
        .get_task_and_result(&task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, TaskStatus::Completed);
    assert_eq!(stored.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(
        stored.subtasks[0].output_excerpt.as_deref(),
        Some("produced report")
    );
    assert!(result.unwrap().verifications[0].passed);
    assert!(service
        .review_task(&task.id, &revision, "reviewer", true)
        .await
        .is_err());
    let db = rusqlite::Connection::open(store.db_path()).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM task_reviews WHERE task_id=?1",
            rusqlite::params![task.id.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    drop(db);
    drop(service);
    drop(store);
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn approved_review_resumes_only_remaining_steps() {
    let ws = unique_temp_dir();
    std::fs::write(ws.join("proof"), "ok").unwrap();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 1, None, None);
    let mut task = Task::new("review", "review", Priority::Normal);
    task.engine_name = Some("mock".into());
    let reason = "Manual verification required".to_string();
    task.status = TaskStatus::WaitingApproval {
        reason: reason.clone(),
    };
    let mut first = SubTask::new("first", "first", "first action", "manual");
    first.status = TaskStatus::WaitingApproval { reason };
    first.output_excerpt = Some("already done".into());
    task.subtasks.push(first);
    task.subtasks.push(SubTask::new(
        "second",
        "second",
        "second action",
        "file: proof",
    ));
    store.save_task(&task).await.unwrap();
    store
        .save_checkpoint(&task.id, &task.subtasks)
        .await
        .unwrap();
    let revision = service.review_revision(&task).unwrap();
    assert!(service
        .review_task(&task.id, &revision, "reviewer", true)
        .await
        .unwrap());
    let (task, result) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some((task, Some(result))) = service.get_task_and_result(&task.id).await.unwrap()
            {
                if result.status == TaskStatus::Completed {
                    break (task, result);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        task.subtasks[0].output_excerpt.as_deref(),
        Some("already done")
    );
    assert_eq!(task.subtasks[1].status, TaskStatus::Completed);
    assert!(result
        .verifications
        .iter()
        .any(|v| v.details.contains("Manual review approved")));
    drop(service);
    drop(store);
    let _ = std::fs::remove_dir_all(ws);
}

struct WaitingEngine {
    started: tokio::sync::Notify,
    dropped: std::sync::atomic::AtomicBool,
}

struct ExecutionDrop<'a>(&'a std::sync::atomic::AtomicBool);
impl Drop for ExecutionDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl ComputeEngine for WaitingEngine {
    async fn execute(&self, _: &EngineInput) -> Result<EngineOutput, EngineError> {
        let _guard = ExecutionDrop(&self.dropped);
        self.started.notify_one();
        std::future::pending().await
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

#[tokio::test]
async fn cancellation_interrupts_running_engine_and_preserves_user_context() {
    let ws = unique_temp_dir();
    let engine = Arc::new(WaitingEngine {
        started: tokio::sync::Notify::new(),
        dropped: std::sync::atomic::AtomicBool::new(false),
    });
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ooda = AutonomousOodaLoop::new(
        engine.clone(),
        GuardrailPolicy::default_enterprise(),
        ScratchpadManager::new(&ws),
        DurableMemoryStore::new(&ws),
        ws.to_str().unwrap(),
    )
    .with_cancellation_token(cancel.clone());
    let mut task = Task::new("test", "test goal", Priority::Normal);
    task.subtasks
        .push(SubTask::new("one", "work", "do work", "manual review"));
    let handle = tokio::spawn(async move { ooda.run_goal_detailed(&mut task).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), engine.started.notified())
        .await
        .unwrap();
    cancel.store(true, Ordering::SeqCst);
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(result.status, TaskStatus::Canceled { .. }));
    assert!(engine.dropped.load(Ordering::SeqCst));
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn ooda_preserves_existing_context_files() {
    let ws = unique_temp_dir();
    std::fs::create_dir_all(ws.join(".multica")).unwrap();
    for name in [
        ".agent_context.md",
        ".multica/brief.md",
        ".multica/execution_context.md",
    ] {
        std::fs::write(ws.join(name), "user data").unwrap();
    }
    let ooda = AutonomousOodaLoop::new(
        Arc::new(MockSuccessEngine),
        GuardrailPolicy::default_enterprise(),
        ScratchpadManager::new(&ws),
        DurableMemoryStore::new(&ws),
        ws.to_str().unwrap(),
    );
    let mut task = Task::new("test", "test goal", Priority::Normal);
    task.subtasks
        .push(SubTask::new("one", "do work", "do work", "manual review"));
    let _ = ooda.run_goal_detailed(&mut task).await;
    for name in [
        ".agent_context.md",
        ".multica/brief.md",
        ".multica/execution_context.md",
    ] {
        assert_eq!(std::fs::read_to_string(ws.join(name)).unwrap(), "user data");
    }
    let _ = std::fs::remove_dir_all(ws);
}

#[async_trait]
impl ComputeEngine for MockSuccessEngine {
    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        Ok(EngineOutput {
            stdout_content: "Operation completed successfully.".to_string(),
            exit_code: 0,
            execution_duration_ms: 10,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

#[tokio::test]
async fn test_verifier_file_and_json_checks() {
    let ws = unique_temp_dir();
    let verifier = TaskVerifier::new(&ws);

    let test_file = ws.join("output.json");
    std::fs::write(&test_file, b"{\"status\": \"ok\", \"count\": 42}").unwrap();

    let file_spec = VerificationSpec::FileExists {
        path: "output.json".to_string(),
        min_bytes: Some(10),
    };
    let file_res = verifier.verify_spec(&file_spec).await;
    assert!(
        file_res.passed,
        "FileExists should pass: {}",
        file_res.details
    );

    let json_spec = VerificationSpec::JsonValid {
        path: "output.json".to_string(),
    };
    let json_res = verifier.verify_spec(&json_spec).await;
    assert!(
        json_res.passed,
        "JsonValid should pass: {}",
        json_res.details
    );

    let content_spec = VerificationSpec::ContentContains {
        path: "output.json".to_string(),
        pattern: "\"count\": 42".to_string(),
    };
    let content_res = verifier.verify_spec(&content_spec).await;
    assert!(
        content_res.passed,
        "ContentContains should pass: {}",
        content_res.details
    );

    // Negative check: missing file
    let missing_spec = VerificationSpec::FileExists {
        path: "missing.txt".to_string(),
        min_bytes: None,
    };
    let missing_res = verifier.verify_spec(&missing_spec).await;
    assert!(!missing_res.passed, "Missing file should fail verification");

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn verifier_applies_sandbox_and_guardrail_to_commands() {
    let ws = unique_temp_dir();
    let sandbox = agent_runtime::SandboxConfig {
        driver: agent_runtime::SandboxDriverKind::Host,
        ..Default::default()
    };
    let verifier = TaskVerifier::with_policy(&ws, sandbox, GuardrailPolicy::default_enterprise());
    let safe = VerificationSpec::Command {
        command: "printf ok > marker".into(),
        expected_exit_code: 0,
    };
    assert!(verifier.verify_spec(&safe).await.passed);
    let blocked = VerificationSpec::Command {
        command: "rm -rf marker".into(),
        expected_exit_code: 0,
    };
    assert!(!verifier.verify_spec(&blocked).await.passed);
    assert_eq!(std::fs::read_to_string(ws.join("marker")).unwrap(), "ok");
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn test_sqlite_task_store_persistence_and_checkpoints() {
    let ws = unique_temp_dir();
    let store = SqliteTaskStore::new(&ws).unwrap();

    let task = Task::new(
        "Deploy Microservice",
        "Run deployment pipeline",
        Priority::High,
    );
    let task_id = task.id.clone();
    store.save_task(&task).await.unwrap();

    let loaded = store.get_task(&task_id).await.unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.title, "Deploy Microservice");

    // Test subtask checkpointing
    let subtasks = vec![
        SubTask::new("ST-1", "Compile Code", "cargo build", "exit 0"),
        SubTask::new("ST-2", "Run Migrations", "sqlx migrate run", "exit 0"),
    ];
    store.save_checkpoint(&task_id, &subtasks).await.unwrap();

    let restored = store.get_checkpoint(&task_id).await.unwrap();
    assert!(restored.is_some());
    let restored = restored.unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(restored[0].title, "Compile Code");

    // Test TaskResult saving
    let result = TaskResult {
        task_id: task_id.clone(),
        status: TaskStatus::Completed,
        summary: "Pipeline finished without warnings".to_string(),
        artifacts: vec!["target/release/app".to_string()],
        verifications: Vec::new(),
        duration_ms: 1200,
        engine_used: Some("mock".to_string()),
    };
    store.save_result(&result).await.unwrap();

    let loaded_res = store.get_result(&task_id).await.unwrap();
    assert!(loaded_res.is_some());
    assert_eq!(
        loaded_res.unwrap().summary,
        "Pipeline finished without warnings"
    );

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idempotent_submit_is_atomic_across_connections_and_restarts() {
    let ws = unique_temp_dir();
    let mut handles = Vec::new();
    for _ in 0..16 {
        let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
        let ws = ws.clone();
        handles.push(tokio::spawn(async move {
            TaskService::new(store, ws, 1, None, None)
                .submit_task_idempotent("title", "goal", Priority::Normal, "codex", "request-1")
                .await
                .unwrap()
        }));
    }
    let mut created = 0;
    let mut id = None;
    for handle in handles {
        let result = handle.await.unwrap();
        let current = match result {
            agent_runtime::IdempotentSubmission::Created(id) => {
                created += 1;
                id
            }
            agent_runtime::IdempotentSubmission::Existing(id) => id,
            agent_runtime::IdempotentSubmission::Conflict => panic!("unexpected conflict"),
        };
        if let Some(ref expected) = id {
            assert_eq!(expected, &current);
        } else {
            id = Some(current);
        }
    }
    assert_eq!(created, 1);
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 1, None, None);
    assert_eq!(
        service
            .submit_task_idempotent("title", "goal", Priority::Normal, "codex", "request-1")
            .await
            .unwrap(),
        agent_runtime::IdempotentSubmission::Existing(id.unwrap())
    );
    assert_eq!(
        service
            .submit_task_idempotent("title", "changed", Priority::Normal, "codex", "request-1")
            .await
            .unwrap(),
        agent_runtime::IdempotentSubmission::Conflict
    );
    assert_eq!(store.list_tasks(100, 0).await.unwrap().len(), 1);
    drop(service);
    drop(store);
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_claim_is_atomic_across_store_connections() {
    let ws = unique_temp_dir();
    let initial = SqliteTaskStore::new(&ws).unwrap();
    let task = Task::new("claim", "claim", Priority::Normal);
    initial.save_task(&task).await.unwrap();
    let mut handles = Vec::new();
    for _ in 0..12 {
        let store = SqliteTaskStore::new(&ws).unwrap();
        let id = task.id.clone();
        handles.push(tokio::spawn(async move {
            store.claim_queued(&id).await.unwrap().is_some()
        }));
    }
    let mut claims = 0;
    for handle in handles {
        if handle.await.unwrap() {
            claims += 1;
        }
    }
    assert_eq!(claims, 1);
    assert_eq!(
        initial.get_task(&task.id).await.unwrap().unwrap().status,
        TaskStatus::Executing
    );
    drop(initial);
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn restart_recovers_only_unstarted_tasks() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 1, None, None);
    let queued_id = service
        .submit_task_with_engine("queued", "test", Priority::Normal, "mock")
        .await
        .unwrap();
    let mut interrupted = Task::new("interrupted", "must not retry", Priority::Normal);
    interrupted.engine_name = Some("mock".into());
    interrupted.status = TaskStatus::Executing;
    store.save_task(&interrupted).await.unwrap();
    drop(service);
    drop(store);
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store, &ws, 1, None, None);
    service.recover_after_restart().await.unwrap();
    let interrupted = service.get_task(&interrupted.id).await.unwrap().unwrap();
    assert!(
        matches!(interrupted.status, TaskStatus::WaitingApproval { ref reason } if reason.contains("reconciliation"))
    );
    assert!(service
        .approve_task(&interrupted.id, Arc::new(PanickingEngine))
        .await
        .is_err());
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if service.get_result(&queued_id).await.unwrap().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(!matches!(
        service.get_task(&queued_id).await.unwrap().unwrap().status,
        TaskStatus::Queued
    ));
    drop(service);
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn test_task_service_submit_and_cancel() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store, &ws, 2, None, None);

    let task_id = service
        .submit_task("Batch Job", "Process items", Priority::Normal)
        .await
        .unwrap();

    let task = service.get_task(&task_id).await.unwrap();
    assert!(task.is_some());
    assert_eq!(task.unwrap().status, TaskStatus::Queued);

    // Cancel task
    let canceled = service.cancel_task(&task_id).await.unwrap();
    assert!(canceled);

    let task = service.get_task(&task_id).await.unwrap().unwrap();
    assert!(matches!(task.status, TaskStatus::Canceled { .. }));

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn cancelled_task_is_not_started_when_dispatched_later() {
    let ws = unique_temp_dir();
    let service = TaskService::new(
        Arc::new(SqliteTaskStore::new(&ws).unwrap()),
        &ws,
        1,
        None,
        None,
    );
    let id = service
        .submit_task("Cancelled", "Do not execute", Priority::Normal)
        .await
        .unwrap();
    service.cancel_task(&id).await.unwrap();
    service.dispatch(id.clone(), Arc::new(PanickingEngine));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(matches!(
        service.get_task(&id).await.unwrap().unwrap().status,
        TaskStatus::Canceled { .. }
    ));
    assert!(matches!(
        service.get_result(&id).await.unwrap().unwrap().status,
        TaskStatus::Canceled { .. }
    ));
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn test_ooda_verification_gate_blocks_completion_without_evidence() {
    let ws = unique_temp_dir();
    let engine = Arc::new(MockSuccessEngine);
    let mut guardrail = GuardrailPolicy::default_enterprise();
    guardrail.max_retries_on_error = 1;

    let scratchpad = ScratchpadManager::new(&ws);
    let durable = DurableMemoryStore::new(&ws);

    let ooda = AutonomousOodaLoop::new(
        engine,
        guardrail,
        scratchpad,
        durable,
        &ws.to_string_lossy(),
    );

    let mut task = Task::new("Generate Report", "Write final analysis", Priority::High);
    let mut st = SubTask::new(
        "ST-1",
        "Generate Report File",
        "echo done",
        "file: nonexistent_report.pdf", // Verification requires a file that will NOT be created
    );
    st.verification_spec = Some(VerificationSpec::FileExists {
        path: "nonexistent_report.pdf".to_string(),
        min_bytes: Some(10),
    });
    task.subtasks = vec![st];

    let result = ooda.run_goal_detailed(&mut task).await.unwrap();

    // Because the engine succeeded with exit 0, but verification failed,
    // the task MUST NOT be marked Completed!
    assert_ne!(
        result.status,
        TaskStatus::Completed,
        "Goal must not be marked Completed when verification fails!"
    );
    assert!(
        matches!(result.status, TaskStatus::Failed { .. }),
        "Goal should be marked Failed due to unverified results"
    );

    let _ = std::fs::remove_dir_all(&ws);
}

struct PanickingEngine;

#[async_trait]
impl ComputeEngine for PanickingEngine {
    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        panic!("Simulated critical compute engine crash!");
    }
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

#[tokio::test]
async fn test_task_service_dispatch_recovers_from_worker_panic() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).unwrap());
    let service = TaskService::new(store.clone(), &ws, 2, None, None);

    let task_id = service
        .submit_task("Panic Job", "This engine will panic", Priority::Normal)
        .await
        .unwrap();

    let engine = Arc::new(PanickingEngine);
    service.dispatch(task_id.clone(), engine);

    // Wait briefly for the background worker to execute and fail gracefully
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let task = service
        .get_task(&task_id)
        .await
        .unwrap()
        .expect("Task should exist");
    // Verify task did NOT remain stuck in Executing, but transitioned to Failed
    assert!(
        matches!(task.status, TaskStatus::Failed { ref error } if error.contains("panicked")),
        "Task status should transition to Failed on worker panic: {:?}",
        task.status
    );

    let _ = std::fs::remove_dir_all(&ws);
}
