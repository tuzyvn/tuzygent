use super::*;
use crate::bus::InterAgentBus;
use crate::ooda::execution::remediation::RemediationOutcome;
use crate::supplement::{InFlightSupplement, InFlightSupplementController};
use agent_domain::{GuardrailPolicy, Priority, SubTask, Task, TaskStatus};
use agent_engine::cli::MockEngine;
use agent_engine::EngineError;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use std::sync::Arc;

#[test]
fn test_is_auth_or_quota_message_recognizes_fatal_markers_case_insensitively() {
    let fatal_messages = [
        "Quota exhausted",
        "quota exceeded",
        "Rate limit reached",
        "rate_limit_error",
        "401 Unauthorized",
        "403 forbidden",
        "Invalid API key",
        "request unauthorized",
        "insufficient_quota",
        "RESOURCE_EXHAUSTED",
        "402 payment required",
        "Model not found",
        "model_not_found",
        "billing_not_active",
        "credit balance is too low",
    ];

    for message in fatal_messages {
        assert!(
            is_auth_or_quota_message(message),
            "expected fatal marker to be recognized: {message}"
        );
    }

    assert!(!is_auth_or_quota_message("The request timed out"));
}

#[test]
fn test_step_action_from_remediation_outcome() {
    let inserted = RemediationOutcome::BacktrackingInserted;
    assert_eq!(StepAction::from(inserted), StepAction::Backtrack);

    let failed = RemediationOutcome::Failed(QueueExecutionOutcome::Failed {
        error: "max retries reached".to_string(),
    });
    assert_eq!(
        StepAction::from(failed),
        StepAction::Halt(QueueExecutionOutcome::Failed {
            error: "max retries reached".to_string(),
        })
    );
}

#[test]
fn test_queue_execution_outcome_clone_and_equality_preserve_variants() {
    let outcomes = [
        QueueExecutionOutcome::Completed,
        QueueExecutionOutcome::BlockedWaitingApproval {
            reason: "approval required".to_string(),
        },
        QueueExecutionOutcome::Canceled {
            reason: "canceled by user".to_string(),
        },
        QueueExecutionOutcome::Failed {
            error: "execution failed".to_string(),
        },
    ];

    for outcome in outcomes {
        let cloned = outcome.clone();
        assert_eq!(cloned, outcome);
    }

    assert_ne!(
        QueueExecutionOutcome::BlockedWaitingApproval {
            reason: "same text".to_string(),
        },
        QueueExecutionOutcome::Canceled {
            reason: "same text".to_string(),
        }
    );
}

#[tokio::test]
async fn test_resolve_active_system_context_with_supplement(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = std::env::temp_dir().join(format!("test_step_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let scratchpad = ScratchpadManager::new(temp_dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(temp_dir.join("durable.db"));
    let ctrl = InFlightSupplementController::new(5);

    let sup = InFlightSupplement::new("Architect", "Use postgres instead of sqlite");
    ctrl.inject(sup).await?;

    let executor = SubtaskExecutor::new(
        Arc::new(MockEngine::new("")),
        GuardrailPolicy::default_enterprise(),
        scratchpad,
        durable,
        temp_dir.to_string_lossy().to_string(),
        ctrl,
        None,
        Some(InterAgentBus::new(10)),
        30,
    );

    let resolved = resolve_active_system_context(&executor, "BASE_CONTEXT", "PREV_RESULTS").await;
    assert!(resolved.contains("BASE_CONTEXT"));
    assert!(resolved.contains("PREV_RESULTS"));
    assert!(resolved.contains("Architect"));
    assert!(resolved.contains("Use postgres instead of sqlite"));

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}

#[tokio::test]
async fn test_handle_subtask_execution_result_auth_quota_fatal_halt(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = std::env::temp_dir().join(format!("test_step_auth_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let scratchpad = ScratchpadManager::new(temp_dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(temp_dir.join("durable.db"));
    let ctrl = InFlightSupplementController::new(1);

    let executor = SubtaskExecutor::new(
        Arc::new(MockEngine::new("")),
        GuardrailPolicy::default_enterprise(),
        scratchpad,
        durable,
        temp_dir.to_string_lossy().to_string(),
        ctrl,
        None,
        None,
        30,
    );

    let mut task = Task::new("Auth Test", "Quota test", Priority::Normal);
    let subtask = SubTask::new("ST-1", "Test auth fail", "cmd", "none");
    task.subtasks.push(subtask.clone());

    let mut working_ctx = String::new();
    let mut verifs = Vec::new();
    let mut total_remediations = 0;

    let auth_err =
        EngineError::ApiRequestFailed("Invalid API key or Quota exceeded (401)".to_string());
    let result = handle_subtask_execution_result(
        &executor,
        &mut task,
        0,
        subtask,
        Err(auth_err),
        &temp_dir,
        "sys_ctx",
        &mut working_ctx,
        &mut verifs,
        &mut total_remediations,
    )
    .await?;

    assert!(matches!(
        result,
        StepAction::Halt(QueueExecutionOutcome::Failed { error }) if error.contains("Invalid API key")
    ));

    assert!(matches!(task.status, TaskStatus::Failed { .. }));
    assert!(matches!(task.subtasks[0].status, TaskStatus::Failed { .. }));

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}

#[tokio::test]
async fn test_execute_single_step_out_of_bounds_returns_err() {
    let temp_dir = std::env::temp_dir().join(format!("test_oob_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let engine = Arc::new(MockEngine::new(""));
    let scratchpad = ScratchpadManager::for_task(&temp_dir, "test");
    let durable = DurableMemoryStore::new(&temp_dir);
    let ctrl = InFlightSupplementController::new(10);
    let executor = SubtaskExecutor::new(
        engine,
        GuardrailPolicy::default_enterprise(),
        scratchpad,
        durable,
        temp_dir.to_string_lossy().to_string(),
        ctrl,
        None,
        None,
        30,
    );

    let mut task = Task::new("OOB Test", "Goal", Priority::Normal);
    let mut working_ctx = String::new();
    let mut verifs = Vec::new();
    let mut total_remediations = 0;

    let result = execute_single_step(
        &executor,
        &mut task,
        0,
        &temp_dir,
        "ctx",
        &mut working_ctx,
        &mut verifs,
        &mut total_remediations,
    )
    .await;

    assert!(result.is_err());
    let err = match result {
        Err(e) => e,
        Ok(_) => String::new(),
    };
    assert!(err.contains("out of bounds"));
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_handle_subtask_execution_result_policy_rejected_fatal_halt(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = std::env::temp_dir().join(format!("test_policy_halt_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let scratchpad = ScratchpadManager::new(temp_dir.join("scratchpad"));
    let durable = DurableMemoryStore::new(temp_dir.join("durable.db"));
    let ctrl = InFlightSupplementController::new(1);

    let executor = SubtaskExecutor::new(
        Arc::new(MockEngine::new("")),
        GuardrailPolicy::default_enterprise(),
        scratchpad,
        durable,
        temp_dir.to_string_lossy().to_string(),
        ctrl,
        None,
        None,
        30,
    );

    let mut task = Task::new("Policy Test", "Reject test", Priority::Normal);
    let subtask = SubTask::new("ST-1", "Test policy fail", "rm -rf /", "none");
    task.subtasks.push(subtask.clone());

    let mut working_ctx = String::new();
    let mut verifs = Vec::new();
    let mut total_remediations = 0;

    let policy_err =
        EngineError::PolicyRejected("Command violates hard red-line security policy".to_string());
    let result = handle_subtask_execution_result(
        &executor,
        &mut task,
        0,
        subtask,
        Err(policy_err),
        &temp_dir,
        "sys_ctx",
        &mut working_ctx,
        &mut verifs,
        &mut total_remediations,
    )
    .await?;

    assert!(matches!(
        result,
        StepAction::Halt(QueueExecutionOutcome::Failed { error }) if error.contains("Command violates hard red-line")
    ));

    assert!(matches!(task.status, TaskStatus::Failed { .. }));
    assert!(matches!(task.subtasks[0].status, TaskStatus::Failed { .. }));
    assert_eq!(
        total_remediations, 0,
        "Policy violations must not spawn remediations"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}
