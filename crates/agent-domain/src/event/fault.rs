use serde::{Deserialize, Serialize};

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

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }

    pub fn invocation_count(&self) -> u32 {
        self.invocation_count
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
