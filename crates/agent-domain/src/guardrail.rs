use crate::audit_ledger::{AuditEventType, AuditLedger};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GuardrailError {
    #[error("Command blocked by red line rule: {0}")]
    Blocked(String),
    #[error("Simulated socket timeout during guardrail check")]
    SocketTimeout,
    #[error("Connection reset during guardrail check")]
    ConnectionReset,
    #[error("Transient I/O failure during guardrail evaluation or audit write: {0}")]
    IoError(String),
    #[error("Corrupted or invalid policy rule: {0}")]
    CorruptedPolicy(String),
    #[error("Policy service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Max retry attempts ({0}) exceeded during guardrail recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailFault {
    SocketTimeout,
    ConnectionReset,
    CorruptedRule,
    TransientIo,
    ServiceUnavailable,
}

#[derive(Debug, Clone, Default)]
pub struct GuardrailFaultPolicy {
    fault: Option<GuardrailFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl GuardrailFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: GuardrailFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: GuardrailFault, interval: u32) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: Some(interval.max(1)),
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn with_max_injections(mut self, max: usize) -> Self {
        self.max_injections = Some(max);
        self
    }

    pub fn should_inject(&mut self) -> Option<GuardrailFault> {
        let fault = self.fault.as_ref()?;

        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.wrapping_add(1);

        let inject = match self.fail_interval {
            Some(interval) if interval > 0 => (self.invocation_count - 1).is_multiple_of(interval),
            Some(_) => true,
            None => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }
}

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
            GuardrailFault::TransientIo => {
                Err(GuardrailError::IoError("Simulated transient I/O failure".to_string()))
            }
            GuardrailFault::CorruptedRule => {
                Err(GuardrailError::CorruptedPolicy("Rule pattern corrupted or unparseable".to_string()))
            }
            GuardrailFault::ServiceUnavailable => {
                Err(GuardrailError::ServiceUnavailable("Guardrail service temporarily unavailable".to_string()))
            }
        }
    }

    pub fn check_command_safety(&self, command: &str) -> Result<(), String> {
        let normalized = command
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        for rule in &self.red_lines {
            let matched = rule.pattern.split('|').any(|pattern| {
                let clean_pattern = pattern
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase();
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

    pub fn record_exception(
        &self,
        ledger: &mut AuditLedger,
        command: &str,
        error: &str,
    ) {
        ledger.record_event(
            AuditEventType::CommandException,
            ActionSeverity::HighImpact,
            format!("Exception occurred while processing command '{}': {}", command, error),
        );
    }
}

#[derive(Debug, Clone)]
pub struct ResilientGuardrailEvaluator {
    policy: GuardrailPolicy,
    fault_policy: GuardrailFaultPolicy,
    max_retries: usize,
    evaluated_count: usize,
    recovered_count: usize,
    blocked_count: usize,
}

impl ResilientGuardrailEvaluator {
    pub fn new(
        policy: GuardrailPolicy,
        fault_policy: GuardrailFaultPolicy,
        max_retries: usize,
    ) -> Self {
        Self {
            policy,
            fault_policy,
            max_retries,
            evaluated_count: 0,
            recovered_count: 0,
            blocked_count: 0,
        }
    }

    pub fn with_fault_policy(policy: GuardrailPolicy, fault_policy: GuardrailFaultPolicy) -> Self {
        Self::new(policy, fault_policy, 3)
    }

    pub fn evaluated_count(&self) -> usize {
        self.evaluated_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn blocked_count(&self) -> usize {
        self.blocked_count
    }

    pub fn policy(&self) -> &GuardrailPolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut GuardrailPolicy {
        &mut self.policy
    }

    pub fn fault_policy(&self) -> &GuardrailFaultPolicy {
        &self.fault_policy
    }

    pub fn fault_policy_mut(&mut self) -> &mut GuardrailFaultPolicy {
        &mut self.fault_policy
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn reset_counts(&mut self) {
        self.evaluated_count = 0;
        self.recovered_count = 0;
        self.blocked_count = 0;
    }

    pub fn check_command_safety(&mut self, command: &str) -> Result<(), GuardrailError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    GuardrailFault::SocketTimeout
                    | GuardrailFault::ConnectionReset
                    | GuardrailFault::TransientIo
                    | GuardrailFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            return match fault {
                                GuardrailFault::SocketTimeout => Err(GuardrailError::SocketTimeout),
                                GuardrailFault::ConnectionReset => {
                                    Err(GuardrailError::ConnectionReset)
                                }
                                GuardrailFault::TransientIo => Err(GuardrailError::IoError(
                                    "Simulated transient I/O failure".to_string(),
                                )),
                                GuardrailFault::ServiceUnavailable => {
                                    Err(GuardrailError::ServiceUnavailable(
                                        "Guardrail service unavailable".to_string(),
                                    ))
                                }
                                _ => unreachable!(),
                            };
                        }
                    }
                    GuardrailFault::CorruptedRule => {
                        return Err(GuardrailError::CorruptedPolicy(
                            "Policy rule pattern corruption detected".to_string(),
                        ));
                    }
                }
            } else {
                match self.policy.check_command_safety(command) {
                    Ok(()) => {
                        self.evaluated_count += 1;
                        return Ok(());
                    }
                    Err(err) => {
                        self.blocked_count += 1;
                        return Err(GuardrailError::Blocked(err));
                    }
                }
            }
        }
    }

    pub fn evaluate_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        command: &str,
    ) -> Result<(), GuardrailError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    GuardrailFault::SocketTimeout
                    | GuardrailFault::ConnectionReset
                    | GuardrailFault::TransientIo
                    | GuardrailFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            let err = match fault {
                                GuardrailFault::SocketTimeout => GuardrailError::SocketTimeout,
                                GuardrailFault::ConnectionReset => {
                                    GuardrailError::ConnectionReset
                                }
                                GuardrailFault::TransientIo => {
                                    GuardrailError::IoError("Simulated transient I/O failure".to_string())
                                }
                                GuardrailFault::ServiceUnavailable => {
                                    GuardrailError::ServiceUnavailable(
                                        "Guardrail service unavailable".to_string(),
                                    )
                                }
                                _ => unreachable!(),
                            };
                            self.policy
                                .record_exception(ledger, command, &err.to_string());
                            return Err(err);
                        }
                    }
                    GuardrailFault::CorruptedRule => {
                        let err = GuardrailError::CorruptedPolicy(
                            "Policy rule pattern corruption detected".to_string(),
                        );
                        self.policy
                            .record_exception(ledger, command, &err.to_string());
                        return Err(err);
                    }
                }
            } else {
                match self.policy.check_command_safety(command) {
                    Ok(()) => {
                        self.evaluated_count += 1;
                        ledger.record_event(
                            AuditEventType::CommandAllowed,
                            ActionSeverity::Routine,
                            format!("Command approved for execution: {}", command),
                        );
                        return Ok(());
                    }
                    Err(err) => {
                        self.blocked_count += 1;
                        ledger.record_event(
                            AuditEventType::CommandBlocked,
                            ActionSeverity::Destructive,
                            format!("Command blocked: {}. Detail: {}", command, err),
                        );
                        return Err(GuardrailError::Blocked(err));
                    }
                }
            }
        }
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
