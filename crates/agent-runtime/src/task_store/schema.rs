use anyhow::{anyhow, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Safely acquire a SQLite connection guard, recovering from poison if a previous thread panicked.
pub fn acquire_conn(conn: &Mutex<Connection>) -> std::sync::MutexGuard<'_, Connection> {
    let guard = match conn.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !guard.is_autocommit() {
        let _ = guard.execute_batch("ROLLBACK;");
    }
    guard
}

/// Resolves storage directory with multi-tier self-healing fallback when target path is invalid/forbidden.
pub fn resolve_storage_dir(workspace_dir: &Path) -> PathBuf {
    let primary = workspace_dir.join("storage");
    if primary.is_file() {
        let backup = workspace_dir.join(format!(
            "storage_file_corrupt_{}",
            chrono::Utc::now().timestamp_millis()
        ));
        let _ = std::fs::rename(&primary, backup);
    }
    if std::fs::create_dir_all(&primary).is_ok() {
        return primary;
    }
    let unique = format!("agent_task_store_{}", uuid::Uuid::new_v4());
    let candidates = [
        std::env::temp_dir().join(&unique).join("storage"),
        PathBuf::from("target/tmp").join(&unique).join("storage"),
        PathBuf::from("/tmp").join(&unique).join("storage"),
    ];
    for candidate in candidates {
        if std::fs::create_dir_all(&candidate).is_ok() {
            return candidate;
        }
    }
    std::env::temp_dir().join("storage")
}

/// Opens resilient sqlite connection with self-healing quarantine on corrupt database file or schema.
pub fn open_resilient_sqlite(
    workspace_dir: &Path,
    busy_timeout_ms: u32,
    wal_enabled: bool,
) -> Result<(PathBuf, Connection)> {
    if workspace_dir.to_str() == Some(":memory:") {
        let conn = Connection::open_in_memory()?;
        init_sqlite_schema(&conn, false)?;
        return Ok((PathBuf::from(":memory:"), conn));
    }

    let storage_dir = resolve_storage_dir(workspace_dir);
    let _ = std::fs::create_dir_all(&storage_dir);
    let db_path = storage_dir.join("tasks.db");

    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            let backup = storage_dir.join(format!(
                "tasks.db.corrupted.{}",
                chrono::Utc::now().timestamp_millis()
            ));
            let _ = std::fs::rename(&db_path, &backup);
            let _ = std::fs::remove_file(&db_path);
            Connection::open(&db_path).map_err(|retry_err| {
                anyhow!(
                    "Failed to open sqlite db at {:?} after quarantine: {} (initial error: {})",
                    db_path,
                    retry_err,
                    e
                )
            })?
        }
    };

    if let Err(schema_err) = init_sqlite_schema_with_options(&conn, true, busy_timeout_ms, wal_enabled) {
        drop(conn);
        let backup = storage_dir.join(format!(
            "tasks.db.malformed.{}",
            chrono::Utc::now().timestamp_millis()
        ));
        let _ = std::fs::rename(&db_path, &backup);
        let _ = std::fs::remove_file(&db_path);
        let fresh_conn = Connection::open(&db_path).map_err(|e| {
            anyhow!("Failed to open fresh db after schema corruption at {:?}: {}", db_path, e)
        })?;
        init_sqlite_schema_with_options(&fresh_conn, true, busy_timeout_ms, wal_enabled).map_err(|e| {
            anyhow!("Failed to initialize schema on fresh db: {} (schema error was: {})", e, schema_err)
        })?;
        return Ok((db_path, fresh_conn));
    }

    Ok((db_path, conn))
}

/// Initialize the SQLite database schema with default options.
pub fn init_sqlite_schema(conn: &Connection, is_file_backed: bool) -> Result<()> {
    init_sqlite_schema_with_options(conn, is_file_backed, 5000, true)
}

/// Initialize the SQLite database schema and configuration pragmas with tunable options.
pub fn init_sqlite_schema_with_options(
    conn: &Connection,
    is_file_backed: bool,
    busy_timeout_ms: u32,
    wal_enabled: bool,
) -> Result<()> {
    if is_file_backed {
        let pragma_sql = format!(
            "PRAGMA busy_timeout = {};
             PRAGMA synchronous = NORMAL;",
            busy_timeout_ms
        );
        let _ = conn.execute_batch(&pragma_sql);

        if wal_enabled {
            if conn.execute_batch("PRAGMA journal_mode = WAL;").is_err() {
                let _ = conn.execute_batch("PRAGMA journal_mode = DELETE;");
            }
        } else {
            let _ = conn.execute_batch("PRAGMA journal_mode = DELETE;");
        }
    }

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
             updated_at TEXT NOT NULL,
             metadata_json TEXT
         );

         CREATE TABLE IF NOT EXISTS task_results (
             task_id TEXT PRIMARY KEY,
             status TEXT NOT NULL,
             summary TEXT NOT NULL,
             duration_ms INTEGER NOT NULL,
             raw_json TEXT NOT NULL,
             created_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS task_idempotency (
             key TEXT PRIMARY KEY,
             request TEXT NOT NULL,
             task_id TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS task_reviews (
             id INTEGER PRIMARY KEY,
             task_id TEXT NOT NULL,
             revision TEXT NOT NULL,
             reviewer TEXT NOT NULL,
             decision TEXT NOT NULL,
             created_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS task_schedules (
             schedule_id TEXT PRIMARY KEY,
             task_id TEXT,
             title TEXT NOT NULL,
             goal TEXT NOT NULL,
             priority TEXT NOT NULL,
             spec_json TEXT NOT NULL,
             next_run_at TEXT NOT NULL,
             last_run_at TEXT,
             last_spawned_task_id TEXT,
             run_count INTEGER NOT NULL DEFAULT 0,
             max_runs INTEGER,
             enabled INTEGER NOT NULL DEFAULT 1,
             engine_name TEXT,
             created_at TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             raw_json TEXT NOT NULL
         );

         CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
         CREATE INDEX IF NOT EXISTS idx_task_idempotency_key ON task_idempotency(key);
         CREATE INDEX IF NOT EXISTS idx_task_reviews_task_id ON task_reviews(task_id);
         CREATE INDEX IF NOT EXISTS idx_task_schedules_next_run ON task_schedules(enabled, next_run_at ASC);",
    )
    .map_err(|e| anyhow!("Failed to initialize tasks schema: {}", e))?;

    let _ = conn.execute("ALTER TABLE subtask_checkpoints ADD COLUMN metadata_json TEXT;", []);

    Ok(())
}
