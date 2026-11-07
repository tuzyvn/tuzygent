use agent_domain::{
    CheckpointMetadata, Priority, SubTask, Task, TaskId, TaskStatus, VerificationSpec,
};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_runtime::{SqliteTaskStore, TaskService, TaskStore};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn unique_temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("checkpoint_resume_test_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

struct MultiStepMockEngine {
    step1_count: Arc<AtomicUsize>,
    step2_count: Arc<AtomicUsize>,
    step3_count: Arc<AtomicUsize>,
    step2_started: Option<Arc<tokio::sync::Notify>>,
    interrupt_on_step2: bool,
}

#[async_trait]
impl ComputeEngine for MultiStepMockEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let workdir = std::path::Path::new(&input.working_dir);
        if input.prompt_or_command.contains("step1") {
            self.step1_count.fetch_add(1, Ordering::SeqCst);
            let _ = std::fs::write(workdir.join("step1.txt"), "step1_done");
            return Ok(EngineOutput {
                stdout_content: "step1 output".into(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            });
        }

        if input.prompt_or_command.contains("step2") {
            self.step2_count.fetch_add(1, Ordering::SeqCst);
            if let Some(ref notify) = self.step2_started {
                notify.notify_waiters();
            }
            if self.interrupt_on_step2 {
                // Simulate long-running or interrupted work
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let _ = std::fs::write(workdir.join("step2.txt"), "step2_done");
            return Ok(EngineOutput {
                stdout_content: "step2 output".into(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            });
        }

        if input.prompt_or_command.contains("step3") {
            self.step3_count.fetch_add(1, Ordering::SeqCst);
            let _ = std::fs::write(workdir.join("step3.txt"), "step3_done");
            return Ok(EngineOutput {
                stdout_content: "step3 output".into(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            });
        }

        Ok(EngineOutput {
            stdout_content: "default output".into(),
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

/// TDD Test Case: TOPIC-20261106-5a610b
/// Simulates process interruption mid-task during step 2, verifies checkpoint metadata persistence in SqliteTaskStore,
/// and verifies safe resumption where completed steps are skipped and in-flight steps resume properly.
#[tokio::test]
async fn test_in_flight_checkpointing_and_safe_resume_from_checkpoint() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).expect("store init"));

    let step1_count = Arc::new(AtomicUsize::new(0));
    let step2_count = Arc::new(AtomicUsize::new(0));
    let step3_count = Arc::new(AtomicUsize::new(0));
    let step2_started = Arc::new(tokio::sync::Notify::new());

    let initial_engine = Arc::new(MultiStepMockEngine {
        step1_count: step1_count.clone(),
        step2_count: step2_count.clone(),
        step3_count: step3_count.clone(),
        step2_started: Some(step2_started.clone()),
        interrupt_on_step2: true,
    });

    let service =
        Arc::new(TaskService::new(store.clone(), &ws, 2, None, None).with_auto_approve(true));

    let mut task = Task::new(
        "Long Running Data Pipeline",
        "Process data in 3 steps",
        Priority::High,
    );
    let mut st1 = SubTask::new(
        "ST-1",
        "Step 1 Data Extract",
        "run step1",
        "file: step1.txt",
    );
    st1.verification_spec = Some(VerificationSpec::FileExists {
        path: "step1.txt".into(),
        min_bytes: None,
    });
    let mut st2 = SubTask::new(
        "ST-2",
        "Step 2 Transformation",
        "run step2",
        "file: step2.txt",
    );
    st2.verification_spec = Some(VerificationSpec::FileExists {
        path: "step2.txt".into(),
        min_bytes: None,
    });
    let mut st3 = SubTask::new("ST-3", "Step 3 Validation", "run step3", "file: step3.txt");
    st3.verification_spec = Some(VerificationSpec::FileExists {
        path: "step3.txt".into(),
        min_bytes: None,
    });

    task.subtasks = vec![st1, st2, st3];
    let task_id: TaskId = task.id.clone();
    store.save_task(&task).await.expect("save task");

    // Dispatch the task with initial engine
    service.dispatch(task_id.clone(), initial_engine.clone());

    // Wait until step 2 begins execution
    tokio::time::timeout(Duration::from_secs(3), step2_started.notified())
        .await
        .expect("step 2 should have started");

    // Verify in-flight checkpoint metadata while step 2 is actively running
    let in_flight_meta: CheckpointMetadata = store
        .get_checkpoint_metadata(&task_id)
        .await
        .expect("query in-flight checkpoint metadata")
        .expect("in-flight checkpoint metadata must exist");
    assert_eq!(in_flight_meta.total_subtasks, 3);
    assert_eq!(in_flight_meta.completed_subtasks, 1);
    assert_eq!(
        in_flight_meta.last_completed_subtask_id.as_deref(),
        Some("ST-1")
    );
    assert_eq!(in_flight_meta.in_flight_subtask_id.as_deref(), Some("ST-2"));

    // Simulate abrupt shutdown / kill: cancel the active task to interrupt it
    let canceled = service.cancel_task(&task_id).await.expect("cancel task");
    assert!(canceled, "Task should be canceled to simulate interruption");

    // Wait for worker to exit
    let _ = service.drain(Duration::from_secs(2)).await;

    // Verify Checkpoint Metadata in SqliteTaskStore
    let cp_meta: CheckpointMetadata = store
        .get_checkpoint_metadata(&task_id)
        .await
        .expect("query checkpoint metadata")
        .expect("checkpoint metadata must exist");

    assert_eq!(cp_meta.total_subtasks, 3);
    assert_eq!(
        cp_meta.completed_subtasks, 1,
        "Only step 1 was completed before interruption"
    );
    assert_eq!(cp_meta.last_completed_subtask_id.as_deref(), Some("ST-1"));

    // Verify raw checkpoint subtasks
    let checkpoint_subtasks = store
        .get_checkpoint(&task_id)
        .await
        .expect("get checkpoint")
        .expect("checkpoint exists");
    assert_eq!(checkpoint_subtasks.len(), 3);
    assert_eq!(checkpoint_subtasks[0].status, TaskStatus::Completed);

    // Simulate server / daemon restart:
    // Create fresh TaskService instance with the same store
    drop(service);
    let restarted_service =
        Arc::new(TaskService::new(store.clone(), &ws, 2, None, None).with_auto_approve(true));
    restarted_service
        .recover_after_restart()
        .await
        .expect("recover after restart");

    // Check task status after restart recovery
    let task_after_restart = restarted_service
        .get_task(&task_id)
        .await
        .expect("get task")
        .expect("task exists");
    assert!(
        matches!(
            task_after_restart.status,
            TaskStatus::WaitingApproval { .. }
        ),
        "Interrupted task should be in WaitingApproval after restart, got: {:?}",
        task_after_restart.status
    );

    // Resume execution with fresh engine without interruption
    let resume_engine = Arc::new(MultiStepMockEngine {
        step1_count: step1_count.clone(),
        step2_count: step2_count.clone(),
        step3_count: step3_count.clone(),
        step2_started: None,
        interrupt_on_step2: false,
    });

    let resumed = restarted_service
        .resume_task_from_checkpoint(&task_id, resume_engine)
        .await
        .expect("resume_task_from_checkpoint must succeed");
    assert!(resumed, "Task must be resumed successfully");

    // Wait for resumed task to complete
    let final_result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(res) = restarted_service
                .get_result(&task_id)
                .await
                .expect("get result")
            {
                if matches!(
                    res.status,
                    TaskStatus::Completed | TaskStatus::Failed { .. }
                ) {
                    break res;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Resumed task must finish within timeout");

    assert_eq!(
        final_result.status,
        TaskStatus::Completed,
        "Resumed task must complete successfully"
    );

    // Critical assertion: step 1 was ALREADY completed before interruption, so it must NOT be re-executed!
    assert_eq!(
        step1_count.load(Ordering::SeqCst),
        1,
        "Step 1 must execute exactly once (skipped during resume)"
    );
    assert_eq!(
        step3_count.load(Ordering::SeqCst),
        1,
        "Step 3 must execute and complete"
    );

    // Check final metadata
    let final_meta = store
        .get_checkpoint_metadata(&task_id)
        .await
        .expect("get final metadata")
        .expect("final metadata exists");
    assert_eq!(final_meta.completed_subtasks, 3);
    assert_eq!(
        final_meta.last_completed_subtask_id.as_deref(),
        Some("ST-3")
    );
    assert_eq!(final_meta.in_flight_subtask_id, None);

    let _ = std::fs::remove_dir_all(&ws);
}

struct CrashOnStep2Engine {
    step1_count: Arc<AtomicUsize>,
    step2_count: Arc<AtomicUsize>,
    step3_count: Arc<AtomicUsize>,
    crash_on_step2: bool,
}

#[async_trait]
impl ComputeEngine for CrashOnStep2Engine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let workdir = std::path::Path::new(&input.working_dir);
        let cmd = input.prompt_or_command.trim();
        if cmd == "run step1" {
            let run = self.step1_count.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = std::fs::write(workdir.join("step1.txt"), format!("step1_run_{}", run));
            return Ok(EngineOutput {
                stdout_content: "step1 completed output".into(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            });
        }

        if cmd == "run step2" {
            self.step2_count.fetch_add(1, Ordering::SeqCst);
            if self.crash_on_step2 {
                let _ = std::fs::write(workdir.join("step2.txt"), "corrupt_partial_step2");
                panic!("Simulated abrupt worker crash during in-flight step 2");
            }
            let _ = std::fs::write(workdir.join("step2.txt"), "step2_clean_done");
            return Ok(EngineOutput {
                stdout_content: "step2 completed output".into(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            });
        }

        if cmd == "run step3" {
            self.step3_count.fetch_add(1, Ordering::SeqCst);
            let _ = std::fs::write(workdir.join("step3.txt"), "step3_clean_done");
            return Ok(EngineOutput {
                stdout_content: "step3 completed output".into(),
                exit_code: 0,
                execution_duration_ms: 10,
                engine_used: EngineKind::Mock,
                session_id: None,
            });
        }

        Ok(EngineOutput {
            stdout_content: "unrecoverable crash during step2".into(),
            exit_code: 1,
            execution_duration_ms: 5,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

/// Simulates an abrupt worker crash (panic) mid-flight during step 2 of a 3-step Long-Running Task,
/// verifying that:
/// 1. Step 1's completed state and Step 2's in-flight state are preserved in SQLite checkpoint storage.
/// 2. Resuming from the nearest checkpoint skips Step 1 (no duplicate execution or data overwrite),
///    sanitizes and re-runs Step 2 cleanly, and executes Step 3 to completion.
#[tokio::test]
async fn test_crash_abort_mid_flight_and_safe_resume_without_duplicate_execution() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).expect("store init"));

    let step1_count = Arc::new(AtomicUsize::new(0));
    let step2_count = Arc::new(AtomicUsize::new(0));
    let step3_count = Arc::new(AtomicUsize::new(0));

    let crashing_engine = Arc::new(CrashOnStep2Engine {
        step1_count: step1_count.clone(),
        step2_count: step2_count.clone(),
        step3_count: step3_count.clone(),
        crash_on_step2: true,
    });

    let service =
        Arc::new(TaskService::new(store.clone(), &ws, 2, None, None).with_auto_approve(true));

    let mut task = Task::new(
        "Crash Recovery Pipeline",
        "Long running task that crashes on step 2",
        Priority::High,
    );
    let mut st1 = SubTask::new("ST-1", "Step 1", "run step1", "file: step1.txt");
    st1.verification_spec = Some(VerificationSpec::FileExists {
        path: "step1.txt".into(),
        min_bytes: None,
    });
    let mut st2 = SubTask::new("ST-2", "Step 2", "run step2", "file: step2.txt");
    st2.verification_spec = Some(VerificationSpec::FileExists {
        path: "step2.txt".into(),
        min_bytes: None,
    });
    let mut st3 = SubTask::new("ST-3", "Step 3", "run step3", "file: step3.txt");
    st3.verification_spec = Some(VerificationSpec::FileExists {
        path: "step3.txt".into(),
        min_bytes: None,
    });

    task.subtasks = vec![st1, st2, st3];
    let task_id = task.id.clone();
    store.save_task(&task).await.expect("save task");

    service.dispatch(task_id.clone(), crashing_engine);

    // Wait for worker panic to record failure result
    let crashed_result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(res) = service.get_result(&task_id).await.expect("get result") {
                if matches!(res.status, TaskStatus::Failed { .. }) {
                    break res;
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("Crashed worker should record failure result");

    assert!(matches!(crashed_result.status, TaskStatus::Failed { .. }));

    // Verify checkpoint was preserved despite the worker crash on step 2
    let cp_meta = store
        .get_checkpoint_metadata(&task_id)
        .await
        .expect("query checkpoint metadata")
        .expect("checkpoint metadata must exist after crash");
    assert_eq!(cp_meta.total_subtasks, 3);
    assert_eq!(cp_meta.completed_subtasks, 1);
    assert_eq!(cp_meta.last_completed_subtask_id.as_deref(), Some("ST-1"));

    let cp_subtasks = store
        .get_checkpoint(&task_id)
        .await
        .expect("query checkpoint subtasks")
        .expect("checkpoint subtasks must exist");
    assert_eq!(cp_subtasks[0].status, TaskStatus::Completed);
    assert!(matches!(cp_subtasks[1].status, TaskStatus::Failed { .. }));
    assert_eq!(cp_subtasks[2].status, TaskStatus::Queued);

    // Resume from checkpoint with a healthy engine
    let healthy_engine = Arc::new(CrashOnStep2Engine {
        step1_count: step1_count.clone(),
        step2_count: step2_count.clone(),
        step3_count: step3_count.clone(),
        crash_on_step2: false,
    });

    let resumed = service
        .resume_task_from_checkpoint(&task_id, healthy_engine)
        .await
        .expect("resume_task_from_checkpoint must succeed");
    assert!(resumed);

    // Wait for resumed task to complete
    let final_result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(res) = service.get_result(&task_id).await.expect("get result") {
                if matches!(
                    res.status,
                    TaskStatus::Completed | TaskStatus::Failed { .. }
                ) {
                    break res;
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("Resumed task must finish within timeout");

    assert_eq!(final_result.status, TaskStatus::Completed);

    // Verify execution counts: Step 1 ran once, Step 2 ran twice (1 crash + 1 resume), Step 3 ran once
    assert_eq!(
        step1_count.load(Ordering::SeqCst),
        1,
        "Step 1 must not be re-executed"
    );
    assert_eq!(
        step2_count.load(Ordering::SeqCst),
        2,
        "Step 2 ran once before crash and once on resume"
    );
    assert_eq!(
        step3_count.load(Ordering::SeqCst),
        1,
        "Step 3 ran once on resume"
    );

    // Verify data integrity on disk
    assert_eq!(
        std::fs::read_to_string(ws.join("step1.txt")).expect("read step1.txt"),
        "step1_run_1"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("step2.txt")).expect("read step2.txt"),
        "step2_clean_done"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("step3.txt")).expect("read step3.txt"),
        "step3_clean_done"
    );

    let _ = std::fs::remove_dir_all(&ws);
}

/// Verifies that atomic checkpoint persistence synchronously updates both subtask_checkpoints and tasks table.
#[tokio::test]
async fn test_atomic_checkpoint_persistence_syncs_tasks_table() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).expect("store init"));

    let mut task = Task::new("Pipeline", "Test atomic persistence", Priority::High);
    let st1 = SubTask::new("ST-1", "Step 1", "run 1", "none");
    let st2 = SubTask::new("ST-2", "Step 2", "run 2", "none");
    task.subtasks = vec![st1.clone(), st2.clone()];
    let task_id = task.id.clone();
    store.save_task(&task).await.expect("save task");

    // Initially tasks table has st1 and st2 in Queued
    let loaded_initial = store.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(loaded_initial.subtasks[0].status, TaskStatus::Queued);

    // Now update checkpoint atomically with st1 Completed and st2 Executing
    let mut st1_completed = st1.clone();
    st1_completed.status = TaskStatus::Completed;
    st1_completed.output_excerpt = Some("st1 finished".into());

    let mut st2_executing = st2.clone();
    st2_executing.status = TaskStatus::Executing;

    let updated_subtasks = vec![st1_completed, st2_executing];
    store
        .save_checkpoint(&task_id, &updated_subtasks)
        .await
        .expect("save checkpoint atomically");

    // 1. Verify subtask_checkpoints table
    let checkpoint = store.get_task_checkpoint(&task_id).await.unwrap().unwrap();
    assert_eq!(checkpoint.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(checkpoint.subtasks[1].status, TaskStatus::Executing);
    assert!(checkpoint.verify_integrity().is_ok());

    // 2. Verify tasks table was atomically synced in the same transaction
    let loaded_after_cp = store.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(loaded_after_cp.subtasks.len(), 2);
    assert_eq!(loaded_after_cp.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(
        loaded_after_cp.subtasks[0].output_excerpt.as_deref(),
        Some("st1 finished")
    );
    assert_eq!(loaded_after_cp.subtasks[1].status, TaskStatus::Executing);

    let _ = std::fs::remove_dir_all(&ws);
}

/// Verifies that any tampering with checkpoint subtasks or metadata causes integrity verification failure,
/// and prevents resume_task_from_checkpoint from executing untrusted or corrupted states.
#[tokio::test]
async fn test_checkpoint_tampering_fails_integrity_check_and_prevents_unsafe_resume() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).expect("store init"));

    let step1_count = Arc::new(AtomicUsize::new(0));
    let step2_count = Arc::new(AtomicUsize::new(0));
    let step3_count = Arc::new(AtomicUsize::new(0));

    let engine = Arc::new(MultiStepMockEngine {
        step1_count: step1_count.clone(),
        step2_count: step2_count.clone(),
        step3_count: step3_count.clone(),
        step2_started: None,
        interrupt_on_step2: false,
    });

    let service =
        Arc::new(TaskService::new(store.clone(), &ws, 2, None, None).with_auto_approve(true));

    let mut task = Task::new(
        "Tamper Pipeline",
        "Pipeline to test integrity checks",
        Priority::High,
    );
    let mut st1 = SubTask::new("ST-1", "Step 1", "run step1", "file: step1.txt");
    st1.status = TaskStatus::Completed;
    st1.output_excerpt = Some("step1 output".into());
    let mut st2 = SubTask::new("ST-2", "Step 2", "run step2", "file: step2.txt");
    st2.status = TaskStatus::Executing;
    let st3 = SubTask::new("ST-3", "Step 3", "run step3", "file: step3.txt");

    task.subtasks = vec![st1.clone(), st2.clone(), st3.clone()];
    let task_id = task.id.clone();
    store.save_task(&task).await.expect("save task");
    store
        .save_checkpoint(&task_id, &task.subtasks)
        .await
        .expect("save valid checkpoint");

    // Verify initial checkpoint is intact and valid
    let initial_cp = store.get_task_checkpoint(&task_id).await.unwrap().unwrap();
    assert!(initial_cp.verify_integrity().is_ok());

    // Tamper with SQLite directly: modify subtask payload in subtasks_json without updating metadata_json
    {
        let db_path = store.db_path();
        let conn = rusqlite::Connection::open(db_path).expect("open raw sqlite");
        let mut tampered_subtasks = task.subtasks.clone();
        tampered_subtasks[1].command_or_prompt = "malicious payload injection".into();
        let tampered_json = serde_json::to_string(&tampered_subtasks).unwrap();
        conn.execute(
            "UPDATE subtask_checkpoints SET subtasks_json = ?1 WHERE task_id = ?2",
            rusqlite::params![tampered_json, task_id.0],
        )
        .expect("direct tamper execute");
    }

    // Check that get_task_checkpoint detects the tamper (ChecksumMismatch) and returns Err
    let cp_err = store.get_task_checkpoint(&task_id).await;
    assert!(
        cp_err.is_err(),
        "Loading tampered checkpoint must fail integrity check"
    );

    // Check that resume_task_from_checkpoint fails safely and refuses to run
    let resume_result = service
        .resume_task_from_checkpoint(&task_id, engine.clone())
        .await;
    assert!(
        resume_result.is_err(),
        "resume_task_from_checkpoint must reject tampered checkpoint"
    );
    let err_str = resume_result.unwrap_err().to_string();
    assert!(
        err_str.contains("integrity") || err_str.contains("Checksum mismatch"),
        "Error message should mention integrity: {}",
        err_str
    );

    // Step 2 and 3 must not have been executed because resume was rejected
    assert_eq!(step2_count.load(Ordering::SeqCst), 0);
    assert_eq!(step3_count.load(Ordering::SeqCst), 0);

    // Now restore a legitimate, valid checkpoint
    store
        .save_checkpoint(&task_id, &[st1, st2, st3])
        .await
        .expect("re-save valid checkpoint");

    // Valid resume should now succeed
    let resumed = service
        .resume_task_from_checkpoint(&task_id, engine)
        .await
        .expect("valid checkpoint must resume successfully");
    assert!(resumed);

    // Wait for task completion
    let final_result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(res) = service.get_result(&task_id).await.expect("get result") {
                if matches!(
                    res.status,
                    TaskStatus::Completed | TaskStatus::Failed { .. }
                ) {
                    break res;
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("Task must finish");

    assert_eq!(final_result.status, TaskStatus::Completed);
    assert_eq!(
        step1_count.load(Ordering::SeqCst),
        0,
        "Step 1 was already completed, skipped"
    );
    assert_eq!(step2_count.load(Ordering::SeqCst), 1);
    assert_eq!(step3_count.load(Ordering::SeqCst), 1);

    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test]
async fn test_task_service_replay_and_fork_from_checkpoint_execution() {
    let ws = unique_temp_dir();
    let store = Arc::new(SqliteTaskStore::new(&ws).expect("store init"));
    let service =
        Arc::new(TaskService::new(store.clone(), &ws, 2, None, None).with_auto_approve(true));

    let mut source_task = Task::new(
        "Completed 3-Step Pipeline",
        "Run 3 steps and then fork/replay from step 2",
        Priority::High,
    );
    let mut st1 = SubTask::new("ST-1", "Step 1", "run step1", "none");
    st1.status = TaskStatus::Completed;
    st1.output_excerpt = Some("step1 cached".into());
    let mut st2 = SubTask::new("ST-2", "Step 2", "run step2", "none");
    st2.status = TaskStatus::Completed;
    st2.output_excerpt = Some("step2 cached".into());
    let mut st3 = SubTask::new("ST-3", "Step 3", "run step3", "none");
    st3.status = TaskStatus::Completed;
    st3.output_excerpt = Some("step3 cached".into());

    source_task.status = TaskStatus::Completed;
    source_task.subtasks = vec![st1, st2, st3];
    let source_id = source_task.id.clone();
    store
        .save_task(&source_task)
        .await
        .expect("save source task");
    store
        .save_checkpoint(&source_id, &source_task.subtasks)
        .await
        .expect("save source checkpoint");

    // 1. Fork from Step 2 and dispatch execution
    let fork_step1_count = Arc::new(AtomicUsize::new(0));
    let fork_step2_count = Arc::new(AtomicUsize::new(0));
    let fork_step3_count = Arc::new(AtomicUsize::new(0));
    let fork_engine = Arc::new(MultiStepMockEngine {
        step1_count: fork_step1_count.clone(),
        step2_count: fork_step2_count.clone(),
        step3_count: fork_step3_count.clone(),
        step2_started: None,
        interrupt_on_step2: false,
    });

    let forked_task = service
        .fork_and_dispatch_from_checkpoint(
            &source_id,
            Some(2),
            Some("Forked Pipeline from Step 2".into()),
            None,
            fork_engine,
        )
        .await
        .expect("fork_and_dispatch_from_checkpoint");

    assert_ne!(forked_task.id, source_id);
    assert_eq!(forked_task.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(forked_task.subtasks[1].status, TaskStatus::Queued);
    assert_eq!(forked_task.subtasks[2].status, TaskStatus::Queued);

    let fork_result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(res) = service
                .get_result(&forked_task.id)
                .await
                .expect("get fork result")
            {
                if matches!(
                    res.status,
                    TaskStatus::Completed | TaskStatus::Failed { .. }
                ) {
                    break res;
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("Forked task must complete");

    assert_eq!(fork_result.status, TaskStatus::Completed);
    assert_eq!(
        fork_step1_count.load(Ordering::SeqCst),
        0,
        "Step 1 inherited as Completed, must not re-run"
    );
    assert_eq!(
        fork_step2_count.load(Ordering::SeqCst),
        1,
        "Step 2 rewound and executed once"
    );
    assert_eq!(
        fork_step3_count.load(Ordering::SeqCst),
        1,
        "Step 3 rewound and executed once"
    );

    // Original task must still be intact
    let orig = store.get_task(&source_id).await.unwrap().unwrap();
    assert_eq!(orig.status, TaskStatus::Completed);

    // 2. Replay original task in-place from Step 3
    let replay_step1_count = Arc::new(AtomicUsize::new(0));
    let replay_step2_count = Arc::new(AtomicUsize::new(0));
    let replay_step3_count = Arc::new(AtomicUsize::new(0));
    let replay_engine = Arc::new(MultiStepMockEngine {
        step1_count: replay_step1_count.clone(),
        step2_count: replay_step2_count.clone(),
        step3_count: replay_step3_count.clone(),
        step2_started: None,
        interrupt_on_step2: false,
    });

    let replayed = service
        .replay_task_from_checkpoint(&source_id, Some(3), replay_engine)
        .await
        .expect("replay_task_from_checkpoint");
    assert_eq!(replayed.id, source_id);

    let replay_result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(res) = service
                .get_result(&source_id)
                .await
                .expect("get replay result")
            {
                if matches!(
                    res.status,
                    TaskStatus::Completed | TaskStatus::Failed { .. }
                ) {
                    break res;
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("Replayed task must complete");

    assert_eq!(replay_result.status, TaskStatus::Completed);
    assert_eq!(
        replay_step1_count.load(Ordering::SeqCst),
        0,
        "Step 1 skipped"
    );
    assert_eq!(
        replay_step2_count.load(Ordering::SeqCst),
        0,
        "Step 2 skipped"
    );
    assert_eq!(
        replay_step3_count.load(Ordering::SeqCst),
        1,
        "Step 3 replayed once"
    );

    let _ = std::fs::remove_dir_all(&ws);
}
