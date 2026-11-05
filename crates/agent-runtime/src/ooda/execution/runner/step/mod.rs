pub mod context;
pub mod result_handler;
pub mod types;

pub use context::resolve_active_system_context;
pub use result_handler::handle_subtask_execution_result;
pub use types::{is_auth_or_quota_message, QueueExecutionOutcome, StepAction};

use super::helpers::{build_engine_input, publish_subtask_executing};
use crate::ooda::execution::safety::check_and_apply_guardrail_policy;
use crate::ooda::execution::SubtaskExecutor;
use agent_domain::{Task, TaskStatus, VerificationResult};
use std::path::Path;
use tracing::{info, info_span, Instrument};

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

    let act_span = info_span!(
        "ooda_phase",
        task_id = %task.id.0,
        subtask_id = %subtask.id,
        execution_phase = crate::ooda::OODA_PHASE_ACT,
    );

    let engine_input = build_engine_input(
        executor,
        task,
        &subtask,
        &active_system_context,
        active_workdir,
    );

    let execution_result = executor
        .execute_engine(&engine_input)
        .instrument(act_span.clone())
        .await;

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
    .instrument(act_span)
    .await
}

#[cfg(test)]
mod tests;
