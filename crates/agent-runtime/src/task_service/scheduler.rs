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

    /// Register a recurring fixed-interval schedule (`ScheduleSpec::Interval`).
    pub async fn register_interval_task(
        &self,
        title: &str,
        goal: &str,
        priority: Priority,
        interval: std::time::Duration,
        now: DateTime<Utc>,
    ) -> Result<TaskScheduleRecord> {
        if interval.is_zero() {
            anyhow::bail!(agent_domain::ScheduleError::InvalidInterval(
                "interval must be > 0".into()
            ));
        }
        let spec = ScheduleSpec::every(interval);
        let record = TaskScheduleRecord::new(title, goal, priority, spec, now)?;
        self.store.save_schedule(&record).await?;
        info!(
            "🔁 [TaskService] Interval schedule {} ({:?}) registered, next run at {}",
            record.schedule_id,
            interval,
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

    /// Poll due delayed tasks and recurring cron/interval schedules at `now`, moving them to `Queued`
    /// and optionally dispatching them onto `engine`.
    pub async fn poll_due_tasks(
        &self,
        now: DateTime<Utc>,
        engine: Option<Arc<dyn ComputeEngine>>,
    ) -> Result<SchedulerTickReport> {
        self.tick_scheduler(now, engine).await
    }

    /// Poll due delayed tasks and recurring schedules using the current system UTC time.
    pub async fn poll_scheduler(
        &self,
        engine: Option<Arc<dyn ComputeEngine>>,
    ) -> Result<SchedulerTickReport> {
        self.tick_scheduler(Utc::now(), engine).await
    }

    /// Check whether a task has reached its due time (`scheduled_at` / `run_after <= now`).
    pub async fn is_task_due(&self, id: &TaskId, now: DateTime<Utc>) -> Result<bool> {
        let Some(task) = self.store.get_task(id).await? else {
            return Ok(false);
        };
        let due_at = match task.status {
            TaskStatus::Scheduled { run_after } => Some(task.scheduled_at.unwrap_or(run_after)),
            _ => task.scheduled_at.or(task.run_after),
        };
        Ok(due_at.is_none_or(|t| t <= now))
    }

    /// List tasks currently waiting in the Delayed Task Queue (`TaskStatus::Scheduled`).
    pub async fn list_scheduled_tasks(&self, limit: usize, offset: usize) -> Result<Vec<Task>> {
        self.store
            .list_tasks_by_status("Scheduled", limit, offset)
            .await
    }

    /// List all enabled schedules whose `next_run_at <= now`.
    pub async fn list_due_schedules(&self, now: DateTime<Utc>) -> Result<Vec<TaskScheduleRecord>> {
        let schedules = self.store.list_schedules(usize::MAX, 0).await?;
        Ok(schedules
            .into_iter()
            .filter(|rec| rec.is_due(now))
            .collect())
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
