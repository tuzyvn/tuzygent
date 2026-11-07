use super::fixtures::*;
use super::verification::{mock_ok, mock_remediation};
use crate::ooda::execution::runner::QueueExecutionOutcome;
use agent_domain::{GuardrailPolicy, Priority, SubTask, Task, TaskStatus};
use agent_engine::{EngineError, EngineKind, EngineOutput};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn test_execute_queue_fatal_cli_not_found_aborts_immediately(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_cli_not_found");
    let workdir = env.path();
    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = call_count.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |_input, _count| {
        call_count_clone.fetch_add(1, Ordering::SeqCst);
        Err(EngineError::CliNotFound(
            "claude CLI executable not found in PATH".into(),
        ))
    }));

    let executor = setup_test_executor(engine, None, workdir);

    let mut task = Task::new(
        "CLI Check Task",
        "Task needing missing CLI",
        Priority::Normal,
    );
    task.subtasks.push(SubTask::new(
        "ST-1",
        "Run claude code",
        "claude --version",
        "exit 0",
    ));

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "system ctx", &mut verifs, &mut context)
        .await?;

    match outcome {
        QueueExecutionOutcome::Failed { error } => {
            assert!(error.contains("CLI executable not found"));
        }
        _ => return Err(format!("Expected Failed outcome, got: {:?}", outcome).into()),
    }

    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert_eq!(task.subtasks.len(), 1);
    assert!(matches!(task.status, TaskStatus::Failed { .. }));
    assert!(matches!(task.subtasks[0].status, TaskStatus::Failed { .. }));
    Ok(())
}

#[tokio::test]
async fn test_execute_queue_recovers_from_engine_error_via_remediation(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_engine_error_recovery");
    let workdir = env.path();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |input, _| {
        if input
            .prompt_or_command
            .contains("Autonomous AI Problem Solver")
        {
            Ok(mock_remediation(
                "ST-SOCKET-FIX-1",
                "Clean broken daemon socket",
                "rm -f /tmp/test-daemon.sock",
            ))
        } else if input.prompt_or_command == "rm -f /tmp/test-daemon.sock" {
            Ok(mock_ok("Socket cleaned"))
        } else if attempts_clone.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(EngineError::ProcessFailed(
                "Socket connection failed: connection refused".to_string(),
            ))
        } else {
            Ok(mock_ok("Connected to daemon successfully"))
        }
    }));

    let executor = setup_test_executor(engine, None, workdir);
    let mut task = Task::new(
        "Daemon Connection Task",
        "Connect to local socket",
        Priority::Normal,
    );
    task.subtasks.push(SubTask::new(
        "ST-DAEMON",
        "Connect to daemon",
        "connect-daemon",
        "file: verified_fixture",
    ));

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "system ctx", &mut verifs, &mut context)
        .await?;

    assert_eq!(outcome, QueueExecutionOutcome::Completed);
    assert_eq!(task.subtasks.len(), 2);
    assert_eq!(task.subtasks[0].id, "ST-SOCKET-FIX-1");
    assert_eq!(task.subtasks[0].status, TaskStatus::Completed);
    assert!(task.subtasks[0].is_remediation());
    assert_eq!(task.subtasks[1].id, "ST-DAEMON");
    assert_eq!(task.subtasks[1].status, TaskStatus::Completed);
    assert_eq!(task.subtasks[1].retry_count, 1);

    let memory = executor.durable.load_long_term_context().await?;
    assert!(memory.contains("Self-Correction"));
    assert!(memory.contains("Clean broken daemon socket"));
    Ok(())
}

#[tokio::test]
async fn test_execute_queue_exhausts_retries_and_fails() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_retry_exhaustion");
    let workdir = env.path();

    let engine = Arc::new(CallbackMockEngine::new(|input, count| {
        if input
            .prompt_or_command
            .contains("Autonomous AI Problem Solver")
        {
            Ok(mock_remediation(
                &format!("ST-PERSIST-FIX-{}", count),
                "Apply candidate workaround",
                "apply-workaround",
            ))
        } else if input.prompt_or_command == "apply-workaround" {
            Ok(mock_ok("Workaround applied but underlying issue persists"))
        } else {
            Ok(EngineOutput {
                stdout_content: "Fatal unrecoverable syntax error in third-party library"
                    .to_string(),
                exit_code: 1,
                execution_duration_ms: 15,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        }
    }));

    let guardrail = GuardrailPolicy {
        max_retries_on_error: 1,
        ..Default::default()
    };
    let executor = setup_test_executor(engine, Some(guardrail), workdir);

    let mut task = Task::new("Doomed Task", "Persistent failing task", Priority::Normal);
    task.subtasks.push(SubTask::new(
        "ST-DOOMED",
        "Run doomed step",
        "compile-broken",
        "exit 0",
    ));

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "system ctx", &mut verifs, &mut context)
        .await?;

    match outcome {
        QueueExecutionOutcome::Failed { error } => assert!(error.contains("Exit code non-zero: 1")),
        _ => {
            return Err(
                format!("Expected QueueExecutionOutcome::Failed, got: {:?}", outcome).into(),
            )
        }
    }
    assert!(matches!(task.status, TaskStatus::Failed { .. }));
    let failed_subtask = task
        .subtasks
        .iter()
        .find(|s| s.id == "ST-DOOMED")
        .ok_or("subtask ST-DOOMED should exist")?;
    assert!(matches!(failed_subtask.status, TaskStatus::Failed { .. }));
    assert_eq!(failed_subtask.retry_count, 1);

    let memory = executor.durable.load_long_term_context().await?;
    assert!(memory.contains("Exhausted retries in subtask"));
    Ok(())
}
