use serde::{Deserialize, Serialize};
use super::types::AuthError;

/// Simulated authentication fault injection types for chaos engineering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthFault {
    SocketTimeout,
    ConnectionReset,
    IntermittentIo,
    TokenDecodingFailure,
    MalformedHeader,
}

impl AuthFault {
    pub fn to_auth_error(&self) -> AuthError {
        match self {
            Self::SocketTimeout => AuthError::SocketTimeout,
            Self::ConnectionReset => AuthError::ConnectionReset,
            Self::IntermittentIo => {
                AuthError::IntermittentIo("Transient I/O failure during auth stream".to_string())
            }
            Self::TokenDecodingFailure => {
                AuthError::TokenDecodingFailure("Failed to decode token from request".to_string())
            }
            Self::MalformedHeader => {
                AuthError::MalformedHeader("Malformed authorization header structure".to_string())
            }
        }
    }

    pub fn simulate_fault(&self) -> Result<(), AuthError> {
        Err(self.to_auth_error())
    }
}

pub fn simulate_auth_fault(fault: &AuthFault) -> Result<(), AuthError> {
    fault.simulate_fault()
}

/// Fault injection policy for chaos resilience testing.
#[derive(Debug, Clone, Default)]
pub struct AuthFaultPolicy {
    fault: Option<AuthFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl AuthFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: AuthFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: AuthFault, interval: u32) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: Some(interval),
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn with_max_injections(mut self, max: usize) -> Self {
        self.max_injections = Some(max);
        self
    }

    pub fn should_inject(&mut self) -> Option<AuthFault> {
        let fault = self.fault.as_ref()?;
        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.saturating_add(1);
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

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }

    pub fn has_active_fault(&self) -> bool {
        self.fault.is_some()
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }
}
