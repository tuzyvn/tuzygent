use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GuardrailError {
    #[error("Command blocked by red line rule: {0}")]
    Blocked(String),
    #[error("Simulated socket timeout during guardrail check")]
    SocketTimeout,
    #[error("Connection reset during guardrail check")]
    ConnectionReset,
    #[error("Transient I/O failure during guardrail evaluation or audit write: {0}")]
    IoError(String),
    #[error("Corrupted or invalid policy rule: {0}")]
    CorruptedPolicy(String),
    #[error("Policy service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Max retry attempts ({0}) exceeded during guardrail recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardrailFault {
    SocketTimeout,
    ConnectionReset,
    CorruptedRule,
    TransientIo,
    ServiceUnavailable,
}

impl GuardrailFault {
    pub fn to_error(&self) -> GuardrailError {
        match self {
            Self::SocketTimeout => GuardrailError::SocketTimeout,
            Self::ConnectionReset => GuardrailError::ConnectionReset,
            Self::TransientIo => {
                GuardrailError::IoError("Simulated transient I/O failure".to_string())
            }
            Self::CorruptedRule => {
                GuardrailError::CorruptedPolicy("Rule pattern corrupted or unparseable".to_string())
            }
            Self::ServiceUnavailable => GuardrailError::ServiceUnavailable(
                "Guardrail service temporarily unavailable".to_string(),
            ),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GuardrailFaultPolicy {
    fault: Option<GuardrailFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl GuardrailFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: GuardrailFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: GuardrailFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<GuardrailFault> {
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
