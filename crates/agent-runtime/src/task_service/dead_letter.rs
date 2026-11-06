use super::TaskService;
use agent_domain::{Task, TaskId, TaskResult, TaskStatus};
use agent_engine::ComputeEngine;
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

pub const DEFAULT_POISON_THRESHOLD: usize = 3;

/// Helper function to evaluate task failure, increment failure counter,
/// and determine if the task should be quarantined to Dead-Letter Queue.
pub(crate) fn evaluate_failure_and_quarantine(
    task: &mut Task,
    poison_threshold: usize,
    err_msg: &str,
) -> (TaskStatus, String) {
    task.failure_count = task.failure_count.saturating_add(1);
    if task.failure_count >= poison_threshold {
        let reason = format!(
            "Poison task isolated to dead-letter queue ({} failures): {}",
            task.failure_count, err_msg
        );
        (TaskStatus::Quarantined { reason: reason.clone() }, reason)
    } else {
        (
            TaskStatus::Failed { error: err_msg.to_string() },
            format!("Task execution failed: {}", err_msg),
        )
    }
}

impl TaskService {
    /// Check whether a task is currently in Quarantined status
    pub async fn is_quarantined(&self, id: &TaskId) -> Result<bool> {
        match self.store.get_task(id).await? {
            Some(task) => Ok(task.is_quarantined()),
            None => Ok(false),
        }
    }

    /// Retrieve a quarantined task by ID, returning None if not found or not quarantined
    pub async fn get_quarantined_task(&self, id: &TaskId) -> Result<Option<Task>> {
        match self.store.get_task(id).await? {
            Some(task) if task.is_quarantined() => Ok(Some(task)),
            _ => Ok(None),
        }
    }

    /// List all quarantined tasks in Dead-Letter Queue (paginated)
    pub async fn list_quarantined_tasks(&self, limit: usize, offset: usize) -> Result<Vec<Task>> {
        self.store.list_tasks_by_status("quarantined", limit, offset).await
    }

    /// Count total number of tasks in Dead-Letter Queue
    pub async fn count_quarantined_tasks(&self) -> Result<usize> {
        self.store.count_tasks_by_status("quarantined").await
    }

    /// Explicitly isolate a task into Dead-Letter Queue
    pub async fn quarantine_task(&self, id: &TaskId, reason: &str) -> Result<bool> {
        self.trigger_cancellation_flag(id);
        if let Some(mut task) = self.store.get_task(id).await? {
            let reason_str = if reason.trim().is_empty() {
                "Manually quarantined by operator".to_string()
            } else {
                reason.trim().to_string()
            };
            task.status = TaskStatus::Quarantined {
                reason: reason_str.clone(),
            };
            task.updated_at = chrono::Utc::now();
            self.store.save_task(&task).await?;

            let result = TaskResult {
                task_id: id.clone(),
                status: task.status.clone(),
                summary: format!("Task quarantined: {}", reason_str),
                artifacts: Vec::new(),
                verifications: Vec::new(),
                duration_ms: 0,
                engine_used: task.engine_name.clone(),
            };
            let _ = self.store.save_result(&result).await;
            info!("☣️ [TaskService] Task {} isolated to Dead-Letter Queue: {}", id, reason_str);
            return Ok(true);
        }
        Ok(false)
    }

    /// Release a quarantined task from Dead-Letter Queue, resetting its failure counter and returning it to Queued
    pub async fn release_quarantined_task(
        &self,
        id: &TaskId,
        engine: Option<Arc<dyn ComputeEngine>>,
    ) -> Result<bool> {
        if let Some(mut task) = self.store.get_task(id).await? {
            if task.is_quarantined() {
                task.status = TaskStatus::Queued;
                task.failure_count = 0;
                task.updated_at = chrono::Utc::now();

                // If no subtasks completed, clear subtasks and checkpoint so the new engine
                // can cleanly decompose the goal afresh.
                // Otherwise reset incomplete subtasks to Queued for resumption.
                if !task.subtasks.iter().any(|st| st.status == TaskStatus::Completed) {
                    task.subtasks.clear();
                    let _ = self.store.save_checkpoint(id, &[]).await;
                } else {
                    for st in &mut task.subtasks {
                        if st.status != TaskStatus::Completed {
                            st.status = TaskStatus::Queued;
                        }
                    }
                    let _ = self.store.save_checkpoint(id, &task.subtasks).await;
                }

                self.store.save_task(&task).await?;
                info!("🟢 [TaskService] Task {} released from Dead-Letter Queue to Queued", id);
                if let Some(eng) = engine {
                    self.dispatch(id.clone(), eng);
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Batch release all quarantined tasks from Dead-Letter Queue back into Queued status
    pub async fn release_all_quarantined(
        &self,
        engine: Option<Arc<dyn ComputeEngine>>,
    ) -> Result<usize> {
        let mut released_count = 0;
        let page_size = 50;
        loop {
            let tasks = self.list_quarantined_tasks(page_size, 0).await?;
            if tasks.is_empty() {
                break;
            }
            let mut batch_released = 0;
            for task in tasks {
                if self.release_quarantined_task(&task.id, engine.clone()).await? {
                    released_count += 1;
                    batch_released += 1;
                }
            }
            if batch_released == 0 {
                break;
            }
        }
        Ok(released_count)
    }
}
