use super::fixtures::*;
use crate::ooda::execution::runner::QueueExecutionOutcome;
use agent_domain::{Priority, SubTask, Task, TaskStatus, VerificationSpec};
use agent_engine::{EngineError, EngineKind, EngineOutput};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub(super) fn mock_remediation(id: &str, title: &str, cmd: &str) -> EngineOutput {
    let rem_json = serde_json::json!({
        "id": id,
        "title": title,
        "command_or_prompt": cmd,
        "expected_verification": "file: verified_fixture"
    });
    EngineOutput {
        stdout_content: rem_json.to_string(),
        exit_code: 0,
        execution_duration_ms: 10,
        engine_used: EngineKind::Mock,
        session_id: None,
    }
}

pub(super) fn mock_ok(stdout: &str) -> EngineOutput {
    EngineOutput {
        stdout_content: stdout.to_string(),
        exit_code: 0,
        execution_duration_ms: 10,
        engine_used: EngineKind::Mock,
        session_id: None,
    }
}

#[tokio::test]
async fn test_execute_queue_triggers_remediation_on_verification_failure(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_remediation");
    let workdir = env.path();
    let target_file = workdir.join("generated_artifact.txt");
    let target_file_clone = target_file.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |input, count| {
        if input
            .prompt_or_command
            .contains("Autonomous AI Problem Solver")
        {
            Ok(mock_remediation(
                "ST-FAIL-FIX-1",
                "Create artifact file",
                "generate-artifact",
            ))
        } else if input.prompt_or_command == "generate-artifact" {
            let _ = std::fs::write(&target_file_clone, "artifact content");
            Ok(mock_ok("Generated artifact file successfully"))
        } else {
            Ok(mock_ok(&format!("Normal execution attempt {}", count)))
        }
    }));

    let executor = setup_test_executor(engine, None, workdir);
    let mut task = Task::new(
        "Verification Task",
        "Task requiring file verification",
        Priority::Normal,
    );
    let mut st = SubTask::new(
        "ST-FAIL",
        "Produce artifact",
        "run producer",
        "file: generated_artifact.txt",
    );
    st.verification_spec = Some(VerificationSpec::FileExists {
        path: "generated_artifact.txt".to_string(),
        min_bytes: None,
    });
    task.subtasks.push(st);

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "system ctx", &mut verifs, &mut context)
        .await?;

    assert_eq!(outcome, QueueExecutionOutcome::Completed);
    assert_eq!(task.subtasks.len(), 2);
    assert_eq!(task.subtasks[0].id, "ST-FAIL-FIX-1");
    assert_eq!(task.subtasks[0].status, TaskStatus::Completed);
    assert!(task.subtasks[0].is_remediation());
    assert_eq!(task.subtasks[1].id, "ST-FAIL");
    assert_eq!(task.subtasks[1].status, TaskStatus::Completed);
    assert_eq!(task.subtasks[1].retry_count, 1);
    assert!(verifs.iter().any(|v| !v.passed) && verifs.iter().any(|v| v.passed));

    let memory_content = executor.durable.load_long_term_context().await?;
    assert!(memory_content.contains("Verification Self-Correction"));
    Ok(())
}

#[tokio::test]
async fn test_execute_queue_verification_failed() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_verif_exhausted");
    let workdir = env.path();

    let engine = Arc::new(CallbackMockEngine::new(|input, _| {
        if input
            .prompt_or_command
            .contains("Autonomous AI Problem Solver")
        {
            Ok(mock_ok("I do not know how to fix this."))
        } else {
            Ok(mock_ok("executed command"))
        }
    }));

    let executor = setup_test_executor(engine, None, workdir);
    let mut task = Task::new(
        "Failing Verification Task",
        "Will fail verification",
        Priority::Normal,
    );
    let mut st = SubTask::new(
        "ST-VERIF-FAIL",
        "Produce missing",
        "echo test",
        "file: never_exists.txt",
    );
    st.verification_spec = Some(VerificationSpec::FileExists {
        path: "never_exists.txt".to_string(),
        min_bytes: None,
    });
    task.subtasks.push(st);

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "ctx", &mut verifs, &mut context)
        .await?;

    match outcome {
        QueueExecutionOutcome::Failed { error } => assert!(error.contains("Verification failed")),
        _ => {
            return Err(
                format!("Expected QueueExecutionOutcome::Failed, got: {:?}", outcome).into(),
            )
        }
    }
    assert!(matches!(task.status, TaskStatus::Failed { .. }));
    assert!(matches!(task.subtasks[0].status, TaskStatus::Failed { .. }));
    Ok(())
}

#[test]
fn test_execute_queue_verification_failure_without_remediation_fails(
) -> Result<(), Box<dyn std::error::Error>> {
    test_execute_queue_verification_failed()
}

#[tokio::test]
async fn test_execute_queue_recovers_from_exit_code_failure_via_remediation(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_exit_code_recovery");
    let workdir = env.path();

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_clone = attempts.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |input, _| {
        if input
            .prompt_or_command
            .contains("Autonomous AI Problem Solver")
        {
            Ok(mock_remediation(
                "ST-BUILD-FIX-1",
                "Install missing build tools",
                "install-tools",
            ))
        } else if input.prompt_or_command == "install-tools" {
            Ok(mock_ok("Tools installed successfully"))
        } else if attempts_clone.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(EngineOutput {
                stdout_content: "Compilation error: missing toolchain".to_string(),
                exit_code: 1,
                execution_duration_ms: 15,
                engine_used: EngineKind::Mock,
                session_id: None,
            })
        } else {
            Ok(mock_ok("Build completed successfully"))
        }
    }));

    let executor = setup_test_executor(engine, None, workdir);
    let mut task = Task::new("Build Task", "Compile mission artifact", Priority::Normal);
    task.subtasks.push(SubTask::new(
        "ST-BUILD",
        "Build project",
        "compile-project",
        "file: verified_fixture",
    ));

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "system ctx", &mut verifs, &mut context)
        .await?;

    assert_eq!(outcome, QueueExecutionOutcome::Completed);
    assert_eq!(task.subtasks.len(), 2);
    assert_eq!(task.subtasks[0].id, "ST-BUILD-FIX-1");
    assert_eq!(task.subtasks[0].status, TaskStatus::Completed);
    assert!(task.subtasks[0].is_remediation());
    assert_eq!(task.subtasks[1].id, "ST-BUILD");
    assert_eq!(task.subtasks[1].status, TaskStatus::Completed);
    assert_eq!(task.subtasks[1].retry_count, 1);

    let memory = executor.durable.load_long_term_context().await?;
    assert!(memory.contains("Self-Correction"));
    assert!(memory.contains("Install missing build tools"));
    Ok(())
}

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

