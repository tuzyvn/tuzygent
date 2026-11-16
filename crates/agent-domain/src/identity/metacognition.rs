use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single proven capability or domain constraint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompetenceEntry {
    pub claim: String,
    pub verification_basis: String,
    pub updated_at: DateTime<Utc>,
}

/// A recognized boundary or current limitation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundaryEntry {
    pub limit: String,
    pub observed_evidence: String,
    pub updated_at: DateTime<Utc>,
}

/// An actionable learning strategy or heuristic (trigger-action rule).
/// Must be an imperative ("WHEN ... ALWAYS / NEVER ..."), never vague description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningStrategy {
    pub trigger_condition: String,
    pub imperative_action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workaround: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Self-knowledge profile for an agent or persona.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MetacognitionProfile {
    pub persona_id: String,
    pub competencies: Vec<CompetenceEntry>,
    pub boundaries: Vec<BoundaryEntry>,
    pub strategies: Vec<LearningStrategy>,
    pub last_reflected_at: Option<DateTime<Utc>>,
}

impl MetacognitionProfile {
    pub fn new(persona_id: &str) -> Self {
        Self {
            persona_id: persona_id.to_string(),
            competencies: Vec::new(),
            boundaries: Vec::new(),
            strategies: Vec::new(),
            last_reflected_at: None,
        }
    }

    /// Exports the competencies and boundaries into standardized COMPETENCE.md format.
    pub fn to_competence_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Agent Competence & Boundaries\n\n");

        out.push_str("## Demonstrated Competencies\n");
        if self.competencies.is_empty() {
            out.push_str("- No verified competencies recorded yet.\n");
        } else {
            for c in &self.competencies {
                out.push_str(&format!(
                    "- **{}** (Verified by: {})\n",
                    c.claim, c.verification_basis
                ));
            }
        }

        out.push_str("\n## Known Boundaries & Limits\n");
        if self.boundaries.is_empty() {
            out.push_str("- No boundary limitations recorded.\n");
        } else {
            for b in &self.boundaries {
                out.push_str(&format!(
                    "- **Limit:** {} (Evidence: {})\n",
                    b.limit, b.observed_evidence
                ));
            }
        }

        out
    }

    /// Exports learning strategies into standardized LEARNING_STRATEGIES.md format.
    pub fn to_strategies_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Agent Learning Strategies & Heuristics\n\n");
        out.push_str("<!-- Actionable trigger-action imperatives distilled from verified task executions -->\n\n");

        if self.strategies.is_empty() {
            out.push_str("- No strategies recorded yet.\n");
        } else {
            for s in &self.strategies {
                out.push_str(&format!("- WHEN {}\n", s.trigger_condition));
                out.push_str(&format!("  {} \n", s.imperative_action));
                if let Some(ref wa) = s.workaround {
                    out.push_str(&format!("  *Workaround:* {}\n", wa));
                }
            }
        }

        out
    }

    /// Formats a compact prompt section for injection into the Agent system prompt.
    pub fn format_prompt_injection(&self) -> String {
        if self.competencies.is_empty() && self.strategies.is_empty() {
            return String::new();
        }

        let mut section = String::new();
        section.push_str("\n# Self-Understanding & Reusable Methods\n");

        if !self.competencies.is_empty() {
            section.push_str("## Core Competencies & Limits\n");
            for c in &self.competencies {
                section.push_str(&format!("- {}\n", c.claim));
            }
            for b in &self.boundaries {
                section.push_str(&format!("- LIMIT: {}\n", b.limit));
            }
        }

        if !self.strategies.is_empty() {
            section.push_str("## Learned Execution Strategies\n");
            for s in &self.strategies {
                section.push_str(&format!("- WHEN {} => {}\n", s.trigger_condition, s.imperative_action));
            }
        }

        section
    }

    /// Parses COMPETENCE.md and LEARNING_STRATEGIES.md back into profile entries.
    pub fn parse_from_markdown(persona_id: &str, comp_md: &str, strat_md: &str) -> Self {
        let mut profile = Self::new(persona_id);
        let now = Utc::now();

        // Parse competencies
        let mut in_competencies = false;
        let mut in_boundaries = false;
        for line in comp_md.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("## Demonstrated Competencies") {
                in_competencies = true;
                in_boundaries = false;
                continue;
            } else if trimmed.starts_with("## Known Boundaries") {
                in_competencies = false;
                in_boundaries = true;
                continue;
            }

            if trimmed.starts_with("- ") {
                let text = trimmed.trim_start_matches("- ").trim();
                if text.starts_with("No verified") || text.starts_with("No boundary") {
                    continue;
                }
                if in_competencies {
                    // Try splitting "claim (Verified by: basis)"
                    if let Some(pos) = text.find("(Verified by:") {
                        let claim = text[..pos].trim().trim_matches('*').trim();
                        let basis = text[pos + 13..].trim().trim_end_matches(')').trim();
                        profile.competencies.push(CompetenceEntry {
                            claim: claim.to_string(),
                            verification_basis: basis.to_string(),
                            updated_at: now,
                        });
                    } else {
                        profile.competencies.push(CompetenceEntry {
                            claim: text.to_string(),
                            verification_basis: "historical evidence".to_string(),
                            updated_at: now,
                        });
                    }
                } else if in_boundaries {
                    profile.boundaries.push(BoundaryEntry {
                        limit: text.to_string(),
                        observed_evidence: "documented limit".to_string(),
                        updated_at: now,
                    });
                }
            }
        }

        // Parse strategies
        let mut current_trigger: Option<String> = None;
        for line in strat_md.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("- WHEN ") {
                current_trigger = Some(trimmed.trim_start_matches("- WHEN ").trim().to_string());
            } else if let Some(trigger) = current_trigger.take() {
                if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("<!--") {
                    profile.strategies.push(LearningStrategy {
                        trigger_condition: trigger,
                        imperative_action: trimmed.to_string(),
                        workaround: None,
                        updated_at: now,
                    });
                }
            }
        }

        profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metacognition_markdown_roundtrip() {
        let mut profile = MetacognitionProfile::new("developer");
        profile.competencies.push(CompetenceEntry {
            claim: "Rust Clean Architecture design".to_string(),
            verification_basis: "Passed 40 workspace tests".to_string(),
            updated_at: Utc::now(),
        });
        profile.boundaries.push(BoundaryEntry {
            limit: "Requires native toolchain for C-bindings".to_string(),
            observed_evidence: "libsqlite3 build failure".to_string(),
            updated_at: Utc::now(),
        });
        profile.strategies.push(LearningStrategy {
            trigger_condition: "running shell commands on Windows".to_string(),
            imperative_action: "ALWAYS normalize path separators to backslashes".to_string(),
            workaround: Some("Use powershell wrapper".to_string()),
            updated_at: Utc::now(),
        });

        let comp_md = profile.to_competence_markdown();
        let strat_md = profile.to_strategies_markdown();

        assert!(comp_md.contains("Rust Clean Architecture design"));
        assert!(comp_md.contains("libsqlite3 build failure"));
        assert!(strat_md.contains("running shell commands on Windows"));

        let roundtrip = MetacognitionProfile::parse_from_markdown("developer", &comp_md, &strat_md);
        assert_eq!(roundtrip.competencies.len(), 1);
        assert_eq!(roundtrip.competencies[0].claim, "Rust Clean Architecture design");
        assert_eq!(roundtrip.boundaries.len(), 1);
        assert_eq!(roundtrip.strategies.len(), 1);
        assert_eq!(roundtrip.strategies[0].trigger_condition, "running shell commands on Windows");

        let prompt_injection = profile.format_prompt_injection();
        assert!(prompt_injection.contains("Core Competencies & Limits"));
        assert!(prompt_injection.contains("Rust Clean Architecture design"));
    }
}
