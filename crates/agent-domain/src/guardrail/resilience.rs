use crate::audit_ledger::{AuditEventType, AuditLedger};
use crate::guardrail::fault::{GuardrailError, GuardrailFault, GuardrailFaultPolicy};
use crate::guardrail::policy::{
    normalize_command_str, ActionSeverity, GuardrailPolicy, RedLineRule,
};
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct CompiledRedLineRule {
    pub patterns: Vec<String>,
    pub description: String,
    pub severity: ActionSeverity,
}

impl CompiledRedLineRule {
    pub fn compile(rule: &RedLineRule) -> Self {
        let patterns = rule
            .pattern
            .split('|')
            .map(normalize_command_str)
            .filter(|p| !p.is_empty())
            .collect();
        Self {
            patterns,
            description: rule.description.clone(),
            severity: rule.severity,
        }
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
    compiled_rules: Vec<CompiledRedLineRule>,
    seen_commands: HashSet<String>,
    command_queue: VecDeque<String>,
    max_capacity: Option<usize>,
}

impl ResilientGuardrailEvaluator {
    pub fn new(
        policy: GuardrailPolicy,
        fault_policy: GuardrailFaultPolicy,
        max_retries: usize,
    ) -> Self {
        let compiled_rules = policy
            .red_lines
            .iter()
            .map(CompiledRedLineRule::compile)
            .collect();
        Self {
            policy,
            fault_policy,
            max_retries,
            evaluated_count: 0,
            recovered_count: 0,
            blocked_count: 0,
            compiled_rules,
            seen_commands: HashSet::new(),
            command_queue: VecDeque::new(),
            max_capacity: None,
        }
    }

    pub fn with_fault_policy(policy: GuardrailPolicy, fault_policy: GuardrailFaultPolicy) -> Self {
        Self::new(policy, fault_policy, 3)
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.seen_commands.reserve(capacity);
        self.command_queue.reserve(capacity);
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.seen_commands.reserve(max_capacity);
        self.command_queue.reserve(max_capacity);
        self
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
        self.seen_commands.clear();
        self.command_queue.clear();
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let set_mem = self.seen_commands.capacity() * (std::mem::size_of::<String>() + 36);
        let queue_mem = self.command_queue.capacity() * (std::mem::size_of::<String>() + 36);
        let rules_mem = self
            .compiled_rules
            .iter()
            .map(|r| {
                r.patterns.iter().map(|p| p.capacity()).sum::<usize>() + r.description.capacity()
            })
            .sum::<usize>();
        std::mem::size_of::<Self>() + set_mem + queue_mem + rules_mem
    }

    fn ensure_compiled(&mut self) {
        if self.compiled_rules.len() != self.policy.red_lines.len() {
            self.compiled_rules = self
                .policy
                .red_lines
                .iter()
                .map(CompiledRedLineRule::compile)
                .collect();
        }
    }

    fn record_seen(&mut self, command: &str) {
        if let Some(max_cap) = self.max_capacity {
            if self.command_queue.len() >= max_cap {
                if let Some(oldest) = self.command_queue.pop_front() {
                    self.seen_commands.remove(&oldest);
                }
            }
            self.command_queue.push_back(command.to_string());
        }
        self.seen_commands.insert(command.to_string());
    }

    fn check_safety_internal(&mut self, command: &str) -> Result<(), String> {
        self.ensure_compiled();
        let normalized = normalize_command_str(command);
        for rule in &self.compiled_rules {
            for pattern in &rule.patterns {
                if !pattern.is_empty() && normalized.contains(pattern) {
                    return Err(format!(
                        "RED_LINE_BLOCKED: Command violates policy '{}' (Severity: {:?})",
                        rule.description, rule.severity
                    ));
                }
            }
        }
        Ok(())
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
                            return Err(fault.to_error());
                        }
                    }
                    GuardrailFault::CorruptedRule => {
                        return Err(GuardrailError::CorruptedPolicy(
                            "Policy rule pattern corruption detected".to_string(),
                        ));
                    }
                }
            } else {
                match self.check_safety_internal(command) {
                    Ok(()) => {
                        self.record_seen(command);
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
                            let err = fault.to_error();
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
                match self.check_safety_internal(command) {
                    Ok(()) => {
                        self.record_seen(command);
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

    pub fn evaluate_batch(&mut self, commands: &[&str]) -> Vec<Result<(), GuardrailError>> {
        let mut results = Vec::with_capacity(commands.len());
        for cmd in commands {
            results.push(self.check_command_safety(cmd));
        }
        results
    }
}
