pub mod dead_letter;
pub mod dispatch;
pub mod lifecycle;
pub mod resume;
pub mod review;
pub mod scheduler;
mod worker;
#[cfg(test)]
mod tests;

pub use dead_letter::DEFAULT_POISON_THRESHOLD;
pub use scheduler::SchedulerTickReport;

use crate::sandbox::SandboxConfig;
use crate::task_store::TaskStore;
use agent_domain::{GuardrailPolicy, Priority, Task, TaskId, TaskResult};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::Semaphore;
use tracing::info;

/// TaskService manages task lifecycles with asynchronous background dispatch,
/// persistence, checkpointing, concurrency limits, cancellation, and human-in-the-loop approvals.
pub struct TaskService {
    pub(crate) store: Arc<dyn TaskStore>,
    pub(crate) concurrency_limiter: Arc<Semaphore>,
    pub(crate) cancellation_flags: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
    pub(crate) workspace_dir: PathBuf,
    pub(crate) guardrail: GuardrailPolicy,
    pub(crate) sandbox_config: Option<SandboxConfig>,
    pub(crate) max_concurrency: usize,
    pub(crate) poison_threshold: usize,
    pub(crate) auto_approve: bool,
}

impl TaskService {
    pub fn new(
        store: Arc<dyn TaskStore>,
        workspace_dir: impl AsRef<Path>,
        max_concurrency: usize,
        guardrail: Option<GuardrailPolicy>,
        sandbox_config: Option<SandboxConfig>,
    ) -> Self {
        let max_concurrency = max_concurrency.max(1);
        Self {
            store,
            concurrency_limiter: Arc::new(Semaphore::new(max_concurrency)),
            cancellation_flags: Arc::new(RwLock::new(HashMap::new())),
            workspace_dir: workspace_dir.as_ref().to_path_buf(),
            guardrail: guardrail.unwrap_or_else(GuardrailPolicy::default_enterprise),
            sandbox_config,
            max_concurrency,
            poison_threshold: DEFAULT_POISON_THRESHOLD,
            auto_approve: false,
        }
    }

    pub fn store(&self) -> &Arc<dyn TaskStore> {
        &self.store
    }
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }
    pub fn resolve_workspace_dir(&self, task_id: &TaskId) -> PathBuf {
        Self::resolve_workspace(&self.workspace_dir, &task_id.0)
    }
    pub fn guardrail(&self) -> &GuardrailPolicy {
        &self.guardrail
    }
    pub fn sandbox_config(&self) -> Option<&SandboxConfig> {
        self.sandbox_config.as_ref()
    }
    pub fn concurrency_limit(&self) -> usize {
        self.max_concurrency
    }
    pub fn available_concurrency(&self) -> usize {
        self.concurrency_limiter.available_permits()
    }

    pub async fn active_task_count(&self) -> usize {
        match self.cancellation_flags.read() {
            Ok(lock) => lock.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    pub async fn is_active(&self, id: &TaskId) -> bool {
        match self.cancellation_flags.read() {
            Ok(lock) => lock.contains_key(&id.0),
            Err(poisoned) => poisoned.into_inner().contains_key(&id.0),
        }
    }

    pub async fn active_task_ids(&self) -> Vec<TaskId> {
        let flags = match self.cancellation_flags.read() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        flags.keys().map(|id| TaskId(id.clone())).collect()
    }

    pub async fn is_healthy(&self) -> bool {
        self.store.is_healthy().await
    }

    pub async fn count_tasks(&self) -> Result<usize> {
        self.store.count_tasks().await
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        let limit = concurrency.max(1);
        self.max_concurrency = limit;
        self.concurrency_limiter = Arc::new(Semaphore::new(limit));
        self
    }

    pub fn with_guardrail(mut self, guardrail: GuardrailPolicy) -> Self {
        self.guardrail = guardrail;
        self
    }

    pub fn with_sandbox_config(mut self, config: SandboxConfig) -> Self {
        self.sandbox_config = Some(config);
        self
    }

    pub fn with_workspace_dir(mut self, workspace_dir: impl AsRef<Path>) -> Self {
        self.workspace_dir = workspace_dir.as_ref().to_path_buf();
        self
    }

    pub fn with_poison_threshold(mut self, threshold: usize) -> Self {
        self.poison_threshold = threshold.max(1);
        self
    }

    pub fn poison_threshold(&self) -> usize {
        self.poison_threshold
    }

    pub fn with_auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    pub fn auto_approve(&self) -> bool {
        self.auto_approve
    }

    fn trigger_all_cancellation_flags(&self) {
        let flags = match self.cancellation_flags.read() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        for flag in flags.values() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn trigger_cancellation_flag(&self, id: &TaskId) {
        let flags = match self.cancellation_flags.read() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(flag) = flags.get(&id.0) {
            flag.store(true, Ordering::SeqCst);
        }
    }

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
            let id = self.submit_task(&title, &goal, priority).await?;
            ids.push(id);
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
