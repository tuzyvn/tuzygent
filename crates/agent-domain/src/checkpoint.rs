use crate::task::{SubTask, TaskId, TaskStatus};
use serde::{Deserialize, Serialize};

/// Error types when validating the integrity of a task checkpoint.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CheckpointIntegrityError {
    #[error("Task ID mismatch: expected {expected}, found {found}")]
    TaskIdMismatch { expected: String, found: String },
    #[error("Subtask count mismatch: metadata says {metadata_count}, but found {actual_count}")]
    CountMismatch {
        metadata_count: usize,
        actual_count: usize,
    },
    #[error("Completed subtask count mismatch: metadata says {metadata_completed}, but found {actual_completed}")]
    CompletedMismatch {
        metadata_completed: usize,
        actual_completed: usize,
    },
    #[error("Last completed subtask ID mismatch: metadata says {metadata_last:?}, but found {actual_last:?}")]
    LastCompletedMismatch {
        metadata_last: Option<String>,
        actual_last: Option<String>,
    },
    #[error("Checksum mismatch: expected {expected}, computed {computed}")]
    ChecksumMismatch { expected: String, computed: String },
    #[error("Corrupted state: {0}")]
    CorruptedState(String),
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn completed_subtask(id: &str) -> SubTask {
        let mut subtask = SubTask::new(id, id, "run", "verify");
        subtask.status = TaskStatus::Completed;
        subtask.retry_count = 2;
        subtask.verification_result = Some(crate::task::VerificationResult {
            passed: true,
            spec: crate::task::VerificationSpec::Custom {
                description: "verified".into(),
            },
            details: "success".into(),
            executed_at: chrono::Utc::now(),
        });
        subtask.output_excerpt = Some(format!("output-{id}"));
        subtask
    }

    #[test]
    fn test_checkpoint_metadata_creation_and_sanitization() {
        let task_id = TaskId::new();
        let mut st1 = SubTask::new("st-1", "Step 1", "echo 1", "none");
        st1.status = TaskStatus::Completed;
        st1.output_excerpt = Some("step 1 completed output".into());
        let mut st2 = SubTask::new("st-2", "Step 2", "echo 2", "none");
        st2.status = TaskStatus::Executing;
        st2.output_excerpt = Some("partial step 2 output".into());
        let st3 = SubTask::new("st-3", "Step 3", "echo 3", "none");

        let mut cp = TaskCheckpoint::new(task_id.clone(), vec![st1, st2, st3]);
        assert_eq!(cp.metadata.total_subtasks, 3);
        assert_eq!(cp.metadata.completed_subtasks, 1);
        assert_eq!(
            cp.metadata.last_completed_subtask_id.as_deref(),
            Some("st-1")
        );
        assert_eq!(cp.metadata.in_flight_subtask_id.as_deref(), Some("st-2"));
        assert!(cp.metadata.checksum.is_some());
        assert!(cp.is_valid());
        assert!(cp.verify_integrity_for_task(&task_id).is_ok());

        cp.sanitize_for_resume();
        assert_eq!(cp.metadata.completed_subtasks, 1);
        assert_eq!(cp.metadata.in_flight_subtask_id, None);
        assert_eq!(cp.subtasks[0].status, TaskStatus::Completed);
        assert_eq!(
            cp.subtasks[0].output_excerpt.as_deref(),
            Some("step 1 completed output")
        );
        assert_eq!(cp.subtasks[1].status, TaskStatus::Queued);
        assert_eq!(cp.subtasks[1].output_excerpt, None);
        assert_eq!(cp.subtasks[2].status, TaskStatus::Queued);
        assert!(cp.is_valid());
    }

    #[test]
    fn test_checkpoint_integrity_tamper_detection() {
        let task_id = TaskId::new();
        let mut st1 = SubTask::new("st-1", "Step 1", "echo 1", "none");
        st1.status = TaskStatus::Completed;
        let st2 = SubTask::new("st-2", "Step 2", "echo 2", "none");

        let cp = TaskCheckpoint::new(task_id.clone(), vec![st1, st2]);
        assert!(cp.verify_integrity().is_ok());

        // Tamper with subtask count
        let mut tampered_count = cp.clone();
        tampered_count.metadata.total_subtasks = 5;
        match tampered_count.verify_integrity() {
            Err(CheckpointIntegrityError::CountMismatch {
                metadata_count,
                actual_count,
            }) => {
                assert_eq!(metadata_count, 5);
                assert_eq!(actual_count, 2);
            }
            other => panic!("Expected CountMismatch, got {:?}", other),
        }

        // Tamper with completed count
        let mut tampered_completed = cp.clone();
        tampered_completed.metadata.completed_subtasks = 2;
        match tampered_completed.verify_integrity() {
            Err(CheckpointIntegrityError::CompletedMismatch {
                metadata_completed,
                actual_completed,
            }) => {
                assert_eq!(metadata_completed, 2);
                assert_eq!(actual_completed, 1);
            }
            other => panic!("Expected CompletedMismatch, got {:?}", other),
        }

        // Tamper with payload (causes ChecksumMismatch)
        let mut tampered_payload = cp.clone();
        tampered_payload.subtasks[1].command_or_prompt = "malicious injection".into();
        match tampered_payload.verify_integrity() {
            Err(CheckpointIntegrityError::ChecksumMismatch { .. }) => {}
            other => panic!("Expected ChecksumMismatch, got {:?}", other),
        }

        // Tamper with task ID
        let other_id = TaskId::new();
        match cp.verify_integrity_for_task(&other_id) {
            Err(CheckpointIntegrityError::TaskIdMismatch { expected, found }) => {
                assert_eq!(expected, other_id.0);
                assert_eq!(found, task_id.0);
            }
            other => panic!("Expected TaskIdMismatch, got {:?}", other),
        }
    }

    #[test]
    fn rewind_to_step_replays_from_one_based_step_and_refreshes_metadata() {
        let task_id = TaskId::new();
        let first = completed_subtask("st-1");
        let mut second = SubTask::new("st-2", "Step 2", "run", "verify");
        second.status = TaskStatus::Executing;
        second.retry_count = 3;
        second.verification_result = completed_subtask("placeholder").verification_result;
        second.output_excerpt = Some("partial output".into());
        let third = completed_subtask("st-3");
        let mut cp = TaskCheckpoint::new(task_id.clone(), vec![first.clone(), second, third]);

        cp.rewind_to_step(2).unwrap();

        assert_eq!(cp.subtasks[0], first);
        for subtask in &cp.subtasks[1..] {
            assert_eq!(subtask.status, TaskStatus::Queued);
            assert_eq!(subtask.retry_count, 0);
            assert_eq!(subtask.verification_result, None);
            assert_eq!(subtask.output_excerpt, None);
        }
        assert_eq!(cp.metadata.task_id, task_id);
        assert_eq!(cp.metadata.total_subtasks, 3);
        assert_eq!(cp.metadata.completed_subtasks, 1);
        assert_eq!(
            cp.metadata.last_completed_subtask_id.as_deref(),
            Some("st-1")
        );
        assert_eq!(
            cp.metadata.checksum.as_deref(),
            Some(CheckpointMetadata::compute_checksum(&cp.subtasks).as_str())
        );
        assert!(cp.verify_integrity().is_ok());
    }

    #[test]
    fn rewind_to_step_validates_boundaries_and_handles_empty_checkpoint() {
        let mut cp = TaskCheckpoint::new(
            TaskId::new(),
            vec![completed_subtask("st-1"), completed_subtask("st-2")],
        );
        for invalid_step in [0, 3] {
            assert!(matches!(
                cp.rewind_to_step(invalid_step),
                Err(CheckpointIntegrityError::CorruptedState(_))
            ));
        }
        assert_eq!(cp.subtasks[0].status, TaskStatus::Completed);

        let mut empty = TaskCheckpoint::new(TaskId::new(), vec![]);
        assert!(matches!(
            empty.rewind_to_step(0),
            Err(CheckpointIntegrityError::CorruptedState(_))
        ));
        empty.rewind_to_step(1).unwrap();
        assert!(empty.subtasks.is_empty());
        assert!(empty.verify_integrity().is_ok());
        assert!(matches!(
            empty.rewind_to_step(2),
            Err(CheckpointIntegrityError::CorruptedState(_))
        ));
    }

    #[test]
    fn rewind_to_subtask_returns_one_based_step_and_reports_missing_id() {
        let mut cp = TaskCheckpoint::new(
            TaskId::new(),
            vec![completed_subtask("st-1"), completed_subtask("st-2")],
        );

        assert_eq!(cp.rewind_to_subtask("st-2").unwrap(), 2);
        assert_eq!(cp.subtasks[0].status, TaskStatus::Completed);
        assert_eq!(cp.subtasks[1].status, TaskStatus::Queued);
        assert!(cp.verify_integrity().is_ok());
        assert!(matches!(
            cp.rewind_to_subtask("unknown"),
            Err(CheckpointIntegrityError::CorruptedState(message))
                if message.contains("unknown")
        ));
    }

    #[test]
    fn fork_for_task_rewinds_or_sanitizes_independent_copy() {
        let source_task_id = TaskId::new();
        let mut in_flight = SubTask::new("st-2", "Step 2", "run", "verify");
        in_flight.status = TaskStatus::Executing;
        in_flight.retry_count = 4;
        in_flight.output_excerpt = Some("partial output".into());
        let source = TaskCheckpoint::new(
            source_task_id.clone(),
            vec![completed_subtask("st-1"), in_flight],
        );

        let fork_task_id = TaskId::new();
        let fork = source.fork_for_task(fork_task_id.clone(), Some(2)).unwrap();
        assert_eq!(fork.metadata.task_id, fork_task_id);
        assert_eq!(fork.subtasks[0].status, TaskStatus::Completed);
        assert_eq!(fork.subtasks[1].status, TaskStatus::Queued);
        assert!(fork.verify_integrity().is_ok());

        let sanitized_task_id = TaskId::new();
        let sanitized = source
            .fork_for_task(sanitized_task_id.clone(), None)
            .unwrap();
        assert_eq!(sanitized.metadata.task_id, sanitized_task_id);
        assert_eq!(sanitized.subtasks[0].status, TaskStatus::Completed);
        assert_eq!(sanitized.subtasks[1].status, TaskStatus::Queued);
        assert!(sanitized.verify_integrity().is_ok());

        let mut changed_fork = fork;
        changed_fork.subtasks[0].output_excerpt = Some("fork-only change".into());
        assert_ne!(changed_fork.subtasks, source.subtasks);
        assert_eq!(source.metadata.task_id, source_task_id);
        assert_eq!(source.subtasks[0].status, TaskStatus::Completed);
        assert!(matches!(
            source.fork_for_task(TaskId::new(), Some(0)),
            Err(CheckpointIntegrityError::CorruptedState(_))
        ));
    }

    #[test]
    fn verify_integrity_reports_last_completed_subtask_id_mismatch() {
        let mut cp = TaskCheckpoint::new(
            TaskId::new(),
            vec![
                completed_subtask("st-1"),
                SubTask::new("st-2", "Step 2", "run", "verify"),
            ],
        );
        cp.metadata.last_completed_subtask_id = Some("wrong-id".into());

        assert_eq!(
            cp.verify_integrity(),
            Err(CheckpointIntegrityError::LastCompletedMismatch {
                metadata_last: Some("wrong-id".into()),
                actual_last: Some("st-1".into()),
            })
        );
    }
}
