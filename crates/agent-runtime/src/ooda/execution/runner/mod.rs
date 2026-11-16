use super::SubtaskExecutor;
use agent_domain::{Task, TaskStatus, VerificationResult};
use std::path::Path;
use tracing::{info, warn};

pub mod helpers;
pub mod step;

pub use helpers::{build_engine_input, publish_subtask_executing, update_working_context};
pub use step::{
    compact_working_context_with_budget, estimate_tokens, execute_single_step,
    handle_subtask_execution_result, resolve_active_system_context,
    resolve_active_system_context_with_budget, QueueExecutionOutcome, StepAction,
    DEFAULT_WORKING_CONTEXT_TOKEN_BUDGET,
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

    let loop_config = crate::sentinel::ExecutionLoopConfig {
        max_total_turns: executor.guardrail.max_subtasks_per_goal.max(40),
        repeat_warn_at: 3,
        repeat_break_at: 5,
        consecutive_error_break_at: executor.guardrail.max_retries_on_error.max(3),
    };
    let mut loop_guard = crate::sentinel::ExecutionLoopGuard::new(loop_config);

    while subtask_index < task.subtasks.len() {
        // Sentinel Loop Guard: step turn budget enforcement
        if let crate::sentinel::LoopVerdict::Break { reason, .. } = loop_guard.step_turn() {
            warn!("🛑 [OODA LoopGuard] Turn budget limit exceeded: {}", reason);
            let err = format!("LoopGuard halted: {}", reason);
            task.status = TaskStatus::Failed { error: err.clone() };
            return Ok(QueueExecutionOutcome::Failed { error: err });
        }

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

            // Sentinel Loop Guard: detect repeated identical subtask commands
            let verdict = loop_guard.observe_tool_call(
                &task.id.0,
                "subtask_exec",
                &current_subtask.command_or_prompt,
            );
            match verdict {
                crate::sentinel::LoopVerdict::Break { reason, .. } => {
                    warn!("🛑 [OODA LoopGuard] Repeated identical subtask execution detected: {}", reason);
                    let err = format!("LoopGuard halted: {}", reason);
                    task.status = TaskStatus::Failed { error: err.clone() };
                    return Ok(QueueExecutionOutcome::Failed { error: err });
                }
                crate::sentinel::LoopVerdict::Warn { reason, .. } => {
                    warn!("⚠️ [OODA LoopGuard] {}", reason);
                }
                crate::sentinel::LoopVerdict::Allow { .. } => {}
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
            StepAction::Advance => {
                loop_guard.clear_failure_streak(&task.id.0);
                subtask_index += 1;
            }
            StepAction::Backtrack => {
                // If a subtask failed and remediation backtracking was inserted, observe failure streak
                if let Some(failed_subtask) = task.subtasks.get(subtask_index) {
                    if let Some(ref err) = failed_subtask.remediation_for_error {
                        let verdict = loop_guard.observe_failure(
                            &task.id.0,
                            "remediation_failure",
                            &failed_subtask.id,
                            err,
                        );
                        if let crate::sentinel::LoopVerdict::Break { reason, .. } = verdict {
                            warn!("🛑 [OODA LoopGuard] Consecutive failure streak limit reached: {}", reason);
                            let err_msg = format!("LoopGuard halted on failure streak: {}", reason);
                            task.status = TaskStatus::Failed { error: err_msg.clone() };
                            return Ok(QueueExecutionOutcome::Failed { error: err_msg });
                        }
                    }
                }
                continue;
            }
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
