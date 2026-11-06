use crate::context_builder::builder::ContextBuilder;
use crate::context_builder::fault::{ContextBuilderError, ContextBuilderFault};
use crate::context_builder::policy::ContextBuilderFaultPolicy;
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct ResilientContextBuilder {
    builder: ContextBuilder,
    policy: ContextBuilderFaultPolicy,
    max_retries: usize,
    built_count: usize,
    recovered_count: usize,
    rejected_count: usize,
    seen_contexts: HashSet<String>,
    context_queue: VecDeque<String>,
    max_capacity: Option<usize>,
}

impl ResilientContextBuilder {
    pub fn new(
        builder: ContextBuilder,
        policy: ContextBuilderFaultPolicy,
        max_retries: usize,
    ) -> Self {
        Self {
            builder,
            policy,
            max_retries,
            built_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            seen_contexts: HashSet::new(),
            context_queue: VecDeque::new(),
            max_capacity: None,
        }
    }

    pub fn with_policy(builder: ContextBuilder, policy: ContextBuilderFaultPolicy) -> Self {
        Self::new(builder, policy, 3)
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.seen_contexts.reserve(capacity);
        self.context_queue.reserve(capacity);
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.seen_contexts.reserve(max_capacity);
        self.context_queue.reserve(max_capacity);
        self
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
        self.seen_contexts.clear();
        self.context_queue.clear();
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let set_mem = self.seen_contexts.capacity() * (std::mem::size_of::<String>() + 40);
        let queue_mem = self.context_queue.capacity() * (std::mem::size_of::<String>() + 40);
        std::mem::size_of::<Self>() + set_mem + queue_mem
    }

    fn record_built(&mut self, goal: &str) {
        if let Some(max_cap) = self.max_capacity {
            if self.context_queue.len() >= max_cap {
                if let Some(oldest) = self.context_queue.pop_front() {
                    self.seen_contexts.remove(&oldest);
                }
            }
            self.context_queue.push_back(goal.to_string());
        }
        self.seen_contexts.insert(goal.to_string());
    }

    pub fn build_with(
        &mut self,
        builder: ContextBuilder,
    ) -> Result<(String, String), ContextBuilderError> {
        if let Err(e) = builder.validate() {
            self.rejected_count += 1;
            return Err(e);
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
                            max_budget: builder.max_total_chars(),
                        });
                    }
                }
            } else {
                self.built_count += 1;
                self.record_built(builder.goal());
                return Ok(builder.build());
            }
        }
    }

    pub fn build(&mut self) -> Result<(String, String), ContextBuilderError> {
        let b = self.builder.clone();
        self.build_with(b)
    }

    pub fn build_batch(
        &mut self,
        builders: &[ContextBuilder],
    ) -> Vec<Result<(String, String), ContextBuilderError>> {
        let mut results = Vec::with_capacity(builders.len());
        for b in builders {
            results.push(self.build_with(b.clone()));
        }
        results
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
                            max_budget: self.builder.max_total_chars(),
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
                let goal = self.builder.goal().to_string();
                self.record_built(&goal);
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Context successfully built for goal: '{}'", goal),
                );
                return Ok(self.builder.clone().build());
            }
        }
    }
}
