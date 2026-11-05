use anyhow::{ensure, Result};
use rusqlite::{params, OptionalExtension};

use super::service::MemoryService;

impl MemoryService {
    pub(crate) fn insert(
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
        let changed = db.execute(
            "UPDATE memories SET status='active',verified=1 WHERE id=?1 AND project=?2 AND persona=?3 AND status='candidate' AND EXISTS(SELECT 1 FROM evidence e JOIN memories s ON s.id=e.episode WHERE e.memory=?1 AND s.project=?2 AND s.persona=?3 AND s.kind='episode' AND s.verified=1)",
            params![id, self.project, self.persona],
        )?;
        ensure!(changed == 1, "verified own episode required");
        Ok(())
    }

    pub(crate) fn set_status(&self, id: &str, status: &str) -> Result<()> {
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
        tx.execute(
            "INSERT INTO memories SELECT ?1,project,persona,'knowledge','project','active',content,conditions,id,1,version,created,NULL FROM memories WHERE id=?2 AND project=?3 AND persona=?4 AND status='active'",
            params![new_id, id, self.project, self.persona],
        )?;
        tx.commit()?;
        Ok(new_id)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let db = self.connect()?;
        db.execute(
            "DELETE FROM evidence WHERE memory IN (SELECT id FROM memories WHERE id=?1 AND project=?2 AND persona=?3)",
            params![id, self.project, self.persona],
        )?;
        ensure!(
            db.execute(
                "DELETE FROM memories WHERE id=?1 AND project=?2 AND persona=?3",
                params![id, self.project, self.persona],
            )? == 1,
            "memory not owned"
        );
        Ok(())
    }
}
