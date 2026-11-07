use crate::task_store::schema::acquire_conn;
use agent_domain::{Task, TaskId, TaskResult};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub fn get_task_and_result(
    conn: &Mutex<Connection>,
    id: &TaskId,
) -> Result<Option<(Task, Option<TaskResult>)>> {
    let conn = acquire_conn(conn);
    let mut stmt = conn.prepare(
        "SELECT t.raw_json, r.raw_json FROM tasks t LEFT JOIN task_results r ON r.task_id = t.id WHERE t.id = ?1",
    )?;
    let mut rows = stmt.query(params![id.0])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let task_json: String = row.get(0)?;
    let result_json: Option<String> = row.get(1)?;
    Ok(Some((
        serde_json::from_str(&task_json)?,
        result_json
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?,
    )))
}

pub fn save_task(conn: &Mutex<Connection>, task: &Task) -> Result<()> {
    let conn = acquire_conn(conn);
    let raw_json = serde_json::to_string(task)?;
    let status_json = serde_json::to_string(&task.status)?;
    let priority_json = serde_json::to_string(&task.priority)?;

    conn.execute(
        "INSERT INTO tasks (id, title, goal, priority, status, created_at, updated_at, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
             title = excluded.title,
             goal = excluded.goal,
             priority = excluded.priority,
             status = excluded.status,
             updated_at = excluded.updated_at,
             raw_json = excluded.raw_json",
        params![
            task.id.0,
            task.title,
            task.goal_description,
            priority_json,
            status_json,
            task.created_at.to_rfc3339(),
            task.updated_at.to_rfc3339(),
            raw_json,
        ],
    )?;

    if task.status == agent_domain::TaskStatus::Queued {
        let _ = conn.execute(
            "DELETE FROM task_results WHERE task_id = ?1",
            params![task.id.0],
        );
    }

    Ok(())
}

pub fn get_task(conn: &Mutex<Connection>, id: &TaskId) -> Result<Option<Task>> {
    let conn = acquire_conn(conn);
    let mut stmt = conn.prepare("SELECT raw_json FROM tasks WHERE id = ?1")?;
    let mut rows = stmt.query(params![id.0])?;

    if let Some(row) = rows.next()? {
        let raw: String = row.get(0)?;
        let task = serde_json::from_str(&raw)?;
        Ok(Some(task))
    } else {
        Ok(None)
    }
}

pub fn list_tasks(conn: &Mutex<Connection>, limit: usize, offset: usize) -> Result<Vec<Task>> {
    let conn = acquire_conn(conn);
    let mut stmt =
        conn.prepare("SELECT raw_json FROM tasks ORDER BY created_at DESC LIMIT ?1 OFFSET ?2")?;
    let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
        row.get::<_, String>(0)
    })?;

    let mut tasks = Vec::new();
    for raw in rows.flatten() {
        if let Ok(task) = serde_json::from_str(&raw) {
            tasks.push(task);
        }
    }
    Ok(tasks)
}

pub fn list_tasks_by_status(
    conn: &Mutex<Connection>,
    status_filter: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<Task>> {
    let conn = acquire_conn(conn);
    let mut stmt = conn.prepare(
        "SELECT raw_json FROM tasks WHERE status = ?1 OR status LIKE ?2 ORDER BY created_at DESC LIMIT ?3 OFFSET ?4",
    )?;
    let exact_pat = format!("\"{}\"", status_filter);
    let like_pat = format!("%{}%", status_filter);
    let rows = stmt.query_map(
        params![exact_pat, like_pat, limit as i64, offset as i64],
        |row| row.get::<_, String>(0),
    )?;

    let mut tasks = Vec::new();
    for raw in rows.flatten() {
        if let Ok(task) = serde_json::from_str(&raw) {
            tasks.push(task);
        }
    }
    Ok(tasks)
}

pub fn count_tasks(conn: &Mutex<Connection>) -> Result<usize> {
    let conn = acquire_conn(conn);
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?;
    Ok(count as usize)
}

pub fn count_tasks_by_status(conn: &Mutex<Connection>, status_filter: &str) -> Result<usize> {
    let conn = acquire_conn(conn);
    let exact_pat = format!("\"{}\"", status_filter);
    let like_pat = format!("%{}%", status_filter);
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = ?1 OR status LIKE ?2",
        params![exact_pat, like_pat],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}
