use crate::task::{SubTask, TaskId, TaskStatus};
use serde::{Deserialize, Serialize};

/// Checkpoint metadata describing the execution progress of a task's subtasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointMetadata {
    pub task_id: TaskId,
    pub total_subtasks: usize,
    pub completed_subtasks: usize,
    pub last_completed_subtask_id: Option<String>,
    pub in_flight_subtask_id: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub checksum: Option<String>,
}

impl CheckpointMetadata {
    pub fn compute_checksum(subtasks: &[SubTask]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        if let Ok(bytes) = serde_json::to_vec(subtasks) {
            hasher.update(&bytes);
        }
        format!("{:x}", hasher.finalize())
    }

    pub fn new(task_id: TaskId, subtasks: &[SubTask]) -> Self {
        let total_subtasks = subtasks.len();
        let completed_subtasks = subtasks
            .iter()
            .filter(|s| s.status == TaskStatus::Completed)
            .count();
        let last_completed_subtask_id = subtasks
            .iter()
            .rev()
            .find(|s| s.status == TaskStatus::Completed)
            .map(|s| s.id.clone());
        let in_flight_subtask_id = subtasks
            .iter()
            .find(|s| {
                matches!(
                    s.status,
                    TaskStatus::Executing
                        | TaskStatus::PreparingSandbox
                        | TaskStatus::SelfChecking
                        | TaskStatus::Supplementing { .. }
                )
            })
            .map(|s| s.id.clone());
        let checksum = Some(Self::compute_checksum(subtasks));

        Self {
            task_id,
            total_subtasks,
            completed_subtasks,
            last_completed_subtask_id,
            in_flight_subtask_id,
            updated_at: chrono::Utc::now(),
            checksum,
        }
    }
}
