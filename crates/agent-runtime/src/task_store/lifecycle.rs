use crate::task_store::schema::acquire_conn;
use crate::task_store::IdempotentSubmission;
use agent_domain::{Task, TaskId, TaskResult, TaskStatus};
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
        params![serde_json::to_string(&updated.status)?, now, serde_json::to_string(updated)?, updated.id.0],
    )?;
    tx.execute(
        "UPDATE subtask_checkpoints SET subtasks_json=?1, updated_at=?2 WHERE task_id=?3",
        params![serde_json::to_string(&updated.subtasks)?, now, updated.id.0],
    )?;
    tx.execute(
        "INSERT INTO task_reviews(task_id,revision,reviewer,decision,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![updated.id.0, revision, reviewer, decision, now],
    )?;
    if updated.status == TaskStatus::Queued {
        tx.execute("DELETE FROM task_results WHERE task_id=?1", params![updated.id.0])?;
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

pub fn cancel_task(conn: &Mutex<Connection>, id: &TaskId) -> Result<bool> {
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
        return Ok(false);
    };
    let mut task: Task = serde_json::from_str(&raw)?;
    if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed { .. }) {
        return Err(anyhow!("Cannot cancel a finished task"));
    }
    if matches!(task.status, TaskStatus::Canceled { .. }) {
        return Ok(true);
    }
    let reason = "Canceled by user request".to_string();
    task.status = TaskStatus::Canceled { reason: reason.clone() };
    task.updated_at = chrono::Utc::now();
    let result = TaskResult {
        task_id: id.clone(),
        status: task.status.clone(),
        summary: reason,
        artifacts: Vec::new(),
        verifications: Vec::new(),
        duration_ms: 0,
        engine_used: None,
    };
    let now = task.updated_at.to_rfc3339();
    tx.execute(
        "UPDATE tasks SET status=?1, updated_at=?2, raw_json=?3 WHERE id=?4",
        params![serde_json::to_string(&task.status)?, now, serde_json::to_string(&task)?, id.0],
    )?;
    tx.execute(
        "INSERT INTO task_results (task_id, status, summary, duration_ms, raw_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(task_id) DO UPDATE SET status=excluded.status, summary=excluded.summary, duration_ms=excluded.duration_ms, raw_json=excluded.raw_json",
        params![id.0, serde_json::to_string(&result.status)?, result.summary, 0, serde_json::to_string(&result)?, now],
    )?;
    tx.commit()?;
    Ok(true)
}

pub fn finish_task(
    conn: &Mutex<Connection>,
    task: &Task,
    result: &TaskResult,
    preserve_checkpoint: bool,
) -> Result<()> {
    if task.id != result.task_id {
        return Err(anyhow!("Task and result IDs differ"));
    }
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let raw: Option<String> = tx
        .query_row(
            "SELECT raw_json FROM tasks WHERE id = ?1",
            params![task.id.0],
            |row| row.get(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Err(anyhow!("Task {} not found", task.id.0));
    };
    let current: Task = serde_json::from_str(&raw)?;
    if matches!(current.status, TaskStatus::Completed | TaskStatus::Failed { .. }) {
        return Err(anyhow!("Task is already finished: {:?}", current.status));
    }
    let mut final_task = task.clone();
    let mut final_result = result.clone();
    if let TaskStatus::Canceled { ref reason } = current.status {
        final_task.status = current.status.clone();
        final_result.status = current.status.clone();
        final_result.summary = reason.clone();
    } else if matches!(result.status, TaskStatus::Completed | TaskStatus::Failed { .. }) {
        final_task.status = result.status.clone();
        final_result.status = result.status.clone();
    } else {
        final_result.status = final_task.status.clone();
    }
    final_task.updated_at = chrono::Utc::now();
    let now = final_task.updated_at.to_rfc3339();
    tx.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2, raw_json = ?3 WHERE id = ?4",
        params![
            serde_json::to_string(&final_task.status)?,
            now,
            serde_json::to_string(&final_task)?,
            final_task.id.0
        ],
    )?;
    if !preserve_checkpoint {
        tx.execute(
            "INSERT INTO subtask_checkpoints (task_id, subtasks_json, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(task_id) DO UPDATE SET subtasks_json=excluded.subtasks_json, updated_at=excluded.updated_at",
            params![final_task.id.0, serde_json::to_string(&final_task.subtasks)?, now],
        )?;
    }
    tx.execute(
        "INSERT INTO task_results (task_id, status, summary, duration_ms, raw_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(task_id) DO UPDATE SET status=excluded.status, summary=excluded.summary, duration_ms=excluded.duration_ms, raw_json=excluded.raw_json",
        params![
            final_task.id.0,
            serde_json::to_string(&final_result.status)?,
            final_result.summary,
            i64::try_from(final_result.duration_ms).unwrap_or(i64::MAX),
            serde_json::to_string(&final_result)?,
            now
        ],
    )?;
    tx.commit()?;
    Ok(())
}

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
    if task.status != TaskStatus::Queued {
        return Ok(None);
    }
    task.status = TaskStatus::Executing;
    task.updated_at = chrono::Utc::now();
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
    let raw = {
        let mut stmt = tx.prepare(
            "SELECT raw_json FROM tasks WHERE status = '\"Queued\"' OR status LIKE '%Queued%'
             ORDER BY CASE
                 WHEN priority LIKE '%Critical%' THEN 1
                 WHEN priority LIKE '%High%' THEN 2
                 WHEN priority LIKE '%Normal%' THEN 3
                 ELSE 4 END, created_at ASC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
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
    task.status = TaskStatus::Executing;
    task.updated_at = chrono::Utc::now();
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
