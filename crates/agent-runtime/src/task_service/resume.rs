use super::TaskService;
use agent_domain::{TaskCheckpoint, TaskId, TaskStatus};
use agent_engine::ComputeEngine;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tracing::info;

impl TaskService {
    /// Resumes an interrupted, waiting-approval, or canceled task safely from its last saved checkpoint.
    /// Completed subtasks are retained and skipped, while in-flight or interrupted subtasks are sanitized to Queued.
    pub async fn resume_task_from_checkpoint(
        &self,
        id: &TaskId,
        engine: Arc<dyn ComputeEngine>,
    ) -> Result<bool> {
        let Some(mut task) = self.store.get_task(id).await? else {
            return Ok(false);
        };

        if task.status == TaskStatus::Completed {
            return Ok(false);
        }

        let cp_opt = self.store.get_task_checkpoint(id).await?;
        let cp = match cp_opt {
            Some(mut cp) => {
                cp.verify_integrity_for_task(id)
                    .map_err(|e| anyhow!("Checkpoint integrity verification failed for task {id}: {e}"))?;
                cp.sanitize_for_resume();
                cp
            }
            None => {
                let mut cp = TaskCheckpoint::new(task.id.clone(), task.subtasks.clone());
                cp.sanitize_for_resume();
                cp
            }
        };

        task.subtasks = cp.subtasks.clone();
        self.store.save_checkpoint(&task.id, &task.subtasks).await?;

        task.status = TaskStatus::Queued;
        task.updated_at = chrono::Utc::now();
        self.store.save_task(&task).await?;

        self.dispatch(task.id.clone(), engine);
        info!("🔄 [TaskService] Task {} resumed from checkpoint and dispatched", id);
        Ok(true)
    }
}
