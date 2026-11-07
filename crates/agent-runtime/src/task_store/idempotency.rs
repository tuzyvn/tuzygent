use crate::task_store::schema::acquire_conn;
use agent_domain::TaskId;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub fn get_idempotent_task(conn: &Mutex<Connection>, key: &str) -> Result<Option<TaskId>> {
    let conn = acquire_conn(conn);
    let mut stmt = conn.prepare("SELECT task_id FROM task_idempotency WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    if let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        Ok(Some(TaskId(id)))
    } else {
        Ok(None)
    }
}

pub fn delete_idempotent_key(conn: &Mutex<Connection>, key: &str) -> Result<bool> {
    let conn = acquire_conn(conn);
    let affected = conn.execute("DELETE FROM task_idempotency WHERE key = ?1", params![key])?;
    Ok(affected > 0)
}
