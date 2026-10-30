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
        for rule in &self.red_lines {
            if command.contains(&rule.pattern) || command.to_uppercase().contains(&rule.pattern) {
                if rule.requires_human_approval {
                    return Err(format!(
                        "RED_LINE_BLOCKED: Command violates policy '{}' (Severity: {:?})",
                        rule.description, rule.severity
                    ));
                }
            }
        }
        Ok(())
    }
}
