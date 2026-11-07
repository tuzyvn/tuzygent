use anyhow::Result;

use crate::cli::OutputFormat;

use super::{CheckpointListPayload, ReplayOutcomePayload};

pub(super) fn print_checkpoint_list(
    payload: &CheckpointListPayload,
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(payload)?),
        OutputFormat::Table => {
            println!(
                "CHECKPOINT STEPS FOR TASK: {} ({})",
                payload.task_id.0, payload.title
            );
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
    Ok(())
}

pub(super) fn print_replay_outcome(
    outcome: &ReplayOutcomePayload,
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(outcome)?),
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
