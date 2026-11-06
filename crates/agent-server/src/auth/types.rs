use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use super::fault::{simulate_auth_fault, AuthFault, AuthFaultPolicy};
pub use super::state::AuthState;

/// Categorizes authentication errors into severity tiers for resilient recovery and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthErrorTier {
    /// Transient network or I/O interruptions that can be safely retried.
    TransientNetwork,
    /// Client-side protocol or encoding errors (bad headers, malformed UTF-8).
    ClientProtocol,
    /// Security violations (unauthorized token, forbidden origin).
    SecurityViolation,
    /// Exhaustion of retry budget.
    Exhausted,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    #[error("Socket timeout during authentication")]
    SocketTimeout,
    #[error("Connection reset by peer during auth")]
    ConnectionReset,
    #[error("Transient I/O failure during auth: {0}")]
    IntermittentIo(String),
    #[error("Token decoding failure: {0}")]
    TokenDecodingFailure(String),
    #[error("Malformed authorization header: {0}")]
    MalformedHeader(String),
    #[error("Authentication required: token invalid or missing")]
    Unauthorized,
    #[error("Origin forbidden")]
    Forbidden,
    #[error("Max retry attempts ({0}) exceeded during auth recovery")]
    MaxRetriesExceeded(usize),
}

impl AuthError {
    pub fn tier(&self) -> AuthErrorTier {
        match self {
            Self::SocketTimeout | Self::ConnectionReset | Self::IntermittentIo(_) => {
                AuthErrorTier::TransientNetwork
            }
            Self::TokenDecodingFailure(_) | Self::MalformedHeader(_) => {
                AuthErrorTier::ClientProtocol
            }
            Self::Unauthorized | Self::Forbidden => AuthErrorTier::SecurityViolation,
            Self::MaxRetriesExceeded(_) => AuthErrorTier::Exhausted,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self.tier(), AuthErrorTier::TransientNetwork)
    }

    pub fn to_status_code(&self) -> StatusCode {
        match self {
            Self::SocketTimeout => StatusCode::REQUEST_TIMEOUT,
            Self::ConnectionReset => StatusCode::BAD_GATEWAY,
            Self::IntermittentIo(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::TokenDecodingFailure(_) => StatusCode::UNAUTHORIZED,
            Self::MalformedHeader(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::MaxRetriesExceeded(_) => StatusCode::GATEWAY_TIMEOUT,
        }
    }
}
