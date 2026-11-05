use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_DDL: &str = r#"
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS profiles(
    project TEXT,
    persona TEXT,
    version INTEGER,
    profile TEXT,
    PRIMARY KEY(project, persona)
);
CREATE TABLE IF NOT EXISTS memories(
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    persona TEXT NOT NULL,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL,
    status TEXT NOT NULL,
    content TEXT NOT NULL,
    conditions TEXT NOT NULL,
    source TEXT NOT NULL,
    verified INTEGER NOT NULL,
    version INTEGER NOT NULL,
    created TEXT NOT NULL,
    import_key TEXT UNIQUE
);
CREATE INDEX IF NOT EXISTS memory_scope ON memories(project, persona, scope, status);
CREATE VIRTUAL TABLE IF NOT EXISTS memory_search USING fts5(id UNINDEXED, content, conditions);
CREATE TRIGGER IF NOT EXISTS memory_insert AFTER INSERT ON memories BEGIN
    INSERT INTO memory_search(id, content, conditions) VALUES(new.id, new.content, new.conditions);
END;
CREATE TRIGGER IF NOT EXISTS memory_delete AFTER DELETE ON memories BEGIN
    DELETE FROM memory_search WHERE id=old.id;
END;
CREATE TABLE IF NOT EXISTS evidence(
    memory TEXT,
    episode TEXT,
    PRIMARY KEY(memory, episode)
);
CREATE TABLE IF NOT EXISTS retrievals(
    project TEXT,
    persona TEXT,
    task TEXT,
    ids TEXT,
    created TEXT
);
"#;

pub fn init_schema(db: &Connection) -> Result<()> {
    db.execute_batch(SCHEMA_DDL)?;
    Ok(())
}

pub fn connect_db(path: &Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(db)
}
