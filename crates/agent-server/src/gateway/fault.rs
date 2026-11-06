use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Categorizes gateway errors into severity tiers for resilient recovery, routing, and fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayErrorTier {
    /// Transient network or socket interruptions (timeout, reset, disconnection) that can be safely retried.
    TransientNetwork,
    /// Intermittent filesystem or vault storage I/O failures.
    IoFailure,
    /// Engine process execution or streaming failures (process crash, execution timeout, buffer exhaustion).
    EngineFailure,
    /// Client-side protocol or encoding errors (bad payload, unknown model spec).
    ClientProtocol,
    /// Resource not found (account missing).
    NotFound,
    /// Exhaustion of retry budget.
    Exhausted,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum GatewayError {
    #[error("Socket timeout while communicating with gateway")]
    SocketTimeout,

    #[error("Connection reset by peer during gateway streaming")]
    ConnectionReset,

    #[error("Network disconnection during gateway operation: {0}")]
    NetworkDisconnection(String),

    #[error("I/O error during gateway operations: {0}")]
    IoError(String),

    #[error("Stream transmission interrupted: {0}")]
    StreamInterrupted(String),

    #[error("Compute engine execution timed out: {0}")]
    EngineTimeout(String),

    #[error("Compute engine execution failed: {0}")]
    EngineFailure(String),

    #[error("Invalid model specification: {0}")]
    InvalidModel(String),

    #[error("Account not found: {0}")]
    AccountNotFound(String),

    #[error("Max retry attempts ({0}) exceeded during gateway recovery")]
    MaxRetriesExceeded(usize),
}

impl GatewayError {
    pub fn tier(&self) -> GatewayErrorTier {
        match self {
            Self::SocketTimeout | Self::ConnectionReset | Self::NetworkDisconnection(_) => {
                GatewayErrorTier::TransientNetwork
            }
            Self::IoError(_) => GatewayErrorTier::IoFailure,
            Self::StreamInterrupted(_) | Self::EngineTimeout(_) | Self::EngineFailure(_) => {
                GatewayErrorTier::EngineFailure
            }
            Self::InvalidModel(_) => GatewayErrorTier::ClientProtocol,
            Self::AccountNotFound(_) => GatewayErrorTier::NotFound,
            Self::MaxRetriesExceeded(_) => GatewayErrorTier::Exhausted,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self.tier(),
            GatewayErrorTier::TransientNetwork | GatewayErrorTier::IoFailure
        )
    }

    pub fn to_status_code(&self) -> StatusCode {
        match self {
            Self::SocketTimeout => StatusCode::REQUEST_TIMEOUT,
            Self::ConnectionReset | Self::NetworkDisconnection(_) => StatusCode::BAD_GATEWAY,
            Self::IoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::StreamInterrupted(_) => StatusCode::BAD_GATEWAY,
            Self::EngineTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
            Self::EngineFailure(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::InvalidModel(_) => StatusCode::BAD_REQUEST,
            Self::AccountNotFound(_) => StatusCode::NOT_FOUND,
            Self::MaxRetriesExceeded(_) => StatusCode::GATEWAY_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayFault {
    SocketTimeout,
    ConnectionReset,
    NetworkDisconnection,
    IoError,
    StreamInterrupted,
    EngineTimeout,
    EngineFailure,
    InvalidModel,
    AccountNotFound,
}

impl GatewayFault {
    pub fn to_gateway_error(&self) -> GatewayError {
        match self {
            Self::SocketTimeout => GatewayError::SocketTimeout,
            Self::ConnectionReset => GatewayError::ConnectionReset,
            Self::NetworkDisconnection => GatewayError::NetworkDisconnection(
                "Injected network disconnection: peer disconnected during transfer".to_string(),
            ),
            Self::IoError => GatewayError::IoError(
                "Injected I/O error: disk or vault read failed intermittently".to_string(),
            ),
            Self::StreamInterrupted => GatewayError::StreamInterrupted(
                "Injected SSE stream interruption: transmission buffer stalled".to_string(),
            ),
            Self::EngineTimeout => GatewayError::EngineTimeout(
                "Injected execution timeout: compute engine took too long to complete".to_string(),
            ),
            Self::EngineFailure => GatewayError::EngineFailure(
                "Injected engine crash: compute process exited abnormally".to_string(),
            ),
            Self::InvalidModel => GatewayError::InvalidModel(
                "Injected invalid model specification: model unsupported".to_string(),
            ),
            Self::AccountNotFound => GatewayError::AccountNotFound(
                "Injected account not found: target account is not registered".to_string(),
            ),
        }
    }

    pub fn simulate_fault(&self) -> Result<(), GatewayError> {
        Err(self.to_gateway_error())
    }
}

pub fn simulate_gateway_fault(fault: &GatewayFault) -> Result<(), GatewayError> {
    fault.simulate_fault()
}

#[derive(Debug, Clone, Default)]
pub struct GatewayFaultPolicy {
    fault: Option<GatewayFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl GatewayFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn has_active_fault(&self) -> bool {
        self.fault.is_some()
    }

    pub fn always(fault: GatewayFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: GatewayFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<GatewayFault> {
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

    pub fn invocation_count(&self) -> u32 {
        self.invocation_count
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }
}
