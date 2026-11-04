//! Project-scoped persona memory. Callers supply trusted project/persona context;
//! model output must never select that context or assert verification success.
use agent_domain::AgentPersonaProfile;
use anyhow::{bail, ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub persona_id: String,
    pub kind: String,
    pub scope: String,
    pub status: String,
    pub content: String,
    pub conditions: String,
    pub source: String,
    pub verified: bool,
    pub profile_version: i64,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedContext {
    pub memory_ids: Vec<String>,
    pub text: String,
}

/// One workspace database; an explicit project key also protects shared databases.
#[derive(Debug, Clone)]
pub struct MemoryService {
    path: PathBuf,
    project: String,
    persona: String,
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
        db.execute_batch("PRAGMA journal_mode=WAL;
          CREATE TABLE IF NOT EXISTS profiles(project TEXT, persona TEXT, version INTEGER, profile TEXT, PRIMARY KEY(project,persona));
          CREATE TABLE IF NOT EXISTS memories(id TEXT PRIMARY KEY, project TEXT NOT NULL, persona TEXT NOT NULL, kind TEXT NOT NULL, scope TEXT NOT NULL, status TEXT NOT NULL, content TEXT NOT NULL, conditions TEXT NOT NULL, source TEXT NOT NULL, verified INTEGER NOT NULL, version INTEGER NOT NULL, created TEXT NOT NULL, import_key TEXT UNIQUE);
          CREATE INDEX IF NOT EXISTS memory_scope ON memories(project,persona,scope,status);
          CREATE VIRTUAL TABLE IF NOT EXISTS memory_search USING fts5(id UNINDEXED, content, conditions);
          CREATE TRIGGER IF NOT EXISTS memory_insert AFTER INSERT ON memories BEGIN INSERT INTO memory_search(id,content,conditions) VALUES(new.id,new.content,new.conditions); END;
          CREATE TRIGGER IF NOT EXISTS memory_delete AFTER DELETE ON memories BEGIN DELETE FROM memory_search WHERE id=old.id; END;
          CREATE TABLE IF NOT EXISTS evidence(memory TEXT, episode TEXT, PRIMARY KEY(memory,episode));
          CREATE TABLE IF NOT EXISTS retrievals(project TEXT,persona TEXT,task TEXT,ids TEXT,created TEXT);")?;
        Ok(service)
    }
    fn connect(&self) -> Result<Connection> {
        let db = Connection::open(&self.path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(db)
    }
    pub fn register_profile(&self, profile: &AgentPersonaProfile) -> Result<i64> {
        ensure!(
            profile.identity.id.0 == self.persona,
            "profile identity mismatch"
        );
        let mut db = self.connect()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let old: Option<(i64, String)> = tx
            .query_row(
                "SELECT version,profile FROM profiles WHERE project=?1 AND persona=?2",
                params![self.project, self.persona],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let json = serde_json::to_string(profile)?;
        let mut version = 1;
        if let Some((v, prior)) = old {
            if prior == json {
                return Ok(v);
            }
            version = v + 1;
            let prior: AgentPersonaProfile = serde_json::from_str(&prior)?;
            if serde_json::to_value(&prior.role)? != serde_json::to_value(&profile.role)?
                || prior.mission.constraints != profile.mission.constraints
            {
                tx.execute("UPDATE memories SET status='candidate' WHERE project=?1 AND persona=?2 AND kind='procedure' AND status='active'",params![self.project,self.persona])?;
            }
        }
        tx.execute("INSERT INTO profiles VALUES(?1,?2,?3,?4) ON CONFLICT(project,persona) DO UPDATE SET version=excluded.version,profile=excluded.profile",params![self.project,self.persona,version,json])?;
        tx.commit()?;
        Ok(version)
    }
    fn insert(
        &self,
        kind: &str,
        content: &str,
        conditions: &str,
        source: &str,
        verified: bool,
        import_key: Option<&str>,
    ) -> Result<String> {
        ensure!(!content.trim().is_empty(), "empty memory");
        let mut db = self.connect()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(key) = import_key {
            if let Some(id) = tx
                .query_row("SELECT id FROM memories WHERE import_key=?1", [key], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
            {
                return Ok(id);
            }
        }
        let version: i64 = tx
            .query_row(
                "SELECT version FROM profiles WHERE project=?1 AND persona=?2",
                params![self.project, self.persona],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO memories VALUES(?1,?2,?3,?4,'private',?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                id,
                self.project,
                self.persona,
                kind,
                if kind == "episode" {
                    "active"
                } else {
                    "candidate"
                },
                content,
                conditions,
                source,
                verified,
                version,
                chrono::Utc::now().to_rfc3339(),
                import_key
            ],
        )?;
        tx.commit()?;
        Ok(id)
    }
    /// `verified` must come from a trusted verifier, never an engine exit code.
    pub fn record_episode(&self, task_id: &str, content: &str, verified: bool) -> Result<String> {
        ensure!(!task_id.is_empty(), "task id required");
        self.insert("episode", content, "", task_id, verified, None)
    }
    pub fn propose_lesson(
        &self,
        content: &str,
        conditions: &str,
        episode_id: &str,
    ) -> Result<String> {
        let episode = self.show(episode_id)?;
        ensure!(
            episode.kind == "episode" && episode.persona_id == self.persona,
            "own episode required"
        );
        let existing = self.list()?.into_iter().find(|r| {
            r.persona_id == self.persona
                && r.kind == "procedure"
                && r.content == content
                && r.conditions == conditions
                && (r.status == "candidate" || r.status == "active")
        });
        let id = match existing {
            Some(r) => r.id,
            None => self.insert("procedure", content, conditions, episode_id, false, None)?,
        };
        self.connect()?.execute(
            "INSERT OR IGNORE INTO evidence VALUES(?1,?2)",
            params![id, episode_id],
        )?;
        Ok(id)
    }
    /// Explicit review activates a candidate; conflicts are never auto-resolved.
    pub fn activate(&self, id: &str) -> Result<()> {
        let record = self.show(id)?;
        ensure!(
            record.kind == "procedure" && record.status == "candidate",
            "candidate procedure required"
        );
        let db = self.connect()?;
        let changed = db.execute("UPDATE memories SET status='active',verified=1 WHERE id=?1 AND project=?2 AND persona=?3 AND status='candidate' AND EXISTS(SELECT 1 FROM evidence e JOIN memories s ON s.id=e.episode WHERE e.memory=?1 AND s.project=?2 AND s.persona=?3 AND s.kind='episode' AND s.verified=1)",params![id,self.project,self.persona])?;
        ensure!(changed == 1, "verified own episode required");
        Ok(())
    }
    fn set_status(&self, id: &str, status: &str) -> Result<()> {
        let db = self.connect()?;
        ensure!(
            db.execute(
                "UPDATE memories SET status=?1 WHERE id=?2 AND project=?3 AND persona=?4",
                params![status, id, self.project, self.persona]
            )? == 1,
            "memory not owned"
        );
        Ok(())
    }
    pub fn reject(&self, id: &str) -> Result<()> {
        self.set_status(id, "rejected")
    }
    pub fn supersede(&self, id: &str) -> Result<()> {
        self.set_status(id, "superseded")
    }
    pub fn publish(&self, id: &str) -> Result<String> {
        let r = self.show(id)?;
        ensure!(
            r.persona_id == self.persona && r.status == "active" && r.kind == "procedure",
            "active owned procedure required"
        );
        let mut db = self.connect()?;
        let tx = db.transaction()?;
        let new_id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO memories SELECT ?1,project,persona,'knowledge','project','active',content,conditions,id,1,version,created,NULL FROM memories WHERE id=?2 AND project=?3 AND persona=?4 AND status='active'",params![new_id,id,self.project,self.persona])?;
        tx.commit()?;
        Ok(new_id)
    }
    pub fn delete(&self, id: &str) -> Result<()> {
        let db = self.connect()?;
        db.execute("DELETE FROM evidence WHERE memory IN (SELECT id FROM memories WHERE id=?1 AND project=?2 AND persona=?3)",params![id,self.project,self.persona])?;
        ensure!(
            db.execute(
                "DELETE FROM memories WHERE id=?1 AND project=?2 AND persona=?3",
                params![id, self.project, self.persona]
            )? == 1,
            "memory not owned"
        );
        Ok(())
    }
    fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
        Ok(MemoryRecord {
            id: r.get(0)?,
            persona_id: r.get(1)?,
            kind: r.get(2)?,
            scope: r.get(3)?,
            status: r.get(4)?,
            content: r.get(5)?,
            conditions: r.get(6)?,
            source: r.get(7)?,
            verified: r.get(8)?,
            profile_version: r.get(9)?,
            created_at: r.get(10)?,
        })
    }
    pub fn list(&self) -> Result<Vec<MemoryRecord>> {
        let db = self.connect()?;
        let mut stmt=db.prepare("SELECT id,persona,kind,scope,status,content,conditions,source,verified,version,created FROM memories WHERE project=?1 AND (persona=?2 OR (scope='project' AND status='active')) ORDER BY created DESC,id")?;
        let rows = stmt
            .query_map(params![self.project, self.persona], Self::row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
    pub fn show(&self, id: &str) -> Result<MemoryRecord> {
        self.list()?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| anyhow::anyhow!("memory unavailable"))
    }
    pub fn retrieve(&self, task_id: &str, query: &str) -> Result<RetrievedContext> {
        let tokens = crate::extract_query_tokens(query);
        let mut result = RetrievedContext {
            memory_ids: vec![],
            text: String::new(),
        };
        if tokens.is_empty() {
            return Ok(result);
        }
        let expression = tokens
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let db = self.connect()?;
        let mut stmt=db.prepare("SELECT m.id FROM memory_search JOIN memories m ON m.id=memory_search.id WHERE memory_search MATCH ?1 AND m.project=?2 AND (m.persona=?3 OR m.scope='project') AND m.status='active' AND m.kind IN ('procedure','knowledge') ORDER BY bm25(memory_search,0,1,2), m.verified DESC,m.created DESC,m.id LIMIT 8")?;
        let ids = stmt
            .query_map(params![expression, self.project, self.persona], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            let r = self.show(&id)?;
            let entry = format!(
                "\n[Memory {} | source {}]\n{}\nApplies when: {}\n",
                r.id, r.source, r.content, r.conditions
            );
            let remaining = 12000usize.saturating_sub(result.text.chars().count());
            if remaining == 0 {
                break;
            }
            result.text.extend(entry.chars().take(remaining));
            result.memory_ids.push(id);
        }
        db.execute(
            "INSERT INTO retrievals VALUES(?1,?2,?3,?4,?5)",
            params![
                self.project,
                self.persona,
                task_id,
                serde_json::to_string(&result.memory_ids)?,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(result)
    }
    /// Explicit operator assignment; imported text remains a candidate, never trusted instructions.
    pub fn adopt_legacy(&self, id: &str) -> Result<String> {
        let legacy = Self {
            persona: "__legacy__".into(),
            ..self.clone()
        };
        let r = legacy.show(id)?;
        ensure!(r.kind == "legacy", "legacy record required");
        self.insert(
            "legacy",
            &r.content,
            "",
            id,
            false,
            Some(&format!("adopt:{}:{}:{}", self.project, self.persona, id)),
        )
    }

    /// Import quarantines old text. It cannot become prompt context by accident.
    pub fn import_legacy(&self, path: &Path) -> Result<String> {
        let content = std::fs::read_to_string(path)?;
        if content.trim().is_empty() {
            bail!("empty legacy file")
        }
        let backup = path.with_extension("persona-memory-backup");
        if !backup.exists() {
            std::fs::copy(path, &backup)?;
        }
        let legacy = Self {
            persona: "__legacy__".into(),
            ..self.clone()
        };
        let key = format!(
            "{}:{}:{}",
            self.project,
            path.canonicalize()?.display(),
            content
        );
        legacy.insert(
            "legacy",
            &content,
            "",
            &path.display().to_string(),
            false,
            Some(&key),
        )
    }
}
