use super::helpers::{build_engine_input, publish_subtask_executing, update_working_context};
use crate::ooda::execution::remediation::{
    attempt_remediation_retry, RemediationFailureKind, RemediationOutcome,
};
use crate::ooda::execution::safety::check_and_apply_guardrail_policy;
use crate::ooda::execution::verification::{
    execute_subtask_verification, finalize_subtask_completion, SubtaskVerificationOutcome,
};
use crate::ooda::execution::SubtaskExecutor;
use agent_domain::{SubTask, Task, TaskStatus, VerificationResult};
use agent_engine::{EngineError, EngineOutput};
use std::path::Path;
use tracing::{error, info, warn};

/// Outcome of running a subtask queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueExecutionOutcome {
    Completed,
    BlockedWaitingApproval { reason: String },
    Canceled { reason: String },
    Failed { error: String },
}

/// Returns true if the message indicates a fatal authentication or quota error.
pub fn is_auth_or_quota_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("quota exhausted")
        || lower.contains("quota exceeded")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("401 unauthorized")
        || lower.contains("403 forbidden")
        || lower.contains("invalid api key")
        || lower.contains("unauthorized")
        || lower.contains("insufficient_quota")
        || lower.contains("resource_exhausted")
        || lower.contains("402 payment")
        || lower.contains("model not found")
        || lower.contains("model_not_found")
        || lower.contains("billing_not_active")
        || lower.contains("credit balance")
}

/// Action to be taken by the runner loop after handling a subtask execution step.
#[derive(Debug, PartialEq, Eq)]
pub enum StepAction {
    /// Step finished successfully (verified and finalized); advance to next subtask.
    Advance,
    /// Backtracking subtask was inserted; repeat current loop iteration without advancing index.
    Backtrack,
    /// Step encountered a fatal error or unrecoverable failure; terminate queue execution.
    Halt(QueueExecutionOutcome),
}

impl From<RemediationOutcome> for StepAction {
    fn from(outcome: RemediationOutcome) -> Self {
        match outcome {
            RemediationOutcome::BacktrackingInserted => StepAction::Backtrack,
            RemediationOutcome::Failed(queue_outcome) => StepAction::Halt(queue_outcome),
        }
    }
}

/// Incorporates in-flight supplements into system context if available.
pub async fn resolve_active_system_context(
    executor: &SubtaskExecutor,
    long_term_ctx: &str,
    working_context: &str,
) -> String {
    let mut active_system_context = format!(
        "{}\n# Previous step results (reference data)\n{}",
        long_term_ctx, working_context
    );
    if let Some(sup) = executor.supplement_ctrl.try_recv().await {
        info!(
            "💡 [OODA] Incorporating In-Flight Supplement from: {}",
            sup.author_name
        );
        active_system_context.push_str("\n\n");
        active_system_context.push_str(&sup.to_instruction_block());
    }
    active_system_context
}

/// Executes a single subtask step including safety check, event emission, execution, and outcome resolution.
#[allow(clippy::too_many_arguments)]
pub async fn execute_single_step(
    executor: &SubtaskExecutor,
    task: &mut Task,
    subtask_index: usize,
    active_workdir: &Path,
    long_term_ctx: &str,
    working_context: &mut String,
    all_verifications: &mut Vec<VerificationResult>,
    total_remediations_spawned: &mut usize,
) -> Result<StepAction, String> {
    let mut subtask = match task.subtasks.get(subtask_index) {
        Some(s) => s.clone(),
        None => {
            let err = format!(
                "Subtask index {} out of bounds (subtasks len: {})",
                subtask_index,
                task.subtasks.len()
            );
            task.status = TaskStatus::Failed {
                error: err.clone(),
            };
            return Err(err);
        }
    };

    info!(
        "⚙️ [OODA] [{}/{}] Executing: {} ({})",
        subtask_index + 1,
        task.subtasks.len(),
        subtask.title,
        subtask.id
    );

    // Safety Guardrail Check (Red-line & Shell policy)
    if let Err(blocked_reason) = check_and_apply_guardrail_policy(
        &executor.guardrail,
        &executor.durable,
        task,
        subtask_index,
    )
    .await
    {
        return Ok(StepAction::Halt(
            QueueExecutionOutcome::BlockedWaitingApproval {
                reason: blocked_reason,
            },
        ));
    }

    subtask.status = TaskStatus::Executing;
    if let Some(slot) = task.subtasks.get_mut(subtask_index) {
        *slot = subtask.clone();
    }

    publish_subtask_executing(executor.bus.as_ref(), &task.id.0, &subtask);

    let active_system_context =
        resolve_active_system_context(executor, long_term_ctx, working_context).await;

    let engine_input = build_engine_input(
        executor,
        task,
        &subtask,
        &active_system_context,
        active_workdir,
    );

    let execution_result = executor.execute_engine(&engine_input).await;

    handle_subtask_execution_result(
        executor,
        task,
        subtask_index,
        subtask,
        execution_result,
        active_workdir,
        &active_system_context,
        working_context,
        all_verifications,
        total_remediations_spawned,
    )
    .await
}

/// Handles execution outcome for a single subtask, running verification or triggering remediation retry.
#[allow(clippy::too_many_arguments)]
pub async fn handle_subtask_execution_result(
    executor: &SubtaskExecutor,
    task: &mut Task,
    subtask_index: usize,
    mut subtask: SubTask,
    execution_result: Result<EngineOutput, EngineError>,
    active_workdir: &Path,
    active_system_context: &str,
    working_context: &mut String,
    all_verifications: &mut Vec<VerificationResult>,
    total_remediations_spawned: &mut usize,
) -> Result<StepAction, String> {
    match execution_result {
        Ok(output) if output.exit_code == 0 => {
            let excerpt: String = output.stdout_content.chars().take(2000).collect();
            update_working_context(working_context, &subtask.id, &excerpt);

            match execute_subtask_verification(&mut subtask, active_workdir, all_verifications).await
            {
                SubtaskVerificationOutcome::Passed { verif_results } => {
                    finalize_subtask_completion(
                        executor,
                        task,
                        subtask_index,
                        subtask,
                        excerpt,
                        verif_results,
                        output.execution_duration_ms,
                        active_workdir,
                    )
                    .await?;
                    Ok(StepAction::Advance)
                }
                SubtaskVerificationOutcome::Failed { error, .. } => {
                    let outcome = attempt_remediation_retry(
                        executor,
                        task,
                        subtask_index,
                        subtask,
                        &error,
                        RemediationFailureKind::Verification,
                        active_workdir,
                        active_system_context,
                        total_remediations_spawned,
                    )
                    .await;
                    Ok(outcome.into())
                }
            }
        }
        Ok(output) => {
            let err_msg = format!(
                "Exit code non-zero: {} | stdout: {}",
                output.exit_code,
                output.stdout_content.trim()
            );
            warn!("⚠️ [OODA] SubTask {} failed: {}", subtask.id, err_msg);

            if is_auth_or_quota_message(&err_msg) {
                warn!(
                    "🚨 [OODA] Fatal authentication or quota error: {}. Halting remediation.",
                    err_msg
                );
                subtask.status = TaskStatus::Failed {
                    error: err_msg.clone(),
                };
                if let Some(slot) = task.subtasks.get_mut(subtask_index) {
                    *slot = subtask;
                }
                task.status = TaskStatus::Failed {
                    error: err_msg.clone(),
                };
                return Ok(StepAction::Halt(QueueExecutionOutcome::Failed {
                    error: err_msg,
                }));
            }

            let outcome = attempt_remediation_retry(
                executor,
                task,
                subtask_index,
                subtask,
                &err_msg,
                RemediationFailureKind::ExitCode,
                active_workdir,
                active_system_context,
                total_remediations_spawned,
            )
            .await;
            Ok(outcome.into())
        }
        Err(err) => {
            let err_msg = err.to_string();
            error!("❌ [OODA] SubTask execution error: {}", err_msg);

            if matches!(err, EngineError::PolicyRejected(_) | EngineError::CliNotFound(_)) {
                warn!(
                    "🚨 [OODA] Engine policy or CLI setup error: {}. Halting remediation.",
                    err_msg
                );
                subtask.status = TaskStatus::Failed {
                    error: err_msg.clone(),
                };
                if let Some(slot) = task.subtasks.get_mut(subtask_index) {
                    *slot = subtask;
                }
                task.status = TaskStatus::Failed {
                    error: err_msg.clone(),
                };
                return Ok(StepAction::Halt(QueueExecutionOutcome::Failed {
                    error: err_msg,
                }));
            }

            if err.is_auth_or_quota() || is_auth_or_quota_message(&err_msg) {
                warn!(
                    "🚨 [OODA] Fatal authentication or quota error: {}. Halting remediation.",
                    err_msg
                );
                subtask.status = TaskStatus::Failed {
                    error: err_msg.clone(),
                };
                if let Some(slot) = task.subtasks.get_mut(subtask_index) {
                    *slot = subtask;
                }
                task.status = TaskStatus::Failed {
                    error: err_msg.clone(),
                };
                return Ok(StepAction::Halt(QueueExecutionOutcome::Failed {
                    error: err_msg,
                }));
            }

            let outcome = attempt_remediation_retry(
                executor,
                task,
                subtask_index,
                subtask,
                &err_msg,
                RemediationFailureKind::EngineError,
                active_workdir,
                active_system_context,
                total_remediations_spawned,
            )
            .await;
            Ok(outcome.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::InterAgentBus;
    use crate::supplement::{InFlightSupplement, InFlightSupplementController};
    use agent_domain::{GuardrailPolicy, Priority, SubTask, Task};
    use agent_engine::cli::MockEngine;
    use agent_memory::{DurableMemoryStore, ScratchpadManager};
    use std::sync::Arc;

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

    #[tokio::test]
    async fn test_resolve_active_system_context_with_supplement() -> Result<(), Box<dyn std::error::Error>> {
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
    async fn test_handle_subtask_execution_result_auth_quota_fatal_halt() -> Result<(), Box<dyn std::error::Error>> {
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

        let auth_err = EngineError::ApiRequestFailed("Invalid API key or Quota exceeded (401)".to_string());
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
    async fn test_handle_subtask_execution_result_policy_rejected_fatal_halt() -> Result<(), Box<dyn std::error::Error>> {
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

        let policy_err = EngineError::PolicyRejected("Command violates hard red-line security policy".to_string());
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
        assert_eq!(total_remediations, 0, "Policy violations must not spawn remediations");

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(())
    }
}
