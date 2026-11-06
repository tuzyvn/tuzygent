use super::TaskService;
use agent_domain::{Task, TaskCheckpoint, TaskId, TaskStatus};
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

    /// Rewinds and prepares an existing task for replay from a specific checkpoint step (1-based)
    /// without immediately dispatching background execution.
    pub async fn prepare_replay_from_checkpoint(
        &self,
        id: &TaskId,
        from_step: Option<usize>,
    ) -> Result<Task> {
        let Some(mut task) = self.store.get_task(id).await? else {
            return Err(anyhow!("Task not found: {}", id));
        };

        let cp_opt = self.store.get_task_checkpoint(id).await?;
        let mut cp = match cp_opt {
            Some(cp) => {
                cp.verify_integrity_for_task(id).map_err(|e| {
                    anyhow!("Checkpoint integrity verification failed for task {id}: {e}")
                })?;
                cp
            }
            None => TaskCheckpoint::new(task.id.clone(), task.subtasks.clone()),
        };

        if let Some(step) = from_step {
            cp.rewind_to_step(step)
                .map_err(|e| anyhow!("Failed to rewind checkpoint for task {id}: {e}"))?;
        } else {
            cp.sanitize_for_resume();
        }

        task.subtasks = cp.subtasks.clone();
        task.status = TaskStatus::Queued;
        task.failure_count = 0;
        task.updated_at = chrono::Utc::now();

        self.store.save_checkpoint(&task.id, &task.subtasks).await?;
        self.store.save_task(&task).await?;
        info!(
            "⏪ [TaskService] Task {} rewound for replay (from_step={:?})",
            id, from_step
        );
        Ok(task)
    }

    /// Rewinds an existing task to a specific checkpoint step and immediately dispatches it on `engine`.
    pub async fn replay_task_from_checkpoint(
        &self,
        id: &TaskId,
        from_step: Option<usize>,
        engine: Arc<dyn ComputeEngine>,
    ) -> Result<Task> {
        let task = self.prepare_replay_from_checkpoint(id, from_step).await?;
        self.dispatch(task.id.clone(), engine);
        Ok(task)
    }

    /// Forks a source task from its verified checkpoint (optionally rewound to `from_step`)
    /// into a brand new `Task` with an independent `TaskId`, persisting both the new task
    /// and its checkpoint without mutating the source task.
    pub async fn fork_task_from_checkpoint(
        &self,
        source_id: &TaskId,
        from_step: Option<usize>,
        new_title: Option<String>,
        new_goal: Option<String>,
    ) -> Result<Task> {
        let Some(source_task) = self.store.get_task(source_id).await? else {
            return Err(anyhow!("Source task not found: {}", source_id));
        };

        let cp_opt = self.store.get_task_checkpoint(source_id).await?;
        let cp = match cp_opt {
            Some(cp) => {
                cp.verify_integrity_for_task(source_id).map_err(|e| {
                    anyhow!("Checkpoint integrity verification failed for task {source_id}: {e}")
                })?;
                cp
            }
            None => TaskCheckpoint::new(source_task.id.clone(), source_task.subtasks.clone()),
        };

        let (forked_task, forked_cp) = source_task
            .fork_from_checkpoint(&cp, from_step, new_title, new_goal)
            .map_err(|e| anyhow!("Failed to fork task {source_id}: {e}"))?;

        self.store.save_task(&forked_task).await?;
        self.store
            .save_checkpoint(&forked_task.id, &forked_cp.subtasks)
            .await?;

        info!(
            "🌿 [TaskService] Forked task {} -> new task {} (from_step={:?})",
            source_id, forked_task.id, from_step
        );
        Ok(forked_task)
    }

    /// Forks a source task from its checkpoint and immediately dispatches the forked task on `engine`.
    pub async fn fork_and_dispatch_from_checkpoint(
        &self,
        source_id: &TaskId,
        from_step: Option<usize>,
        new_title: Option<String>,
        new_goal: Option<String>,
        engine: Arc<dyn ComputeEngine>,
    ) -> Result<Task> {
        let forked = self
            .fork_task_from_checkpoint(source_id, from_step, new_title, new_goal)
            .await?;
        self.dispatch(forked.id.clone(), engine);
        Ok(forked)
    }
}
