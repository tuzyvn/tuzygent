use super::parser::parse_remediation_json;
use super::types::{RemediationFailureKind, RemediationOutcome};
use crate::ooda::execution::runner::QueueExecutionOutcome;
use crate::ooda::execution::{SubtaskExecutor, MAX_SUBTASK_RETRIES, MAX_TOTAL_REMEDIATIONS};
use agent_domain::{SubTask, Task, TaskStatus};
use agent_engine::EngineInput;
use std::path::Path;
use tracing::{info, info_span, Instrument};

/// Tự động sinh Remediation SubTask khi gặp lỗi để tự sửa sai (Backtracking Re-Act)
pub async fn generate_remediation_subtask(
    executor: &SubtaskExecutor,
    task_title: &str,
    failed_subtask: &SubTask,
    error_context: &str,
    workdir: &Path,
    system_context: &str,
    retry_num: u32,
) -> Option<SubTask> {
    let span = info_span!(
        "ooda_phase",
        task_id = "",
        subtask_id = %failed_subtask.id,
        execution_phase = crate::ooda::OODA_PHASE_REMEDIATE,
    );
    async move {
        info!(
            "🛠️ [OODA Self-Healing] Đang phân tích nguyên nhân lỗi để sinh bước khắc phục (Lần thử {}/{})...",
            retry_num, MAX_SUBTASK_RETRIES
        );

        let remediation_prompt = format!(
            "You are an Autonomous AI Problem Solver.\n\
            Overall Goal: {goal}\n\
            Failed Subtask ID: {subtask_id}\n\
            Failed Subtask Title: {subtask_title}\n\
            Failed Instruction: {cmd}\n\
            Error / Failure Details:\n{err}\n\n\
            Expected Verification: {verification}\n\n\
            Analyze why this step failed and formulate a concrete Remediation Subtask to fix the root cause.\n\
            Output ONLY a JSON object with keys:\n\
            - \"id\": \"{subtask_id}-FIX-{retry}\"\n\
            - \"title\": \"Remediation: concise fix title\"\n\
            - \"command_or_prompt\": \"specific corrective command or prompt\"\n\
            - \"expected_verification\": \"condition proving the fix worked\"\n\
            Do not include markdown preamble outside the json object.",
            goal = task_title,
            subtask_id = failed_subtask.id,
            subtask_title = failed_subtask.title,
            cmd = failed_subtask.command_or_prompt,
            err = error_context,
            verification = failed_subtask.expected_verification,
            retry = retry_num,
        );

        let input = EngineInput {
            prompt_or_command: remediation_prompt,
            system_context: system_context.to_string(),
            working_dir: workdir.to_string_lossy().to_string(),
            timeout_seconds: executor.timeout_seconds.min(60),
            on_stream_chunk: None,
            command_sandbox: None,
            on_event: None,
            temperature: None,
            max_tokens: None,
            max_output_bytes: Some(executor.guardrail.max_output_bytes),
        };

        if let Ok(out) = executor.execute_engine(&input).await {
            let raw = out.stdout_content.trim();
            if let Some(rem) = parse_remediation_json(raw, failed_subtask, error_context, retry_num) {
                info!(
                    "✨ [OODA Self-Healing] Đã tạo thành công Remediation SubTask: {} ({})",
                    rem.title, rem.id
                );
                return Some(rem);
            }
        }

        None
    }
    .instrument(span)
    .await
}

/// Xử lý vòng lặp remediation retry và chèn backtracking task vào hàng đợi subtasks
#[allow(clippy::too_many_arguments)]
pub async fn attempt_remediation_retry(
    executor: &SubtaskExecutor,
    task: &mut Task,
    subtask_index: usize,
    mut subtask: SubTask,
    err_msg: &str,
    failure_kind: RemediationFailureKind,
    active_workdir: &Path,
    active_system_context: &str,
    total_remediations_spawned: &mut usize,
) -> RemediationOutcome {
    let span = info_span!(
        "ooda_phase",
        task_id = %task.id.0,
        subtask_id = %subtask.id,
        execution_phase = crate::ooda::OODA_PHASE_REMEDIATE,
    );
    async move {
        let max_subtask_retries = executor.guardrail.max_retries_on_error as u32;

        if subtask.retry_count < max_subtask_retries
            && *total_remediations_spawned < MAX_TOTAL_REMEDIATIONS
        {
            subtask.retry_count += 1;
            *total_remediations_spawned += 1;
            if let Some(slot) = task.subtasks.get_mut(subtask_index) {
                *slot = subtask.clone();
            }

            let remediation = generate_remediation_subtask(
                executor,
                &task.title,
                &subtask,
                err_msg,
                active_workdir,
                active_system_context,
                subtask.retry_count,
            )
            .await;

            if let Some(rem_task) = remediation {
                let lesson = match failure_kind {
                    RemediationFailureKind::Verification => format!(
                        "Verification Self-Correction in '{}': step '{}' to fix {}",
                        task.title, rem_task.title, err_msg
                    ),
                    RemediationFailureKind::ExitCode | RemediationFailureKind::EngineError => format!(
                        "Self-Correction in '{}': Formulated step '{}' to fix {}",
                        task.title, rem_task.title, err_msg
                    ),
                };

                let _ = executor.durable.save_lesson(&lesson).await;
                subtask.status = TaskStatus::Queued;
                if let Some(slot) = task.subtasks.get_mut(subtask_index) {
                    *slot = subtask;
                }
                if subtask_index <= task.subtasks.len() {
                    task.subtasks.insert(subtask_index, rem_task);
                } else {
                    task.subtasks.push(rem_task);
                }
                return RemediationOutcome::BacktrackingInserted;
            }
        }

        // Khi vượt quá giới hạn retry hoặc không thể sinh remediation task
        let subtask_title = subtask.title.clone();
        subtask.status = TaskStatus::Failed {
            error: err_msg.to_string(),
        };
        if let Some(slot) = task.subtasks.get_mut(subtask_index) {
            *slot = subtask;
        }
        task.status = TaskStatus::Failed {
            error: err_msg.to_string(),
        };

        let lesson = format!(
            "Exhausted retries in subtask '{}' of task '{}' ({:?}): {}",
            subtask_title, task.title, failure_kind, err_msg
        );
        let _ = executor.durable.save_lesson(&lesson).await;

        RemediationOutcome::Failed(QueueExecutionOutcome::Failed {
            error: err_msg.to_string(),
        })
    }
    .instrument(span)
    .await
}
