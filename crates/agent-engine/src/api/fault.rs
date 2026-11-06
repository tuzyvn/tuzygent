use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::traits::EngineError;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    #[error("Socket timeout communicating with API service")]
    SocketTimeout,
    #[error("Connection reset by peer")]
    ConnectionReset,
    #[error("Transient I/O failure during API operation: {0}")]
    TransientIo(String),
    #[error("API service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("API rate limit exceeded: {0}")]
    RateLimitExceeded(String),
    #[error("Corrupted or malformed API response: {0}")]
    CorruptedResponse(String),
    #[error("Max retry attempts ({0}) exceeded during API recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiFault {
    SocketTimeout,
    ConnectionReset,
    RateLimitExceeded,
    TransientIo,
    CorruptedResponse,
    ServiceUnavailable,
}

impl ApiFault {
    pub fn to_api_error(&self) -> ApiError {
        match self {
            ApiFault::SocketTimeout => ApiError::SocketTimeout,
            ApiFault::ConnectionReset => ApiError::ConnectionReset,
            ApiFault::RateLimitExceeded => {
                ApiError::RateLimitExceeded("429 Too Many Requests / Rate limit exceeded".to_string())
            }
            ApiFault::TransientIo => {
                ApiError::TransientIo("Simulated transient I/O network failure".to_string())
            }
            ApiFault::CorruptedResponse => {
                ApiError::CorruptedResponse("Corrupted or malformed API response".to_string())
            }
            ApiFault::ServiceUnavailable => {
                ApiError::ServiceUnavailable("503 Service Temporarily Unavailable".to_string())
            }
        }
    }

    pub fn to_engine_error(&self) -> EngineError {
        EngineError::ApiRequestFailed(self.to_api_error().to_string())
    }

    pub fn simulate_fault(&self) -> Result<(), ApiError> {
        Err(self.to_api_error())
    }
}

pub fn simulate_fault(fault: &ApiFault) -> Result<(), ApiError> {
    fault.simulate_fault()
}

#[derive(Debug, Clone, Default)]
pub struct ApiFaultPolicy {
    fault: Option<ApiFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl ApiFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: ApiFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: ApiFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<ApiFault> {
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

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }

    pub fn invocation_count(&self) -> u32 {
        self.invocation_count
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }

    pub fn fault(&self) -> Option<&ApiFault> {
        self.fault.as_ref()
    }
}

pub fn is_retryable_api_error_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("broken pipe")
        || lower.contains("reset by peer")
        || lower.contains("error sending request")
        || lower.contains("premature eof")
        || lower.contains("unexpected eof")
        || lower.contains("failed reading response")
        || lower.contains("failed parsing llm json")
        || lower.contains("error reading a body")
        || lower.contains("503")
        || lower.contains("service unavailable")
        || lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("502")
        || lower.contains("bad gateway")
        || lower.contains("504")
        || lower.contains("gateway timeout")
        || lower.contains("transient")
}

pub fn is_retryable_engine_error(err: &EngineError) -> bool {
    match err {
        EngineError::Timeout => true,
        EngineError::ApiRequestFailed(msg) => is_retryable_api_error_message(msg),
        _ => false,
    }
}
