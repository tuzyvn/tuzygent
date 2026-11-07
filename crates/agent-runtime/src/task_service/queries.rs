use super::TaskService;
use agent_domain::{Task, TaskId, TaskResult};
use anyhow::Result;
use std::collections::HashMap;

impl TaskService {
    pub async fn is_healthy(&self) -> bool {
        self.store.is_healthy().await
    }

    pub async fn count_tasks(&self) -> Result<usize> {
        self.store.count_tasks().await
    }

    pub async fn get_task(&self, id: &TaskId) -> Result<Option<Task>> {
        self.store.get_task(id).await
    }

    pub async fn get_task_and_result(
        &self,
        id: &TaskId,
    ) -> Result<Option<(Task, Option<TaskResult>)>> {
        self.store.get_task_and_result(id).await
    }

    pub async fn get_result(&self, id: &TaskId) -> Result<Option<TaskResult>> {
        self.store.get_result(id).await
    }

    pub async fn list_tasks(&self, limit: usize, offset: usize) -> Result<Vec<Task>> {
        self.store.list_tasks(limit, offset).await
    }

    /// Provide a status count summary across common task states
    pub async fn status_summary(&self) -> Result<HashMap<String, usize>> {
        let mut summary = HashMap::new();
        let statuses = [
            "queued",
            "executing",
            "completed",
            "failed",
            "canceled",
            "waiting_approval",
            "quarantined",
        ];
        for status in statuses {
            let count = self.store.count_tasks_by_status(status).await.unwrap_or(0);
            summary.insert(status.to_string(), count);
        }
        Ok(summary)
    }
}
