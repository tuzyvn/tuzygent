//! Checkpoint types and operations for task execution state.

mod error;
mod metadata;
mod task_checkpoint;

pub use error::CheckpointIntegrityError;
pub use metadata::CheckpointMetadata;
pub use task_checkpoint::TaskCheckpoint;

#[cfg(test)]
mod tests;
