use super::fixtures::*;
use crate::ooda::execution::runner::QueueExecutionOutcome;
use agent_domain::{GuardrailPolicy, Priority, SubTask, Task, TaskStatus};
use agent_engine::cli::MockEngine;
use std::sync::Arc;

#[tokio::test]
async fn test_execute_queue_blocked_by_guardrail() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_guardrail_redline");
    let workdir = env.path();

    let mock_engine = Arc::new(MockEngine::new("ok"));
    let guardrail = GuardrailPolicy::default_enterprise();
    let executor = setup_test_executor(mock_engine, Some(guardrail), workdir);

    let mut task = Task::new("Dangerous Task", "A task with dangerous commands", Priority::High);
    task.subtasks.push(SubTask::new("1", "Remove root", "rm -rf data", "exit 0"));

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(
            &mut task,
            workdir,
            "context",
            &mut verifs,
            &mut context,
        )
        .await?;

    match &outcome {
        QueueExecutionOutcome::BlockedWaitingApproval { reason } => {
            assert!(reason.contains("rm -rf") || reason.contains("Recursive deletion"));
        }
        _ => return Err(format!("Expected BlockedWaitingApproval outcome, got: {:?}", outcome).into()),
    }

    assert!(matches!(task.status, TaskStatus::WaitingApproval { .. }));
    assert!(matches!(task.subtasks[0].status, TaskStatus::WaitingApproval { .. }));

    // Ensure lesson was saved in durable memory
    let memory = executor.durable.load_long_term_context().await?;
    assert!(memory.contains("Guardrail blocked command"));

    Ok(())
}

#[tokio::test]
async fn test_execute_queue_blocked_by_shell_policy() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_guardrail_shell");
    let workdir = env.path();

    let guardrail = GuardrailPolicy {
        allow_shell_execution: false,
        ..Default::default()
    };

    let executor = setup_test_executor(Arc::new(MockEngine::new("ok")), Some(guardrail), workdir);

    let mut task = Task::new("Shell Task", "Shell execution disabled", Priority::Normal);
    task.subtasks.push(SubTask::new("sh-1", "Run bash script", "bash deploy.sh", "exit 0"));

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "context", &mut verifs, &mut context)
        .await?;

    match outcome {
        QueueExecutionOutcome::BlockedWaitingApproval { reason } => {
            assert!(reason.contains("Shell execution is prohibited by guardrail policy"));
        }
        _ => return Err(format!("Expected BlockedWaitingApproval outcome, got: {:?}", outcome).into()),
    }

    assert!(matches!(task.status, TaskStatus::WaitingApproval { .. }));
    assert!(matches!(task.subtasks[0].status, TaskStatus::WaitingApproval { .. }));

    Ok(())
}
