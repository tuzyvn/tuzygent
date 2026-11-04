use agent_domain::{
    GuardrailPolicy, Priority, SubTask, Task, TaskResult, TaskStatus, VerificationSpec,
};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{
    AutonomousOodaLoop, SqliteTaskStore, TaskService, TaskStore, TaskVerifier,
};
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
    assert!(file_res.passed, "FileExists should pass: {}", file_res.details);

    let json_spec = VerificationSpec::JsonValid {
        path: "output.json".to_string(),
    };
    let json_res = verifier.verify_spec(&json_spec).await;
    assert!(json_res.passed, "JsonValid should pass: {}", json_res.details);

    let content_spec = VerificationSpec::ContentContains {
        path: "output.json".to_string(),
        pattern: "\"count\": 42".to_string(),
    };
    let content_res = verifier.verify_spec(&content_spec).await;
    assert!(content_res.passed, "ContentContains should pass: {}", content_res.details);

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
async fn test_sqlite_task_store_persistence_and_checkpoints() {
    let ws = unique_temp_dir();
    let store = SqliteTaskStore::new(&ws).unwrap();

    let task = Task::new("Deploy Microservice", "Run deployment pipeline", Priority::High);
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
    assert_eq!(loaded_res.unwrap().summary, "Pipeline finished without warnings");

    let _ = std::fs::remove_dir_all(&ws);
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
