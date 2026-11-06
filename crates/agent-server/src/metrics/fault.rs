use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricsErrorTier {
    /// Transient delay or lock contention that can be resolved via backoff retry.
    Transient,
    /// Degraded state where fresh metrics cannot be computed, but a cached fallback can be served.
    Degraded,
    /// Permanent error such as memory exhaustion or fatal I/O failure.
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricsFault {
    Timeout,
    LockContention,
    CorruptedOutput,
    IoError(String),
    MemoryExhausted,
}

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum MetricsError {
    #[error("Metrics collection timed out")]
    Timeout,

    #[error("Metrics collector lock contention")]
    LockContention,

    #[error("Corrupted metrics payload format")]
    CorruptedOutput,

    #[error("I/O error during metrics exposition: {0}")]
    IoError(String),

    #[error("Exceeded maximum retry attempts ({0}) during metrics collection")]
    MaxRetriesExceeded(usize),

    #[error("Operating in degraded mode with cached fallback: {0}")]
    DegradedFallback(String),

    #[error("Metrics buffer capacity exhausted: {0}")]
    MemoryExhausted(String),
}

impl MetricsError {
    pub fn tier(&self) -> MetricsErrorTier {
        match self {
            Self::Timeout | Self::LockContention => MetricsErrorTier::Transient,
            Self::DegradedFallback(_) => MetricsErrorTier::Degraded,
            Self::CorruptedOutput | Self::IoError(_) | Self::MaxRetriesExceeded(_) | Self::MemoryExhausted(_) => {
                MetricsErrorTier::Permanent
            }
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self.tier(), MetricsErrorTier::Transient)
    }

    pub fn to_status_code(&self) -> StatusCode {
        match self.tier() {
            MetricsErrorTier::Transient => StatusCode::REQUEST_TIMEOUT,
            MetricsErrorTier::Degraded => StatusCode::OK, // 200 with degraded header
            MetricsErrorTier::Permanent => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub fn simulate_metrics_fault(fault: MetricsFault) -> MetricsError {
    match fault {
        MetricsFault::Timeout => MetricsError::Timeout,
        MetricsFault::LockContention => MetricsError::LockContention,
        MetricsFault::CorruptedOutput => MetricsError::CorruptedOutput,
        MetricsFault::IoError(msg) => MetricsError::IoError(msg),
        MetricsFault::MemoryExhausted => MetricsError::MemoryExhausted("Max capacity reached".to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct MetricsFaultPolicy {
    active_fault: Option<MetricsFault>,
    interval: usize,
    call_count: usize,
    max_injections: Option<usize>,
    injected_count: usize,
}

impl Default for MetricsFaultPolicy {
    fn default() -> Self {
        Self::none()
    }
}

impl MetricsFaultPolicy {
    pub fn none() -> Self {
        Self {
            active_fault: None,
            interval: 1,
            call_count: 0,
            max_injections: None,
            injected_count: 0,
        }
    }

    pub fn always(fault: MetricsFault) -> Self {
        Self {
            active_fault: Some(fault),
            interval: 1,
            call_count: 0,
            max_injections: None,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: MetricsFault, interval: usize) -> Self {
        Self {
            active_fault: Some(fault),
            interval: interval.max(1),
            call_count: 0,
            max_injections: None,
            injected_count: 0,
        }
    }

    pub fn with_max_injections(mut self, max: usize) -> Self {
        self.max_injections = Some(max);
        self
    }

    pub fn should_inject(&mut self) -> Option<MetricsFault> {
        let fault = self.active_fault.clone()?;
        self.call_count += 1;

        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        if self.call_count.is_multiple_of(self.interval) {
            self.injected_count += 1;
            Some(fault)
        } else {
            None
        }
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }

    pub fn reset(&mut self) {
        self.call_count = 0;
        self.injected_count = 0;
    }
}
