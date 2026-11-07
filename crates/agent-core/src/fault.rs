use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;
use thiserror::Error;

/// Structured errors emitted during resilient agent-core operations.
#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoreExecutionError {
    #[error("Operation timed out after {0} ms")]
    Timeout(u64),

    #[error("Lock contention encountered on resource: {0}")]
    LockContention(String),

    #[error("Transient engine execution failure: {0}")]
    TransientEngineFailure(String),

    #[error("Memory capacity exhausted (max {0} entries)")]
    MemoryExhausted(usize),

    #[error("Circuit breaker is OPEN (rejection count: {0})")]
    CircuitOpen(usize),

    #[error("Unwrap safety violation avoided: {0}")]
    UnwrapSafetyViolation(String),

    #[error("Core operation panicked and was safely caught: {0}")]
    Panic(String),

    #[error("Core execution error: {0}")]
    Other(String),
}

impl From<std::io::Error> for CoreExecutionError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::TimedOut => CoreExecutionError::Timeout(5000),
            _ => CoreExecutionError::Other(err.to_string()),
        }
    }
}

impl From<anyhow::Error> for CoreExecutionError {
    fn from(err: anyhow::Error) -> Self {
        let msg = err.to_string();
        if msg.contains("timed out") || msg.contains("timeout") {
            CoreExecutionError::Timeout(5000)
        } else if msg.contains("contention") || msg.contains("lock") {
            CoreExecutionError::LockContention(msg)
        } else {
            CoreExecutionError::Other(msg)
        }
    }
}

/// Simulated fault kinds for chaos testing and resilient self-healing verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoreFaultKind {
    None,
    SimulateTimeout,
    SimulateLockContention,
    SimulateTransientFailure,
    SimulatePanic,
    SimulateUnwrapViolation,
}

/// Circuit breaker states for graceful degradation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Thread-safe policy for chaos fault injection and circuit breaker management.
#[derive(Debug)]
pub struct CoreFaultPolicy {
    injected_count: AtomicUsize,
    failure_threshold: AtomicUsize,
    consecutive_failures: AtomicUsize,
    active_fault: RwLock<CoreFaultKind>,
}

impl Default for CoreFaultPolicy {
    fn default() -> Self {
        Self::new(5)
    }
}

impl CoreFaultPolicy {
    pub fn new(failure_threshold: usize) -> Self {
        Self {
            injected_count: AtomicUsize::new(0),
            failure_threshold: AtomicUsize::new(failure_threshold),
            consecutive_failures: AtomicUsize::new(0),
            active_fault: RwLock::new(CoreFaultKind::None),
        }
    }

    pub fn set_fault(&self, fault: CoreFaultKind) {
        if let Ok(mut lock) = self.active_fault.write() {
            *lock = fault;
        } else if let Err(poisoned) = self.active_fault.write() {
            *poisoned.into_inner() = fault;
        }
    }

    pub fn clear_fault(&self) {
        self.set_fault(CoreFaultKind::None);
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count.load(Ordering::Relaxed)
    }

    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn circuit_state(&self) -> CircuitState {
        let threshold = self.failure_threshold.load(Ordering::Relaxed);
        let failures = self.consecutive_failures.load(Ordering::Relaxed);
        if failures >= threshold {
            CircuitState::Open
        } else if failures > 0 {
            CircuitState::HalfOpen
        } else {
            CircuitState::Closed
        }
    }

    pub fn get_fault(&self) -> CoreFaultKind {
        match self.active_fault.read() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    /// Evaluates if an operation should be failed or degraded according to active fault policy.
    pub fn check_and_inject(&self) -> Result<(), CoreExecutionError> {
        if self.circuit_state() == CircuitState::Open {
            return Err(CoreExecutionError::CircuitOpen(
                self.consecutive_failures.load(Ordering::Relaxed),
            ));
        }

        let fault = self.get_fault();
        match fault {
            CoreFaultKind::None => Ok(()),
            CoreFaultKind::SimulateTimeout => {
                self.injected_count.fetch_add(1, Ordering::Relaxed);
                self.record_failure();
                Err(CoreExecutionError::Timeout(3000))
            }
            CoreFaultKind::SimulateLockContention => {
                self.injected_count.fetch_add(1, Ordering::Relaxed);
                self.record_failure();
                Err(CoreExecutionError::LockContention(
                    "sharded-agent-mutex".to_string(),
                ))
            }
            CoreFaultKind::SimulateTransientFailure => {
                self.injected_count.fetch_add(1, Ordering::Relaxed);
                self.record_failure();
                Err(CoreExecutionError::TransientEngineFailure(
                    "LLM HTTP 503 Provider Unavailable".to_string(),
                ))
            }
            CoreFaultKind::SimulatePanic => {
                self.injected_count.fetch_add(1, Ordering::Relaxed);
                self.record_failure();
                Err(CoreExecutionError::Panic(
                    "Simulated panic caught safely".to_string(),
                ))
            }
            CoreFaultKind::SimulateUnwrapViolation => {
                self.injected_count.fetch_add(1, Ordering::Relaxed);
                self.record_failure();
                Err(CoreExecutionError::UnwrapSafetyViolation(
                    "Attempted to unwrap empty option in core agent".to_string(),
                ))
            }
        }
    }
}
