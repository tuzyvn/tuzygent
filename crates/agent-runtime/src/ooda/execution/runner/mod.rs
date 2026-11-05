use super::SubtaskExecutor;
use agent_domain::{Task, TaskStatus, VerificationResult};
use std::path::Path;
use tracing::info;

pub mod helpers;
pub mod step;

pub use helpers::{build_engine_input, publish_subtask_executing, update_working_context};
pub use step::{
    execute_single_step, handle_subtask_execution_result, resolve_active_system_context,
    QueueExecutionOutcome, StepAction,
};

/// Thực thi hàng đợi subtask thích ứng (Act & Re-Act với dynamic backtracking)
pub async fn execute_queue(
    executor: &SubtaskExecutor,
    task: &mut Task,
    active_workdir: &Path,
    long_term_ctx: &str,
    all_verifications: &mut Vec<VerificationResult>,
    working_context: &mut String,
) -> Result<QueueExecutionOutcome, String> {
    let mut subtask_index = 0;
    let mut total_remediations_spawned = 0;

    while subtask_index < task.subtasks.len() {
        // Real-time cancellation check
        if let Some(ref cancel) = executor.cancellation_token {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                info!("🛑 [OODA] Task execution was canceled via cancellation token");
                task.status = TaskStatus::Canceled {
                    reason: "Canceled by user request".into(),
                };
                return Ok(QueueExecutionOutcome::Canceled {
                    reason: "Canceled by user request".into(),
                });
            }
        }

        // Resume support: skip already completed subtasks
        if let Some(current_subtask) = task.subtasks.get(subtask_index) {
            if current_subtask.status == TaskStatus::Completed {
                info!(
                    "⏩ [OODA] SubTask {} already completed in previous run, skipping.",
                    current_subtask.id
                );
                subtask_index += 1;
                continue;
            }
        } else {
            break;
        }

        let mut before_step = task.clone();
        before_step.subtasks[subtask_index].status = TaskStatus::Executing;
        executor.checkpoint(&before_step).await?;
        let action = tokio::select! {
            biased;
            _ = crate::ooda::wait_for_cancellation(&executor.cancellation_token) => {
                let reason = "Canceled by user request".to_string();
                if let Some(subtask) = task.subtasks.get_mut(subtask_index) {
                    subtask.status = TaskStatus::Canceled { reason: reason.clone() };
                }
                task.status = TaskStatus::Canceled { reason: reason.clone() };
                executor.checkpoint(task).await?;
                return Ok(QueueExecutionOutcome::Canceled { reason });
            }
            result = execute_single_step(
            executor,
            task,
            subtask_index,
            active_workdir,
            long_term_ctx,
            working_context,
            all_verifications,
            &mut total_remediations_spawned,
            ) => result?,
        };
        executor.checkpoint(task).await?;
        match action {
            StepAction::Advance => subtask_index += 1,
            StepAction::Backtrack => continue,
            StepAction::Halt(outcome) => return Ok(outcome),
        }
    }

    Ok(QueueExecutionOutcome::Completed)
}

impl SubtaskExecutor {
    /// Thực thi hàng đợi subtask thích ứng (Act & Re-Act với dynamic backtracking)
    pub async fn execute_queue(
        &self,
        task: &mut Task,
        active_workdir: &Path,
        long_term_ctx: &str,
        all_verifications: &mut Vec<VerificationResult>,
        working_context: &mut String,
    ) -> Result<QueueExecutionOutcome, String> {
        execute_queue(
            self,
            task,
            active_workdir,
            long_term_ctx,
            all_verifications,
            working_context,
        )
        .await
    }
}
