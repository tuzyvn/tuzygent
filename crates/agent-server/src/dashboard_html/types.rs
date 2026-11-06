use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Categorizes dashboard errors into severity tiers for resilient recovery and fallback routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DashboardErrorTier {
    /// Transient network or socket interruptions (timeout, reset, disconnection) that can be safely retried.
    TransientNetwork,
    /// Filesystem or stream I/O failure when reading templates or static assets.
    IoFailure,
    /// Template compilation, syntax, or variable substitution failure.
    RenderFailure,
    /// Missing static assets or unresolved template routes.
    NotFound,
    /// Exhaustion of retry budget.
    Exhausted,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DashboardError {
    #[error("Template render failure: {0}")]
    TemplateRenderFailure(String),

    #[error("Missing asset: {0}")]
    MissingAsset(String),

    #[error("I/O error during dashboard asset loading: {0}")]
    IoError(String),

    #[error("Network disconnection during dashboard transfer: {0}")]
    NetworkDisconnection(String),

    #[error("Socket timeout while streaming dashboard")]
    SocketTimeout,

    #[error("Connection reset by peer during dashboard streaming")]
    ConnectionReset,

    #[error("Max retry attempts ({0}) exceeded during dashboard recovery")]
    MaxRetriesExceeded(usize),
}

impl DashboardError {
    pub fn tier(&self) -> DashboardErrorTier {
        match self {
            Self::SocketTimeout | Self::ConnectionReset | Self::NetworkDisconnection(_) => {
                DashboardErrorTier::TransientNetwork
            }
            Self::IoError(_) => DashboardErrorTier::IoFailure,
            Self::TemplateRenderFailure(_) => DashboardErrorTier::RenderFailure,
            Self::MissingAsset(_) => DashboardErrorTier::NotFound,
            Self::MaxRetriesExceeded(_) => DashboardErrorTier::Exhausted,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self.tier(),
            DashboardErrorTier::TransientNetwork | DashboardErrorTier::IoFailure
        )
    }

    pub fn to_status_code(&self) -> StatusCode {
        match self {
            Self::SocketTimeout => StatusCode::REQUEST_TIMEOUT,
            Self::ConnectionReset | Self::NetworkDisconnection(_) => StatusCode::BAD_GATEWAY,
            Self::IoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::TemplateRenderFailure(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::MissingAsset(_) => StatusCode::NOT_FOUND,
            Self::MaxRetriesExceeded(_) => StatusCode::GATEWAY_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DashboardFault {
    TemplateRenderFailure,
    MissingAsset,
    IoError,
    NetworkDisconnection,
    SocketTimeout,
    ConnectionReset,
}

impl DashboardFault {
    pub fn to_dashboard_error(&self) -> DashboardError {
        match self {
            Self::TemplateRenderFailure => DashboardError::TemplateRenderFailure(
                "Injected template rendering failure: placeholder evaluation failed".to_string(),
            ),
            Self::MissingAsset => {
                DashboardError::MissingAsset("Injected missing asset fault: static asset not found".to_string())
            }
            Self::IoError => DashboardError::IoError(
                "Injected I/O failure: underlying storage read aborted".to_string(),
            ),
            Self::NetworkDisconnection => DashboardError::NetworkDisconnection(
                "Injected network disconnection: client connection abruptly terminated".to_string(),
            ),
            Self::SocketTimeout => DashboardError::SocketTimeout,
            Self::ConnectionReset => DashboardError::ConnectionReset,
        }
    }

    pub fn simulate_fault(&self) -> Result<(), DashboardError> {
        Err(self.to_dashboard_error())
    }
}

pub fn simulate_dashboard_fault(fault: &DashboardFault) -> Result<(), DashboardError> {
    fault.simulate_fault()
}

#[derive(Debug, Clone, Default)]
pub struct DashboardFaultPolicy {
    fault: Option<DashboardFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl DashboardFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: DashboardFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: DashboardFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<DashboardFault> {
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

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }
}

/// Dynamic contextual properties passed into the dashboard template engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardContext {
    pub title: Option<String>,
    pub gateway_url: Option<String>,
    pub version: Option<String>,
    pub auth_token_preset: Option<String>,
    pub custom_banner: Option<String>,
    pub extra_variables: HashMap<String, String>,
}

impl DashboardContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_gateway_url(mut self, url: impl Into<String>) -> Self {
        self.gateway_url = Some(url.into());
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_variables.insert(key.into(), value.into());
        self
    }
}

/// Shared wrapper for testing chaos fault injection within handlers.
#[derive(Clone, Default)]
pub struct DashboardState {
    pub fault_policy: Arc<Mutex<DashboardFaultPolicy>>,
}

impl DashboardState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_fault_policy(policy: DashboardFaultPolicy) -> Self {
        Self {
            fault_policy: Arc::new(Mutex::new(policy)),
        }
    }
}
