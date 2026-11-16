use agent_domain::{GuardrailPolicy, Priority, SubTask, Task, VerificationResult};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use agent_runtime::{InFlightSupplementController, QueueExecutionOutcome, SubtaskExecutor};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

struct MockSuccessEngine;

#[async_trait]
impl ComputeEngine for MockSuccessEngine {
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }

    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        Ok(EngineOutput {
            session_id: None,
            stdout_content: "mock output".to_string(),
            exit_code: 0,
            execution_duration_ms: 5,
            engine_used: EngineKind::Mock,
        })
    }
}

struct MockFailingEngine;

#[async_trait]
impl ComputeEngine for MockFailingEngine {
    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }

    async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
        Ok(EngineOutput {
            session_id: None,
            stdout_content: "Connection refused on port 9090".to_string(),
            exit_code: 1,
            execution_duration_ms: 5,
            engine_used: EngineKind::Mock,
        })
    }
}

fn create_temp_executor() -> (SubtaskExecutor, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ooda_guard_test_{}", uuid::Uuid::new_v4().simple()));
    let _ = std::fs::create_dir_all(&dir);
    let guardrail = GuardrailPolicy::default();
    let scratchpad = ScratchpadManager::new(dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(dir.join("durable.db"));
    let supplement_ctrl = InFlightSupplementController::new(4);
    let engine = Arc::new(MockSuccessEngine);

    let executor = SubtaskExecutor::new(
        engine,
        guardrail,
        scratchpad,
        durable,
        dir.to_string_lossy().to_string(),
        supplement_ctrl,
        None,
        None,
        30,
    )
    .with_auto_approve(true);
    (executor, dir)
}

#[tokio::test]
async fn test_ooda_loop_guard_allows_distinct_subtasks() {
    let (executor, dir) = create_temp_executor();
    let mut task = Task::new("Distinct Steps", "Testing distinct subtasks", Priority::Normal);
    task.subtasks = vec![
        SubTask::new("ST-1", "Step 1", "echo alpha", "none"),
        SubTask::new("ST-2", "Step 2", "echo beta", "none"),
        SubTask::new("ST-3", "Step 3", "echo gamma", "none"),
    ];

    let mut verifications = Vec::<VerificationResult>::new();
    let mut working_context = String::new();

    let outcome = executor
        .execute_queue(
            &mut task,
            &dir,
            "system context",
            &mut verifications,
            &mut working_context,
        )
        .await
        .expect("queue execution error");

    assert_eq!(outcome, QueueExecutionOutcome::Completed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_ooda_loop_guard_halts_on_repeated_identical_subtasks() {
    let (executor, dir) = create_temp_executor();
    let mut task = Task::new("Looping Steps", "Testing identical repeated subtasks", Priority::Normal);
    // Create 6 identical subtasks
    task.subtasks = (1..=6)
        .map(|i| SubTask::new(&format!("ST-{i}"), "Identical Step", "run_exact_same_command --flag", "none"))
        .collect();

    let mut verifications = Vec::<VerificationResult>::new();
    let mut working_context = String::new();

    let outcome = executor
        .execute_queue(
            &mut task,
            &dir,
            "system context",
            &mut verifications,
            &mut working_context,
        )
        .await
        .expect("queue execution error");

    match outcome {
        QueueExecutionOutcome::Failed { error } => {
            assert!(
                error.contains("LoopGuard halted") || error.contains("repeated identical"),
                "Expected LoopGuard halt message, got: {error}"
            );
        }
        other => panic!("Expected QueueExecutionOutcome::Failed from LoopGuard, got: {:?}", other),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_ooda_loop_guard_halts_on_consecutive_failure_streak() {
    let dir = std::env::temp_dir().join(format!("ooda_fail_guard_test_{}", uuid::Uuid::new_v4().simple()));
    let _ = std::fs::create_dir_all(&dir);
    let mut guardrail = GuardrailPolicy::default();
    guardrail.max_retries_on_error = 3;
    let scratchpad = ScratchpadManager::new(dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(dir.join("durable.db"));
    let supplement_ctrl = InFlightSupplementController::new(4);
    let engine = Arc::new(MockFailingEngine);

    let executor = SubtaskExecutor::new(
        engine,
        guardrail,
        scratchpad,
        durable,
        dir.to_string_lossy().to_string(),
        supplement_ctrl,
        None,
        None,
        30,
    )
    .with_auto_approve(true);

    let mut task = Task::new("Failing Task", "Testing failure streak", Priority::Normal);
    task.subtasks = vec![
        SubTask::new("ST-1", "Step 1", "connect to dead port", "none"),
    ];

    let mut verifications = Vec::<VerificationResult>::new();
    let mut working_context = String::new();

    let outcome = executor
        .execute_queue(
            &mut task,
            &dir,
            "system context",
            &mut verifications,
            &mut working_context,
        )
        .await
        .expect("queue execution error");

    match outcome {
        QueueExecutionOutcome::Failed { error } => {
            assert!(
                error.contains("LoopGuard halted") || error.contains("failure") || error.contains("streak") || error.contains("Connection refused"),
                "Expected failure message, got: {error}"
            );
        }
        other => panic!("Expected QueueExecutionOutcome::Failed, got: {:?}", other),
    }

    let _ = std::fs::remove_dir_all(&dir);
}
