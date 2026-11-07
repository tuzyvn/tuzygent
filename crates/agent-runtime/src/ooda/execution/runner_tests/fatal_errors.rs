use super::fixtures::*;
use crate::ooda::execution::runner::QueueExecutionOutcome;
use agent_domain::{Priority, SubTask, Task, TaskStatus};
use agent_engine::EngineError;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn test_execute_queue_fatal_quota_or_auth_error_aborts_immediately(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_fatal_auth_quota");
    let workdir = env.path();
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |_, _| {
        call_count_clone.fetch_add(1, Ordering::SeqCst);
        Err(EngineError::ApiRequestFailed(
            "HTTP 429: API rate limit exceeded quota exhausted".to_string(),
        ))
    }));

    let executor = setup_test_executor(engine, None, workdir);
    let mut task = Task::new(
        "Quota Limited Task",
        "Will trigger rate limit",
        Priority::High,
    );
    task.subtasks.push(SubTask::new(
        "ST-QUOTA",
        "Heavy LLM step",
        "run inference",
        "exit 0",
    ));

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "system ctx", &mut verifs, &mut context)
        .await?;

    match outcome {
        QueueExecutionOutcome::Failed { error } => {
            assert!(error.contains("quota exhausted") || error.contains("rate limit"));
        }
        _ => return Err(format!("Expected Failed outcome, got: {:?}", outcome).into()),
    }

    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert_eq!(task.subtasks.len(), 1);
    assert!(matches!(task.status, TaskStatus::Failed { .. }));
    assert!(matches!(task.subtasks[0].status, TaskStatus::Failed { .. }));
    Ok(())
}
