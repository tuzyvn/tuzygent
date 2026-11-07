use crate::task_store::schema::acquire_conn;
use agent_domain::{Task, TaskId, TaskStatus};
use anyhow::Result;
use rusqlite::{params, Connection, TransactionBehavior};
use std::sync::Mutex;

pub fn claim_queued(conn: &Mutex<Connection>, id: &TaskId) -> Result<Option<Task>> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let raw = {
        let mut stmt = tx.prepare("SELECT raw_json FROM tasks WHERE id = ?1")?;
        let mut rows = stmt.query(params![id.0])?;
        if let Some(row) = rows.next()? {
            Some(row.get::<_, String>(0)?)
        } else {
            None
        }
    };
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut task: Task = serde_json::from_str(&raw)?;
    let now = chrono::Utc::now();
    let effective_now = now.max(task.updated_at);
    if task.status != TaskStatus::Queued || task.run_after.is_some_and(|t| t > effective_now) {
        return Ok(None);
    }
    task.status = TaskStatus::Executing;
    task.updated_at = now;
    tx.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2, raw_json = ?3 WHERE id = ?4",
        params![
            serde_json::to_string(&task.status)?,
            task.updated_at.to_rfc3339(),
            serde_json::to_string(&task)?,
            id.0
        ],
    )?;
    tx.commit()?;
    Ok(Some(task))
}

pub fn claim_next_queued(conn: &Mutex<Connection>) -> Result<Option<Task>> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = chrono::Utc::now();
    let candidate = {
        let mut stmt = tx.prepare(
            "SELECT raw_json FROM tasks WHERE status = '\"Queued\"' OR status LIKE '%Queued%'
             ORDER BY CASE
                 WHEN priority LIKE '%Critical%' THEN 1
                 WHEN priority LIKE '%High%' THEN 2
                 WHEN priority LIKE '%Normal%' THEN 3
                 ELSE 4 END, created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut found = None;
        for raw in rows.flatten() {
            if let Ok(t) = serde_json::from_str::<Task>(&raw) {
                let effective_now = now.max(t.updated_at);
                if t.status == TaskStatus::Queued
                    && t.run_after.is_none_or(|dt| dt <= effective_now)
                {
                    found = Some(t);
                    break;
                }
            }
        }
        found
    };
    let Some(mut task) = candidate else {
        return Ok(None);
    };
    task.status = TaskStatus::Executing;
    task.updated_at = now;
    tx.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2, raw_json = ?3 WHERE id = ?4",
        params![
            serde_json::to_string(&task.status)?,
            task.updated_at.to_rfc3339(),
            serde_json::to_string(&task)?,
            task.id.0
        ],
    )?;
    tx.commit()?;
    Ok(Some(task))
}
