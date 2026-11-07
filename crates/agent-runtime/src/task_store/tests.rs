use super::{SqliteTaskStore, TaskStore};
use agent_domain::{Priority, ScheduleSpec, Task, TaskScheduleRecord, TaskStatus};
use chrono::{DateTime, TimeZone, Utc};

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
