use super::super::dead_letter::evaluate_failure_and_quarantine;
use super::super::TaskService;
use agent_domain::{Task, TaskId, TaskResult, TaskStatus};
use agent_engine::ComputeEngine;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tracing::error;

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
                        let err_msg =
                            "TaskService closed before execution could acquire permit".to_string();
                        let (new_status, reason) =
                            evaluate_failure_and_quarantine(&mut task, poison_threshold, &err_msg);
                        task.status = new_status.clone();
                        task.updated_at = chrono::Utc::now();
                        let failure_result =
                            make_task_result(task_id.clone(), new_status, reason, None);
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
                let mut task = store
                    .get_task(&task_id)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| {
                        Task::new(
                            "Failed Task",
                            "Panicked Task",
                            agent_domain::Priority::Normal,
                        )
                    });
                let (new_status, reason) =
                    evaluate_failure_and_quarantine(&mut task, poison_threshold, &err_msg);
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
                if flags
                    .get(&task_id.0)
                    .is_some_and(|f| Arc::ptr_eq(f, &task_cancel_flag))
                {
                    flags.remove(&task_id.0);
                }
            }

            drop(permit);
        });
    }
}
