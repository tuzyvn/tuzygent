use super::{CheckpointIntegrityError, CheckpointMetadata};
use crate::task::{SubTask, TaskId, TaskStatus};
use serde::{Deserialize, Serialize};

/// Container pairing checkpoint metadata with the list of subtask states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskCheckpoint {
    pub metadata: CheckpointMetadata,
    pub subtasks: Vec<SubTask>,
}

impl TaskCheckpoint {
    pub fn new(task_id: TaskId, subtasks: Vec<SubTask>) -> Self {
        let metadata = CheckpointMetadata::new(task_id, &subtasks);
        Self { metadata, subtasks }
    }

    /// Reset any in-flight or non-completed subtask to Queued for safe resumption.
    pub fn sanitize_for_resume(&mut self) {
        for subtask in &mut self.subtasks {
            if subtask.status != TaskStatus::Completed {
                subtask.status = TaskStatus::Queued;
                subtask.verification_result = None;
                subtask.output_excerpt = None;
            }
        }
        self.metadata = CheckpointMetadata::new(self.metadata.task_id.clone(), &self.subtasks);
    }

    /// Rewinds checkpoint state to start replaying from `from_step` (1-based step index).
    /// Subtasks prior to `from_step` retain their `Completed` state and outputs (any non-completed
    /// prior step is sanitized to `Queued`), while subtasks from `from_step` onwards are reset to `Queued`.
    pub fn rewind_to_step(&mut self, from_step: usize) -> Result<(), CheckpointIntegrityError> {
        if from_step == 0 {
            return Err(CheckpointIntegrityError::CorruptedState(
                "Step index must be >= 1 (1-based)".to_string(),
            ));
        }
        if self.subtasks.is_empty() {
            if from_step == 1 {
                self.metadata =
                    CheckpointMetadata::new(self.metadata.task_id.clone(), &self.subtasks);
                return Ok(());
            }
            return Err(CheckpointIntegrityError::CorruptedState(format!(
                "Step {} is out of bounds for empty subtask list",
                from_step
            )));
        }
        if from_step > self.subtasks.len() {
            return Err(CheckpointIntegrityError::CorruptedState(format!(
                "Step {} is out of bounds (total subtasks: {})",
                from_step,
                self.subtasks.len()
            )));
        }

        let target_idx = from_step - 1;
        for (idx, subtask) in self.subtasks.iter_mut().enumerate() {
            if idx >= target_idx || subtask.status != TaskStatus::Completed {
                subtask.status = TaskStatus::Queued;
                subtask.verification_result = None;
                subtask.output_excerpt = None;
                subtask.retry_count = 0;
            }
        }
        self.metadata = CheckpointMetadata::new(self.metadata.task_id.clone(), &self.subtasks);
        Ok(())
    }

    /// Rewinds checkpoint state starting from the subtask matching `subtask_id`.
    pub fn rewind_to_subtask(
        &mut self,
        subtask_id: &str,
    ) -> Result<usize, CheckpointIntegrityError> {
        let Some(idx) = self.subtasks.iter().position(|s| s.id == subtask_id) else {
            return Err(CheckpointIntegrityError::CorruptedState(format!(
                "Subtask '{}' not found in checkpoint",
                subtask_id
            )));
        };
        let step = idx + 1;
        self.rewind_to_step(step)?;
        Ok(step)
    }

    /// Creates a new independent `TaskCheckpoint` bound to `new_task_id`, inheriting state
    /// from this checkpoint (optionally rewound to `from_step`).
    pub fn fork_for_task(
        &self,
        new_task_id: TaskId,
        from_step: Option<usize>,
    ) -> Result<Self, CheckpointIntegrityError> {
        let mut cloned_subtasks = self.subtasks.clone();
        let mut forked = Self {
            metadata: CheckpointMetadata::new(new_task_id, &cloned_subtasks),
            subtasks: std::mem::take(&mut cloned_subtasks),
        };
        if let Some(step) = from_step {
            forked.rewind_to_step(step)?;
        } else {
            forked.sanitize_for_resume();
        }
        Ok(forked)
    }

    /// Validates the integrity of the checkpoint metadata against the stored subtasks.
    pub fn verify_integrity(&self) -> Result<(), CheckpointIntegrityError> {
        if self.metadata.total_subtasks != self.subtasks.len() {
            return Err(CheckpointIntegrityError::CountMismatch {
                metadata_count: self.metadata.total_subtasks,
                actual_count: self.subtasks.len(),
            });
        }

        let actual_completed = self
            .subtasks
            .iter()
            .filter(|s| s.status == TaskStatus::Completed)
            .count();
        if self.metadata.completed_subtasks != actual_completed {
            return Err(CheckpointIntegrityError::CompletedMismatch {
                metadata_completed: self.metadata.completed_subtasks,
                actual_completed,
            });
        }

        let actual_last_completed = self
            .subtasks
            .iter()
            .rev()
            .find(|s| s.status == TaskStatus::Completed)
            .map(|s| s.id.clone());
        if self.metadata.last_completed_subtask_id != actual_last_completed {
            return Err(CheckpointIntegrityError::LastCompletedMismatch {
                metadata_last: self.metadata.last_completed_subtask_id.clone(),
                actual_last: actual_last_completed,
            });
        }

        if let Some(ref expected_checksum) = self.metadata.checksum {
            let computed = CheckpointMetadata::compute_checksum(&self.subtasks);
            if expected_checksum != &computed {
                return Err(CheckpointIntegrityError::ChecksumMismatch {
                    expected: expected_checksum.clone(),
                    computed,
                });
            }
        }

        Ok(())
    }

    /// Validates integrity and verifies that the checkpoint belongs to the expected task.
    pub fn verify_integrity_for_task(
        &self,
        expected_task_id: &TaskId,
    ) -> Result<(), CheckpointIntegrityError> {
        if &self.metadata.task_id != expected_task_id {
            return Err(CheckpointIntegrityError::TaskIdMismatch {
                expected: expected_task_id.0.clone(),
                found: self.metadata.task_id.0.clone(),
            });
        }
        self.verify_integrity()
    }

    /// Returns true if the checkpoint metadata and subtasks pass integrity checks.
    pub fn is_valid(&self) -> bool {
        self.verify_integrity().is_ok()
    }
}
