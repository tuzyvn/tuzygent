use anyhow::Result;
use rusqlite::params;

use super::service::MemoryService;
use super::types::{MemoryRecord, RetrievedContext};

impl MemoryService {
    pub(crate) fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
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
        let mut stmt = db.prepare(
            "SELECT id,persona,kind,scope,status,content,conditions,source,verified,version,created FROM memories WHERE project=?1 AND (persona=?2 OR (scope='project' AND status='active')) ORDER BY created DESC,id",
        )?;
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
        let mut stmt = db.prepare(
            "SELECT m.id FROM memory_search JOIN memories m ON m.id=memory_search.id WHERE memory_search MATCH ?1 AND m.project=?2 AND (m.persona=?3 OR m.scope='project') AND m.status='active' AND m.kind IN ('procedure','knowledge') ORDER BY bm25(memory_search,0,1,2), m.verified DESC,m.created DESC,m.id LIMIT 8",
        )?;
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
}
