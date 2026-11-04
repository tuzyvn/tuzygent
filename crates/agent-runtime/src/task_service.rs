use crate::ooda_loop::AutonomousOodaLoop;
use crate::sandbox::SandboxConfig;
use crate::task_store::TaskStore;
use agent_domain::{GuardrailPolicy, Priority, Task, TaskId, TaskResult, TaskStatus};
use agent_engine::ComputeEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use tracing::{error, info};

/// TaskService manages task lifecycles with asynchronous background dispatch,
/// persistence, checkpointing, concurrency limits, cancellation, and human-in-the-loop approvals.
pub struct TaskService {
    store: Arc<dyn TaskStore>,
    concurrency_limiter: Arc<Semaphore>,
    cancellation_flags: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
    workspace_dir: PathBuf,
    guardrail: GuardrailPolicy,
    sandbox_config: Option<SandboxConfig>,
}

impl TaskService {
    pub fn new(
        store: Arc<dyn TaskStore>,
        workspace_dir: impl AsRef<Path>,
        max_concurrency: usize,
        guardrail: Option<GuardrailPolicy>,
        sandbox_config: Option<SandboxConfig>,
    ) -> Self {
        Self {
            store,
            concurrency_limiter: Arc::new(Semaphore::new(max_concurrency.max(1))),
            cancellation_flags: Arc::new(RwLock::new(HashMap::new())),
            workspace_dir: workspace_dir.as_ref().to_path_buf(),
            guardrail: guardrail.unwrap_or_else(GuardrailPolicy::default_enterprise),
            sandbox_config,
        }
    }

    pub fn store(&self) -> &Arc<dyn TaskStore> {
        &self.store
    }

    /// Submit a new task, immediately recording it as Queued and returning the TaskId (HTTP 202 pattern)
    pub async fn submit_task(
        &self,
        title: &str,
        goal: &str,
        priority: Priority,
    ) -> Result<TaskId> {
        let task = Task::new(title, goal, priority);
        let id = task.id.clone();
        self.store.save_task(&task).await?;
        info!("📥 [TaskService] Task {} queued: '{}'", id, title);
        Ok(id)
    }

    /// Dispatch task for asynchronous execution in background
    pub fn dispatch(
        &self,
        task_id: TaskId,
        engine: Arc<dyn ComputeEngine>,
    ) {
        let store = self.store.clone();
        let limiter = self.concurrency_limiter.clone();
        let cancel_flags = self.cancellation_flags.clone();
        let workspace = self.workspace_dir.clone();
        let guardrail = self.guardrail.clone();
        let sandbox_config = self.sandbox_config.clone();

        tokio::spawn(async move {
            let permit = match limiter.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    error!("TaskService semaphore closed");
                    return;
                }
            };

            let cancel_flag = Arc::new(AtomicBool::new(false));
            {
                let mut lock = cancel_flags.write().await;
                lock.insert(task_id.0.clone(), cancel_flag.clone());
            }

            let mut task = match store.get_task(&task_id).await {
                Ok(Some(t)) => t,
                _ => {
                    error!("Task {} not found in store for dispatch", task_id);
                    drop(permit);
                    return;
                }
            };

            // Check if cancelled before execution started
            if cancel_flag.load(Ordering::SeqCst) {
                task.status = TaskStatus::Canceled { reason: "Canceled before execution".into() };
                let _ = store.save_task(&task).await;
                drop(permit);
                return;
            }

            // Restore from checkpoint if available
            if let Ok(Some(cached_subtasks)) = store.get_checkpoint(&task_id).await {
                if !cached_subtasks.is_empty() {
                    info!("🔄 [TaskService] Restoring {} subtasks from checkpoint for task {}", cached_subtasks.len(), task_id);
                    task.subtasks = cached_subtasks;
                }
            }

            task.status = TaskStatus::Executing;
            let _ = store.save_task(&task).await;

            let scratchpad = ScratchpadManager::for_task(&workspace, &task.id.0);
            let durable = DurableMemoryStore::new(&workspace);

            let mut ooda = AutonomousOodaLoop::new(
                engine.clone(),
                guardrail.clone(),
                scratchpad,
                durable,
                &workspace.to_string_lossy(),
            );

            if let Some(cfg) = sandbox_config {
                ooda = ooda.with_sandbox_config(cfg);
            }

            info!("🚀 [TaskService] Running OODA loop for task {}", task_id);
            let execution_outcome = ooda.run_goal_detailed(&mut task).await;

            match execution_outcome {
                Ok(task_result) => {
                    info!("🏁 [TaskService] Task {} reached terminal status: {:?}", task_id, task.status);
                    let _ = store.save_task(&task).await;
                    let _ = store.save_checkpoint(&task_id, &task.subtasks).await;
                    let _ = store.save_result(&task_result).await;
                }
                Err(err) => {
                    error!("❌ [TaskService] Task {} failed with error: {}", task_id, err);
                    task.status = TaskStatus::Failed { error: err.clone() };
                    let _ = store.save_task(&task).await;
                    let _ = store.save_checkpoint(&task_id, &task.subtasks).await;
                }
            }

            {
                let mut lock = cancel_flags.write().await;
                lock.remove(&task_id.0);
            }

            drop(permit);
        });
    }

    /// Cancel a running or queued task
    pub async fn cancel_task(&self, id: &TaskId) -> Result<bool> {
        let flag = {
            let lock = self.cancellation_flags.read().await;
            lock.get(&id.0).cloned()
        };

        if let Some(flag) = flag {
            flag.store(true, Ordering::SeqCst);
        }

        if let Some(mut task) = self.store.get_task(id).await? {
            task.status = TaskStatus::Canceled { reason: "Canceled by user request".into() };
            task.updated_at = chrono::Utc::now();
            self.store.save_task(&task).await?;
            info!("🛑 [TaskService] Task {} canceled", id);
            return Ok(true);
        }

        Ok(false)
    }

    /// Approve a task that is currently in WaitingApproval status, resuming execution
    pub async fn approve_task(
        &self,
        id: &TaskId,
        engine: Arc<dyn ComputeEngine>,
    ) -> Result<bool> {
        if let Some(mut task) = self.store.get_task(id).await? {
            if matches!(task.status, TaskStatus::WaitingApproval { .. }) {
                info!("👍 [TaskService] Task {} approved, resuming execution from checkpoint", id);
                task.status = TaskStatus::Queued;
                task.updated_at = chrono::Utc::now();
                self.store.save_task(&task).await?;
                self.dispatch(id.clone(), engine);
                return Ok(true);
            } else {
                return Err(anyhow!("Task is not in WaitingApproval status (current: {:?})", task.status));
            }
        }
        Ok(false)
    }

    pub async fn get_task(&self, id: &TaskId) -> Result<Option<Task>> {
        self.store.get_task(id).await
    }

    pub async fn get_result(&self, id: &TaskId) -> Result<Option<TaskResult>> {
        self.store.get_result(id).await
    }

    pub async fn list_tasks(&self, limit: usize, offset: usize) -> Result<Vec<Task>> {
        self.store.list_tasks(limit, offset).await
    }
}
