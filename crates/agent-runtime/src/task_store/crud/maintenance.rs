use crate::task_store::schema::acquire_conn;
use agent_domain::TaskId;
use anyhow::Result;
use rusqlite::{params, Connection, TransactionBehavior};
use std::sync::Mutex;

pub fn delete_task(conn: &Mutex<Connection>, id: &TaskId) -> Result<bool> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let deleted = tx.execute("DELETE FROM tasks WHERE id = ?1", params![id.0])?;
    if deleted > 0 {
        let _ = tx.execute(
            "DELETE FROM subtask_checkpoints WHERE task_id = ?1",
            params![id.0],
        );
        let _ = tx.execute("DELETE FROM task_results WHERE task_id = ?1", params![id.0]);
        let _ = tx.execute(
            "DELETE FROM task_idempotency WHERE task_id = ?1",
            params![id.0],
        );
        let _ = tx.execute("DELETE FROM task_reviews WHERE task_id = ?1", params![id.0]);
        let _ = tx.execute(
            "DELETE FROM task_schedules WHERE task_id = ?1 OR schedule_id = ?1",
            params![id.0],
        );
        tx.commit()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn prune_finished_tasks(conn: &Mutex<Connection>, older_than_rfc3339: &str) -> Result<usize> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut ids = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id FROM tasks WHERE created_at < ?1 AND (status LIKE '%Completed%' OR status LIKE '%Failed%' OR status LIKE '%Canceled%')",
        )?;
        let rows = stmt.query_map(params![older_than_rfc3339], |row| row.get::<_, String>(0))?;
        for id in rows.flatten() {
            ids.push(id);
        }
    }
    let mut pruned_count = 0;
    for id in &ids {
        let _ = tx.execute(
            "DELETE FROM subtask_checkpoints WHERE task_id = ?1",
            params![id],
        );
        let _ = tx.execute("DELETE FROM task_results WHERE task_id = ?1", params![id]);
        let _ = tx.execute(
            "DELETE FROM task_idempotency WHERE task_id = ?1",
            params![id],
        );
        let _ = tx.execute("DELETE FROM task_reviews WHERE task_id = ?1", params![id]);
        if tx
            .execute("DELETE FROM tasks WHERE id = ?1", params![id])
            .is_ok()
        {
            pruned_count += 1;
        }
    }
    tx.commit()?;
    Ok(pruned_count)
}

pub fn clear_all(conn: &Mutex<Connection>) -> Result<()> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let _ = tx.execute("DELETE FROM task_schedules", []);
    tx.execute("DELETE FROM task_reviews", [])?;
    tx.execute("DELETE FROM task_idempotency", [])?;
    tx.execute("DELETE FROM task_results", [])?;
    tx.execute("DELETE FROM subtask_checkpoints", [])?;
    tx.execute("DELETE FROM tasks", [])?;
    tx.commit()?;
    Ok(())
}
