use agent_domain::AgentPersonaProfile;
use anyhow::{ensure, Result};
use rusqlite::{params, OptionalExtension};

use super::service::MemoryService;

impl MemoryService {
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
                tx.execute(
                    "UPDATE memories SET status='candidate' WHERE project=?1 AND persona=?2 AND kind='procedure' AND status='active'",
                    params![self.project, self.persona],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO profiles VALUES(?1,?2,?3,?4) ON CONFLICT(project,persona) DO UPDATE SET version=excluded.version,profile=excluded.profile",
            params![self.project, self.persona, version, json],
        )?;
        tx.commit()?;
        Ok(version)
    }
}
