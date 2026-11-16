use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RedFlagLevel {
    Extreme,
    Warning,
}

impl std::fmt::Display for RedFlagLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RedFlagLevel::Extreme => write!(f, "EXTREME"),
            RedFlagLevel::Warning => write!(f, "WARNING"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScanTarget {
    Script,
    SkillMarkdown,
    AgentSpec,
}

impl std::fmt::Display for ScanTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanTarget::Script => write!(f, "Script"),
            ScanTarget::SkillMarkdown => write!(f, "SkillMarkdown"),
            ScanTarget::AgentSpec => write!(f, "AgentSpec"),
        }
    }
}

/// A security red-flag violation detected by static analysis before skill or agent loading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedFlagViolation {
    pub rule_id: String,
    pub level: RedFlagLevel,
    pub target: ScanTarget,
    pub pattern_matched: String,
    pub description: String,
    pub suggested_fix: String,
}

impl std::fmt::Display for RedFlagViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}: {} (pattern: '{}', fix: {})",
            self.level, self.rule_id, self.description, self.pattern_matched, self.suggested_fix
        )
    }
}
