use super::TaskService;
use crate::task_store::SqliteTaskStore;
use agent_domain::{Priority, ScheduleSpec, TaskStatus};
use agent_engine::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct CountingMockEngine {
    executions: Arc<AtomicUsize>,
}

impl CountingMockEngine {
    fn new() -> (Arc<Self>, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                executions: count.clone(),
            }),
            count,
        )
    }
}

#[async_trait]
impl ComputeEngine for CountingMockEngine {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let stdout = if input.prompt_or_command.contains("Architect")
            || input.prompt_or_command.contains("JSON")
        {
            r#"[{"id":"ST-1","title":"Execute scheduled step","command_or_prompt":"echo scheduled_ok","expected_verification":"cmd: true"}]"#.to_string()
        } else {
            self.executions.fetch_add(1, Ordering::SeqCst);
            "Scheduled task completed.".to_string()
        };
        Ok(EngineOutput {
            stdout_content: stdout,
            exit_code: 0,
            execution_duration_ms: 2,
            engine_used: EngineKind::Mock,
            session_id: None,
        })
    }

    fn engine_kind(&self) -> EngineKind {
        EngineKind::Mock
    }
}

fn build_test_service() -> TaskService {
    let store = Arc::new(SqliteTaskStore::memory().expect("in-memory sqlite store"));
    TaskService::new(store, "/tmp", 2, None, None)
}

#[tokio::test]
async fn test_with_concurrency_clamps_zero_to_one_permit() {
    let service = build_test_service().with_concurrency(0);

    assert_eq!(service.concurrency_limit(), 1);
    assert_eq!(service.available_concurrency(), 1);
}

#[tokio::test]
async fn test_submit_and_query_task_service_api() {
    let service = build_test_service();
    let task_id = service
        .submit_task("API regression", "Keep task access stable", Priority::High)
        .await
        .expect("submit task");

    let task = service.get_task(&task_id).await.expect("get task").unwrap();
    assert_eq!(task.title, "API regression");
    assert_eq!(service.count_tasks().await.expect("count tasks"), 1);
    assert_eq!(
        service.list_tasks(10, 0).await.expect("list tasks").len(),
        1
    );
    assert_eq!(service.status_summary().await.unwrap()["queued"], 1);
}

#[tokio::test]
async fn test_dispatch_executes_queued_task_and_cleans_up_tracking() {
    let service = build_test_service();
    let (engine, exec_count) = CountingMockEngine::new();
    let task_id = service
        .submit_task(
            "Dispatch smoke test",
            "Run through dispatch",
            Priority::Normal,
        )
        .await
        .expect("submit task");

    service.dispatch(task_id.clone(), engine);

    assert!(service
        .drain(std::time::Duration::from_secs(5))
        .await
        .unwrap());
    let task = service.get_task(&task_id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    assert_eq!(exec_count.load(Ordering::SeqCst), 1);
    assert!(!service.is_active(&task_id).await);
}

#[tokio::test]
async fn test_delayed_task_queue_promotion_with_explicit_timestamps() {
    let service = build_test_service();
    let t0 = Utc
        .with_ymd_and_hms(2026, 11, 7, 10, 0, 0)
        .single()
        .unwrap();
    let scheduled_at = Utc
        .with_ymd_and_hms(2026, 11, 7, 10, 5, 0)
        .single()
        .unwrap();

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
    assert_eq!(
        initial.status,
        TaskStatus::Scheduled {
            run_after: scheduled_at
        }
    );
    assert!(!initial.is_ready_to_execute(t0));

    // Before scheduled_at: task must remain Scheduled and not ready to execute
    let due_early = service.dispatch_due_tasks(t0, None).await.unwrap();
    assert!(due_early.is_empty());
    let still_scheduled = service.get_task(&task_id).await.unwrap().unwrap();
    assert!(still_scheduled.is_scheduled());
    assert!(!still_scheduled.is_ready_to_execute(t0));

    // 1 second before scheduled_at: still Scheduled
    let one_sec_before = scheduled_at - Duration::seconds(1);
    let due_almost = service
        .dispatch_due_tasks(one_sec_before, None)
        .await
        .unwrap();
    assert!(due_almost.is_empty());
    let still_waiting = service.get_task(&task_id).await.unwrap().unwrap();
    assert!(still_waiting.is_scheduled());
    assert!(!still_waiting.is_ready_to_execute(one_sec_before));

    // At scheduled_at: task transitions to Queued and is ready to execute
    let due_now = service
        .dispatch_due_tasks(scheduled_at, None)
        .await
        .unwrap();
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
    let base = Utc
        .with_ymd_and_hms(2026, 11, 7, 14, 0, 0)
        .single()
        .unwrap();
    let delay_short = base + Duration::seconds(30);
    let delay_long = base + Duration::seconds(120);

    let short_id = service
        .submit_delayed_task(
            "Short Delay Task",
            "Run after 30s",
            Priority::Normal,
            delay_short,
        )
        .await
        .expect("submit short delayed task");
    let long_id = service
        .submit_delayed_task(
            "Long Delay Task",
            "Run after 120s",
            Priority::High,
            delay_long,
        )
        .await
        .expect("submit long delayed task");

    // At base + 15s: neither task has passed its delay
    let t_15s = base + Duration::seconds(15);
    let report_15s = BaseTick::run(&service, t_15s).await;
    assert!(report_15s.promoted_delayed_tasks.is_empty());
    assert!(!service
        .get_task(&short_id)
        .await
        .unwrap()
        .unwrap()
        .is_ready_to_execute(t_15s));
    assert!(!service
        .get_task(&long_id)
        .await
        .unwrap()
        .unwrap()
        .is_ready_to_execute(t_15s));

    // At base + 30s: only short_id transitions to Queued; long_id remains Scheduled
    let report_30s = BaseTick::run(&service, delay_short).await;
    assert_eq!(report_30s.promoted_delayed_tasks, vec![short_id.clone()]);
    let short_task = service.get_task(&short_id).await.unwrap().unwrap();
    let long_task = service.get_task(&long_id).await.unwrap().unwrap();
    assert_eq!(short_task.status, TaskStatus::Queued);
    assert!(short_task.is_ready_to_execute(delay_short));
    assert_eq!(
        long_task.status,
        TaskStatus::Scheduled {
            run_after: delay_long
        }
    );
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
    let base_time = Utc
        .with_ymd_and_hms(2026, 11, 7, 12, 0, 0)
        .single()
        .unwrap();

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

    let expected_first_run = Utc
        .with_ymd_and_hms(2026, 11, 7, 12, 5, 0)
        .single()
        .unwrap();
    assert_eq!(schedule.next_run_at, expected_first_run);

    // Tick before due time spawns nothing
    let early_tick = BaseTick::run(
        &service,
        Utc.with_ymd_and_hms(2026, 11, 7, 12, 4, 59)
            .single()
            .unwrap(),
    )
    .await;
    assert!(early_tick.spawned_cron_tasks.is_empty());

    // Tick at 12:05:00 spawns a Queued task and advances next_run_at to 12:10:00
    let due_tick = BaseTick::run(&service, expected_first_run).await;
    assert_eq!(due_tick.spawned_cron_tasks.len(), 1);

    let spawned_id = &due_tick.spawned_cron_tasks[0];
    let spawned_task = service.get_task(spawned_id).await.unwrap().unwrap();
    assert_eq!(spawned_task.status, TaskStatus::Queued);
    assert_eq!(spawned_task.cron_expression.as_deref(), Some("*/5 * * * *"));
    assert!(spawned_task.is_ready_to_execute(expected_first_run));

    let updated_sched = service
        .get_schedule(&schedule.schedule_id)
        .await
        .unwrap()
        .unwrap();
    let expected_second_run = Utc
        .with_ymd_and_hms(2026, 11, 7, 12, 10, 0)
        .single()
        .unwrap();
    assert_eq!(updated_sched.run_count, 1);
    assert_eq!(updated_sched.last_run_at, Some(expected_first_run));
    assert_eq!(
        updated_sched.last_spawned_task_id.as_ref(),
        Some(spawned_id)
    );
    assert_eq!(updated_sched.next_run_at, expected_second_run);
}

#[tokio::test]
async fn test_cron_task_multi_cycle_progression_and_combined_scheduler_tick() {
    let service = build_test_service();
    let t0 = Utc
        .with_ymd_and_hms(2026, 11, 7, 15, 0, 0)
        .single()
        .unwrap();

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

    let cycle1 = Utc
        .with_ymd_and_hms(2026, 11, 7, 15, 10, 0)
        .single()
        .unwrap();
    let cycle2 = Utc
        .with_ymd_and_hms(2026, 11, 7, 15, 20, 0)
        .single()
        .unwrap();
    let cycle3 = Utc
        .with_ymd_and_hms(2026, 11, 7, 15, 30, 0)
        .single()
        .unwrap();
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

    let sched_after_1 = service
        .get_schedule(&cron_sched.schedule_id)
        .await
        .unwrap()
        .unwrap();
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

    let sched_after_2 = service
        .get_schedule(&cron_sched.schedule_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sched_after_2.run_count, 2);
    assert_eq!(sched_after_2.last_run_at, Some(cycle2));
    assert_eq!(sched_after_2.next_run_at, cycle3);

    // Cycle 3 (15:30:00): Cron triggers run #3 and computes 15:40:00 as next_run_at
    let tick3 = BaseTick::run(&service, cycle3).await;
    assert!(tick3.promoted_delayed_tasks.is_empty());
    assert_eq!(tick3.spawned_cron_tasks.len(), 1);

    let sched_after_3 = service
        .get_schedule(&cron_sched.schedule_id)
        .await
        .unwrap()
        .unwrap();
    let cycle4 = Utc
        .with_ymd_and_hms(2026, 11, 7, 15, 40, 0)
        .single()
        .unwrap();
    assert_eq!(sched_after_3.run_count, 3);
    assert_eq!(sched_after_3.last_run_at, Some(cycle3));
    assert_eq!(sched_after_3.next_run_at, cycle4);

    // Deleting schedule stops future cron spawns
    assert!(service
        .delete_schedule(&cron_sched.schedule_id)
        .await
        .unwrap());
    let tick4 = BaseTick::run(&service, cycle4).await;
    assert!(tick4.spawned_cron_tasks.is_empty());
}

#[tokio::test]
async fn test_delayed_task_cannot_be_claimed_or_executed_before_due_time_and_executes_once_due() {
    let service = build_test_service();
    let (engine, exec_count) = CountingMockEngine::new();
    let t0 = Utc
        .with_ymd_and_hms(2026, 11, 7, 16, 0, 0)
        .single()
        .unwrap();
    let due_time = t0 + Duration::minutes(10);

    let delayed_id = service
        .submit_delayed_task(
            "Deferred Audit Job",
            "Run compliance verification only after due time",
            Priority::High,
            due_time,
        )
        .await
        .expect("submit delayed task");

    // 1. Before due time: cannot be claimed via claim_queued or claim_next_queued
    assert!(service
        .store()
        .claim_queued(&delayed_id)
        .await
        .unwrap()
        .is_none());
    assert!(service.store().claim_next_queued().await.unwrap().is_none());

    // 2. Before due time: dispatching due tasks with an active engine must not execute the task
    let before_due = due_time - Duration::seconds(5);
    let early_dispatched = service
        .dispatch_due_tasks(before_due, Some(engine.clone()))
        .await
        .unwrap();
    assert!(early_dispatched.is_empty());
    assert_eq!(exec_count.load(Ordering::SeqCst), 0);

    // Direct dispatch while still Scheduled must also refuse to claim/execute
    service.dispatch(delayed_id.clone(), engine.clone());
    assert!(service
        .drain(std::time::Duration::from_secs(2))
        .await
        .unwrap());
    assert_eq!(exec_count.load(Ordering::SeqCst), 0);
    let task_before = service.get_task(&delayed_id).await.unwrap().unwrap();
    assert_eq!(
        task_before.status,
        TaskStatus::Scheduled {
            run_after: due_time
        }
    );

    // 3. At due time: promoting and dispatching executes the task to Completed
    let promoted = service
        .dispatch_due_tasks(due_time, Some(engine.clone()))
        .await
        .unwrap();
    assert_eq!(promoted, vec![delayed_id.clone()]);
    assert!(service
        .drain(std::time::Duration::from_secs(5))
        .await
        .unwrap());

    let completed_task = service.get_task(&delayed_id).await.unwrap().unwrap();
    assert_eq!(completed_task.status, TaskStatus::Completed);
    assert_eq!(exec_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_delayed_task_dequeue_order_only_returns_tasks_whose_due_time_has_passed() {
    let service = build_test_service();
    let t0 = Utc
        .with_ymd_and_hms(2026, 11, 7, 17, 0, 0)
        .single()
        .unwrap();
    let due_first = t0 + Duration::seconds(20);
    let due_second = t0 + Duration::seconds(60);

    let first_id = service
        .submit_delayed_task("First Due", "Due at +20s", Priority::Normal, due_first)
        .await
        .unwrap();
    let second_id = service
        .submit_delayed_task("Second Due", "Due at +60s", Priority::Critical, due_second)
        .await
        .unwrap();

    // Before +20s: queue claim returns nothing
    let _ = service
        .dispatch_due_tasks(t0 + Duration::seconds(10), None)
        .await
        .unwrap();
    assert!(service.store().claim_next_queued().await.unwrap().is_none());

    // At +20s: only first_id is promoted and claimable from the queue
    let promoted_1 = service.dispatch_due_tasks(due_first, None).await.unwrap();
    assert_eq!(promoted_1, vec![first_id.clone()]);
    let claimed_1 = service
        .store()
        .claim_next_queued()
        .await
        .unwrap()
        .expect("first due task should be claimable");
    assert_eq!(claimed_1.id, first_id);
    assert_eq!(claimed_1.status, TaskStatus::Executing);

    // second_id is still Scheduled and cannot be claimed yet
    assert!(service.store().claim_next_queued().await.unwrap().is_none());

    // At +60s: second_id becomes due, is promoted, and can now be claimed
    let promoted_2 = service.dispatch_due_tasks(due_second, None).await.unwrap();
    assert_eq!(promoted_2, vec![second_id.clone()]);
    let claimed_2 = service
        .store()
        .claim_next_queued()
        .await
        .unwrap()
        .expect("second due task should be claimable");
    assert_eq!(claimed_2.id, second_id);
    assert_eq!(claimed_2.status, TaskStatus::Executing);
}

#[tokio::test]
async fn test_interval_and_cron_schedules_automatically_spawn_and_execute_tasks_periodically() {
    let service = build_test_service();
    let (engine, exec_count) = CountingMockEngine::new();
    let t0 = Utc
        .with_ymd_and_hms(2026, 11, 7, 18, 0, 0)
        .single()
        .unwrap();

    // Register a fixed-interval schedule (every 30 seconds)
    let interval_sched = service
        .register_interval_task(
            "Heartbeat Probe",
            "Verify worker liveness every 30s",
            Priority::High,
            std::time::Duration::from_secs(30),
            t0,
        )
        .await
        .expect("register interval schedule");

    assert_eq!(
        interval_sched.spec,
        ScheduleSpec::Interval {
            interval_ms: 30_000
        }
    );
    assert_eq!(interval_sched.next_run_at, t0 + Duration::seconds(30));

    // Also register a @every cron descriptor schedule (every 60 seconds)
    let cron_every_sched = service
        .register_cron_task(
            "Minute Snapshot",
            "Capture state every 60s",
            Priority::Normal,
            "@every 60s",
            t0,
        )
        .await
        .expect("register @every cron schedule");
    assert_eq!(cron_every_sched.next_run_at, t0 + Duration::seconds(60));

    // Before 30s: neither schedule triggers
    let tick_early = service
        .tick_scheduler(t0 + Duration::seconds(29), Some(engine.clone()))
        .await
        .unwrap();
    assert!(tick_early.spawned_cron_tasks.is_empty());
    assert_eq!(exec_count.load(Ordering::SeqCst), 0);

    // At +30s: interval schedule spawns its 1st task and advances to +60s
    let t_30s = t0 + Duration::seconds(30);
    let tick_30s = service
        .tick_scheduler(t_30s, Some(engine.clone()))
        .await
        .unwrap();
    assert_eq!(tick_30s.spawned_cron_tasks.len(), 1);
    assert!(service
        .drain(std::time::Duration::from_secs(5))
        .await
        .unwrap());
    assert_eq!(exec_count.load(Ordering::SeqCst), 1);

    let first_spawned = service
        .get_task(&tick_30s.spawned_cron_tasks[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_spawned.status, TaskStatus::Completed);

    let updated_interval = service
        .get_schedule(&interval_sched.schedule_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated_interval.run_count, 1);
    assert_eq!(updated_interval.next_run_at, t0 + Duration::seconds(60));

    // At +60s: BOTH interval schedule (run #2) and @every 60s cron schedule (run #1) trigger and execute
    let t_60s = t0 + Duration::seconds(60);
    let tick_60s = service
        .tick_scheduler(t_60s, Some(engine.clone()))
        .await
        .unwrap();
    assert_eq!(tick_60s.spawned_cron_tasks.len(), 2);
    assert!(service
        .drain(std::time::Duration::from_secs(5))
        .await
        .unwrap());
    assert_eq!(exec_count.load(Ordering::SeqCst), 3);

    for spawned_id in &tick_60s.spawned_cron_tasks {
        let task = service.get_task(spawned_id).await.unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
    }

    let updated_interval_2 = service
        .get_schedule(&interval_sched.schedule_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated_interval_2.run_count, 2);
    assert_eq!(updated_interval_2.next_run_at, t0 + Duration::seconds(90));

    let updated_cron_1 = service
        .get_schedule(&cron_every_sched.schedule_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated_cron_1.run_count, 1);
    assert_eq!(updated_cron_1.next_run_at, t0 + Duration::seconds(120));
}

#[tokio::test]
async fn test_poll_and_due_inspection_without_affecting_existing_apis() {
    let service = build_test_service();
    let t0 = Utc
        .with_ymd_and_hms(2026, 11, 7, 19, 0, 0)
        .single()
        .unwrap();
    let due_at = t0 + Duration::seconds(45);

    // Existing submit_task API continues to queue immediately and is ready at t0
    let immediate_id = service
        .submit_task("Immediate Task", "Run right away", Priority::Normal)
        .await
        .expect("submit immediate task");
    let immediate_task = service.get_task(&immediate_id).await.unwrap().unwrap();
    assert_eq!(immediate_task.status, TaskStatus::Queued);
    assert!(immediate_task.is_ready_to_execute(t0));
    assert!(service.is_task_due(&immediate_id, t0).await.unwrap());

    // Delayed task is stored as Scheduled and is not due before due_at
    let delayed_id = service
        .submit_delayed_task("Delayed Poll Task", "Wait 45s", Priority::High, due_at)
        .await
        .expect("submit delayed task");
    assert!(!service.is_task_due(&delayed_id, t0).await.unwrap());

    let scheduled_list = service.list_scheduled_tasks(10, 0).await.unwrap();
    assert_eq!(scheduled_list.len(), 1);
    assert_eq!(scheduled_list[0].id, delayed_id);

    // Register a recurring interval schedule (every 45s)
    let sched = service
        .register_interval_task(
            "Poll Interval Job",
            "Run every 45s",
            Priority::Normal,
            std::time::Duration::from_secs(45),
            t0,
        )
        .await
        .expect("register interval schedule");

    assert!(service
        .list_due_schedules(t0 + Duration::seconds(30))
        .await
        .unwrap()
        .is_empty());
    let due_schedules = service.list_due_schedules(due_at).await.unwrap();
    assert!(due_schedules
        .iter()
        .any(|s| s.schedule_id == sched.schedule_id));

    // Poll at due_at moves both the delayed task and interval task into the execution queue
    let poll_report = service.poll_due_tasks(due_at, None).await.unwrap();
    assert_eq!(poll_report.promoted_delayed_tasks, vec![delayed_id.clone()]);
    assert_eq!(poll_report.spawned_cron_tasks.len(), 1);
    assert!(service.is_task_due(&delayed_id, due_at).await.unwrap());
    assert!(service
        .list_scheduled_tasks(10, 0)
        .await
        .unwrap()
        .is_empty());
}

struct BaseTick;
impl BaseTick {
    async fn run(service: &TaskService, now: chrono::DateTime<Utc>) -> super::SchedulerTickReport {
        service
            .tick_scheduler(now, None)
            .await
            .expect("tick scheduler")
    }
}
