use std::path::{Path, PathBuf};

use anyhow::{ensure, Result};
use rusqlite::Connection;

/// One workspace database; an explicit project key also protects shared databases.
#[derive(Debug, Clone)]
pub struct MemoryService {
    pub(crate) path: PathBuf,
    pub(crate) project: String,
    pub(crate) persona: String,
}

impl MemoryService {
    pub fn open(workspace: &Path, project: &str, persona: &str) -> Result<Self> {
        ensure!(
            !project.trim().is_empty() && !persona.trim().is_empty(),
            "project/persona required"
        );
        let dir = workspace.join("storage/persona_memory");
        std::fs::create_dir_all(&dir)?;
        let service = Self {
            path: dir.join("memory.sqlite3"),
            project: project.into(),
            persona: persona.into(),
        };
        let db = service.connect()?;
        super::schema::init_schema(&db)?;
        Ok(service)
    }

    pub(crate) fn connect(&self) -> Result<Connection> {
        super::schema::connect_db(&self.path)
    }

    pub fn project(&self) -> &str {
        &self.project
    }

    pub fn persona(&self) -> &str {
        &self.persona
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
