use crate::audit_ledger::{AuditEventType, AuditLedger};
use crate::guardrail::ActionSeverity;
use crate::identity::fault::{IdentityError, IdentityFault, IdentityFaultPolicy};
use crate::identity::types::{AgentId, AgentPersonaProfile};
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone)]
pub struct ResilientIdentityManager {
    fault_policy: IdentityFaultPolicy,
    max_retries: usize,
    resolved_count: usize,
    recovered_count: usize,
    rejected_count: usize,
    resolved_cache: HashMap<AgentId, AgentPersonaProfile>,
    identity_queue: VecDeque<AgentId>,
    max_capacity: Option<usize>,
}

impl ResilientIdentityManager {
    pub fn new(fault_policy: IdentityFaultPolicy, max_retries: usize) -> Self {
        Self {
            fault_policy,
            max_retries,
            resolved_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            resolved_cache: HashMap::new(),
            identity_queue: VecDeque::new(),
            max_capacity: None,
        }
    }

    pub fn with_fault_policy(fault_policy: IdentityFaultPolicy) -> Self {
        Self::new(fault_policy, 3)
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.resolved_cache.reserve(capacity);
        self.identity_queue.reserve(capacity);
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.evict_if_needed();
        self
    }

    pub fn resolved_count(&self) -> usize {
        self.resolved_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn fault_policy(&self) -> &IdentityFaultPolicy {
        &self.fault_policy
    }

    pub fn fault_policy_mut(&mut self) -> &mut IdentityFaultPolicy {
        &mut self.fault_policy
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn reset_counts(&mut self) {
        self.resolved_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let cache_overhead = self.resolved_cache.capacity()
            * (std::mem::size_of::<AgentId>() + std::mem::size_of::<AgentPersonaProfile>());
        let queue_overhead = self.identity_queue.capacity() * std::mem::size_of::<AgentId>();
        let heap_content: usize = self
            .resolved_cache
            .values()
            .map(|p| p.estimated_size_bytes())
            .sum();
        base_size + cache_overhead + queue_overhead + heap_content
    }

    fn evict_if_needed(&mut self) {
        if let Some(max) = self.max_capacity {
            while self.identity_queue.len() >= max && max > 0 {
                if let Some(old_id) = self.identity_queue.pop_front() {
                    self.resolved_cache.remove(&old_id);
                }
            }
        }
    }

    fn cache_identity(&mut self, profile: &AgentPersonaProfile) {
        if let Some(max) = self.max_capacity {
            if max == 0 {
                return;
            }
            while self.identity_queue.len() >= max {
                if let Some(old_id) = self.identity_queue.pop_front() {
                    self.resolved_cache.remove(&old_id);
                }
            }
            self.identity_queue.push_back(profile.identity.id.clone());
            self.resolved_cache
                .insert(profile.identity.id.clone(), profile.clone());
        }
    }

    pub fn resolve_identity(
        &mut self,
        profile: AgentPersonaProfile,
    ) -> Result<AgentPersonaProfile, IdentityError> {
        if let Err(err) = profile.validate() {
            self.rejected_count += 1;
            return Err(err);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    IdentityFault::SocketTimeout
                    | IdentityFault::ConnectionReset
                    | IdentityFault::TransientIo
                    | IdentityFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            return match fault {
                                IdentityFault::SocketTimeout => Err(IdentityError::SocketTimeout),
                                IdentityFault::ConnectionReset => {
                                    Err(IdentityError::ConnectionReset)
                                }
                                IdentityFault::TransientIo => Err(IdentityError::TransientIo(
                                    "Simulated transient I/O failure during identity resolution"
                                        .to_string(),
                                )),
                                IdentityFault::ServiceUnavailable => {
                                    Err(IdentityError::ServiceUnavailable(
                                        "Identity service temporarily unavailable".to_string(),
                                    ))
                                }
                                _ => unreachable!(),
                            };
                        }
                    }
                    IdentityFault::CorruptedProfile => {
                        return Err(IdentityError::CorruptedProfile(
                            "Agent persona profile corruption detected".to_string(),
                        ));
                    }
                }
            } else {
                self.cache_identity(&profile);
                self.resolved_count += 1;
                return Ok(profile);
            }
        }
    }

    pub fn resolve_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        profile: AgentPersonaProfile,
    ) -> Result<AgentPersonaProfile, IdentityError> {
        if let Err(err) = profile.validate() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                format!(
                    "Agent identity rejected for agent '{}': {}",
                    profile.identity.id, err
                ),
            );
            return Err(err);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    IdentityFault::SocketTimeout
                    | IdentityFault::ConnectionReset
                    | IdentityFault::TransientIo
                    | IdentityFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            let err = match fault {
                                IdentityFault::SocketTimeout => IdentityError::SocketTimeout,
                                IdentityFault::ConnectionReset => IdentityError::ConnectionReset,
                                IdentityFault::TransientIo => IdentityError::TransientIo(
                                    "Simulated transient I/O failure during identity resolution"
                                        .to_string(),
                                ),
                                IdentityFault::ServiceUnavailable => {
                                    IdentityError::ServiceUnavailable(
                                        "Identity service temporarily unavailable".to_string(),
                                    )
                                }
                                _ => unreachable!(),
                            };
                            ledger.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "Identity resolution exception for agent '{}': {}",
                                    profile.identity.id, err
                                ),
                            );
                            return Err(err);
                        }
                    }
                    IdentityFault::CorruptedProfile => {
                        let err = IdentityError::CorruptedProfile(
                            "Agent persona profile corruption detected".to_string(),
                        );
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Identity resolution exception for agent '{}': {}",
                                profile.identity.id, err
                            ),
                        );
                        return Err(err);
                    }
                }
            } else {
                self.cache_identity(&profile);
                self.resolved_count += 1;
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Identity resolved for agent '{}'", profile.identity.id),
                );
                return Ok(profile);
            }
        }
    }

    pub fn resolve_batch(
        &mut self,
        profiles: &[AgentPersonaProfile],
    ) -> Vec<Result<AgentPersonaProfile, IdentityError>> {
        let mut results = Vec::with_capacity(profiles.len());
        for p in profiles {
            results.push(self.resolve_identity(p.clone()));
        }
        results
    }
}
