use crate::task_store::schema::acquire_conn;
use agent_domain::{Task, TaskResult, TaskStatus};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::sync::Mutex;

pub fn save_review(
    conn: &Mutex<Connection>,
    expected: &Task,
    updated: &Task,
    result: &TaskResult,
    reviewer: &str,
    decision: &str,
    revision: &str,
) -> Result<bool> {
    if expected.id != updated.id || updated.id != result.task_id {
        return Err(anyhow!("Review task IDs differ"));
    }
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current_opt: Option<String> = tx
        .query_row(
            "SELECT raw_json FROM tasks WHERE id=?1",
            params![expected.id.0],
            |row| row.get(0),
        )
        .optional()?;
    let Some(current_raw) = current_opt else {
        return Ok(false);
    };
    let current: Task = serde_json::from_str(&current_raw)?;
    if serde_json::to_value(&current)? != serde_json::to_value(expected)? {
        return Ok(false);
    }
    let checkpoint: Option<String> = tx
        .query_row(
            "SELECT subtasks_json FROM subtask_checkpoints WHERE task_id=?1",
            params![expected.id.0],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(cp) = checkpoint {
        if serde_json::from_str::<serde_json::Value>(&cp)?
            != serde_json::to_value(&expected.subtasks)?
        {
            return Ok(false);
        }
    } else if !expected.subtasks.is_empty() {
        return Ok(false);
    }
    let now = updated.updated_at.to_rfc3339();
    tx.execute(
        "UPDATE tasks SET status=?1, updated_at=?2, raw_json=?3 WHERE id=?4",
        params![
            serde_json::to_string(&updated.status)?,
            now,
            serde_json::to_string(updated)?,
            updated.id.0
        ],
    )?;
    let metadata = agent_domain::CheckpointMetadata::new(updated.id.clone(), &updated.subtasks);
    let metadata_json = serde_json::to_string(&metadata)?;
    tx.execute(
        "UPDATE subtask_checkpoints SET subtasks_json=?1, updated_at=?2, metadata_json=?3 WHERE task_id=?4",
        params![serde_json::to_string(&updated.subtasks)?, now, metadata_json, updated.id.0],
    )?;
    tx.execute(
        "INSERT INTO task_reviews(task_id,revision,reviewer,decision,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![updated.id.0, revision, reviewer, decision, now],
    )?;
    if updated.status == TaskStatus::Queued {
        tx.execute(
            "DELETE FROM task_results WHERE task_id=?1",
            params![updated.id.0],
        )?;
    } else {
        tx.execute(
            "INSERT INTO task_results (task_id,status,summary,duration_ms,raw_json,created_at) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(task_id) DO UPDATE SET status=excluded.status,summary=excluded.summary,duration_ms=excluded.duration_ms,raw_json=excluded.raw_json",
            params![
                updated.id.0,
                serde_json::to_string(&result.status)?,
                result.summary,
                i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
                serde_json::to_string(result)?,
                now
            ],
        )?;
    }
    tx.commit()?;
    Ok(true)
}
