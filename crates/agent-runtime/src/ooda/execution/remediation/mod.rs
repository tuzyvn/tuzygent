pub mod parser;
pub mod retry;
pub mod types;

pub use parser::parse_remediation_json;
pub use retry::{attempt_remediation_retry, generate_remediation_subtask};
pub use types::{RemediationFailureKind, RemediationOutcome};

use super::SubtaskExecutor;
use agent_domain::{SubTask, Task};
use std::path::Path;

impl SubtaskExecutor {
    /// Tự động sinh Remediation SubTask khi gặp lỗi để tự sửa sai (Backtracking Re-Act)
    pub async fn generate_remediation_subtask(
        &self,
        task_title: &str,
        failed_subtask: &SubTask,
        error_context: &str,
        workdir: &Path,
        system_context: &str,
        retry_num: u32,
    ) -> Option<SubTask> {
        generate_remediation_subtask(
            self,
            task_title,
            failed_subtask,
            error_context,
            workdir,
            system_context,
            retry_num,
        )
        .await
    }

    /// Trích xuất và parse Remediation SubTask từ văn bản JSON phản hồi
    pub fn parse_remediation_json(
        raw: &str,
        failed_subtask: &SubTask,
        error_context: &str,
        retry_num: u32,
    ) -> Option<SubTask> {
        parse_remediation_json(raw, failed_subtask, error_context, retry_num)
    }

    /// Xử lý vòng lặp remediation retry và chèn backtracking task vào hàng đợi subtasks
    #[allow(clippy::too_many_arguments)]
    pub async fn attempt_remediation_retry(
        &self,
        task: &mut Task,
        subtask_index: usize,
        subtask: SubTask,
        err_msg: &str,
        failure_kind: RemediationFailureKind,
        active_workdir: &Path,
        active_system_context: &str,
        total_remediations_spawned: &mut usize,
    ) -> RemediationOutcome {
        attempt_remediation_retry(
            self,
            task,
            subtask_index,
            subtask,
            err_msg,
            failure_kind,
            active_workdir,
            active_system_context,
            total_remediations_spawned,
        )
        .await
    }
}

#[cfg(test)]
mod tests;
