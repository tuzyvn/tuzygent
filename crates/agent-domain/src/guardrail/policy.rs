use crate::audit_ledger::{AuditEventType, AuditLedger};
use crate::guardrail::fault::{GuardrailError, GuardrailFault};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailPolicy {
    pub red_lines: Vec<RedLineRule>,
    pub max_subtasks_per_goal: usize,
    pub max_retries_on_error: usize,
    pub allow_shell_execution: bool,
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
}

fn default_max_output_bytes() -> usize {
    10 * 1024 * 1024 // 10MB default buffer limit
}

impl Default for GuardrailPolicy {
    fn default() -> Self {
        Self {
            red_lines: Vec::new(),
            max_subtasks_per_goal: 30,
            max_retries_on_error: 3,
            allow_shell_execution: true,
            max_output_bytes: default_max_output_bytes(),
        }
    }
}

pub(crate) fn normalize_command_str(s: &str) -> String {
    let mut normalized = String::with_capacity(s.len());
    let mut first = true;
    for word in s.split_whitespace() {
        if !first {
            normalized.push(' ');
        }
        first = false;
        for ch in word.chars() {
            for lower in ch.to_lowercase() {
                normalized.push(lower);
            }
        }
    }
    normalized
}

impl GuardrailPolicy {
    pub fn default_enterprise() -> Self {
        Self {
            red_lines: vec![
                RedLineRule {
                    pattern: "rm -rf|rm -fr|rm -r -f|rm -f -r|rm --recursive".to_string(),
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
                    pattern: "chmod 777|chmod 0777|chmod a+rwx|chown root".to_string(),
                    description: "Insecure permission changes".to_string(),
                    severity: ActionSeverity::HighImpact,
                    requires_human_approval: true,
                },
            ],
            max_subtasks_per_goal: 30,
            max_retries_on_error: 3,
            allow_shell_execution: true,
            max_output_bytes: default_max_output_bytes(),
        }
    }

    pub fn simulate_fault(&self, fault: &GuardrailFault) -> Result<(), GuardrailError> {
        match fault {
            GuardrailFault::SocketTimeout => Err(GuardrailError::SocketTimeout),
            GuardrailFault::ConnectionReset => Err(GuardrailError::ConnectionReset),
            GuardrailFault::TransientIo => Err(GuardrailError::IoError(
                "Simulated transient I/O failure".to_string(),
            )),
            GuardrailFault::CorruptedRule => Err(GuardrailError::CorruptedPolicy(
                "Rule pattern corrupted or unparseable".to_string(),
            )),
            GuardrailFault::ServiceUnavailable => Err(GuardrailError::ServiceUnavailable(
                "Guardrail service temporarily unavailable".to_string(),
            )),
        }
    }

    pub fn check_command_safety(&self, command: &str) -> Result<(), String> {
        let normalized = normalize_command_str(command);
        for rule in &self.red_lines {
            let matched = rule.pattern.split('|').any(|pattern| {
                let clean_pattern = normalize_command_str(pattern);
                !clean_pattern.is_empty() && normalized.contains(&clean_pattern)
            });
            if matched {
                return Err(format!(
                    "RED_LINE_BLOCKED: Command violates policy '{}' (Severity: {:?})",
                    rule.description, rule.severity
                ));
            }
        }
        Ok(())
    }

    pub fn evaluate_and_record(
        &self,
        ledger: &mut AuditLedger,
        command: &str,
    ) -> Result<(), String> {
        match self.check_command_safety(command) {
            Ok(()) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Command approved for execution: {}", command),
                );
                Ok(())
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandBlocked,
                    ActionSeverity::Destructive,
                    format!("Command blocked: {}. Detail: {}", command, err),
                );
                Err(err)
            }
        }
    }

    pub fn check_command_safety_with_audit(
        &self,
        ledger: &mut AuditLedger,
        command: &str,
    ) -> Result<(), String> {
        self.evaluate_and_record(ledger, command)
    }

    pub fn record_exception(&self, ledger: &mut AuditLedger, command: &str, error: &str) {
        ledger.record_event(
            AuditEventType::CommandException,
            ActionSeverity::HighImpact,
            format!(
                "Exception occurred while processing command '{}': {}",
                command, error
            ),
        );
    }
}
