use super::fixtures::*;
use crate::ooda::execution::runner::QueueExecutionOutcome;
use agent_domain::{Priority, SubTask, Task, TaskStatus};
use agent_engine::{EngineKind, EngineOutput};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn test_execute_queue_resumes_completed_subtask() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_resume");
    let workdir = env.path();

    let executed_commands = Arc::new(Mutex::new(Vec::new()));
    let exec_cmd_clone = executed_commands.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |input, _| {
        exec_cmd_clone
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(input.prompt_or_command.clone());
        Ok(EngineOutput {
            stdout_content: "Executed pending step".to_string(),
            exit_code: 0,
            execution_duration_ms: 10,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }));

    let executor = setup_test_executor(engine, None, workdir);

    let mut task = Task::new(
        "Resume Task",
        "Skip already completed subtasks",
        Priority::Normal,
    );

    // Subtask 1: Already completed from a previous run
    let mut subtask_1 = SubTask::new("1", "Done step", "echo step1", "Custom check");
    subtask_1.status = TaskStatus::Completed;
    subtask_1.output_excerpt = Some("previous output excerpt".to_string());
    task.subtasks.push(subtask_1);

    // Subtask 2: Pending/Queued step needing execution
    let subtask_2 = SubTask::new("2", "Pending step", "echo step2", "file: verified_fixture");
    task.subtasks.push(subtask_2);

    let mut verifs = Vec::new();
    let mut context = String::new();

    let outcome = executor
        .execute_queue(&mut task, workdir, "context", &mut verifs, &mut context)
        .await?;

    assert_eq!(outcome, QueueExecutionOutcome::Completed);

    // Subtask 1 should remain Completed with its original excerpt untouched
    assert_eq!(task.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(
        task.subtasks[0].output_excerpt.as_deref(),
        Some("previous output excerpt")
    );

    // Subtask 2 should be executed and marked Completed
    assert_eq!(task.subtasks[1].status, TaskStatus::Completed);
    assert_eq!(
        task.subtasks[1].output_excerpt.as_deref(),
        Some("Executed pending step")
    );

    // Ensure engine was called ONLY for Subtask 2, never for Subtask 1
    let cmds = executed_commands.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(cmds.as_slice(), &["echo step2"]);

    Ok(())
}

#[tokio::test]
async fn test_execute_queue_all_subtasks_already_completed(
) -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_all_done");
    let workdir = env.path();

    let executed = Arc::new(AtomicUsize::new(0));
    let executed_clone = executed.clone();

    let engine = Arc::new(CallbackMockEngine::new(move |_, _| {
        executed_clone.fetch_add(1, Ordering::SeqCst);
        Ok(EngineOutput {
            stdout_content: "should not be called".to_string(),
            exit_code: 0,
            execution_duration_ms: 1,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }));

    let executor = setup_test_executor(engine, None, workdir);

    let mut task = Task::new("All Done Task", "All subtasks done", Priority::Normal);
    let mut st1 = SubTask::new("1", "Step 1", "echo 1", "Custom");
    st1.status = TaskStatus::Completed;
    task.subtasks.push(st1);

    let mut st2 = SubTask::new("2", "Step 2", "echo 2", "Custom");
    st2.status = TaskStatus::Completed;
    task.subtasks.push(st2);

    let mut verifs = Vec::new();
    let mut context = String::new();

    let outcome = executor
        .execute_queue(&mut task, workdir, "context", &mut verifs, &mut context)
        .await?;

    assert_eq!(outcome, QueueExecutionOutcome::Completed);
    assert_eq!(executed.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn test_execute_queue_successful_execution() -> Result<(), Box<dyn std::error::Error>> {
    let env = TestEnv::new("test_runner_success");
    let workdir = env.path();

    let engine = Arc::new(CallbackMockEngine::new(|input, _| {
        Ok(EngineOutput {
            stdout_content: format!("Output of: {}", input.prompt_or_command),
            exit_code: 0,
            execution_duration_ms: 25,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }));

    let executor = setup_test_executor(engine, None, workdir);

    let mut task = Task::new(
        "Success Task",
        "Task that completes smoothly",
        Priority::Normal,
    );
    let st = SubTask::new(
        "ST-OK",
        "Create artifact",
        "make build",
        "file: verified_fixture",
    );
    task.subtasks.push(st);

    let mut verifs = Vec::new();
    let mut context = String::new();
    let outcome = executor
        .execute_queue(
            &mut task,
            workdir,
            "system prompt",
            &mut verifs,
            &mut context,
        )
        .await?;

    assert_eq!(outcome, QueueExecutionOutcome::Completed);
    assert_eq!(task.subtasks[0].status, TaskStatus::Completed);
    assert!(matches!(
        &task.subtasks[0].output_excerpt,
        Some(excerpt) if excerpt.contains("Output of: make build")
    ));
    assert!(matches!(
        &task.subtasks[0].verification_result,
        Some(verif) if verif.passed
    ));

    assert_eq!(verifs.len(), 1);
    assert!(verifs[0].passed);
    assert!(context.contains("ST-OK: Output of: make build"));

    // Verify scratchpad log was recorded
    let scratchpad_logs = executor.scratchpad.read().await?;
    assert!(scratchpad_logs.contains("SubTask ST-OK completed successfully in 25ms"));

    Ok(())
}
