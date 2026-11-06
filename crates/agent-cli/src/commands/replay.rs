use std::path::{Path, PathBuf};
use agent_domain::{CheckpointMetadata, SubTask, Task, TaskCheckpoint, TaskId, TaskStatus};
use agent_runtime::{SqliteTaskStore, TaskStore};
use anyhow::{anyhow, Context, Result};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::cli::OutputFormat;

#[derive(Args, Debug, Clone)]
pub struct ReplayArgs {
    /// Target Task ID (positional)
    pub task_id: Option<String>,

    /// Target Task ID via flag (--id / --task-id)
    #[arg(long = "id", visible_alias = "task-id")]
    pub flag_id: Option<String>,

    /// Only list recorded checkpoints and subtask states for the task
    #[arg(short, long)]
    pub list: bool,

    /// Rewind and replay/fork starting from step N (1-based index)
    #[arg(long = "from-step", visible_alias = "step")]
    pub from_step: Option<usize>,

    /// Rewind starting from a specific subtask ID (e.g. ST-2)
    #[arg(long = "subtask-id")]
    pub subtask_id: Option<String>,

    /// Fork into a new Task with an independent TaskId instead of mutating the original task
    #[arg(short, long)]
    pub fork: bool,

    /// Optional new title for the replayed or forked task
    #[arg(long)]
    pub title: Option<String>,

    /// Optional modified goal/context description for the replayed or forked task
    #[arg(short, long)]
    pub goal: Option<String>,

    /// Optional modified command or prompt for the target replay step
    #[arg(long)]
    pub prompt: Option<String>,

    /// Target workspace root path
    #[arg(short, long, default_value = ".", global = true)]
    pub workspace: PathBuf,

    /// Output format: table (default) or json
    #[arg(long, value_enum, default_value = "table", global = true)]
    pub format: OutputFormat,
}

impl ReplayArgs {
    pub fn resolved_task_id(&self) -> Option<&str> {
        self.task_id
            .as_deref()
            .or(self.flag_id.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

#[derive(Args, Debug, Clone)]
pub struct ForkArgs {
    /// Source Task ID to fork from (positional)
    pub task_id: Option<String>,

    /// Source Task ID via flag (--id / --task-id)
    #[arg(long = "id", visible_alias = "task-id")]
    pub flag_id: Option<String>,

    /// Rewind and fork starting from step N (1-based index)
    #[arg(long = "from-step", visible_alias = "step")]
    pub from_step: Option<usize>,

    /// Rewind and fork starting from a specific subtask ID (e.g. ST-2)
    #[arg(long = "subtask-id")]
    pub subtask_id: Option<String>,

    /// Optional new title for the forked task
    #[arg(long)]
    pub title: Option<String>,

    /// Optional modified goal/context description for the forked task
    #[arg(short, long)]
    pub goal: Option<String>,

    /// Optional modified command or prompt for the target starting step
    #[arg(long)]
    pub prompt: Option<String>,

    /// Target workspace root path
    #[arg(short, long, default_value = ".", global = true)]
    pub workspace: PathBuf,

    /// Output format: table (default) or json
    #[arg(long, value_enum, default_value = "table", global = true)]
    pub format: OutputFormat,
}

impl ForkArgs {
    pub fn resolved_task_id(&self) -> Option<&str> {
        self.task_id
            .as_deref()
            .or(self.flag_id.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    pub fn into_replay_args(self) -> ReplayArgs {
        ReplayArgs {
            task_id: self.task_id,
            flag_id: self.flag_id,
            list: false,
            from_step: self.from_step,
            subtask_id: self.subtask_id,
            fork: true,
            title: self.title,
            goal: self.goal,
            prompt: self.prompt,
            workspace: self.workspace,
            format: self.format,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointListPayload {
    pub task_id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub metadata: CheckpointMetadata,
    pub subtasks: Vec<SubTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayOutcomePayload {
    pub action: String,
    pub source_task_id: TaskId,
    pub task: Task,
    pub checkpoint: TaskCheckpoint,
    pub from_step: Option<usize>,
}

pub async fn inspect_checkpoints(workspace: &Path, task_id_str: &str) -> Result<CheckpointListPayload> {
    let store = SqliteTaskStore::new(workspace)
        .context("Failed to open SQLite task store in workspace")?;
    let task_id = TaskId(task_id_str.to_string());
    let task = store
        .get_task(&task_id)
        .await?
        .ok_or_else(|| anyhow!("Task not found with ID: {}", task_id_str))?;

    let cp = match store.get_task_checkpoint(&task_id).await? {
        Some(cp) => cp,
        None => TaskCheckpoint::new(task.id.clone(), task.subtasks.clone()),
    };

    Ok(CheckpointListPayload {
        task_id: task.id,
        title: task.title,
        status: task.status,
        metadata: cp.metadata,
        subtasks: cp.subtasks,
    })
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

pub async fn handle(args: ReplayArgs) -> Result<()> {
    let raw_id = args
        .resolved_task_id()
        .ok_or_else(|| anyhow!("Task ID must be provided"))?;

    if args.list {
        let payload = inspect_checkpoints(&args.workspace, raw_id).await?;
        match args.format {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            }
            OutputFormat::Table => {
                println!("CHECKPOINT STEPS FOR TASK: {} ({})", payload.task_id.0, payload.title);
                println!(
                    "Completed: {}/{} | Last Completed: {:?}",
                    payload.metadata.completed_subtasks,
                    payload.metadata.total_subtasks,
                    payload.metadata.last_completed_subtask_id
                );
                for (idx, sub) in payload.subtasks.iter().enumerate() {
                    println!(
                        "  [Step {}] {} | {:?} | {}",
                        idx + 1,
                        sub.id,
                        sub.status,
                        sub.title
                    );
                }
            }
        }
        return Ok(());
    }

    let outcome = execute_replay_or_fork(&args).await?;
    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&outcome)?);
        }
        OutputFormat::Table => {
            println!(
                "ACTION: {} | SOURCE: {} -> TARGET TASK: {} | STATUS: {:?}",
                outcome.action.to_uppercase(),
                outcome.source_task_id.0,
                outcome.task.id.0,
                outcome.task.status
            );
            for (idx, sub) in outcome.task.subtasks.iter().enumerate() {
                println!(
                    "  [Step {}] {} | {:?} | {}",
                    idx + 1,
                    sub.id,
                    sub.status,
                    sub.title
                );
            }
        }
    }
    Ok(())
}

pub async fn handle_fork(args: ForkArgs) -> Result<()> {
    handle(args.into_replay_args()).await
}
