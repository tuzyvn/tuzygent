use super::dead_letter::evaluate_failure_and_quarantine;
use super::TaskService;
use crate::ooda::AutonomousOodaLoop;
use crate::sandbox::SandboxConfig;
use crate::task_store::TaskStore;
use agent_domain::{GuardrailPolicy, Task, TaskId, TaskResult, TaskStatus};
use agent_engine::ComputeEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info};

fn make_task_result(
    id: TaskId,
    status: TaskStatus,
    summary: String,
    engine: Option<String>,
) -> TaskResult {
    TaskResult {
        task_id: id,
        status,
        summary,
        artifacts: Vec::new(),
        verifications: Vec::new(),
        duration_ms: 0,
        engine_used: engine,
    }
}

impl TaskService {
    /// Safe multi-tier fallback workspace directory resolution
    pub fn resolve_workspace(workspace: &Path, task_id_str: &str) -> PathBuf {
        let trimmed = workspace.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() && (workspace.exists() || std::fs::create_dir_all(workspace).is_ok()) {
            return workspace.to_path_buf();
        }
        let unique = format!("agent_ws_{}_{}", task_id_str, uuid::Uuid::new_v4());
        let candidates = [
            std::env::temp_dir().join(&unique),
            PathBuf::from("target/tmp").join(&unique),
            PathBuf::from("/tmp").join(&unique),
        ];
        for candidate in candidates {
            if std::fs::create_dir_all(&candidate).is_ok() {
                return candidate;
            }
        }
        std::env::temp_dir()
    }

    /// Call once on startup, with exclusive ownership of the workspace's task database.
    /// Running work is never replayed automatically because its side effects are unknown.
    pub async fn recover_after_restart(&self) -> Result<()> {
        let config = agent_engine::EnginesConfig::load_hierarchical(Some(&self.workspace_dir));
        let mut offset = 0;
        loop {
            let tasks = self.store.list_tasks(100, offset).await?;
            if tasks.is_empty() {
                break;
            }
            offset += tasks.len();
            for mut task in tasks {
                match task.status {
                    TaskStatus::Queued => {
                        let engine = task
                            .engine_name
                            .as_deref()
                            .and_then(|name| agent_engine::configured_engine(&config, name, None).ok());
                        if let Some(engine) = engine {
                            self.dispatch(task.id.clone(), engine);
                        } else {
                            task.status = TaskStatus::WaitingApproval {
                                reason: "Recovery requires a valid recorded engine".into(),
                            };
                            task.updated_at = chrono::Utc::now();
                            let _ = self.store.save_task(&task).await;
                        }
                    }
                    TaskStatus::Executing
                    | TaskStatus::PreparingSandbox
                    | TaskStatus::SelfChecking
                    | TaskStatus::Supplementing { .. }
                    | TaskStatus::Dispatched { .. }
                    | TaskStatus::Canceled { .. } => {
                        match self.store.get_task_checkpoint(&task.id).await {
                            Ok(Some(mut cp)) => {
                                cp.sanitize_for_resume();
                                task.subtasks = cp.subtasks;
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::warn!("Interrupted task {} has corrupted checkpoint during recovery: {}", task.id, e);
                            }
                        }
                        task.status = TaskStatus::WaitingApproval {
                            reason: "Interrupted execution requires reconciliation before retry".into(),
                        };
                        task.updated_at = chrono::Utc::now();
                        let _ = self.store.save_task(&task).await;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Dispatch task for asynchronous execution in background
    pub fn dispatch(&self, task_id: TaskId, engine: Arc<dyn ComputeEngine>) {
        let store = self.store.clone();
        let limiter = self.concurrency_limiter.clone();
        let cancel_flags = self.cancellation_flags.clone();
        let workspace = self.workspace_dir.clone();
        let guardrail = self.guardrail.clone();
        let sandbox_config = self.sandbox_config.clone();
        let poison_threshold = self.poison_threshold;
        let auto_approve = self.auto_approve;

        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let mut flags = match self.cancellation_flags.write() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            flags.insert(task_id.0.clone(), cancel_flag.clone());
        }

        let task_cancel_flag = cancel_flag.clone();
        tokio::spawn(async move {
            let permit = match limiter.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    error!("TaskService semaphore closed");
                    {
                        let mut flags = match cancel_flags.write() {
                            Ok(g) => g,
                            Err(p) => p.into_inner(),
                        };
                        flags.remove(&task_id.0);
                    }
                    if let Ok(Some(mut task)) = store.get_task(&task_id).await {
                        let err_msg = "TaskService closed before execution could acquire permit".to_string();
                        let (new_status, reason) = evaluate_failure_and_quarantine(&mut task, poison_threshold, &err_msg);
                        task.status = new_status.clone();
                        task.updated_at = chrono::Utc::now();
                        let failure_result = make_task_result(task_id.clone(), new_status, reason, None);
                        let _ = store.finish_task(&task, &failure_result, true).await;
                    }
                    return;
                }
            };

            let claimed = match store.claim_queued(&task_id).await {
                Ok(Some(task)) => task,
                Ok(None) | Err(_) => {
                    let mut flags = match cancel_flags.write() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    flags.remove(&task_id.0);
                    return;
                }
            };

            let worker_store = store.clone();
            let worker_cancel_flag = task_cancel_flag.clone();
            let worker_engine = engine.clone();
            let worker_workspace = workspace.clone();
            let worker_guardrail = guardrail.clone();
            let worker_sandbox = sandbox_config.clone();

            let worker_handle = tokio::spawn(async move {
                Self::execute_task_worker(
                    claimed,
                    worker_store,
                    worker_cancel_flag,
                    worker_engine,
                    worker_workspace,
                    worker_guardrail,
                    worker_sandbox,
                    poison_threshold,
                    auto_approve,
                )
                .await;
            });

            if let Err(join_err) = worker_handle.await {
                let err_msg = if join_err.is_panic() {
                    "Task execution panicked unexpectedly".to_string()
                } else if join_err.is_cancelled() {
                    "Task execution worker was cancelled".to_string()
                } else {
                    format!("Task execution worker failed: {}", join_err)
                };
                error!("💥 [TaskService] Task {}: {}", task_id, err_msg);
                let mut task = store.get_task(&task_id).await.ok().flatten().unwrap_or_else(|| {
                    Task::new("Failed Task", "Panicked Task", agent_domain::Priority::Normal)
                });
                let (new_status, reason) = evaluate_failure_and_quarantine(&mut task, poison_threshold, &err_msg);
                task.status = new_status.clone();
                task.updated_at = chrono::Utc::now();
                let failure_result = make_task_result(task_id.clone(), new_status, reason, None);
                if let Err(err) = store.finish_task(&task, &failure_result, true).await {
                    error!("Failed to persist worker failure: {}", err);
                }
            }

            {
                let mut flags = match cancel_flags.write() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                if flags.get(&task_id.0).is_some_and(|f| Arc::ptr_eq(f, &task_cancel_flag)) {
                    flags.remove(&task_id.0);
                }
            }

            drop(permit);
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_task_worker(
        mut task: Task,
        store: Arc<dyn TaskStore>,
        cancel_flag: Arc<AtomicBool>,
        engine: Arc<dyn ComputeEngine>,
        workspace: PathBuf,
        guardrail: GuardrailPolicy,
        sandbox_config: Option<SandboxConfig>,
        poison_threshold: usize,
        auto_approve: bool,
    ) {
        let task_id = task.id.clone();
        let resolved_workspace = Self::resolve_workspace(&workspace, &task_id.0);

        // Check if cancelled before execution started
        if cancel_flag.load(Ordering::SeqCst) {
            task.status = TaskStatus::Canceled { reason: "Canceled before execution".into() };
            task.updated_at = chrono::Utc::now();
            let result = make_task_result(task_id.clone(), task.status.clone(), "Canceled before execution".into(), None);
            if let Err(err) = store.finish_task(&task, &result, true).await {
                error!("Failed to persist cancellation: {}", err);
            }
            return;
        }

        // Restore from checkpoint if available
        match store.get_task_checkpoint(&task_id).await {
            Ok(Some(mut cp)) => {
                info!("🔄 [TaskService] Restoring {} subtasks from verified checkpoint for task {}", cp.subtasks.len(), task_id);
                cp.sanitize_for_resume();
                task.subtasks = cp.subtasks;
            }
            Ok(None) => {}
            Err(err) => {
                let message = format!("Cannot load task checkpoint: {err}");
                let (new_status, reason) = evaluate_failure_and_quarantine(&mut task, poison_threshold, &message);
                task.status = new_status.clone();
                task.updated_at = chrono::Utc::now();
                let result = make_task_result(task_id, new_status, reason, None);
                if let Err(err) = store.finish_task(&task, &result, true).await {
                    error!("Failed to persist checkpoint error: {}", err);
                }
                return;
            }
        }

        let scratchpad = ScratchpadManager::for_task(&resolved_workspace, &task.id.0);
        let durable = DurableMemoryStore::new(&resolved_workspace);

        let mut ooda = AutonomousOodaLoop::new(
            engine.clone(),
            guardrail.clone(),
            scratchpad,
            durable,
            &resolved_workspace.to_string_lossy(),
        )
        .with_cancellation_token(cancel_flag.clone())
        .with_checkpoint_store(store.clone());

        if auto_approve {
            ooda = ooda.with_auto_approve(true);
        }

        if let Some(cfg) = sandbox_config {
            ooda = ooda.with_sandbox_config(cfg);
        }

        info!("🚀 [TaskService] Running OODA loop for task {}", task_id);
        let execution_outcome = ooda.run_goal_detailed(&mut task).await;

        // If cancellation happened during or after execution, preserve cancellation state
        if cancel_flag.load(Ordering::SeqCst) && !matches!(task.status, TaskStatus::Canceled { .. }) {
            task.status = TaskStatus::Canceled { reason: "Canceled by user request".into() };
        }

        match execution_outcome {
            Ok(mut task_result) => {
                info!("🏁 [TaskService] Task {} reached terminal status: {:?}", task_id, task.status);
                task.updated_at = chrono::Utc::now();
                if matches!(task.status, TaskStatus::Canceled { .. }) {
                    task_result.status = task.status.clone();
                    task_result.summary = "Canceled by user request".into();
                } else if matches!(task.status, TaskStatus::Failed { .. }) {
                    let err = match &task.status {
                        TaskStatus::Failed { error } => error.clone(),
                        _ => "Task failed".to_string(),
                    };
                    let (new_status, reason) = evaluate_failure_and_quarantine(&mut task, poison_threshold, &err);
                    task.status = new_status.clone();
                    task_result.status = new_status;
                    task_result.summary = reason;
                } else {
                    task_result.status = task.status.clone();
                }
                if let Err(err) = store.finish_task(&task, &task_result, false).await {
                    error!("Failed to persist task result: {}", err);
                }
            }
            Err(err) => {
                error!("❌ [TaskService] Task {} failed with error: {}", task_id, err);
                task.updated_at = chrono::Utc::now();
                let is_canceled = matches!(task.status, TaskStatus::Canceled { .. });
                let (failed_status, summary) = if is_canceled {
                    (task.status.clone(), "Canceled by user request".into())
                } else {
                    let (new_status, reason) = evaluate_failure_and_quarantine(&mut task, poison_threshold, &err);
                    task.status = new_status.clone();
                    (new_status, reason)
                };
                let failed_result = make_task_result(task_id.clone(), failed_status, summary, Some(engine.engine_kind().to_string()));
                if let Err(err) = store.finish_task(&task, &failed_result, false).await {
                    error!("Failed to persist task failure: {}", err);
                }
            }
        }
    }
}
