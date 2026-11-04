use agent_domain::{SubTask, Task, TaskId, TaskResult};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn save_task(&self, task: &Task) -> Result<()>;
    async fn get_task(&self, id: &TaskId) -> Result<Option<Task>>;
    async fn list_tasks(&self, limit: usize, offset: usize) -> Result<Vec<Task>>;
    async fn save_checkpoint(&self, task_id: &TaskId, subtasks: &[SubTask]) -> Result<()>;
    async fn get_checkpoint(&self, task_id: &TaskId) -> Result<Option<Vec<SubTask>>>;
    async fn save_result(&self, result: &TaskResult) -> Result<()>;
    async fn get_result(&self, task_id: &TaskId) -> Result<Option<TaskResult>>;
}

#[derive(Clone)]
pub struct SqliteTaskStore {
    db_path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl SqliteTaskStore {
    pub fn new(workspace_dir: impl AsRef<Path>) -> Result<Self> {
        let storage_dir = workspace_dir.as_ref().join("storage");
        let _ = std::fs::create_dir_all(&storage_dir);
        let db_path = storage_dir.join("tasks.db");

        let conn = Connection::open(&db_path)
            .map_err(|e| anyhow!("Failed to open sqlite tasks db at {:?}: {}", db_path, e))?;

        // Configure WAL mode and busy timeout for high reliability
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;
             
             CREATE TABLE IF NOT EXISTS tasks (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 goal TEXT NOT NULL,
                 priority TEXT NOT NULL,
                 status TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 raw_json TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS subtask_checkpoints (
                 task_id TEXT PRIMARY KEY,
                 subtasks_json TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS task_results (
                 task_id TEXT PRIMARY KEY,
                 status TEXT NOT NULL,
                 summary TEXT NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 raw_json TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );"
        ).map_err(|e| anyhow!("Failed to initialize tasks schema: {}", e))?;

        Ok(Self {
            db_path,
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| anyhow!("Failed to open in-memory sqlite db: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 goal TEXT NOT NULL,
                 priority TEXT NOT NULL,
                 status TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 raw_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS subtask_checkpoints (
                 task_id TEXT PRIMARY KEY,
                 subtasks_json TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS task_results (
                 task_id TEXT PRIMARY KEY,
                 status TEXT NOT NULL,
                 summary TEXT NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 raw_json TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );"
        )?;

        Ok(Self {
            db_path: PathBuf::from(":memory:"),
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

#[async_trait]
impl TaskStore for SqliteTaskStore {
    async fn save_task(&self, task: &Task) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Database lock poisoned"))?;
        let raw_json = serde_json::to_string(task)?;
        let status_json = serde_json::to_string(&task.status)?;
        let priority_json = serde_json::to_string(&task.priority)?;

        conn.execute(
            "INSERT INTO tasks (id, title, goal, priority, status, created_at, updated_at, raw_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 goal = excluded.goal,
                 priority = excluded.priority,
                 status = excluded.status,
                 updated_at = excluded.updated_at,
                 raw_json = excluded.raw_json",
            params![
                task.id.0,
                task.title,
                task.goal_description,
                priority_json,
                status_json,
                task.created_at.to_rfc3339(),
                task.updated_at.to_rfc3339(),
                raw_json,
            ],
        )?;

        Ok(())
    }

    async fn get_task(&self, id: &TaskId) -> Result<Option<Task>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Database lock poisoned"))?;
        let mut stmt = conn.prepare("SELECT raw_json FROM tasks WHERE id = ?1")?;
        let mut rows = stmt.query(params![id.0])?;

        if let Some(row) = rows.next()? {
            let raw: String = row.get(0)?;
            let task = serde_json::from_str(&raw)?;
            Ok(Some(task))
        } else {
            Ok(None)
        }
    }

    async fn list_tasks(&self, limit: usize, offset: usize) -> Result<Vec<Task>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Database lock poisoned"))?;
        let mut stmt = conn.prepare("SELECT raw_json FROM tasks ORDER BY created_at DESC LIMIT ?1 OFFSET ?2")?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
            let raw: String = row.get(0)?;
            Ok(raw)
        })?;

        let mut tasks = Vec::new();
        for raw in rows.flatten() {
            if let Ok(task) = serde_json::from_str(&raw) {
                tasks.push(task);
            }
        }
        Ok(tasks)
    }

    async fn save_checkpoint(&self, task_id: &TaskId, subtasks: &[SubTask]) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Database lock poisoned"))?;
        let subtasks_json = serde_json::to_string(subtasks)?;
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO subtask_checkpoints (task_id, subtasks_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(task_id) DO UPDATE SET
                 subtasks_json = excluded.subtasks_json,
                 updated_at = excluded.updated_at",
            params![task_id.0, subtasks_json, now],
        )?;

        Ok(())
    }

    async fn get_checkpoint(&self, task_id: &TaskId) -> Result<Option<Vec<SubTask>>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Database lock poisoned"))?;
        let mut stmt = conn.prepare("SELECT subtasks_json FROM subtask_checkpoints WHERE task_id = ?1")?;
        let mut rows = stmt.query(params![task_id.0])?;

        if let Some(row) = rows.next()? {
            let json_str: String = row.get(0)?;
            let subtasks = serde_json::from_str(&json_str)?;
            Ok(Some(subtasks))
        } else {
            Ok(None)
        }
    }

    async fn save_result(&self, result: &TaskResult) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Database lock poisoned"))?;
        let raw_json = serde_json::to_string(result)?;
        let status_json = serde_json::to_string(&result.status)?;
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO task_results (task_id, status, summary, duration_ms, raw_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(task_id) DO UPDATE SET
                 status = excluded.status,
                 summary = excluded.summary,
                 duration_ms = excluded.duration_ms,
                 raw_json = excluded.raw_json",
            params![
                result.task_id.0,
                status_json,
                result.summary,
                result.duration_ms as i64,
                raw_json,
                now
            ],
        )?;

        Ok(())
    }

    async fn get_result(&self, task_id: &TaskId) -> Result<Option<TaskResult>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Database lock poisoned"))?;
        let mut stmt = conn.prepare("SELECT raw_json FROM task_results WHERE task_id = ?1")?;
        let mut rows = stmt.query(params![task_id.0])?;

        if let Some(row) = rows.next()? {
            let raw: String = row.get(0)?;
            let res = serde_json::from_str(&raw)?;
            Ok(Some(res))
        } else {
            Ok(None)
        }
    }
}
