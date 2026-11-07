use super::dead_letter::evaluate_failure_and_quarantine;
use super::TaskService;
use crate::ooda::AutonomousOodaLoop;
use crate::sandbox::SandboxConfig;
use crate::task_store::TaskStore;
use agent_domain::{GuardrailPolicy, Task, TaskId, TaskResult, TaskStatus};
use agent_engine::ComputeEngine;
use agent_memory::{DurableMemoryStore, ScratchpadManager};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info};

fn make_task_result(id: TaskId, status: TaskStatus, summary: String, engine: Option<String>) -> TaskResult {
    TaskResult { task_id: id, status, summary, artifacts: Vec::new(), verifications: Vec::new(), duration_ms: 0, engine_used: engine }
}

impl TaskService {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_task_worker(
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
        let resolved_workspace = TaskService::resolve_workspace(&workspace, &task_id.0);

        // Check if cancelled before execution started
        if cancel_flag.load(Ordering::SeqCst) {
            task.status = TaskStatus::Canceled {
                reason: "Canceled before execution".into(),
            };
            task.updated_at = chrono::Utc::now();
            let result = make_task_result(
                task_id.clone(),
                task.status.clone(),
                "Canceled before execution".into(),
                None,
            );
            if let Err(err) = store.finish_task(&task, &result, true).await {
                error!("Failed to persist cancellation: {}", err);
            }
            return;
        }

        // Restore from checkpoint if available
        match store.get_task_checkpoint(&task_id).await {
            Ok(Some(mut cp)) => {
                info!(
                    "🔄 [TaskService] Restoring {} subtasks from verified checkpoint for task {}",
                    cp.subtasks.len(),
                    task_id
                );
                cp.sanitize_for_resume();
                task.subtasks = cp.subtasks;
            }
            Ok(None) => {}
            Err(err) => {
                let message = format!("Cannot load task checkpoint: {err}");
                let (new_status, reason) =
                    evaluate_failure_and_quarantine(&mut task, poison_threshold, &message);
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
        if cancel_flag.load(Ordering::SeqCst) && !matches!(task.status, TaskStatus::Canceled { .. })
        {
            task.status = TaskStatus::Canceled {
                reason: "Canceled by user request".into(),
            };
        }

        match execution_outcome {
            Ok(mut task_result) => {
                info!(
                    "🏁 [TaskService] Task {} reached terminal status: {:?}",
                    task_id, task.status
                );
                task.updated_at = chrono::Utc::now();
                if matches!(task.status, TaskStatus::Canceled { .. }) {
                    task_result.status = task.status.clone();
                    task_result.summary = "Canceled by user request".into();
                } else if matches!(task.status, TaskStatus::Failed { .. }) {
                    let err = match &task.status {
                        TaskStatus::Failed { error } => error.clone(),
                        _ => "Task failed".to_string(),
                    };
                    let (new_status, reason) =
                        evaluate_failure_and_quarantine(&mut task, poison_threshold, &err);
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
                error!(
                    "❌ [TaskService] Task {} failed with error: {}",
                    task_id, err
                );
                task.updated_at = chrono::Utc::now();
                let is_canceled = matches!(task.status, TaskStatus::Canceled { .. });
                let (failed_status, summary) = if is_canceled {
                    (task.status.clone(), "Canceled by user request".into())
                } else {
                    let (new_status, reason) =
                        evaluate_failure_and_quarantine(&mut task, poison_threshold, &err);
                    task.status = new_status.clone();
                    (new_status, reason)
                };
                let failed_result = make_task_result(
                    task_id.clone(),
                    failed_status,
                    summary,
                    Some(engine.engine_kind().to_string()),
                );
                if let Err(err) = store.finish_task(&task, &failed_result, false).await {
                    error!("Failed to persist task failure: {}", err);
                }
            }
        }
    }
}
