use super::TaskService;
use agent_domain::TaskId;
use anyhow::Result;
use std::sync::Arc;

impl TaskService {
    /// Cancel all active and queued tasks tracked by this service across all pages
    pub async fn cancel_all(&self) -> Result<usize> {
        let mut canceled = 0;
        self.trigger_all_cancellation_flags();

        let mut offset = 0;
        let page_size = 100;
        loop {
            let tasks = self.store.list_tasks(page_size, offset).await?;
            if tasks.is_empty() {
                break;
            }
            offset += tasks.len();
            for task in tasks {
                if matches!(
                    task.status,
                    agent_domain::TaskStatus::Queued | agent_domain::TaskStatus::Executing
                ) {
                    if let Ok(true) = self.store.cancel_task(&task.id).await {
                        canceled += 1;
                    }
                }
            }
        }
        Ok(canceled)
    }

    /// Gracefully wait for all currently active tasks to complete or timeout
    pub async fn drain(&self, timeout: std::time::Duration) -> Result<bool> {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if self.active_task_count().await == 0 {
                return Ok(true);
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok(self.active_task_count().await == 0)
    }

    /// Drain active tasks; if timeout expires, cancel all active tasks and wait briefly
    pub async fn drain_or_cancel(&self, timeout: std::time::Duration) -> Result<bool> {
        if self.drain(timeout).await? {
            return Ok(true);
        }
        self.cancel_all().await?;
        self.drain(std::time::Duration::from_millis(500)).await
    }

    /// Retry a previously failed or canceled task
    pub async fn retry_task(
        &self,
        id: &TaskId,
        engine: Arc<dyn agent_engine::ComputeEngine>,
    ) -> Result<bool> {
        if let Some(mut task) = self.store.get_task(id).await? {
            if matches!(
                task.status,
                agent_domain::TaskStatus::Failed { .. } | agent_domain::TaskStatus::Canceled { .. }
            ) {
                task.status = agent_domain::TaskStatus::Queued;
                task.updated_at = chrono::Utc::now();
                self.store.save_task(&task).await?;
                self.dispatch(task.id.clone(), engine);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Retry all failed tasks in store, re-dispatching each with the given engine
    pub async fn retry_failed_tasks(
        &self,
        engine: Arc<dyn agent_engine::ComputeEngine>,
    ) -> Result<usize> {
        let mut retried = 0;
        let mut offset = 0;
        let page_size = 50;
        loop {
            let tasks = self.store.list_tasks(page_size, offset).await?;
            if tasks.is_empty() {
                break;
            }
            offset += tasks.len();
            for task in tasks {
                if matches!(task.status, agent_domain::TaskStatus::Failed { .. })
                    && self.retry_task(&task.id, engine.clone()).await?
                {
                    retried += 1;
                }
            }
        }
        Ok(retried)
    }

}
