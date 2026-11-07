use crate::task_store::schema::acquire_conn;
use crate::task_store::IdempotentSubmission;
use agent_domain::{Task, TaskId};
use anyhow::Result;
use rusqlite::{params, Connection, TransactionBehavior};
use std::sync::Mutex;

pub fn submit_idempotent(
    conn: &Mutex<Connection>,
    task: &Task,
    key: &str,
    request: &str,
) -> Result<IdempotentSubmission> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = {
        let mut statement =
            tx.prepare("SELECT request, task_id FROM task_idempotency WHERE key = ?1")?;
        let mut rows = statement.query(params![key])?;
        if let Some(row) = rows.next()? {
            Some((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        } else {
            None
        }
    };
    if let Some((previous, id)) = existing {
        return Ok(if previous == request {
            IdempotentSubmission::Existing(TaskId(id))
        } else {
            IdempotentSubmission::Conflict
        });
    }
    tx.execute(
        "INSERT INTO tasks (id, title, goal, priority, status, created_at, updated_at, raw_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            task.id.0,
            task.title,
            task.goal_description,
            serde_json::to_string(&task.priority)?,
            serde_json::to_string(&task.status)?,
            task.created_at.to_rfc3339(),
            task.updated_at.to_rfc3339(),
            serde_json::to_string(task)?
        ],
    )?;
    tx.execute(
        "INSERT INTO task_idempotency (key, request, task_id) VALUES (?1, ?2, ?3)",
        params![key, request, task.id.0],
    )?;
    tx.commit()?;
    Ok(IdempotentSubmission::Created(task.id.clone()))
}
