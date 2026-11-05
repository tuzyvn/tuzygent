use crate::skills::SkillMetadata;
use agent_domain::{ActionSeverity, AgentPersonaProfile, AuditEventType, AuditLedger};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ContextBuilderError {
    #[error("Socket timeout communicating with external context provider")]
    SocketTimeout,
    #[error("Connection reset by peer during context retrieval")]
    ConnectionReset,
    #[error("Transient I/O failure during context assembly: {0}")]
    TransientIo(String),
    #[error("Corrupted context payload or invalid token structure: {0}")]
    CorruptedContext(String),
    #[error("Context character budget exceeded: required {required}, max {max_budget}")]
    BudgetExceeded { required: usize, max_budget: usize },
    #[error("Goal cannot be empty or blank")]
    EmptyGoal,
    #[error("Max retry attempts ({0}) exceeded during context recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextBuilderFault {
    SocketTimeout,
    ConnectionReset,
    TransientIo,
    CorruptedContext,
    BudgetExceeded,
}

impl ContextBuilderFault {
    pub fn to_error(&self) -> ContextBuilderError {
        match self {
            Self::SocketTimeout => ContextBuilderError::SocketTimeout,
            Self::ConnectionReset => ContextBuilderError::ConnectionReset,
            Self::TransientIo => ContextBuilderError::TransientIo(
                "Simulated transient I/O failure while assembling memory context".to_string(),
            ),
            Self::CorruptedContext => ContextBuilderError::CorruptedContext(
                "Simulated corrupt or unparseable context fragment".to_string(),
            ),
            Self::BudgetExceeded => ContextBuilderError::BudgetExceeded {
                required: 50000,
                max_budget: 16000,
            },
        }
    }

    pub fn simulate_fault(&self) -> Result<(), ContextBuilderError> {
        Err(self.to_error())
    }
}

pub fn simulate_context_fault(fault: &ContextBuilderFault) -> Result<(), ContextBuilderError> {
    fault.simulate_fault()
}

#[derive(Debug, Clone, Default)]
pub struct ContextBuilderFaultPolicy {
    fault: Option<ContextBuilderFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl ContextBuilderFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: ContextBuilderFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: ContextBuilderFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<ContextBuilderFault> {
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

    pub fn invocation_count(&self) -> u32 {
        self.invocation_count
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }
}

/// ContextBuilder unifies and structures execution context across
/// Persona, Goals, Skills, Lessons, Recent Scratchpad Logs, Subtask History, and Artifacts.
/// It enforces explicit character limits per component to prevent prompt bloat.
#[derive(Debug, Clone)]
pub struct ContextBuilder {
    goal: String,
    persona: Option<AgentPersonaProfile>,
    skills: Vec<SkillMetadata>,
    lessons: Vec<String>,
    recent_logs: Vec<String>,
    subtask_history: Vec<(String, String)>, // (subtask_id, excerpt)
    artifacts: Vec<String>,
    max_total_chars: usize,
}

impl ContextBuilder {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            persona: None,
            skills: Vec::new(),
            lessons: Vec::new(),
            recent_logs: Vec::new(),
            subtask_history: Vec::new(),
            artifacts: Vec::new(),
            max_total_chars: 16000,
        }
    }

    pub fn goal(&self) -> &str {
        &self.goal
    }

    pub fn persona(&self) -> Option<&AgentPersonaProfile> {
        self.persona.as_ref()
    }

    pub fn skills(&self) -> &[SkillMetadata] {
        &self.skills
    }

    pub fn lessons(&self) -> &[String] {
        &self.lessons
    }

    pub fn recent_logs(&self) -> &[String] {
        &self.recent_logs
    }

    pub fn subtask_history(&self) -> &[(String, String)] {
        &self.subtask_history
    }

    pub fn artifacts(&self) -> &[String] {
        &self.artifacts
    }

    pub fn max_total_chars(&self) -> usize {
        self.max_total_chars
    }

    pub fn with_persona(mut self, persona: Option<AgentPersonaProfile>) -> Self {
        self.persona = persona;
        self
    }

    pub fn with_skills(mut self, skills: Vec<SkillMetadata>) -> Self {
        self.skills = skills;
        self
    }

    pub fn with_lessons(mut self, lessons: Vec<String>) -> Self {
        self.lessons = lessons;
        self
    }

    pub fn with_recent_logs(mut self, logs: Vec<String>) -> Self {
        self.recent_logs = logs;
        self
    }

    pub fn with_subtask_history(mut self, history: Vec<(String, String)>) -> Self {
        self.subtask_history = history;
        self
    }

    pub fn add_subtask_step(mut self, id: String, excerpt: String) -> Self {
        self.subtask_history.push((id, excerpt));
        self
    }

    pub fn with_artifacts(mut self, artifacts: Vec<String>) -> Self {
        self.artifacts = artifacts;
        self
    }

    pub fn with_max_total_chars(mut self, max_chars: usize) -> Self {
        self.max_total_chars = max_chars;
        self
    }

    pub fn validate(&self) -> Result<(), ContextBuilderError> {
        if self.goal.trim().is_empty() {
            return Err(ContextBuilderError::EmptyGoal);
        }
        Ok(())
    }

    pub fn build_safe(&self) -> Result<(String, String), ContextBuilderError> {
        self.validate()?;
        Ok(self.clone().build())
    }

    /// Assembles the unified (system_context, working_context)
    pub fn build(self) -> (String, String) {
        let mut sys = String::new();

        // 1. Persona section (up to 2500 chars)
        if let Some(p) = &self.persona {
            sys.push_str("### AGENT IDENTITY & VOICE:\n");
            sys.push_str(&p.build_system_prompt());
            sys.push_str("\n\n");
        }

        // 2. Goal Section
        sys.push_str("### PRIMARY GOAL:\n");
        sys.push_str(&self.goal);
        sys.push_str("\n\n");

        // 3. Relevant Skills Section (up to 2500 chars)
        if !self.skills.is_empty() {
            sys.push_str("### RELEVANT OPERATIONAL SKILLS:\n");
            for skill in &self.skills {
                let desc: String = skill.description.chars().take(200).collect();
                sys.push_str(&format!("- [{}] {}: {}\n", skill.id, skill.name, desc));
            }
            sys.push('\n');
        }

        // 4. Lessons Learned (up to 2500 chars)
        if !self.lessons.is_empty() {
            sys.push_str("### DURABLE LESSONS & GUARDRAILS:\n");
            for l in &self.lessons {
                sys.push_str(l);
                sys.push('\n');
            }
            sys.push('\n');
        }

        // Assemble Working Context
        let mut work = String::new();

        // 5. Recent Scratchpad Logs (up to 1500 chars)
        if !self.recent_logs.is_empty() {
            work.push_str("### RECENT SCRATCHPAD ACTIVITY:\n");
            for log in &self.recent_logs {
                work.push_str(&format!("* {}\n", log));
            }
            work.push('\n');
        }

        // 6. Subtask Execution History (up to 4000 chars)
        if !self.subtask_history.is_empty() {
            work.push_str("### PREVIOUS SUBTASK OUTPUTS:\n");
            for (step_id, output) in &self.subtask_history {
                let trimmed_out: String = output.chars().take(800).collect();
                work.push_str(&format!("#### Step {}:\n{}\n\n", step_id, trimmed_out));
            }
        }

        // 7. Tracked Artifacts
        if !self.artifacts.is_empty() {
            work.push_str("### CREATED ARTIFACTS:\n");
            for art in &self.artifacts {
                work.push_str(&format!("- {}\n", art));
            }
            work.push('\n');
        }

        // Ensure total length complies with limit
        let total = sys.len() + work.len();
        if total > self.max_total_chars {
            let available_work = self.max_total_chars.saturating_sub(sys.len());
            work = work.chars().rev().take(available_work).collect::<String>().chars().rev().collect();
        }

        (sys, work)
    }
}

#[derive(Debug, Clone)]
pub struct ResilientContextBuilder {
    builder: ContextBuilder,
    policy: ContextBuilderFaultPolicy,
    max_retries: usize,
    built_count: usize,
    recovered_count: usize,
    rejected_count: usize,
}

impl ResilientContextBuilder {
    pub fn new(builder: ContextBuilder, policy: ContextBuilderFaultPolicy, max_retries: usize) -> Self {
        Self {
            builder,
            policy,
            max_retries,
            built_count: 0,
            recovered_count: 0,
            rejected_count: 0,
        }
    }

    pub fn with_policy(builder: ContextBuilder, policy: ContextBuilderFaultPolicy) -> Self {
        Self::new(builder, policy, 3)
    }

    pub fn builder(&self) -> &ContextBuilder {
        &self.builder
    }

    pub fn builder_mut(&mut self) -> &mut ContextBuilder {
        &mut self.builder
    }

    pub fn policy(&self) -> &ContextBuilderFaultPolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut ContextBuilderFaultPolicy {
        &mut self.policy
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn built_count(&self) -> usize {
        self.built_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn reset_counts(&mut self) {
        self.built_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
    }

    pub fn build(&mut self) -> Result<(String, String), ContextBuilderError> {
        self.builder.validate()?;

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.policy.should_inject() {
                match fault {
                    ContextBuilderFault::SocketTimeout
                    | ContextBuilderFault::ConnectionReset
                    | ContextBuilderFault::TransientIo => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            return match fault {
                                ContextBuilderFault::SocketTimeout => {
                                    Err(ContextBuilderError::SocketTimeout)
                                }
                                ContextBuilderFault::ConnectionReset => {
                                    Err(ContextBuilderError::ConnectionReset)
                                }
                                ContextBuilderFault::TransientIo => {
                                    Err(ContextBuilderError::TransientIo(
                                        "Simulated transient I/O failure while assembling memory context".to_string(),
                                    ))
                                }
                                _ => unreachable!(),
                            };
                        }
                    }
                    ContextBuilderFault::CorruptedContext => {
                        self.rejected_count += 1;
                        return Err(ContextBuilderError::CorruptedContext(
                            "Simulated corrupt or unparseable context fragment".to_string(),
                        ));
                    }
                    ContextBuilderFault::BudgetExceeded => {
                        self.rejected_count += 1;
                        return Err(ContextBuilderError::BudgetExceeded {
                            required: 50000,
                            max_budget: self.builder.max_total_chars,
                        });
                    }
                }
            } else {
                self.built_count += 1;
                return Ok(self.builder.clone().build());
            }
        }
    }

    pub fn build_and_record(
        &mut self,
        ledger: &mut AuditLedger,
    ) -> Result<(String, String), ContextBuilderError> {
        if let Err(err) = self.builder.validate() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                format!("Context builder rejected invalid goal: {}", err),
            );
            return Err(err);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.policy.should_inject() {
                match fault {
                    ContextBuilderFault::SocketTimeout
                    | ContextBuilderFault::ConnectionReset
                    | ContextBuilderFault::TransientIo => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            let err = match fault {
                                ContextBuilderFault::SocketTimeout => {
                                    ContextBuilderError::SocketTimeout
                                }
                                ContextBuilderFault::ConnectionReset => {
                                    ContextBuilderError::ConnectionReset
                                }
                                ContextBuilderFault::TransientIo => {
                                    ContextBuilderError::TransientIo(
                                        "Simulated transient I/O failure while assembling memory context".to_string(),
                                    )
                                }
                                _ => unreachable!(),
                            };
                            ledger.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!("Context build failure: {}", err),
                            );
                            return Err(err);
                        }
                    }
                    ContextBuilderFault::CorruptedContext => {
                        let err = ContextBuilderError::CorruptedContext(
                            "Simulated corrupt or unparseable context fragment".to_string(),
                        );
                        self.rejected_count += 1;
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!("Context build corruption: {}", err),
                        );
                        return Err(err);
                    }
                    ContextBuilderFault::BudgetExceeded => {
                        let err = ContextBuilderError::BudgetExceeded {
                            required: 50000,
                            max_budget: self.builder.max_total_chars,
                        };
                        self.rejected_count += 1;
                        ledger.record_event(
                            AuditEventType::CommandBlocked,
                            ActionSeverity::Routine,
                            format!("Context budget exceeded: {}", err),
                        );
                        return Err(err);
                    }
                }
            } else {
                self.built_count += 1;
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Context successfully built for goal: '{}'", self.builder.goal),
                );
                return Ok(self.builder.clone().build());
            }
        }
    }
}
