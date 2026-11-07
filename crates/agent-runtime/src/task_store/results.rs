use crate::task_store::schema::acquire_conn;
use agent_domain::{TaskId, TaskResult};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub fn save_result(conn: &Mutex<Connection>, result: &TaskResult) -> Result<()> {
    let conn = acquire_conn(conn);
    let raw_json = serde_json::to_string(result)?;
    let status_json = serde_json::to_string(&result.status)?;
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO task_results (task_id, status, summary, duration_ms, raw_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(task_id) DO UPDATE SET
             status = excluded.status,
             summary = excluded.summary,
             duration_ms = excluded.duration_ms,
             raw_json = excluded.raw_json",
        params![
            result.task_id.0,
            status_json,
            result.summary,
            i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
            raw_json,
            now
        ],
    )?;

    Ok(())
}

pub fn get_result(conn: &Mutex<Connection>, task_id: &TaskId) -> Result<Option<TaskResult>> {
    let conn = acquire_conn(conn);
    let mut stmt = conn.prepare("SELECT raw_json FROM task_results WHERE task_id = ?1")?;
    let mut rows = stmt.query(params![task_id.0])?;

    if let Some(row) = rows.next()? {
        let raw: String = row.get(0)?;
        let res = serde_json::from_str(&raw)?;
        Ok(Some(res))
    } else {
        Ok(None)
    }
}
