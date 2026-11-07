use super::{SqliteTaskStore, TaskStore};
use agent_domain::{Priority, ScheduleSpec, Task, TaskScheduleRecord, TaskStatus};
use chrono::{DateTime, TimeZone, Utc};
use std::path::PathBuf;

fn temp_workspace(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("agent-runtime-{label}-{}", uuid::Uuid::new_v4()))
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .expect("query sqlite schema")
}

#[test]
fn resolve_storage_dir_creates_storage_and_quarantines_a_file_at_that_path() {
    let workspace = temp_workspace("resolve-storage");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let storage_path = workspace.join("storage");
    std::fs::write(&storage_path, b"not a directory").expect("create conflicting file");

    let resolved = super::schema::resolve_storage_dir(&workspace);

    assert_eq!(resolved, storage_path);
    assert!(resolved.is_dir());
    assert!(std::fs::read_dir(&workspace)
        .expect("list workspace")
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with("storage_file_corrupt_")));
    std::fs::remove_dir_all(workspace).expect("clean workspace");
}

#[test]
fn open_resilient_sqlite_recreates_a_database_with_an_incompatible_schema() {
    let workspace = temp_workspace("open-resilient");
    let storage = workspace.join("storage");
    std::fs::create_dir_all(&storage).expect("create storage directory");
    let db_path = storage.join("tasks.db");
    let malformed = rusqlite::Connection::open(&db_path).expect("create malformed database");
    malformed
        .execute_batch("CREATE TABLE tasks (id TEXT PRIMARY KEY);")
        .expect("install incompatible tasks table");
    drop(malformed);

    let (opened_path, conn) = super::schema::open_resilient_sqlite(&workspace, 1_250, false)
        .expect("recreate database with a valid schema");

    assert_eq!(opened_path, db_path);
    assert!(table_exists(&conn, "tasks"));
    assert!(table_exists(&conn, "task_schedules"));
    let title_column_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('tasks') WHERE name = 'title')",
            [],
            |row| row.get(0),
        )
        .expect("query recreated tasks columns");
    assert!(title_column_exists);
    assert_eq!(
        conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .expect("read journal mode"),
        "delete"
    );
    assert!(std::fs::read_dir(&storage)
        .expect("list storage directory")
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with("tasks.db.malformed.")));
    drop(conn);
    std::fs::remove_dir_all(workspace).expect("clean workspace");
}

#[test]
fn init_sqlite_schema_creates_expected_tables_and_is_repeatable() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory database");

    super::schema::init_sqlite_schema(&conn, false).expect("initialize schema");
    super::schema::init_sqlite_schema(&conn, false).expect("initialize schema again");

    for table in [
        "tasks",
        "subtask_checkpoints",
        "task_results",
        "task_idempotency",
        "task_reviews",
        "task_schedules",
    ] {
        assert!(table_exists(&conn, table), "missing table {table}");
    }
}

#[test]
fn init_sqlite_schema_with_options_applies_timeout_and_journal_mode() {
    let workspace = temp_workspace("schema-options");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let conn =
        rusqlite::Connection::open(workspace.join("tasks.db")).expect("open file-backed database");

    super::schema::init_sqlite_schema_with_options(&conn, true, 1_250, false)
        .expect("initialize schema with custom options");

    let timeout: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("read busy timeout");
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("read journal mode");
    assert_eq!(timeout, 1_250);
    assert_eq!(journal_mode, "delete");
    assert!(table_exists(&conn, "tasks"));

    drop(conn);
    std::fs::remove_dir_all(workspace).expect("clean workspace");
}

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 7, hour, minute, 0)
        .single()
        .expect("valid fixture timestamp")
}

fn schedule(id: &str, spec: ScheduleSpec, next_run_at: DateTime<Utc>) -> TaskScheduleRecord {
    let mut record = TaskScheduleRecord::new(
        "scheduled task",
        "run it",
        Priority::Normal,
        spec,
        next_run_at,
    )
    .expect("valid schedule fixture");
    record.schedule_id = id.to_string();
    record.next_run_at = next_run_at;
    record
}

#[tokio::test]
async fn save_schedule_inserts_and_updates_existing_record() {
    let store = SqliteTaskStore::memory().expect("in-memory task store");
    let first = schedule(
        "schedule-upsert",
        ScheduleSpec::every_seconds(60),
        at(10, 0),
    );
    TaskStore::save_schedule(&store, &first)
        .await
        .expect("insert schedule");

    let mut updated = first.clone();
    updated.title = "updated title".to_string();
    updated.next_run_at = at(10, 5);
    TaskStore::save_schedule(&store, &updated)
        .await
        .expect("update schedule");

    assert_eq!(
        TaskStore::get_schedule(&store, "schedule-upsert")
            .await
            .unwrap(),
        Some(updated)
    );
}

#[tokio::test]
async fn save_schedule_returns_database_errors() {
    let store = SqliteTaskStore::memory().expect("in-memory task store");
    {
        let conn = store.conn.lock().expect("lock task store connection");
        conn.execute_batch(
            "CREATE TRIGGER reject_schedule_insert
             BEFORE INSERT ON task_schedules
             BEGIN
                 SELECT RAISE(ABORT, 'schedule insert rejected');
             END;",
        )
        .expect("install rejecting trigger");
    }

    let record = schedule(
        "rejected-schedule",
        ScheduleSpec::every_seconds(60),
        at(10, 0),
    );
    assert!(TaskStore::save_schedule(&store, &record).await.is_err());
    assert_eq!(
        TaskStore::get_schedule(&store, &record.schedule_id)
            .await
            .expect("query rejected schedule"),
        None
    );
}

#[tokio::test]
async fn get_schedule_returns_none_for_an_unknown_id() {
    let store = SqliteTaskStore::memory().expect("in-memory task store");

    assert_eq!(
        TaskStore::get_schedule(&store, "missing-schedule")
            .await
            .expect("get missing schedule"),
        None
    );
}

#[tokio::test]
async fn delete_schedule_reports_deletions_and_preserves_other_records() {
    let store = SqliteTaskStore::memory().expect("in-memory task store");
    let first = schedule("delete-me", ScheduleSpec::every_seconds(60), at(10, 0));
    let other = schedule("keep-me", ScheduleSpec::every_seconds(120), at(11, 0));
    for record in [&first, &other] {
        TaskStore::save_schedule(&store, record)
            .await
            .expect("save schedule");
    }

    assert!(TaskStore::delete_schedule(&store, &first.schedule_id)
        .await
        .expect("delete existing schedule"));
    assert!(!TaskStore::delete_schedule(&store, &first.schedule_id)
        .await
        .expect("delete already deleted schedule"));
    assert!(!TaskStore::delete_schedule(&store, "unknown-schedule")
        .await
        .expect("delete unknown schedule"));
    assert_eq!(
        TaskStore::get_schedule(&store, &first.schedule_id)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        TaskStore::get_schedule(&store, &other.schedule_id)
            .await
            .unwrap(),
        Some(other)
    );
}

#[tokio::test]
async fn list_schedules_orders_and_paginates_with_empty_boundaries() {
    let store = SqliteTaskStore::memory().expect("in-memory task store");
    for (id, minute) in [("later", 20), ("earliest", 5), ("middle", 10)] {
        TaskStore::save_schedule(
            &store,
            &schedule(id, ScheduleSpec::every_seconds(60), at(10, minute)),
        )
        .await
        .expect("save schedule");
    }

    let page = TaskStore::list_schedules(&store, 1, 1)
        .await
        .expect("list page");
    assert_eq!(
        page.iter()
            .map(|item| item.schedule_id.as_str())
            .collect::<Vec<_>>(),
        ["middle"]
    );
    assert!(TaskStore::list_schedules(&store, 2, 3)
        .await
        .unwrap()
        .is_empty());
    assert!(TaskStore::list_schedules(&store, 0, 0)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn promote_due_delayed_tasks_includes_due_boundary_and_fallback_and_skips_future() {
    let store = SqliteTaskStore::memory().expect("in-memory task store");
    let now = at(10, 0);

    let mut boundary = Task::new("boundary", "due now", Priority::Normal);
    boundary.status = TaskStatus::Scheduled { run_after: now };
    boundary.run_after = Some(now);
    boundary.scheduled_at = Some(now);
    store
        .save_task(&boundary)
        .await
        .expect("save boundary task");

    let mut fallback = Task::new("fallback", "uses run_after", Priority::High);
    fallback.status = TaskStatus::Scheduled {
        run_after: now - chrono::Duration::minutes(1),
    };
    fallback.run_after = Some(now - chrono::Duration::minutes(1));
    store
        .save_task(&fallback)
        .await
        .expect("save fallback task");

    let future_time = now + chrono::Duration::minutes(1);
    let mut future = Task::new("future", "not due yet", Priority::Low);
    future.status = TaskStatus::Scheduled {
        run_after: future_time,
    };
    future.run_after = Some(future_time);
    future.scheduled_at = Some(future_time);
    store.save_task(&future).await.expect("save future task");

    let promoted = TaskStore::promote_due_delayed_tasks(&store, now)
        .await
        .expect("promote due tasks");
    assert_eq!(promoted.len(), 2);
    assert!(promoted
        .iter()
        .all(|task| task.status == TaskStatus::Queued));
    assert!(promoted.iter().any(|task| task.id == boundary.id));
    assert!(promoted.iter().any(|task| task.id == fallback.id));
    assert_eq!(
        store.get_task(&future.id).await.unwrap().unwrap().status,
        future.status
    );
    assert!(TaskStore::promote_due_delayed_tasks(&store, now)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn promote_due_delayed_tasks_updates_linked_schedule_clears_results_and_skips_bad_json() {
    let store = SqliteTaskStore::memory().expect("in-memory task store");
    let now = at(10, 0);
    let due_at = now - chrono::Duration::minutes(1);
    let mut task = Task::new("linked task", "due task", Priority::Normal);
    task.status = TaskStatus::Scheduled { run_after: due_at };
    task.run_after = Some(due_at);
    task.scheduled_at = Some(due_at);
    store.save_task(&task).await.expect("save due task");
    store
        .save_result(&agent_domain::TaskResult {
            task_id: task.id.clone(),
            status: TaskStatus::Failed {
                error: "stale result".to_string(),
            },
            summary: "stale result".to_string(),
            artifacts: Vec::new(),
            verifications: Vec::new(),
            duration_ms: 1,
            engine_used: None,
        })
        .await
        .expect("save stale task result");

    let mut linked_schedule = schedule(&task.id.0, ScheduleSpec::every_seconds(60), due_at);
    linked_schedule.run_count = 2;
    TaskStore::save_schedule(&store, &linked_schedule)
        .await
        .expect("save linked schedule");

    let bad_id = "bad-json-task";
    {
        let conn = store.conn.lock().expect("lock task store connection");
        conn.execute(
            "INSERT INTO tasks (id, title, goal, priority, status, created_at, updated_at, raw_json)
             VALUES (?1, 'bad json', '', 'null', ?2, ?3, ?3, '{')",
            rusqlite::params![
                bad_id,
                serde_json::to_string(&TaskStatus::Scheduled { run_after: due_at }).unwrap(),
                now.to_rfc3339(),
            ],
        )
        .expect("insert malformed task row");
    }

    let promoted = TaskStore::promote_due_delayed_tasks(&store, now)
        .await
        .expect("promote due task");
    assert_eq!(promoted.len(), 1);
    assert_eq!(promoted[0].id, task.id);
    assert_eq!(promoted[0].status, TaskStatus::Queued);
    assert_eq!(promoted[0].updated_at, now);
    assert!(store.get_result(&task.id).await.unwrap().is_none());

    let updated_schedule = TaskStore::get_schedule(&store, &linked_schedule.schedule_id)
        .await
        .expect("get linked schedule")
        .expect("linked schedule remains");
    assert_eq!(updated_schedule.run_count, 3);
    assert_eq!(updated_schedule.last_run_at, Some(now));
    assert_eq!(updated_schedule.last_spawned_task_id, Some(task.id.clone()));
    assert!(TaskStore::promote_due_delayed_tasks(&store, now)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn trigger_due_cron_schedules_spawns_once_and_advances_schedule() {
    let store = SqliteTaskStore::memory().expect("in-memory task store");
    let now = at(10, 0);
    let due = schedule(
        "cron-due",
        ScheduleSpec::cron("*/5 * * * *").expect("valid cron"),
        now,
    );
    let future = schedule(
        "cron-future",
        ScheduleSpec::cron("*/5 * * * *").expect("valid cron"),
        now + chrono::Duration::minutes(5),
    );
    let disabled = {
        let mut record = schedule(
            "cron-disabled",
            ScheduleSpec::cron("*/5 * * * *").expect("valid cron"),
            now - chrono::Duration::minutes(5),
        );
        record.enabled = false;
        record
    };
    let one_shot = schedule(
        "not-recurring",
        ScheduleSpec::once_at(now - chrono::Duration::minutes(1)),
        now - chrono::Duration::minutes(1),
    );
    for record in [&due, &future, &disabled, &one_shot] {
        TaskStore::save_schedule(&store, record)
            .await
            .expect("save schedule");
    }

    let triggered = TaskStore::trigger_due_cron_schedules(&store, now)
        .await
        .expect("trigger cron schedules");
    assert_eq!(triggered.len(), 1);
    let (advanced, spawned) = &triggered[0];
    assert_eq!(spawned.status, TaskStatus::Queued);
    assert_eq!(spawned.scheduled_at, Some(now));
    assert_eq!(spawned.cron_expression.as_deref(), Some("*/5 * * * *"));
    assert_eq!(advanced.schedule_id, due.schedule_id);
    assert_eq!(advanced.run_count, 1);
    assert_eq!(advanced.last_run_at, Some(now));
    assert_eq!(advanced.last_spawned_task_id.as_ref(), Some(&spawned.id));
    assert_eq!(advanced.next_run_at, now + chrono::Duration::minutes(5));
    assert_eq!(
        store.get_task(&spawned.id).await.unwrap().unwrap().status,
        TaskStatus::Queued
    );
    assert!(TaskStore::trigger_due_cron_schedules(&store, now)
        .await
        .unwrap()
        .is_empty());
}
