mod args;
mod checkpoint;
mod execute;
mod output;

use anyhow::{anyhow, Result};

pub use args::{ForkArgs, ReplayArgs};
pub use checkpoint::{inspect_checkpoints, CheckpointListPayload};
pub use execute::{execute_replay_or_fork, ReplayOutcomePayload};

pub async fn handle(args: ReplayArgs) -> Result<()> {
    let raw_id = args
        .resolved_task_id()
        .ok_or_else(|| anyhow!("Task ID must be provided"))?;

    if args.list {
        let payload = inspect_checkpoints(&args.workspace, raw_id).await?;
        output::print_checkpoint_list(&payload, args.format)?;
        return Ok(());
    }

    let outcome = execute_replay_or_fork(&args).await?;
    output::print_replay_outcome(&outcome, args.format)?;
    Ok(())
}

pub async fn handle_fork(args: ForkArgs) -> Result<()> {
    handle(args.into_replay_args()).await
}
