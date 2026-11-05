pub mod crud;
pub mod lifecycle;
pub mod schema;

use agent_domain::{SubTask, Task, TaskId, TaskResult};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn submit_idempotent(&self, _task: &Task, _key: &str, _req: &str) -> Result<IdempotentSubmission> {
        Err(anyhow!("Idempotent submissions are not supported by this task store"))
    }
    async fn claim_queued(&self, _id: &TaskId) -> Result<Option<Task>> {
        Err(anyhow!("Atomic task claims are not supported by this task store"))
    }
    async fn claim_next_queued(&self) -> Result<Option<Task>> { Ok(None) }
    async fn finish_task(&self, _task: &Task, _res: &TaskResult, _preserve: bool) -> Result<()> {
        Err(anyhow!("Atomic task completion is not supported by this task store"))
    }
    async fn cancel_task(&self, _id: &TaskId) -> Result<bool> {
        Err(anyhow!("Atomic cancellation is not supported by this task store"))
    }
    async fn save_review(&self, _exp: &Task, _upd: &Task, _res: &TaskResult, _rev: &str, _dec: &str, _revis: &str) -> Result<bool> {
        Err(anyhow!("Atomic task review is not supported by this task store"))
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
    async fn list_tasks_by_status(&self, _filter: &str, _limit: usize, _offset: usize) -> Result<Vec<Task>> { Ok(Vec::new()) }
    async fn save_checkpoint(&self, task_id: &TaskId, subtasks: &[SubTask]) -> Result<()>;
    async fn get_checkpoint(&self, task_id: &TaskId) -> Result<Option<Vec<SubTask>>>;
    async fn save_result(&self, result: &TaskResult) -> Result<()>;
    async fn get_result(&self, task_id: &TaskId) -> Result<Option<TaskResult>>;
    async fn count_tasks(&self) -> Result<usize> { Ok(self.list_tasks(usize::MAX, 0).await?.len()) }
    async fn count_tasks_by_status(&self, _filter: &str) -> Result<usize> { Ok(0) }
    async fn delete_task(&self, _id: &TaskId) -> Result<bool> { Err(anyhow!("Task deletion is not supported by this task store")) }
    async fn prune_finished_tasks(&self, _older_than_rfc3339: &str) -> Result<usize> { Ok(0) }
    async fn check_integrity(&self) -> Result<bool> { Ok(true) }
    async fn is_healthy(&self) -> bool { true }
    async fn wal_checkpoint(&self) -> Result<()> { Ok(()) }
    async fn get_idempotent_task(&self, _key: &str) -> Result<Option<TaskId>> { Ok(None) }
    async fn delete_idempotent_key(&self, _key: &str) -> Result<bool> { Ok(false) }
    async fn clear_all(&self) -> Result<()> { Ok(()) }
    async fn vacuum(&self) -> Result<()> { Ok(()) }
    async fn repair_or_recreate(&self) -> Result<()> { Ok(()) }
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

impl SqliteTaskStore {
    pub fn new(workspace_dir: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(workspace_dir, 5000, true)
    }

    pub fn with_options(workspace_dir: impl AsRef<Path>, busy_timeout_ms: u32, wal_enabled: bool) -> Result<Self> {
        let (db_path, conn) = schema::open_resilient_sqlite(workspace_dir.as_ref(), busy_timeout_ms, wal_enabled)?;
        Ok(Self { db_path, conn: Arc::new(Mutex::new(conn)) })
    }

    pub fn db_path(&self) -> &Path { &self.db_path }

    pub fn is_healthy(&self) -> bool {
        let conn = schema::acquire_conn(&self.conn);
        conn.query_row("SELECT 1", [], |_| Ok(())).is_ok()
    }

    pub fn check_integrity(&self) -> Result<bool> {
        let conn = schema::acquire_conn(&self.conn);
        match conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
            Ok(result) => Ok(result.eq_ignore_ascii_case("ok")),
            Err(_) => Ok(false),
        }
    }

    pub fn vacuum(&self) -> Result<()> {
        let conn = schema::acquire_conn(&self.conn);
        conn.execute("VACUUM", [])?;
        Ok(())
    }

    pub fn wal_checkpoint(&self) -> Result<()> {
        let conn = schema::acquire_conn(&self.conn);
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        Ok(())
    }

    pub fn count_tasks(&self) -> Result<usize> { crud::count_tasks(&self.conn) }
    pub fn count_tasks_by_status(&self, status_filter: &str) -> Result<usize> { crud::count_tasks_by_status(&self.conn, status_filter) }
    pub fn list_tasks_by_status(&self, filter: &str, limit: usize, offset: usize) -> Result<Vec<Task>> { crud::list_tasks_by_status(&self.conn, filter, limit, offset) }
    pub fn claim_next_queued(&self) -> Result<Option<Task>> { lifecycle::claim_next_queued(&self.conn) }
    pub fn delete_task(&self, id: &TaskId) -> Result<bool> { crud::delete_task(&self.conn, id) }
    pub fn prune_finished_tasks(&self, older_than_rfc3339: &str) -> Result<usize> { crud::prune_finished_tasks(&self.conn, older_than_rfc3339) }
    pub fn clear_all(&self) -> Result<()> { crud::clear_all(&self.conn) }
    pub fn get_idempotent_task(&self, key: &str) -> Result<Option<TaskId>> { lifecycle::get_idempotent_task(&self.conn, key) }
    pub fn delete_idempotent_key(&self, key: &str) -> Result<bool> { lifecycle::delete_idempotent_key(&self.conn, key) }

    pub fn repair_or_recreate(&self) -> Result<()> {
        let mut conn = schema::acquire_conn(&self.conn);
        if self.db_path.to_str() == Some(":memory:") {
            let fresh = Connection::open_in_memory().map_err(|e| anyhow!("Failed to reopen in-memory db: {}", e))?;
            schema::init_sqlite_schema(&fresh, false)?;
            *conn = fresh;
            return Ok(());
        }
        if let Ok(temp) = Connection::open_in_memory() {
            *conn = temp;
        }
        let backup = self.db_path.with_extension(format!("corrupted.{}", chrono::Utc::now().timestamp_millis()));
        let _ = std::fs::rename(&self.db_path, &backup);
        let _ = std::fs::remove_file(&self.db_path);
        let wal_path = PathBuf::from(format!("{}-wal", self.db_path.display()));
        let shm_path = PathBuf::from(format!("{}-shm", self.db_path.display()));
        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_file(&shm_path);
        if let Some(parent) = self.db_path.parent() {
            let _ = std::fs::remove_file(parent.join("tasks.db-wal"));
            let _ = std::fs::remove_file(parent.join("tasks.db-shm"));
        }
        let fresh = Connection::open(&self.db_path).map_err(|e| anyhow!("Failed to recreate db at {:?}: {}", self.db_path, e))?;
        schema::init_sqlite_schema_with_options(&fresh, true, 5000, true)?;
        *conn = fresh;
        Ok(())
    }

    pub fn from_raw_parts(db_path: PathBuf, conn: Connection) -> Self {
        Self { db_path, conn: Arc::new(Mutex::new(conn)) }
    }

    pub fn memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| anyhow!("Failed to open in-memory sqlite db: {}", e))?;
        schema::init_sqlite_schema(&conn, false)?;
        Ok(Self { db_path: PathBuf::from(":memory:"), conn: Arc::new(Mutex::new(conn)) })
    }
}

#[async_trait]
impl TaskStore for SqliteTaskStore {
    async fn save_review(&self, exp: &Task, upd: &Task, res: &TaskResult, rev: &str, dec: &str, revis: &str) -> Result<bool> {
        lifecycle::save_review(&self.conn, exp, upd, res, rev, dec, revis)
    }
    async fn get_task_and_result(&self, id: &TaskId) -> Result<Option<(Task, Option<TaskResult>)>> {
        crud::get_task_and_result(&self.conn, id)
    }
    async fn cancel_task(&self, id: &TaskId) -> Result<bool> { lifecycle::cancel_task(&self.conn, id) }
    async fn finish_task(&self, task: &Task, result: &TaskResult, preserve: bool) -> Result<()> {
        lifecycle::finish_task(&self.conn, task, result, preserve)
    }
    async fn claim_queued(&self, id: &TaskId) -> Result<Option<Task>> { lifecycle::claim_queued(&self.conn, id) }
    async fn claim_next_queued(&self) -> Result<Option<Task>> { self.claim_next_queued() }
    async fn submit_idempotent(&self, task: &Task, key: &str, req: &str) -> Result<IdempotentSubmission> {
        lifecycle::submit_idempotent(&self.conn, task, key, req)
    }
    async fn save_task(&self, task: &Task) -> Result<()> { crud::save_task(&self.conn, task) }
    async fn get_task(&self, id: &TaskId) -> Result<Option<Task>> { crud::get_task(&self.conn, id) }
    async fn list_tasks(&self, limit: usize, offset: usize) -> Result<Vec<Task>> { crud::list_tasks(&self.conn, limit, offset) }
    async fn list_tasks_by_status(&self, filter: &str, limit: usize, offset: usize) -> Result<Vec<Task>> { self.list_tasks_by_status(filter, limit, offset) }
    async fn count_tasks(&self) -> Result<usize> { self.count_tasks() }
    async fn count_tasks_by_status(&self, filter: &str) -> Result<usize> { self.count_tasks_by_status(filter) }
    async fn delete_task(&self, id: &TaskId) -> Result<bool> { self.delete_task(id) }
    async fn prune_finished_tasks(&self, older_than_rfc3339: &str) -> Result<usize> { self.prune_finished_tasks(older_than_rfc3339) }
    async fn check_integrity(&self) -> Result<bool> { self.check_integrity() }
    async fn is_healthy(&self) -> bool { self.is_healthy() }
    async fn wal_checkpoint(&self) -> Result<()> { self.wal_checkpoint() }
    async fn get_idempotent_task(&self, key: &str) -> Result<Option<TaskId>> { self.get_idempotent_task(key) }
    async fn delete_idempotent_key(&self, key: &str) -> Result<bool> { self.delete_idempotent_key(key) }
    async fn clear_all(&self) -> Result<()> { self.clear_all() }
    async fn vacuum(&self) -> Result<()> { self.vacuum() }
    async fn repair_or_recreate(&self) -> Result<()> { self.repair_or_recreate() }
    async fn save_checkpoint(&self, task_id: &TaskId, subtasks: &[SubTask]) -> Result<()> { crud::save_checkpoint(&self.conn, task_id, subtasks) }
    async fn get_checkpoint(&self, task_id: &TaskId) -> Result<Option<Vec<SubTask>>> { crud::get_checkpoint(&self.conn, task_id) }
    async fn save_result(&self, result: &TaskResult) -> Result<()> { crud::save_result(&self.conn, result) }
    async fn get_result(&self, task_id: &TaskId) -> Result<Option<TaskResult>> { crud::get_result(&self.conn, task_id) }
}
