use super::TaskService;
use agent_domain::{Priority, ScheduleSpec, Task, TaskId, TaskScheduleRecord, TaskStatus};
use agent_engine::ComputeEngine;
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tracing::info;

/// Summary of delayed tasks promoted and recurring cron tasks spawned during a scheduler tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulerTickReport {
    pub promoted_delayed_tasks: Vec<TaskId>,
    pub spawned_cron_tasks: Vec<TaskId>,
}

impl SchedulerTickReport {
    pub fn total_dispatched(&self) -> usize {
        self.promoted_delayed_tasks.len() + self.spawned_cron_tasks.len()
    }
}

impl TaskService {
    /// Submit a delayed task scheduled to transition from `Scheduled` to `Queued` at `scheduled_at`.
    pub async fn submit_delayed_task(
        &self,
        title: &str,
        goal: &str,
        priority: Priority,
        scheduled_at: DateTime<Utc>,
    ) -> Result<TaskId> {
        let mut task = Task::new_delayed(title, goal, priority.clone(), scheduled_at);
        task.status = TaskStatus::Scheduled {
            run_after: scheduled_at,
        };
        task.run_after = Some(scheduled_at);
        task.scheduled_at = Some(scheduled_at);
        let id = task.id.clone();

        self.store.save_task(&task).await?;

        if let Ok(record) = TaskScheduleRecord::new(
            title,
            goal,
            priority,
            ScheduleSpec::OnceAt {
                run_after: scheduled_at,
            },
            task.created_at,
        ) {
            let linked = record.with_task_id(id.clone());
            let _ = self.store.save_schedule(&linked).await;
        }

        info!(
            "⏰ [TaskService] Delayed task {} scheduled for {}: '{}'",
            id,
            scheduled_at.to_rfc3339(),
            title
        );
        Ok(id)
    }

    /// Submit a delayed task relative to `Utc::now()` using a `Duration`.
    pub async fn submit_delayed_task_after(
        &self,
        title: &str,
        goal: &str,
        priority: Priority,
        delay: std::time::Duration,
    ) -> Result<TaskId> {
        let now = Utc::now();
        let delay_ms = i64::try_from(delay.as_millis()).unwrap_or(i64::MAX);
        let scheduled_at = now + chrono::Duration::milliseconds(delay_ms);
        self.submit_delayed_task(title, goal, priority, scheduled_at)
            .await
    }

    /// Promote all due delayed tasks (`scheduled_at <= now`) from `Scheduled` to `Queued`,
    /// optionally dispatching them onto `engine` if provided.
    pub async fn dispatch_due_tasks(
        &self,
        now: DateTime<Utc>,
        engine: Option<Arc<dyn ComputeEngine>>,
    ) -> Result<Vec<TaskId>> {
        let promoted = self.store.promote_due_delayed_tasks(now).await?;
        let mut ids = Vec::with_capacity(promoted.len());
        for task in promoted {
            if let Some(ref eng) = engine {
                self.dispatch(task.id.clone(), eng.clone());
            }
            ids.push(task.id);
        }
        Ok(ids)
    }

    /// Register a recurring cron schedule (`5-field`, `6-field`, or `@every`/`@daily` descriptor).
    pub async fn register_cron_task(
        &self,
        title: &str,
        goal: &str,
        priority: Priority,
        cron_expression: &str,
        now: DateTime<Utc>,
    ) -> Result<TaskScheduleRecord> {
        let spec = ScheduleSpec::cron(cron_expression)?;
        let record = TaskScheduleRecord::new(title, goal, priority, spec, now)?;
        self.store.save_schedule(&record).await?;
        info!(
            "🗓️ [TaskService] Cron schedule {} ('{}') registered, next run at {}",
            record.schedule_id,
            cron_expression,
            record.next_run_at.to_rfc3339()
        );
        Ok(record)
    }

    /// Evaluate both the Delayed Task Queue and Recurring Cron Schedules at explicit timestamp `now`.
    pub async fn tick_scheduler(
        &self,
        now: DateTime<Utc>,
        engine: Option<Arc<dyn ComputeEngine>>,
    ) -> Result<SchedulerTickReport> {
        let promoted_delayed_tasks = self.dispatch_due_tasks(now, engine.clone()).await?;
        let triggered = self.store.trigger_due_cron_schedules(now).await?;
        let mut spawned_cron_tasks = Vec::with_capacity(triggered.len());

        for (_sched, spawned_task) in triggered {
            if let Some(ref eng) = engine {
                self.dispatch(spawned_task.id.clone(), eng.clone());
            }
            spawned_cron_tasks.push(spawned_task.id);
        }

        Ok(SchedulerTickReport {
            promoted_delayed_tasks,
            spawned_cron_tasks,
        })
    }

    pub async fn get_schedule(&self, schedule_id: &str) -> Result<Option<TaskScheduleRecord>> {
        self.store.get_schedule(schedule_id).await
    }

    pub async fn list_schedules(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TaskScheduleRecord>> {
        self.store.list_schedules(limit, offset).await
    }

    pub async fn delete_schedule(&self, schedule_id: &str) -> Result<bool> {
        self.store.delete_schedule(schedule_id).await
    }
}
