use agent_domain::{CheckpointMetadata, SubTask, TaskCheckpoint, TaskId, TaskStatus};
use agent_runtime::{SqliteTaskStore, TaskStore};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointListPayload {
    pub task_id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub metadata: CheckpointMetadata,
    pub subtasks: Vec<SubTask>,
}

pub async fn inspect_checkpoints(
    workspace: &Path,
    task_id_str: &str,
) -> Result<CheckpointListPayload> {
    let store =
        SqliteTaskStore::new(workspace).context("Failed to open SQLite task store in workspace")?;
    let task_id = TaskId(task_id_str.to_string());
    let task = store
        .get_task(&task_id)
        .await?
        .ok_or_else(|| anyhow!("Task not found with ID: {}", task_id_str))?;

    let cp = match store.get_task_checkpoint(&task_id).await? {
        Some(cp) => cp,
        None => TaskCheckpoint::new(task.id.clone(), task.subtasks.clone()),
    };

    Ok(CheckpointListPayload {
        task_id: task.id,
        title: task.title,
        status: task.status,
        metadata: cp.metadata,
        subtasks: cp.subtasks,
    })
}
