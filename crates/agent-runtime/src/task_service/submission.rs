use super::TaskService;
use agent_domain::{Priority, Task, TaskId};
use anyhow::Result;
use tracing::info;

impl TaskService {
    /// Submit a new task, immediately recording it as Queued and returning the TaskId (HTTP 202 pattern)
    pub async fn submit_task(&self, title: &str, goal: &str, priority: Priority) -> Result<TaskId> {
        let task = Task::new(title, goal, priority);
        let id = task.id.clone();
        self.store.save_task(&task).await?;
        info!("📥 [TaskService] Task {} queued: '{}'", id, title);
        Ok(id)
    }

    /// Submit multiple tasks in batch
    pub async fn submit_batch(
        &self,
        tasks: Vec<(String, String, Priority)>,
    ) -> Result<Vec<TaskId>> {
        let mut ids = Vec::with_capacity(tasks.len());
        for (title, goal, priority) in tasks {
            ids.push(self.submit_task(&title, &goal, priority).await?);
        }
        Ok(ids)
    }

    pub async fn submit_task_idempotent(
        &self,
        title: &str,
        goal: &str,
        priority: Priority,
        engine: &str,
        key: &str,
    ) -> Result<crate::task_store::IdempotentSubmission> {
        let request = serde_json::to_string(&(title, goal, &priority, engine))?;
        let mut task = Task::new(title, goal, priority);
        task.engine_name = Some(engine.to_string());
        self.store.submit_idempotent(&task, key, &request).await
    }

    pub async fn submit_task_with_engine(
        &self,
        title: &str,
        goal: &str,
        priority: Priority,
        engine: &str,
    ) -> Result<TaskId> {
        let mut task = Task::new(title, goal, priority);
        task.engine_name = Some(engine.to_string());
        self.store.save_task(&task).await?;
        Ok(task.id)
    }
}
