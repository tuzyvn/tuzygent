use crate::audit_ledger::{AuditEventType, AuditLedger};
use crate::guardrail::ActionSeverity;
use crate::identity::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum EventError {
    #[error("Simulated socket timeout")]
    SocketTimeout,
    #[error("Connection reset by peer")]
    ConnectionReset,
    #[error("Event payload corrupted or invalid: {0}")]
    CorruptedPayload(String),
    #[error("Event dropped in transit")]
    Dropped,
    #[error("Invalid event structure: {0}")]
    InvalidStructure(String),
    #[error("Transient I/O failure during event transport: {0}")]
    TransientIo(String),
    #[error("Event transport service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Max retry attempts ({0}) exceeded during recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventFault {
    SocketTimeout,
    ConnectionReset,
    CorruptedPayload,
    Dropped,
    Duplicate,
    TransientIo,
    ServiceUnavailable,
}

impl EventFault {
    pub fn to_error(&self) -> EventError {
        match self {
            Self::SocketTimeout => EventError::SocketTimeout,
            Self::ConnectionReset => EventError::ConnectionReset,
            Self::TransientIo => EventError::TransientIo(
                "Simulated transient I/O failure during event transport".to_string(),
            ),
            Self::ServiceUnavailable => EventError::ServiceUnavailable(
                "Event delivery service temporarily unavailable".to_string(),
            ),
            Self::CorruptedPayload => EventError::CorruptedPayload(
                "Packet payload checksum failure or corrupted JSON".to_string(),
            ),
            Self::Dropped => EventError::Dropped,
            Self::Duplicate => {
                EventError::InvalidStructure("Unexpected duplicate fault converted to error".to_string())
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EventFaultPolicy {
    fault: Option<EventFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl EventFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: EventFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: EventFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<EventFault> {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterAgentEvent {
    pub specversion: String,
    pub id: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub time: chrono::DateTime<chrono::Utc>,
    pub from_agent: AgentId,
    pub to_agent: Option<AgentId>,
    pub payload: serde_json::Value,
}

impl InterAgentEvent {
    pub fn new(
        from: AgentId,
        to: Option<AgentId>,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            specversion: "1.0".to_string(),
            id: format!("EVT-{}", Uuid::new_v4().simple()),
            source: format!("urn:agent:{}", from),
            event_type: event_type.to_string(),
            time: chrono::Utc::now(),
            from_agent: from,
            to_agent: to,
            payload,
        }
    }

    pub fn validate(&self) -> Result<(), EventError> {
        if self.specversion != "1.0" {
            return Err(EventError::InvalidStructure(format!(
                "Unsupported specversion: '{}', expected '1.0'",
                self.specversion
            )));
        }
        if !self.id.starts_with("EVT-") || self.id.trim() == "EVT-" {
            return Err(EventError::InvalidStructure(format!(
                "Invalid event id format: '{}', expected 'EVT-*'",
                self.id
            )));
        }
        if !self.source.starts_with("urn:agent:") || self.source.trim() == "urn:agent:" {
            return Err(EventError::InvalidStructure(format!(
                "Invalid source format: '{}', expected 'urn:agent:*'",
                self.source
            )));
        }
        if self.event_type.trim().is_empty() {
            return Err(EventError::InvalidStructure(
                "Event type cannot be empty".to_string(),
            ));
        }
        Ok(())
    }

    pub fn simulate_fault(&self, fault: &EventFault) -> Result<Self, EventError> {
        match fault {
            EventFault::SocketTimeout => Err(EventError::SocketTimeout),
            EventFault::ConnectionReset => Err(EventError::ConnectionReset),
            EventFault::Dropped => Err(EventError::Dropped),
            EventFault::CorruptedPayload => Err(EventError::CorruptedPayload(
                "Injected payload corruption".to_string(),
            )),
            EventFault::Duplicate => Ok(self.clone()),
            EventFault::TransientIo => Err(EventError::TransientIo(
                "Simulated transient I/O failure during event transport".to_string(),
            )),
            EventFault::ServiceUnavailable => Err(EventError::ServiceUnavailable(
                "Event delivery service temporarily unavailable".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResilientEventProcessor {
    policy: EventFaultPolicy,
    max_retries: usize,
    seen_ids: HashSet<String>,
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
            processed_count: 0,
            recovered_count: 0,
            dropped_count: 0,
            rejected_count: 0,
        }
    }

    pub fn with_policy(policy: EventFaultPolicy) -> Self {
        Self::new(policy, 3)
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
            if let Some(fault) = self.policy.should_inject() {
                match fault {
                    EventFault::Dropped => {
                        self.dropped_count += 1;
                        return Ok(None);
                    }
                    EventFault::Duplicate => {
                        self.seen_ids.insert(event.id.clone());
                        self.processed_count += 1;
                        return Ok(Some(event));
                    }
                    EventFault::SocketTimeout
                    | EventFault::ConnectionReset
                    | EventFault::TransientIo
                    | EventFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            return Err(fault.to_error());
                        }
                    }
                    EventFault::CorruptedPayload => {
                        self.rejected_count += 1;
                        return Err(EventError::CorruptedPayload(
                            "Packet payload checksum failure or corrupted JSON".to_string(),
                        ));
                    }
                }
            } else {
                self.seen_ids.insert(event.id.clone());
                self.processed_count += 1;
                return Ok(Some(event));
            }
        }
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
            if let Some(fault) = self.policy.should_inject() {
                match fault {
                    EventFault::Dropped => {
                        self.dropped_count += 1;
                        ledger.record_event(
                            AuditEventType::CommandBlocked,
                            ActionSeverity::Destructive,
                            format!("Event '{}' dropped in transit during chaos test", event.id),
                        );
                        return Ok(None);
                    }
                    EventFault::Duplicate => {
                        self.seen_ids.insert(event.id.clone());
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
                    EventFault::SocketTimeout
                    | EventFault::ConnectionReset
                    | EventFault::TransientIo
                    | EventFault::ServiceUnavailable => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
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
                    EventFault::CorruptedPayload => {
                        self.rejected_count += 1;
                        let err = EventError::CorruptedPayload(
                            "Packet payload checksum failure or corrupted JSON".to_string(),
                        );
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!("Event '{}' rejected due to corrupted payload: {}", event.id, err),
                        );
                        return Err(err);
                    }
                }
            } else {
                self.seen_ids.insert(event.id.clone());
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
