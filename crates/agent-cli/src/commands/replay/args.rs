use clap::Args;
use std::path::PathBuf;

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
