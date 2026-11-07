use crate::task_store::schema::acquire_conn;
use crate::task_store::IdempotentSubmission;
use agent_domain::{Task, TaskId, TaskResult, TaskStatus};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::sync::Mutex;

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
    if matches!(
        task.status,
        TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Quarantined { .. }
    ) {
        return Err(anyhow!("Cannot cancel a finished task"));
    }
    if matches!(task.status, TaskStatus::Canceled { .. }) {
        return Ok(true);
    }
    let reason = "Canceled by user request".to_string();
    task.status = TaskStatus::Canceled {
        reason: reason.clone(),
    };
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
        params![
            serde_json::to_string(&task.status)?,
            now,
            serde_json::to_string(&task)?,
            id.0
        ],
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
    if matches!(
        current.status,
        TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Quarantined { .. }
    ) {
        return Err(anyhow!("Task is already finished: {:?}", current.status));
    }
    let mut final_task = task.clone();
    let mut final_result = result.clone();
    if let TaskStatus::Canceled { ref reason } = current.status {
        final_task.status = current.status.clone();
        final_result.status = current.status.clone();
        final_result.summary = reason.clone();
    } else if matches!(
        result.status,
        TaskStatus::Completed | TaskStatus::Failed { .. } | TaskStatus::Quarantined { .. }
    ) {
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
        let metadata =
            agent_domain::CheckpointMetadata::new(final_task.id.clone(), &final_task.subtasks);
        let metadata_json = serde_json::to_string(&metadata)?;
        tx.execute(
            "INSERT INTO subtask_checkpoints (task_id, subtasks_json, updated_at, metadata_json) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(task_id) DO UPDATE SET subtasks_json=excluded.subtasks_json, updated_at=excluded.updated_at, metadata_json=excluded.metadata_json",
            params![final_task.id.0, serde_json::to_string(&final_task.subtasks)?, now, metadata_json],
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
