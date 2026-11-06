use super::TaskService;
use crate::task_store::SqliteTaskStore;
use agent_domain::{Priority, TaskStatus};
use chrono::{Duration, TimeZone, Utc};
use std::sync::Arc;

fn build_test_service() -> TaskService {
    let store = Arc::new(SqliteTaskStore::memory().expect("in-memory sqlite store"));
    TaskService::new(store, "/tmp", 2, None, None)
}

#[tokio::test]
async fn test_delayed_task_queue_promotion_with_explicit_timestamps() {
    let service = build_test_service();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 7, 10, 0, 0).single().unwrap();
    let scheduled_at = Utc.with_ymd_and_hms(2026, 11, 7, 10, 5, 0).single().unwrap();

    let task_id = service
        .submit_delayed_task(
            "Delayed Index Rebuild",
            "Rebuild vector index after cooldown window",
            Priority::High,
            scheduled_at,
        )
        .await
        .expect("submit delayed task");

    let initial = service.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(initial.scheduled_at, Some(scheduled_at));
    assert_eq!(initial.status, TaskStatus::Scheduled { run_after: scheduled_at });
    assert!(!initial.is_ready_to_execute(t0));

    // Before scheduled_at: task must remain Scheduled and not ready to execute
    let due_early = service.dispatch_due_tasks(t0, None).await.unwrap();
    assert!(due_early.is_empty());
    let still_scheduled = service.get_task(&task_id).await.unwrap().unwrap();
    assert!(still_scheduled.is_scheduled());
    assert!(!still_scheduled.is_ready_to_execute(t0));

    // 1 second before scheduled_at: still Scheduled
    let one_sec_before = scheduled_at - Duration::seconds(1);
    let due_almost = service.dispatch_due_tasks(one_sec_before, None).await.unwrap();
    assert!(due_almost.is_empty());
    let still_waiting = service.get_task(&task_id).await.unwrap().unwrap();
    assert!(still_waiting.is_scheduled());
    assert!(!still_waiting.is_ready_to_execute(one_sec_before));

    // At scheduled_at: task transitions to Queued and is ready to execute
    let due_now = service.dispatch_due_tasks(scheduled_at, None).await.unwrap();
    assert_eq!(due_now, vec![task_id.clone()]);
    let promoted = service.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(promoted.status, TaskStatus::Queued);
    assert!(promoted.is_ready_to_execute(scheduled_at));

    // Subsequent tick must not re-promote the already queued task
    let due_again = service
        .dispatch_due_tasks(scheduled_at + Duration::minutes(1), None)
        .await
        .unwrap();
    assert!(due_again.is_empty());
}

#[tokio::test]
async fn test_staggered_delayed_tasks_only_ready_after_individual_delay_elapsed() {
    let service = build_test_service();
    let base = Utc.with_ymd_and_hms(2026, 11, 7, 14, 0, 0).single().unwrap();
    let delay_short = base + Duration::seconds(30);
    let delay_long = base + Duration::seconds(120);

    let short_id = service
        .submit_delayed_task("Short Delay Task", "Run after 30s", Priority::Normal, delay_short)
        .await
        .expect("submit short delayed task");
    let long_id = service
        .submit_delayed_task("Long Delay Task", "Run after 120s", Priority::High, delay_long)
        .await
        .expect("submit long delayed task");

    // At base + 15s: neither task has passed its delay
    let t_15s = base + Duration::seconds(15);
    let report_15s = BaseTick::run(&service, t_15s).await;
    assert!(report_15s.promoted_delayed_tasks.is_empty());
    assert!(!service.get_task(&short_id).await.unwrap().unwrap().is_ready_to_execute(t_15s));
    assert!(!service.get_task(&long_id).await.unwrap().unwrap().is_ready_to_execute(t_15s));

    // At base + 30s: only short_id transitions to Queued; long_id remains Scheduled
    let report_30s = BaseTick::run(&service, delay_short).await;
    assert_eq!(report_30s.promoted_delayed_tasks, vec![short_id.clone()]);
    let short_task = service.get_task(&short_id).await.unwrap().unwrap();
    let long_task = service.get_task(&long_id).await.unwrap().unwrap();
    assert_eq!(short_task.status, TaskStatus::Queued);
    assert!(short_task.is_ready_to_execute(delay_short));
    assert_eq!(long_task.status, TaskStatus::Scheduled { run_after: delay_long });
    assert!(!long_task.is_ready_to_execute(delay_short));

    // Linked one-shot schedule for short_id is marked completed/disabled
    let short_sched = service.get_schedule(&short_id.0).await.unwrap().unwrap();
    assert_eq!(short_sched.run_count, 1);
    assert!(!short_sched.enabled);

    // At base + 120s: long_id now transitions to Queued and is ready to execute
    let report_120s = BaseTick::run(&service, delay_long).await;
    assert_eq!(report_120s.promoted_delayed_tasks, vec![long_id.clone()]);
    let long_promoted = service.get_task(&long_id).await.unwrap().unwrap();
    assert_eq!(long_promoted.status, TaskStatus::Queued);
    assert!(long_promoted.is_ready_to_execute(delay_long));
}

#[tokio::test]
async fn test_submit_delayed_task_after_duration_transitions_only_after_delay() {
    let service = build_test_service();
    let before_submit = Utc::now();
    let delay = std::time::Duration::from_secs(60);

    let task_id = service
        .submit_delayed_task_after(
            "Duration Delayed Task",
            "Execute after 60 seconds",
            Priority::Normal,
            delay,
        )
        .await
        .expect("submit delayed task after duration");

    let task = service.get_task(&task_id).await.unwrap().unwrap();
    let scheduled_at = task.scheduled_at.expect("scheduled_at must be set");
    assert!(scheduled_at >= before_submit + Duration::seconds(59));
    assert!(task.is_scheduled());
    assert!(!task.is_ready_to_execute(before_submit));

    // Tick before delay elapses
    let early = service
        .dispatch_due_tasks(scheduled_at - Duration::milliseconds(1), None)
        .await
        .unwrap();
    assert!(early.is_empty());

    // Tick after delay has elapsed
    let after = service
        .dispatch_due_tasks(scheduled_at + Duration::milliseconds(1), None)
        .await
        .unwrap();
    assert_eq!(after, vec![task_id.clone()]);
    let ready_task = service.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(ready_task.status, TaskStatus::Queued);
    assert!(ready_task.is_ready_to_execute(scheduled_at + Duration::milliseconds(1)));
}

#[tokio::test]
async fn test_cron_scheduler_registration_and_tick_with_explicit_timestamps() {
    let service = build_test_service();
    let base_time = Utc.with_ymd_and_hms(2026, 11, 7, 12, 0, 0).single().unwrap();

    let schedule = service
        .register_cron_task(
            "Periodic Health Sweep",
            "Run every 5 minutes",
            Priority::Normal,
            "*/5 * * * *",
            base_time,
        )
        .await
        .expect("register cron schedule");

    let expected_first_run = Utc.with_ymd_and_hms(2026, 11, 7, 12, 5, 0).single().unwrap();
    assert_eq!(schedule.next_run_at, expected_first_run);

    // Tick before due time spawns nothing
    let early_tick = BaseTick::run(&service, Utc.with_ymd_and_hms(2026, 11, 7, 12, 4, 59).single().unwrap()).await;
    assert!(early_tick.spawned_cron_tasks.is_empty());

    // Tick at 12:05:00 spawns a Queued task and advances next_run_at to 12:10:00
    let due_tick = BaseTick::run(&service, expected_first_run).await;
    assert_eq!(due_tick.spawned_cron_tasks.len(), 1);

    let spawned_id = &due_tick.spawned_cron_tasks[0];
    let spawned_task = service.get_task(spawned_id).await.unwrap().unwrap();
    assert_eq!(spawned_task.status, TaskStatus::Queued);
    assert_eq!(spawned_task.cron_expression.as_deref(), Some("*/5 * * * *"));
    assert!(spawned_task.is_ready_to_execute(expected_first_run));

    let updated_sched = service.get_schedule(&schedule.schedule_id).await.unwrap().unwrap();
    let expected_second_run = Utc.with_ymd_and_hms(2026, 11, 7, 12, 10, 0).single().unwrap();
    assert_eq!(updated_sched.run_count, 1);
    assert_eq!(updated_sched.last_run_at, Some(expected_first_run));
    assert_eq!(updated_sched.last_spawned_task_id.as_ref(), Some(spawned_id));
    assert_eq!(updated_sched.next_run_at, expected_second_run);
}

#[tokio::test]
async fn test_cron_task_multi_cycle_progression_and_combined_scheduler_tick() {
    let service = build_test_service();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 7, 15, 0, 0).single().unwrap();

    // Register a recurring cron task firing every 10 minutes
    let cron_sched = service
        .register_cron_task(
            "Metrics Aggregation",
            "Aggregate telemetry every 10m",
            Priority::Critical,
            "*/10 * * * *",
            t0,
        )
        .await
        .expect("register 10m cron task");

    let cycle1 = Utc.with_ymd_and_hms(2026, 11, 7, 15, 10, 0).single().unwrap();
    let cycle2 = Utc.with_ymd_and_hms(2026, 11, 7, 15, 20, 0).single().unwrap();
    let cycle3 = Utc.with_ymd_and_hms(2026, 11, 7, 15, 30, 0).single().unwrap();
    assert_eq!(cron_sched.next_run_at, cycle1);

    // Also submit a delayed task due at cycle2 (15:20:00)
    let delayed_id = service
        .submit_delayed_task(
            "One-Off Cache Warmup",
            "Warm cache at 15:20:00",
            Priority::High,
            cycle2,
        )
        .await
        .expect("submit delayed task for cycle2");

    // Cycle 1 (15:10:00): Cron triggers run #1, delayed task is still Scheduled
    let tick1 = BaseTick::run(&service, cycle1).await;
    assert_eq!(tick1.spawned_cron_tasks.len(), 1);
    assert!(tick1.promoted_delayed_tasks.is_empty());
    assert_eq!(tick1.total_dispatched(), 1);

    let sched_after_1 = service.get_schedule(&cron_sched.schedule_id).await.unwrap().unwrap();
    assert_eq!(sched_after_1.run_count, 1);
    assert_eq!(sched_after_1.next_run_at, cycle2);

    // Mid-cycle tick at 15:15:00: nothing triggers
    let tick_mid = BaseTick::run(&service, t0 + Duration::minutes(15)).await;
    assert_eq!(tick_mid.total_dispatched(), 0);

    // Cycle 2 (15:20:00): Both the cron task (run #2) and the delayed task trigger together
    let tick2 = BaseTick::run(&service, cycle2).await;
    assert_eq!(tick2.promoted_delayed_tasks, vec![delayed_id.clone()]);
    assert_eq!(tick2.spawned_cron_tasks.len(), 1);
    assert_eq!(tick2.total_dispatched(), 2);
    assert_ne!(tick1.spawned_cron_tasks[0], tick2.spawned_cron_tasks[0]);

    let sched_after_2 = service.get_schedule(&cron_sched.schedule_id).await.unwrap().unwrap();
    assert_eq!(sched_after_2.run_count, 2);
    assert_eq!(sched_after_2.last_run_at, Some(cycle2));
    assert_eq!(sched_after_2.next_run_at, cycle3);

    // Cycle 3 (15:30:00): Cron triggers run #3 and computes 15:40:00 as next_run_at
    let tick3 = BaseTick::run(&service, cycle3).await;
    assert!(tick3.promoted_delayed_tasks.is_empty());
    assert_eq!(tick3.spawned_cron_tasks.len(), 1);

    let sched_after_3 = service.get_schedule(&cron_sched.schedule_id).await.unwrap().unwrap();
    let cycle4 = Utc.with_ymd_and_hms(2026, 11, 7, 15, 40, 0).single().unwrap();
    assert_eq!(sched_after_3.run_count, 3);
    assert_eq!(sched_after_3.last_run_at, Some(cycle3));
    assert_eq!(sched_after_3.next_run_at, cycle4);

    // Deleting schedule stops future cron spawns
    assert!(service.delete_schedule(&cron_sched.schedule_id).await.unwrap());
    let tick4 = BaseTick::run(&service, cycle4).await;
    assert!(tick4.spawned_cron_tasks.is_empty());
}

struct BaseTick;
impl BaseTick {
    async fn run(service: &TaskService, now: chrono::DateTime<Utc>) -> super::SchedulerTickReport {
        service.tick_scheduler(now, None).await.expect("tick scheduler")
    }
}
