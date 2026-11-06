use crate::bus::fault::BusError;
use agent_domain::InterAgentEvent;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Validates an `InterAgentEvent` before it enters the bus.
/// Ensures standard invariants such as non-empty event ID.
pub fn validate_bus_event(event: &InterAgentEvent) -> Result<(), BusError> {
    if event.id.trim().is_empty() {
        return Err(BusError::CorruptedEvent(
            "Event ID cannot be empty".to_string(),
        ));
    }
    Ok(())
}

/// Computes the destination shard index for an event using a consistent hash.
/// Hashes to_agent if present, otherwise topic/event_type, or from_agent.
pub fn calculate_event_shard(event: &InterAgentEvent, num_shards: usize) -> usize {
    if num_shards <= 1 {
        return 0;
    }
    let mut hasher = DefaultHasher::new();
    if let Some(target) = &event.to_agent {
        target.hash(&mut hasher);
    } else if !event.event_type.is_empty() {
        event.event_type.hash(&mut hasher);
    } else {
        event.from_agent.hash(&mut hasher);
    }
    (hasher.finish() as usize) % num_shards
}

/// Predicate filter for inter-agent events, enabling topic-based and agent-based subscriptions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BusEventFilter {
    pub topic_prefix: Option<String>,
    pub from_agent: Option<String>,
    pub to_agent: Option<String>,
}

impl BusEventFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_topic_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.topic_prefix = Some(prefix.into());
        self
    }

    pub fn with_from_agent(mut self, from: impl Into<String>) -> Self {
        self.from_agent = Some(from.into());
        self
    }

    pub fn with_to_agent(mut self, to: impl Into<String>) -> Self {
        self.to_agent = Some(to.into());
        self
    }

    /// Evaluates if an event matches this filter.
    pub fn matches(&self, event: &InterAgentEvent) -> bool {
        if let Some(prefix) = &self.topic_prefix {
            if !event.event_type.starts_with(prefix) {
                return false;
            }
        }
        if let Some(from) = &self.from_agent {
            if &event.from_agent.0 != from {
                return false;
            }
        }
        if let Some(to) = &self.to_agent {
            match &event.to_agent {
                Some(t) => {
                    if &t.0 != to {
                        return false;
                    }
                }
                None => return false,
            }
        }
        true
    }
}
