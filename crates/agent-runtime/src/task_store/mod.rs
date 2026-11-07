pub mod crud;
pub mod lifecycle;
pub mod idempotency;
pub mod results;
pub mod review;
mod sqlite_impl;
pub mod schedule;
pub mod schema;

use agent_domain::{SubTask, Task, TaskId, TaskResult, TaskScheduleRecord};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn submit_idempotent(
        &self,
        _task: &Task,
        _key: &str,
        _req: &str,
    ) -> Result<IdempotentSubmission> {
        Err(anyhow!(
            "Idempotent submissions are not supported by this task store"
        ))
    }
    async fn claim_queued(&self, _id: &TaskId) -> Result<Option<Task>> {
        Err(anyhow!(
            "Atomic task claims are not supported by this task store"
        ))
    }
    async fn claim_next_queued(&self) -> Result<Option<Task>> {
        Ok(None)
    }
    async fn finish_task(&self, _task: &Task, _res: &TaskResult, _preserve: bool) -> Result<()> {
        Err(anyhow!(
            "Atomic task completion is not supported by this task store"
        ))
    }
    async fn cancel_task(&self, _id: &TaskId) -> Result<bool> {
        Err(anyhow!(
            "Atomic cancellation is not supported by this task store"
        ))
    }
    async fn save_review(
        &self,
        _exp: &Task,
        _upd: &Task,
        _res: &TaskResult,
        _rev: &str,
        _dec: &str,
        _revis: &str,
    ) -> Result<bool> {
        Err(anyhow!(
            "Atomic task review is not supported by this task store"
        ))
    }
    async fn save_task(&self, task: &Task) -> Result<()>;
    async fn get_task(&self, id: &TaskId) -> Result<Option<Task>>;
    async fn get_task_and_result(&self, id: &TaskId) -> Result<Option<(Task, Option<TaskResult>)>> {
        match self.get_task(id).await? {
            Some(task) => Ok(Some((task, self.get_result(id).await?))),
            None => Ok(None),
        }
    }
    async fn list_tasks(&self, limit: usize, offset: usize) -> Result<Vec<Task>>;
    async fn list_tasks_by_status(
        &self,
        _filter: &str,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<Task>> {
        Ok(Vec::new())
    }
    async fn save_checkpoint(&self, task_id: &TaskId, subtasks: &[SubTask]) -> Result<()>;
    async fn get_checkpoint(&self, task_id: &TaskId) -> Result<Option<Vec<SubTask>>>;
    async fn get_checkpoint_metadata(
        &self,
        _task_id: &TaskId,
    ) -> Result<Option<agent_domain::CheckpointMetadata>> {
        Ok(None)
    }
    async fn get_task_checkpoint(
        &self,
        _task_id: &TaskId,
    ) -> Result<Option<agent_domain::TaskCheckpoint>> {
        Ok(None)
    }
    async fn save_result(&self, result: &TaskResult) -> Result<()>;
    async fn get_result(&self, task_id: &TaskId) -> Result<Option<TaskResult>>;
    async fn count_tasks(&self) -> Result<usize> {
        Ok(self.list_tasks(usize::MAX, 0).await?.len())
    }
    async fn count_tasks_by_status(&self, _filter: &str) -> Result<usize> {
        Ok(0)
    }
    async fn delete_task(&self, _id: &TaskId) -> Result<bool> {
        Err(anyhow!("Task deletion is not supported by this task store"))
    }
    async fn prune_finished_tasks(&self, _older_than_rfc3339: &str) -> Result<usize> {
        Ok(0)
    }
    async fn check_integrity(&self) -> Result<bool> {
        Ok(true)
    }
    async fn is_healthy(&self) -> bool {
        true
    }
    async fn wal_checkpoint(&self) -> Result<()> {
        Ok(())
    }
    async fn get_idempotent_task(&self, _key: &str) -> Result<Option<TaskId>> {
        Ok(None)
    }
    async fn delete_idempotent_key(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }
    async fn clear_all(&self) -> Result<()> {
        Ok(())
    }
    async fn vacuum(&self) -> Result<()> {
        Ok(())
    }
    async fn repair_or_recreate(&self) -> Result<()> {
        Ok(())
    }
    async fn save_schedule(&self, _record: &TaskScheduleRecord) -> Result<()> {
        Err(anyhow!(
            "Task scheduling is not supported by this task store"
        ))
    }
    async fn get_schedule(&self, _schedule_id: &str) -> Result<Option<TaskScheduleRecord>> {
        Ok(None)
    }
    async fn list_schedules(
        &self,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<TaskScheduleRecord>> {
        Ok(Vec::new())
    }
    async fn delete_schedule(&self, _schedule_id: &str) -> Result<bool> {
        Ok(false)
    }
    async fn promote_due_delayed_tasks(&self, _now: DateTime<Utc>) -> Result<Vec<Task>> {
        Ok(Vec::new())
    }
    async fn trigger_due_cron_schedules(
        &self,
        _now: DateTime<Utc>,
    ) -> Result<Vec<(TaskScheduleRecord, Task)>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotentSubmission {
    Created(TaskId),
    Existing(TaskId),
    Conflict,
}

#[derive(Clone)]
pub struct SqliteTaskStore {
    db_path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

