use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionSeverity {
    Routine,
    HighImpact,
    Destructive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedLineRule {
    pub pattern: String,
    pub description: String,
    pub severity: ActionSeverity,
    pub requires_human_approval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GuardrailPolicy {
    pub red_lines: Vec<RedLineRule>,
    pub max_subtasks_per_goal: usize,
    pub max_retries_on_error: usize,
    pub allow_shell_execution: bool,
}

impl GuardrailPolicy {
    pub fn default_enterprise() -> Self {
        Self {
            red_lines: vec![
                RedLineRule {
                    pattern: "rm -rf".to_string(),
                    description: "Recursive deletion of directories".to_string(),
                    severity: ActionSeverity::Destructive,
                    requires_human_approval: true,
                },
                RedLineRule {
                    pattern: "DROP TABLE|TRUNCATE".to_string(),
                    description: "Database table deletion".to_string(),
                    severity: ActionSeverity::Destructive,
                    requires_human_approval: true,
                },
                RedLineRule {
                    pattern: "chmod 777|chown root".to_string(),
                    description: "Insecure permission changes".to_string(),
                    severity: ActionSeverity::HighImpact,
                    requires_human_approval: true,
                },
            ],
            max_subtasks_per_goal: 30,
            max_retries_on_error: 3,
            allow_shell_execution: true,
        }
    }

    pub fn check_command_safety(&self, command: &str) -> Result<(), String> {
        let normalized = command
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        for rule in &self.red_lines {
            if rule.requires_human_approval
                && rule.pattern.split('|').any(|pattern| {
                    !pattern.trim().is_empty()
                        && normalized.contains(&pattern.trim().to_lowercase())
                })
            {
                return Err(format!(
                    "RED_LINE_BLOCKED: Command violates policy '{}' (Severity: {:?})",
                    rule.description, rule.severity
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_each_alternative_and_normalizes_whitespace_and_case() {
        let policy = GuardrailPolicy::default_enterprise();
        for command in [
            "DROP TABLE users",
            "truncate users",
            "chmod   777 file",
            "chown root file",
            "rm -rf data",
        ] {
            assert!(
                policy.check_command_safety(command).is_err(),
                "allowed {command}"
            );
        }
        assert!(policy.check_command_safety("cargo test").is_ok());
    }
}
