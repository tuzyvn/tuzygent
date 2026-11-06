use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HarnessError {
    #[error("Socket timeout while accessing harness store")]
    SocketTimeout,
    #[error("Connection reset by peer during harness operation")]
    ConnectionReset,
    #[error("Transient I/O failure in harness memory: {0}")]
    TransientIo(String),
    #[error("Corrupted harness memory or invalid json payload: {0}")]
    CorruptedState(String),
    #[error("Harness memory lock acquisition failed or contention: {0}")]
    LockFailure(String),
    #[error("Invalid or empty harness parameter: {0}")]
    InvalidParameter(String),
    #[error("Max retry attempts ({0}) exceeded during harness operation")]
    MaxRetriesExceeded(usize),
    #[error("Harness operation panicked: {0}")]
    Panic(String),
    #[error("Harness memory error: {0}")]
    Other(String),
}

impl From<std::io::Error> for HarnessError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::TimedOut => HarnessError::SocketTimeout,
            std::io::ErrorKind::ConnectionReset => HarnessError::ConnectionReset,
            std::io::ErrorKind::InvalidData => HarnessError::CorruptedState(err.to_string()),
            _ => HarnessError::TransientIo(err.to_string()),
        }
    }
}

impl From<anyhow::Error> for HarnessError {
    fn from(err: anyhow::Error) -> Self {
        let msg = err.to_string();
        if msg.contains("Invalid harness lessons") || msg.contains("Lỗi serialize") {
            HarnessError::CorruptedState(msg)
        } else if msg.contains("timed out") || msg.contains("timeout") {
            HarnessError::SocketTimeout
        } else if msg.contains("connection reset") {
            HarnessError::ConnectionReset
        } else if msg.contains("lock") {
            HarnessError::LockFailure(msg)
        } else {
            HarnessError::Other(msg)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessFault {
    SocketTimeout,
    ConnectionReset,
    TransientIo,
    CorruptedState,
    LockFailure,
    Panic,
}

impl HarnessFault {
    pub fn to_error(&self) -> HarnessError {
        match self {
            Self::SocketTimeout => HarnessError::SocketTimeout,
            Self::ConnectionReset => HarnessError::ConnectionReset,
            Self::TransientIo => HarnessError::TransientIo(
                "Simulated transient I/O failure during harness operation".to_string(),
            ),
            Self::CorruptedState => HarnessError::CorruptedState(
                "Simulated corrupted or unparseable harness memory structure".to_string(),
            ),
            Self::LockFailure => HarnessError::LockFailure(
                "Simulated lock acquisition failure due to contention".to_string(),
            ),
            Self::Panic => HarnessError::Panic(
                "Simulated unexpected panic during harness operation".to_string(),
            ),
        }
    }

    pub fn is_transient(&self) -> bool {
        match self {
            Self::SocketTimeout
            | Self::ConnectionReset
            | Self::TransientIo
            | Self::LockFailure
            | Self::Panic => true,
            Self::CorruptedState => false,
        }
    }

    pub fn simulate_fault(&self) -> Result<(), HarnessError> {
        Err(self.to_error())
    }
}

pub fn simulate_harness_fault(fault: &HarnessFault) -> Result<(), HarnessError> {
    fault.simulate_fault()
}

pub fn is_retryable_harness_error(err: &HarnessError) -> bool {
    match err {
        HarnessError::SocketTimeout
        | HarnessError::ConnectionReset
        | HarnessError::TransientIo(_)
        | HarnessError::LockFailure(_)
        | HarnessError::Panic(_) => true,
        HarnessError::CorruptedState(_)
        | HarnessError::InvalidParameter(_)
        | HarnessError::MaxRetriesExceeded(_)
        | HarnessError::Other(_) => false,
    }
}

#[derive(Debug, Clone, Default)]
pub struct HarnessFaultPolicy {
    fault: Option<HarnessFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl HarnessFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: HarnessFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: HarnessFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<HarnessFault> {
        let fault = self.fault.as_ref()?;
        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.wrapping_add(1);
        let inject = match self.fail_interval {
            Some(interval) if interval > 0 => (self.invocation_count - 1).is_multiple_of(interval),
            _ => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn invocation_count(&self) -> u32 {
        self.invocation_count
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }
}
