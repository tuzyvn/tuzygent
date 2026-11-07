use super::TaskService;
use agent_domain::TaskId;
use anyhow::Result;
use tracing::info;

impl TaskService {
    /// Cancel a running or queued task
    pub async fn cancel_task(&self, id: &TaskId) -> Result<bool> {
        self.trigger_cancellation_flag(id);

        if self.store.cancel_task(id).await? {
            info!("🛑 [TaskService] Task {} canceled", id);
            return Ok(true);
        }

        Ok(false)
    }

    /// Delete a task from the store, canceling it first if active
    pub async fn delete_task(&self, id: &TaskId) -> Result<bool> {
        if self.is_active(id).await {
            let _ = self.cancel_task(id).await;
        }
        self.store.delete_task(id).await
    }
}
