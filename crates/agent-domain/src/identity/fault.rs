use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum IdentityError {
    #[error("Invalid identity structure: {0}")]
    InvalidIdentity(String),
    #[error("Simulated socket timeout during identity resolution")]
    SocketTimeout,
    #[error("Connection reset during identity resolution")]
    ConnectionReset,
    #[error("Transient I/O failure during identity operation: {0}")]
    TransientIo(String),
    #[error("Corrupted agent persona profile: {0}")]
    CorruptedProfile(String),
    #[error("Identity service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Max retry attempts ({0}) exceeded during identity recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityFault {
    SocketTimeout,
    ConnectionReset,
    CorruptedProfile,
    TransientIo,
    ServiceUnavailable,
}

#[derive(Debug, Clone, Default)]
pub struct IdentityFaultPolicy {
    fault: Option<IdentityFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl IdentityFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: IdentityFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: IdentityFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<IdentityFault> {
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
