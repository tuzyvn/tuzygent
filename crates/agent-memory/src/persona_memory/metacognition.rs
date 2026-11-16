use std::fs;
use std::path::PathBuf;

use agent_domain::{
    CompetenceEntry, LearningStrategy, MetacognitionProfile,
};
use anyhow::Result;
use chrono::Utc;

use super::service::MemoryService;

impl MemoryService {
    /// Returns the directory where human-auditable metacognition files live for this persona.
    pub fn metacognition_dir(&self) -> PathBuf {
        let parent = self.path.parent().unwrap_or_else(|| std::path::Path::new("."));
        parent.join(&self.project).join(&self.persona)
    }

    /// Persists the agent's self-understanding to COMPETENCE.md and LEARNING_STRATEGIES.md.
    pub fn save_metacognition(&self, profile: &MetacognitionProfile) -> Result<()> {
        let dir = self.metacognition_dir();
        fs::create_dir_all(&dir)?;

        let comp_path = dir.join("COMPETENCE.md");
        let strat_path = dir.join("LEARNING_STRATEGIES.md");

        fs::write(comp_path, profile.to_competence_markdown())?;
        fs::write(strat_path, profile.to_strategies_markdown())?;

        Ok(())
    }

    /// Loads the agent's self-understanding from COMPETENCE.md and LEARNING_STRATEGIES.md.
    /// Returns a default profile if the files do not yet exist.
    pub fn load_metacognition(&self) -> Result<MetacognitionProfile> {
        let dir = self.metacognition_dir();
        let comp_path = dir.join("COMPETENCE.md");
        let strat_path = dir.join("LEARNING_STRATEGIES.md");

        if !comp_path.exists() && !strat_path.exists() {
            return Ok(MetacognitionProfile::new(&self.persona));
        }

        let comp_md = if comp_path.exists() {
            fs::read_to_string(comp_path)?
        } else {
            String::new()
        };

        let strat_md = if strat_path.exists() {
            fs::read_to_string(strat_path)?
        } else {
            String::new()
        };

        Ok(MetacognitionProfile::parse_from_markdown(
            &self.persona,
            &comp_md,
            &strat_md,
        ))
    }

    /// Records or updates a demonstrated competence claim backed by explicit verification evidence.
    pub fn record_competence(&self, claim: &str, basis: &str) -> Result<()> {
        let mut profile = self.load_metacognition()?;
        let now = Utc::now();

        if let Some(existing) = profile.competencies.iter_mut().find(|c| c.claim.eq_ignore_ascii_case(claim.trim())) {
            existing.verification_basis = basis.trim().to_string();
            existing.updated_at = now;
        } else {
            profile.competencies.push(CompetenceEntry {
                claim: claim.trim().to_string(),
                verification_basis: basis.trim().to_string(),
                updated_at: now,
            });
        }
        profile.last_reflected_at = Some(now);
        self.save_metacognition(&profile)
    }

    /// Records an actionable trigger-action strategy ("WHEN ... ALWAYS / NEVER ...").
    pub fn record_strategy(&self, trigger: &str, action: &str, workaround: Option<&str>) -> Result<()> {
        let mut profile = self.load_metacognition()?;
        let now = Utc::now();

        if let Some(existing) = profile.strategies.iter_mut().find(|s| s.trigger_condition.eq_ignore_ascii_case(trigger.trim())) {
            existing.imperative_action = action.trim().to_string();
            existing.workaround = workaround.map(|w| w.trim().to_string());
            existing.updated_at = now;
        } else {
            profile.strategies.push(LearningStrategy {
                trigger_condition: trigger.trim().to_string(),
                imperative_action: action.trim().to_string(),
                workaround: workaround.map(|w| w.trim().to_string()),
                updated_at: now,
            });
        }
        profile.last_reflected_at = Some(now);
        self.save_metacognition(&profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_service_metacognition_lifecycle() {
        let tmp = std::env::temp_dir().join(format!("meta_test_{}", uuid::Uuid::new_v4().simple()));
        let service = MemoryService::open(&tmp, "test_proj", "secops").expect("open memory service");

        // Initially empty
        let initial = service.load_metacognition().expect("load initial");
        assert!(initial.competencies.is_empty());
        assert!(initial.strategies.is_empty());

        // Record a verified competence
        service
            .record_competence("Subprocess sandboxing with timeout", "Automated exit code & stdout test")
            .expect("record competence");

        // Record a strategy
        service
            .record_strategy(
                "handling untrusted user skills",
                "ALWAYS run static red-flag regex scan before execution",
                Some("Reject with SecurityRedFlag on EXTREME matches"),
            )
            .expect("record strategy");

        // Reload and verify
        let loaded = service.load_metacognition().expect("reload metacognition");
        assert_eq!(loaded.competencies.len(), 1);
        assert_eq!(loaded.competencies[0].claim, "Subprocess sandboxing with timeout");
        assert_eq!(loaded.strategies.len(), 1);
        assert_eq!(loaded.strategies[0].trigger_condition, "handling untrusted user skills");

        // Check files on disk
        let dir = service.metacognition_dir();
        assert!(dir.join("COMPETENCE.md").exists());
        assert!(dir.join("LEARNING_STRATEGIES.md").exists());

        let _ = fs::remove_dir_all(&tmp);
    }
}
