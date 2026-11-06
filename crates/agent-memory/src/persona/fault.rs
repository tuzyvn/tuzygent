use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersonaError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parsing error: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Socket timeout while accessing persona profile store")]
    SocketTimeout,
    #[error("Connection reset by peer during persona profile synchronization")]
    ConnectionReset,
    #[error("Transient I/O failure: {0}")]
    TransientIo(String),
    #[error("Corrupted persona profile: {0}")]
    CorruptedProfile(String),
    #[error("Invalid persona identity: {0}")]
    InvalidIdentity(String),
    #[error("Persona file lock failure")]
    LockFailure,
    #[error("Max retry attempts ({0}) exceeded during persona operation")]
    MaxRetriesExceeded(usize),
    #[error("Persona directory not found: {0}")]
    NotFound(String),
    #[error("Persona error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersonaFault {
    SocketTimeout,
    ConnectionReset,
    TransientIo,
    CorruptedProfile,
    InvalidIdentity,
    LockFailure,
}

impl PersonaFault {
    pub fn to_error(&self) -> PersonaError {
        match self {
            Self::SocketTimeout => PersonaError::SocketTimeout,
            Self::ConnectionReset => PersonaError::ConnectionReset,
            Self::TransientIo => PersonaError::TransientIo(
                "Simulated transient I/O failure while accessing persona profile".to_string(),
            ),
            Self::CorruptedProfile => PersonaError::CorruptedProfile(
                "Simulated corrupted persona profile file or invalid format".to_string(),
            ),
            Self::InvalidIdentity => PersonaError::InvalidIdentity(
                "Simulated invalid or unauthenticated persona identity".to_string(),
            ),
            Self::LockFailure => PersonaError::LockFailure,
        }
    }

    pub fn simulate_fault(&self) -> Result<(), PersonaError> {
        Err(self.to_error())
    }
}

pub fn simulate_persona_fault(fault: &PersonaFault) -> Result<(), PersonaError> {
    fault.simulate_fault()
}
