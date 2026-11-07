use crate::audit_ledger::{AuditEventType, AuditLedger};
use crate::event::fault::{EventError, EventFault, EventFaultPolicy};
use crate::event::InterAgentEvent;
use crate::guardrail::ActionSeverity;
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct ResilientEventProcessor {
    policy: EventFaultPolicy,
    max_retries: usize,
    seen_ids: HashSet<String>,
    id_queue: VecDeque<String>,
    max_capacity: Option<usize>,
    processed_count: usize,
    recovered_count: usize,
    dropped_count: usize,
    rejected_count: usize,
}

impl ResilientEventProcessor {
    pub fn new(policy: EventFaultPolicy, max_retries: usize) -> Self {
        Self {
            policy,
            max_retries,
            seen_ids: HashSet::new(),
            id_queue: VecDeque::new(),
            max_capacity: None,
            processed_count: 0,
            recovered_count: 0,
            dropped_count: 0,
            rejected_count: 0,
        }
    }

    pub fn with_policy(policy: EventFaultPolicy) -> Self {
        Self::new(policy, 3)
    }
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.seen_ids.reserve(capacity);
        self
    }
    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.seen_ids.reserve(max_capacity);
        self.id_queue.reserve(max_capacity);
        self
    }

    pub fn policy(&self) -> &EventFaultPolicy {
        &self.policy
    }
    pub fn policy_mut(&mut self) -> &mut EventFaultPolicy {
        &mut self.policy
    }
    pub fn max_retries(&self) -> usize {
        self.max_retries
    }
    pub fn set_max_retries(&mut self, max_retries: usize) {
        self.max_retries = max_retries;
    }
    pub fn seen_ids(&self) -> &HashSet<String> {
        &self.seen_ids
    }
    pub fn clear_seen_ids(&mut self) {
        self.seen_ids.clear();
        self.id_queue.clear();
    }
    pub fn reset_counts(&mut self) {
        self.processed_count = 0;
        self.recovered_count = 0;
        self.dropped_count = 0;
        self.rejected_count = 0;
    }
    pub fn processed_count(&self) -> usize {
        self.processed_count
    }
    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }
    pub fn dropped_count(&self) -> usize {
        self.dropped_count
    }
    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let set_mem = self.seen_ids.capacity() * (std::mem::size_of::<String>() + 36);
        let queue_mem = self.id_queue.capacity() * (std::mem::size_of::<String>() + 36);
        std::mem::size_of::<Self>() + set_mem + queue_mem
    }

    #[inline]
    fn record_seen(&mut self, id: String) {
        if let Some(max_cap) = self.max_capacity {
            if self.seen_ids.len() >= max_cap {
                if let Some(oldest) = self.id_queue.pop_front() {
                    self.seen_ids.remove(&oldest);
                }
            }
            self.id_queue.push_back(id.clone());
        }
        self.seen_ids.insert(id);
    }

    pub fn process_event(
        &mut self,
        event: InterAgentEvent,
    ) -> Result<Option<InterAgentEvent>, EventError> {
        if let Err(err) = event.validate() {
            self.rejected_count += 1;
            return Err(err);
        }
        if self.seen_ids.contains(&event.id) {
            return Ok(None);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.policy.should_inject() {
                Some(EventFault::Dropped) => {
                    self.dropped_count += 1;
                    return Ok(None);
                }
                Some(EventFault::Duplicate) => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    return Ok(Some(event));
                }
                Some(EventFault::CorruptedPayload) => {
                    self.rejected_count += 1;
                    return Err(EventError::CorruptedPayload(
                        "Packet payload checksum failure or corrupted JSON".to_string(),
                    ));
                }
                Some(fault) => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                    } else {
                        return Err(fault.to_error());
                    }
                }
                None => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    return Ok(Some(event));
                }
            }
        }
    }

    pub fn process_batch(
        &mut self,
        events: Vec<InterAgentEvent>,
    ) -> Result<Vec<InterAgentEvent>, EventError> {
        let mut processed = Vec::with_capacity(events.len());
        for event in events {
            if let Some(ev) = self.process_event(event)? {
                processed.push(ev);
            }
        }
        Ok(processed)
    }

    pub fn process_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        event: InterAgentEvent,
    ) -> Result<Option<InterAgentEvent>, EventError> {
        if let Err(err) = event.validate() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                format!(
                    "Event '{}' rejected due to invalid structure: {}",
                    event.id, err
                ),
            );
            return Err(err);
        }
        if self.seen_ids.contains(&event.id) {
            ledger.record_event(
                AuditEventType::CommandAllowed,
                ActionSeverity::Routine,
                format!("Event '{}' deduplicated and discarded", event.id),
            );
            return Ok(None);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.policy.should_inject() {
                Some(EventFault::Dropped) => {
                    self.dropped_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandBlocked,
                        ActionSeverity::Destructive,
                        format!("Event '{}' dropped in transit during chaos test", event.id),
                    );
                    return Ok(None);
                }
                Some(EventFault::Duplicate) => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!(
                            "Event '{}' processed under duplicate fault injection",
                            event.id
                        ),
                    );
                    return Ok(Some(event));
                }
                Some(EventFault::CorruptedPayload) => {
                    self.rejected_count += 1;
                    let err = EventError::CorruptedPayload(
                        "Packet payload checksum failure or corrupted JSON".to_string(),
                    );
                    ledger.record_event(
                        AuditEventType::CommandException,
                        ActionSeverity::HighImpact,
                        format!(
                            "Event '{}' rejected due to corrupted payload: {}",
                            event.id, err
                        ),
                    );
                    return Err(err);
                }
                Some(fault) => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                    } else {
                        let err = fault.to_error();
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Event '{}' failed after {} retries: {}",
                                event.id, self.max_retries, err
                            ),
                        );
                        return Err(err);
                    }
                }
                None => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!("Event '{}' processed successfully", event.id),
                    );
                    return Ok(Some(event));
                }
            }
        }
    }
}
