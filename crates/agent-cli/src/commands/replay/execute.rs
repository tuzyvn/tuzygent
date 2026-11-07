use agent_domain::{Task, TaskCheckpoint, TaskId, TaskStatus};
use agent_runtime::{SqliteTaskStore, TaskStore};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::args::ReplayArgs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayOutcomePayload {
    pub action: String,
    pub source_task_id: TaskId,
    pub task: Task,
    pub checkpoint: TaskCheckpoint,
    pub from_step: Option<usize>,
}

pub async fn execute_replay_or_fork(args: &ReplayArgs) -> Result<ReplayOutcomePayload> {
    let raw_id = args
        .resolved_task_id()
        .ok_or_else(|| anyhow!("task_id is required for replay/fork"))?;
    let source_task_id = TaskId(raw_id.to_string());

    let store = SqliteTaskStore::new(&args.workspace)
        .context("Failed to open SQLite task store in workspace")?;

    let mut source_task = store
        .get_task(&source_task_id)
        .await?
        .ok_or_else(|| anyhow!("Task not found with ID: {}", raw_id))?;

    let mut cp = match store.get_task_checkpoint(&source_task_id).await? {
        Some(existing_cp) => {
            existing_cp
                .verify_integrity_for_task(&source_task_id)
                .map_err(|e| {
                    anyhow!(
                        "Checkpoint integrity verification failed for task {}: {}",
                        raw_id,
                        e
                    )
                })?;
            existing_cp
        }
        None => TaskCheckpoint::new(source_task.id.clone(), source_task.subtasks.clone()),
    };

    let effective_step = match (args.from_step, args.subtask_id.as_deref()) {
        (Some(step), _) => Some(step),
        (None, Some(sub_id)) => {
            let pos = cp
                .subtasks
                .iter()
                .position(|s| s.id == sub_id)
                .ok_or_else(|| anyhow!("Subtask '{}' not found in task {}", sub_id, raw_id))?;
            Some(pos + 1)
        }
        (None, None) => None,
    };

    if args.fork {
        let (mut forked_task, mut forked_cp) = source_task
            .fork_from_checkpoint(&cp, effective_step, args.title.clone(), args.goal.clone())
            .map_err(|e| anyhow!("Failed to fork task {}: {}", raw_id, e))?;

        if let Some(ref custom_prompt) = args.prompt {
            let target_idx = effective_step.unwrap_or(1).saturating_sub(1);
            if let Some(sub) = forked_task.subtasks.get_mut(target_idx) {
                sub.command_or_prompt = custom_prompt.clone();
            }
            forked_cp = TaskCheckpoint::new(forked_task.id.clone(), forked_task.subtasks.clone());
        }

        store.save_task(&forked_task).await?;
        store
            .save_checkpoint(&forked_task.id, &forked_cp.subtasks)
            .await?;

        Ok(ReplayOutcomePayload {
            action: "fork".to_string(),
            source_task_id,
            task: forked_task,
            checkpoint: forked_cp,
            from_step: effective_step,
        })
    } else {
        if let Some(step) = effective_step {
            cp.rewind_to_step(step)
                .map_err(|e| anyhow!("Failed to rewind task {}: {}", raw_id, e))?;
        } else {
            cp.sanitize_for_resume();
        }

        if let Some(ref custom_prompt) = args.prompt {
            let target_idx = effective_step.unwrap_or(1).saturating_sub(1);
            if let Some(sub) = cp.subtasks.get_mut(target_idx) {
                sub.command_or_prompt = custom_prompt.clone();
            }
            cp = TaskCheckpoint::new(source_task.id.clone(), cp.subtasks);
        }

        if let Some(ref new_title) = args.title {
            source_task.title = new_title.clone();
        }
        if let Some(ref new_goal) = args.goal {
            source_task.goal_description = new_goal.clone();
        }

        source_task.subtasks = cp.subtasks.clone();
        source_task.status = TaskStatus::Queued;
        source_task.failure_count = 0;
        source_task.updated_at = chrono::Utc::now();

        store
            .save_checkpoint(&source_task.id, &source_task.subtasks)
            .await?;
        store.save_task(&source_task).await?;

        Ok(ReplayOutcomePayload {
            action: "replay".to_string(),
            source_task_id,
            task: source_task,
            checkpoint: cp,
            from_step: effective_step,
        })
    }
}
