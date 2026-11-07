use super::fixtures::*;
use crate::ooda::execution::runner::QueueExecutionOutcome;
use agent_domain::{Priority, SubTask, Task, TaskStatus, VerificationSpec};
use agent_engine::EngineError;
use std::sync::Arc;

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

#[tokio::test]
async fn test_execute_queue_verification_rejects_file_below_minimum_size(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_verif_min_bytes");
    let workdir = env.path();
    let artifact_path = workdir.join("undersized_artifact.txt");
    let artifact_path_clone = artifact_path.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |input, _| {
        if input
            .prompt_or_command
            .contains("Autonomous AI Problem Solver")
        {
            Ok(mock_ok("Unable to remediate the artifact."))
        } else {
            std::fs::write(&artifact_path_clone, "")
                .map_err(|error| EngineError::ProcessFailed(error.to_string()))?;
            Ok(mock_ok("Created an empty artifact."))
        }
    }));

    let executor = setup_test_executor(engine, None, workdir);
    let mut task = Task::new(
        "Minimum Size Verification Task",
        "Reject an artifact that is too small",
        Priority::Normal,
    );
    let mut subtask = SubTask::new(
        "ST-MIN-BYTES",
        "Create nonempty artifact",
        "create-artifact",
        "file: undersized_artifact.txt",
    );
    subtask.verification_spec = Some(VerificationSpec::FileExists {
        path: "undersized_artifact.txt".to_string(),
        min_bytes: Some(1),
    });
    task.subtasks.push(subtask);

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(&mut task, workdir, "ctx", &mut verifs, &mut context)
        .await?;

    assert!(matches!(outcome, QueueExecutionOutcome::Failed { .. }));
    assert_eq!(std::fs::metadata(artifact_path)?.len(), 0);
    assert_eq!(verifs.len(), 1);
    assert!(!verifs[0].passed);
    assert!(verifs[0].details.contains("1"));
    Ok(())
}

#[test]
fn test_execute_queue_verification_failure_without_remediation_fails(
) -> Result<(), Box<dyn std::error::Error>> {
    test_execute_queue_verification_failed()
}
