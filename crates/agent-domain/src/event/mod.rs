mod benchmark;
pub mod fault;
pub mod resilience;

pub use benchmark::*;
pub use fault::*;
pub use resilience::*;

use crate::identity::AgentId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
