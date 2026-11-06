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
