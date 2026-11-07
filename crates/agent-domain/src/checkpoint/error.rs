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
