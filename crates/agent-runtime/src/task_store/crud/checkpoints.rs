use crate::task_store::schema::acquire_conn;
use agent_domain::{SubTask, Task, TaskId};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::sync::Mutex;

pub fn save_checkpoint(
    conn: &Mutex<Connection>,
    task_id: &TaskId,
    subtasks: &[SubTask],
) -> Result<()> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let subtasks_json = serde_json::to_string(subtasks)?;
    let metadata = agent_domain::CheckpointMetadata::new(task_id.clone(), subtasks);
    let metadata_json = serde_json::to_string(&metadata)?;
    let now = chrono::Utc::now().to_rfc3339();

    tx.execute(
        "INSERT INTO subtask_checkpoints (task_id, subtasks_json, updated_at, metadata_json)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(task_id) DO UPDATE SET
             subtasks_json = excluded.subtasks_json,
             updated_at = excluded.updated_at,
             metadata_json = excluded.metadata_json",
        params![task_id.0, subtasks_json, now, metadata_json],
    )?;

    // Atomically synchronize the tasks table raw_json if task exists
    let task_raw: Option<String> = tx
        .query_row(
            "SELECT raw_json FROM tasks WHERE id = ?1",
            params![task_id.0],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(raw) = task_raw {
        if let Ok(mut task) = serde_json::from_str::<Task>(&raw) {
            task.subtasks = subtasks.to_vec();
            let new_raw = serde_json::to_string(&task)?;
            tx.execute(
                "UPDATE tasks SET raw_json = ?1 WHERE id = ?2",
                params![new_raw, task_id.0],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

pub fn get_task_checkpoint(
    conn: &Mutex<Connection>,
    task_id: &TaskId,
) -> Result<Option<agent_domain::TaskCheckpoint>> {
    let conn = acquire_conn(conn);
    let mut stmt = conn.prepare(
        "SELECT subtasks_json, metadata_json FROM subtask_checkpoints WHERE task_id = ?1",
    )?;
    let mut rows = stmt.query(params![task_id.0])?;

    if let Some(row) = rows.next()? {
        let json_str: String = row.get(0)?;
        let subtasks: Vec<SubTask> = serde_json::from_str(&json_str)?;
        let meta_str: Option<String> = row.get(1)?;
        let metadata = if let Some(m) = meta_str {
            serde_json::from_str::<agent_domain::CheckpointMetadata>(&m)?
        } else {
            agent_domain::CheckpointMetadata::new(task_id.clone(), &subtasks)
        };
        let cp = agent_domain::TaskCheckpoint { metadata, subtasks };
        cp.verify_integrity_for_task(task_id)?;
        Ok(Some(cp))
    } else {
        Ok(None)
    }
}

pub fn get_checkpoint(conn: &Mutex<Connection>, task_id: &TaskId) -> Result<Option<Vec<SubTask>>> {
    let cp = get_task_checkpoint(conn, task_id)?;
    Ok(cp.map(|c| c.subtasks))
}

pub fn get_checkpoint_metadata(
    conn: &Mutex<Connection>,
    task_id: &TaskId,
) -> Result<Option<agent_domain::CheckpointMetadata>> {
    let conn = acquire_conn(conn);
    let mut stmt = conn.prepare(
        "SELECT metadata_json, subtasks_json FROM subtask_checkpoints WHERE task_id = ?1",
    )?;
    let mut rows = stmt.query(params![task_id.0])?;

    if let Some(row) = rows.next()? {
        if let Some(meta_str) = row.get::<_, Option<String>>(0)? {
            if let Ok(meta) = serde_json::from_str::<agent_domain::CheckpointMetadata>(&meta_str) {
                if &meta.task_id == task_id {
                    return Ok(Some(meta));
                }
            }
        }
        let json_str: String = row.get(1)?;
        let subtasks: Vec<SubTask> = serde_json::from_str(&json_str)?;
        Ok(Some(agent_domain::CheckpointMetadata::new(
            task_id.clone(),
            &subtasks,
        )))
    } else {
        Ok(None)
    }
}
