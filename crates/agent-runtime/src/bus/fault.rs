use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BusError {
    #[error("Socket timeout while publishing event to inter-agent bus")]
    SocketTimeout,
    #[error("Connection reset by peer during bus communication")]
    ConnectionReset,
    #[error("Transient I/O failure on inter-agent bus: {0}")]
    TransientIo(String),
    #[error("Bus broadcast channel is closed")]
    ChannelClosed,
    #[error("Bus broadcast channel is full / lagging: {0}")]
    ChannelFull(String),
    #[error("Corrupted event detected on bus: {0}")]
    CorruptedEvent(String),
    #[error("Inter-agent bus service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Max retry attempts ({0}) exceeded during bus event dispatch")]
    MaxRetriesExceeded(usize),
    #[error("Bus capacity overload: {0}")]
    CapacityOverload(String),
    #[error("Bus operation panicked: {0}")]
    Panic(String),
    #[error("Bus receiver lagged and missed {0} messages")]
    ReceiverLagged(u64),
    #[error("Inter-agent bus error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusFault {
    SocketTimeout,
    ConnectionReset,
    TransientIo,
    ChannelClosed,
    ChannelFull,
    CapacityOverload,
    CorruptedEvent,
    ServiceUnavailable,
    Panic,
}

impl BusFault {
    pub fn to_error(&self) -> BusError {
        match self {
            Self::SocketTimeout => BusError::SocketTimeout,
            Self::ConnectionReset => BusError::ConnectionReset,
            Self::TransientIo => BusError::TransientIo(
                "Simulated transient I/O failure on inter-agent event bus".to_string(),
            ),
            Self::ChannelClosed => BusError::ChannelClosed,
            Self::ChannelFull => {
                BusError::ChannelFull("Simulated broadcast channel capacity saturation".to_string())
            }
            Self::CapacityOverload => BusError::CapacityOverload(
                "Simulated capacity overload threshold exceeded on bus".to_string(),
            ),
            Self::CorruptedEvent => BusError::CorruptedEvent(
                "Simulated malformed or unparseable event packet".to_string(),
            ),
            Self::ServiceUnavailable => {
                BusError::ServiceUnavailable("Simulated temporary bus backend downtime".to_string())
            }
            Self::Panic => {
                BusError::Panic("Simulated unexpected panic during bus dispatch".to_string())
            }
        }
    }

    pub fn simulate_fault(&self) -> Result<(), BusError> {
        Err(self.to_error())
    }
}

pub fn simulate_bus_fault(fault: &BusFault) -> Result<(), BusError> {
    fault.simulate_fault()
}
