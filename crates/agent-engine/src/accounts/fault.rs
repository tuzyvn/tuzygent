use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum AccountError {
    #[error("Socket timeout communicating with account service")]
    SocketTimeout,
    #[error("Connection reset by peer")]
    ConnectionReset,
    #[error("Rate limit or quota exceeded: {0}")]
    RateLimitExceeded(String),
    #[error("Transient I/O failure during account storage operation: {0}")]
    TransientIo(String),
    #[error("Corrupted account metadata detected: {0}")]
    CorruptedMetadata(String),
    #[error("Account service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Invalid account configuration: {0}")]
    InvalidAccount(String),
    #[error("No available accounts for engine kind: {0}")]
    NoAvailableAccount(String),
    #[error("Max retry attempts ({0}) exceeded during account recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountFault {
    SocketTimeout,
    ConnectionReset,
    RateLimitExceeded,
    TransientIo,
    CorruptedMetadata,
    ServiceUnavailable,
}

#[derive(Debug, Clone, Default)]
pub struct AccountFaultPolicy {
    fault: Option<AccountFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl AccountFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: AccountFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: AccountFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<AccountFault> {
        let fault = self.fault.as_ref()?;

        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.wrapping_add(1);

        let inject = match self.fail_interval {
            Some(interval) => (self.invocation_count - 1).is_multiple_of(interval),
            None => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }
}
