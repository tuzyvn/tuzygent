use crate::task_store::schema::acquire_conn;
use agent_domain::{Task, TaskId, TaskScheduleRecord, TaskStatus};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::sync::Mutex;

pub fn save_schedule(conn: &Mutex<Connection>, record: &TaskScheduleRecord) -> Result<()> {
    let conn = acquire_conn(conn);
    let raw_json = serde_json::to_string(record)?;
    let spec_json = serde_json::to_string(&record.spec)?;
    let priority_json = serde_json::to_string(&record.priority)?;
    let task_id_str = record.task_id.as_ref().map(|id| id.0.clone());
    let last_run_str = record.last_run_at.map(|dt| dt.to_rfc3339());
    let last_spawned_str = record.last_spawned_task_id.as_ref().map(|id| id.0.clone());
    let max_runs_i64 = record.max_runs.map(|m| i64::try_from(m).unwrap_or(i64::MAX));
    let run_count_i64 = i64::try_from(record.run_count).unwrap_or(i64::MAX);

    conn.execute(
        "INSERT INTO task_schedules (
             schedule_id, task_id, title, goal, priority, spec_json,
             next_run_at, last_run_at, last_spawned_task_id, run_count,
             max_runs, enabled, engine_name, created_at, updated_at, raw_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(schedule_id) DO UPDATE SET
             task_id = excluded.task_id,
             title = excluded.title,
             goal = excluded.goal,
             priority = excluded.priority,
             spec_json = excluded.spec_json,
             next_run_at = excluded.next_run_at,
             last_run_at = excluded.last_run_at,
             last_spawned_task_id = excluded.last_spawned_task_id,
             run_count = excluded.run_count,
             max_runs = excluded.max_runs,
             enabled = excluded.enabled,
             engine_name = excluded.engine_name,
             updated_at = excluded.updated_at,
             raw_json = excluded.raw_json",
        params![
            record.schedule_id,
            task_id_str,
            record.title,
            record.goal_description,
            priority_json,
            spec_json,
            record.next_run_at.to_rfc3339(),
            last_run_str,
            last_spawned_str,
            run_count_i64,
            max_runs_i64,
            i32::from(record.enabled),
            record.engine_name,
            record.created_at.to_rfc3339(),
            record.updated_at.to_rfc3339(),
            raw_json,
        ],
    )?;
    Ok(())
}

pub fn get_schedule(conn: &Mutex<Connection>, schedule_id: &str) -> Result<Option<TaskScheduleRecord>> {
    let conn = acquire_conn(conn);
    let raw: Option<String> = conn
        .query_row(
            "SELECT raw_json FROM task_schedules WHERE schedule_id = ?1",
            params![schedule_id],
            |row| row.get(0),
        )
        .optional()?;
    match raw {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

pub fn list_schedules(
    conn: &Mutex<Connection>,
    limit: usize,
    offset: usize,
) -> Result<Vec<TaskScheduleRecord>> {
    let conn = acquire_conn(conn);
    let mut stmt = conn.prepare(
        "SELECT raw_json FROM task_schedules ORDER BY next_run_at ASC LIMIT ?1 OFFSET ?2",
    )?;
    let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
        row.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for raw in rows.flatten() {
        if let Ok(rec) = serde_json::from_str(&raw) {
            out.push(rec);
        }
    }
    Ok(out)
}

pub fn delete_schedule(conn: &Mutex<Connection>, schedule_id: &str) -> Result<bool> {
    let conn = acquire_conn(conn);
    let deleted = conn.execute(
        "DELETE FROM task_schedules WHERE schedule_id = ?1",
        params![schedule_id],
    )?;
    Ok(deleted > 0)
}

/// Promotes all delayed tasks whose `run_after <= now` from `TaskStatus::Scheduled` to `TaskStatus::Queued`.
pub fn promote_due_delayed_tasks(
    conn: &Mutex<Connection>,
    now: DateTime<Utc>,
) -> Result<Vec<Task>> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut candidates: Vec<Task> = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT raw_json FROM tasks WHERE status LIKE '%Scheduled%' ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for raw in rows.flatten() {
            if let Ok(task) = serde_json::from_str::<Task>(&raw) {
                let due = match task.status {
                    TaskStatus::Scheduled { run_after } => {
                        task.scheduled_at.unwrap_or(run_after) <= now
                    }
                    _ => false,
                };
                if due {
                    candidates.push(task);
                }
            }
        }
    }

    let now_str = now.to_rfc3339();
    let mut promoted = Vec::with_capacity(candidates.len());
    for mut task in candidates {
        task.status = TaskStatus::Queued;
        task.updated_at = now;
        let status_json = serde_json::to_string(&task.status)?;
        let raw_json = serde_json::to_string(&task)?;
        tx.execute(
            "UPDATE tasks SET status = ?1, updated_at = ?2, raw_json = ?3 WHERE id = ?4",
            params![status_json, now_str, raw_json, task.id.0],
        )?;
        let _ = tx.execute(
            "DELETE FROM task_results WHERE task_id = ?1",
            params![task.id.0],
        );

        let sched_raw: Option<String> = tx
            .query_row(
                "SELECT raw_json FROM task_schedules WHERE schedule_id = ?1 OR task_id = ?1",
                params![task.id.0],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(s_raw) = sched_raw {
            if let Ok(mut sched) = serde_json::from_str::<TaskScheduleRecord>(&s_raw) {
                let _ = sched.advance_after_trigger(now, task.id.clone());
                let updated_sched_json = serde_json::to_string(&sched)?;
                let _ = tx.execute(
                    "UPDATE task_schedules SET last_run_at = ?1, last_spawned_task_id = ?2, run_count = ?3, enabled = ?4, updated_at = ?5, raw_json = ?6 WHERE schedule_id = ?7",
                    params![
                        now_str,
                        task.id.0,
                        i64::try_from(sched.run_count).unwrap_or(i64::MAX),
                        i32::from(sched.enabled),
                        now_str,
                        updated_sched_json,
                        sched.schedule_id,
                    ],
                );
            }
        }
        promoted.push(task);
    }

    tx.commit()?;
    Ok(promoted)
}

/// Evaluates all enabled recurring schedules whose `next_run_at <= now`,
/// spawns a ready-to-run `TaskStatus::Queued` task for each, and advances `next_run_at` to the next cycle.
pub fn trigger_due_cron_schedules(
    conn: &Mutex<Connection>,
    now: DateTime<Utc>,
) -> Result<Vec<(TaskScheduleRecord, Task)>> {
    let mut conn = acquire_conn(conn);
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut due_records: Vec<TaskScheduleRecord> = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT raw_json FROM task_schedules WHERE enabled = 1 ORDER BY next_run_at ASC",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for raw in rows.flatten() {
            if let Ok(rec) = serde_json::from_str::<TaskScheduleRecord>(&raw) {
                if rec.enabled && rec.spec.is_recurring() && rec.next_run_at <= now {
                    due_records.push(rec);
                }
            }
        }
    }

    let now_str = now.to_rfc3339();
    let mut triggered = Vec::with_capacity(due_records.len());

    for mut sched in due_records {
        let cron_expression = match &sched.spec {
            agent_domain::ScheduleSpec::Cron { expression } => Some(expression.clone()),
            _ => None,
        };
        let spawned_task = Task {
            id: TaskId::new(),
            title: sched.title.clone(),
            goal_description: sched.goal_description.clone(),
            priority: sched.priority.clone(),
            status: TaskStatus::Queued,
            subtasks: Vec::new(),
            created_at: now,
            updated_at: now,
            assigned_agent: None,
            engine_name: sched.engine_name.clone(),
            failure_count: 0,
            run_after: Some(sched.next_run_at),
            scheduled_at: Some(sched.next_run_at),
            cron_expression,
            schedule_spec: Some(sched.spec.clone()),
        };

        let task_raw = serde_json::to_string(&spawned_task)?;
        let status_json = serde_json::to_string(&spawned_task.status)?;
        let prio_json = serde_json::to_string(&spawned_task.priority)?;
        tx.execute(
            "INSERT INTO tasks (id, title, goal, priority, status, created_at, updated_at, raw_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                spawned_task.id.0,
                spawned_task.title,
                spawned_task.goal_description,
                prio_json,
                status_json,
                now_str,
                now_str,
                task_raw,
            ],
        )?;

        let trigger_ref = if now > sched.next_run_at { now } else { sched.next_run_at };
        sched.advance_after_trigger(trigger_ref, spawned_task.id.clone())?;
        let sched_raw = serde_json::to_string(&sched)?;
        tx.execute(
            "UPDATE task_schedules
             SET next_run_at = ?1, last_run_at = ?2, last_spawned_task_id = ?3,
                 run_count = ?4, enabled = ?5, updated_at = ?6, raw_json = ?7
             WHERE schedule_id = ?8",
            params![
                sched.next_run_at.to_rfc3339(),
                sched.last_run_at.map(|dt| dt.to_rfc3339()),
                spawned_task.id.0,
                i64::try_from(sched.run_count).unwrap_or(i64::MAX),
                i32::from(sched.enabled),
                now_str,
                sched_raw,
                sched.schedule_id,
            ],
        )?;

        triggered.push((sched, spawned_task));
    }

    tx.commit()?;
    Ok(triggered)
}
