use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DurableMemoryError {
    #[error("Socket timeout communicating with durable storage")]
    SocketTimeout,
    #[error("Connection reset by peer during memory operation")]
    ConnectionReset,
    #[error("Transient I/O failure during durable memory operation: {0}")]
    TransientIo(String),
    #[error("Corrupted durable state or malformed memory structure: {0}")]
    CorruptedState(String),
    #[error("Lock acquisition failed or file lock contention: {0}")]
    LockFailure(String),
    #[error("Content cannot be empty or blank")]
    EmptyContent,
    #[error("Max retry attempts ({0}) exceeded during durable memory operation")]
    MaxRetriesExceeded(usize),
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<std::io::Error> for DurableMemoryError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::TimedOut => DurableMemoryError::SocketTimeout,
            std::io::ErrorKind::ConnectionReset => DurableMemoryError::ConnectionReset,
            std::io::ErrorKind::InvalidData => DurableMemoryError::CorruptedState(err.to_string()),
            _ => DurableMemoryError::Io(err.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurableMemoryFault {
    SocketTimeout,
    ConnectionReset,
    TransientIo,
    CorruptedState,
    LockFailure,
}

impl DurableMemoryFault {
    pub fn to_error(&self) -> DurableMemoryError {
        match self {
            Self::SocketTimeout => DurableMemoryError::SocketTimeout,
            Self::ConnectionReset => DurableMemoryError::ConnectionReset,
            Self::TransientIo => DurableMemoryError::TransientIo(
                "Simulated transient I/O failure during durable memory operation".to_string(),
            ),
            Self::CorruptedState => DurableMemoryError::CorruptedState(
                "Simulated corrupted or unparseable durable memory structure".to_string(),
            ),
            Self::LockFailure => DurableMemoryError::LockFailure(
                "Simulated lock acquisition failure due to contention".to_string(),
            ),
        }
    }

    pub fn simulate_fault(&self) -> Result<(), DurableMemoryError> {
        Err(self.to_error())
    }
}

pub fn simulate_durable_fault(fault: &DurableMemoryFault) -> Result<(), DurableMemoryError> {
    fault.simulate_fault()
}
