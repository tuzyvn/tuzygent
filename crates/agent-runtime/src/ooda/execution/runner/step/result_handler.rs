use super::super::helpers::update_working_context;
use super::types::{is_auth_or_quota_message, QueueExecutionOutcome, StepAction};
use crate::ooda::execution::remediation::{attempt_remediation_retry, RemediationFailureKind};
use crate::ooda::execution::verification::{
    finalize_subtask_completion, SubtaskVerificationOutcome,
};
use crate::ooda::execution::SubtaskExecutor;
use agent_domain::{SubTask, Task, TaskStatus, VerificationResult};
use agent_engine::{EngineError, EngineOutput};
use std::path::Path;
use tracing::{error, warn};

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

            match executor
                .execute_subtask_verification(&mut subtask, active_workdir, all_verifications)
                .await
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
                SubtaskVerificationOutcome::NeedsReview {
                    reason,
                    verif_results,
                } => {
                    subtask.output_excerpt = Some(excerpt);
                    subtask.verification_result = verif_results.into_iter().find(|r| !r.passed);
                    subtask.status = TaskStatus::WaitingApproval {
                        reason: reason.clone(),
                    };
                    if let Some(slot) = task.subtasks.get_mut(subtask_index) {
                        *slot = subtask;
                    }
                    task.status = TaskStatus::WaitingApproval {
                        reason: reason.clone(),
                    };
                    Ok(StepAction::Halt(
                        QueueExecutionOutcome::BlockedWaitingApproval { reason },
                    ))
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

            if matches!(
                err,
                EngineError::PolicyRejected(_) | EngineError::CliNotFound(_)
            ) {
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
