use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;

use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    #[error("Socket timeout communicating with CLI process or daemon")]
    SocketTimeout,
    #[error("CLI process execution timed out")]
    ProcessTimeout,
    #[error("Broken pipe or connection reset communicating with CLI process")]
    BrokenPipe,
    #[error("Transient I/O failure during CLI operation: {0}")]
    TransientIo(String),
    #[error("CLI process execution failed: {0}")]
    ProcessFailed(String),
    #[error("Corrupted or malformed CLI output: {0}")]
    CorruptedOutput(String),
    #[error("CLI binary not found or unreachable: {0}")]
    CliNotFound(String),
    #[error("Max retry attempts ({0}) exceeded during CLI recovery")]
    MaxRetriesExceeded(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CliFault {
    SocketTimeout,
    ProcessTimeout,
    BrokenPipe,
    TransientIo,
    ProcessFailed,
    CorruptedOutput,
    CliNotFound,
}

impl CliFault {
    pub fn to_cli_error(&self) -> CliError {
        match self {
            CliFault::SocketTimeout => CliError::SocketTimeout,
            CliFault::ProcessTimeout => CliError::ProcessTimeout,
            CliFault::BrokenPipe => CliError::BrokenPipe,
            CliFault::TransientIo => CliError::TransientIo(
                "Simulated transient I/O failure during CLI process execution".to_string(),
            ),
            CliFault::ProcessFailed => CliError::ProcessFailed(
                "Simulated CLI process failure with non-zero exit code".to_string(),
            ),
            CliFault::CorruptedOutput => {
                CliError::CorruptedOutput("Corrupted or malformed CLI stream output".to_string())
            }
            CliFault::CliNotFound => {
                CliError::CliNotFound("Simulated CLI binary not found in PATH".to_string())
            }
        }
    }

    pub fn to_engine_error(&self) -> EngineError {
        match self {
            CliFault::SocketTimeout | CliFault::ProcessTimeout => EngineError::Timeout,
            CliFault::CliNotFound => EngineError::CliNotFound(self.to_cli_error().to_string()),
            _ => EngineError::ProcessFailed(self.to_cli_error().to_string()),
        }
    }

    pub fn simulate_fault(&self) -> Result<(), CliError> {
        Err(self.to_cli_error())
    }
}

pub fn simulate_cli_fault(fault: &CliFault) -> Result<(), CliError> {
    fault.simulate_fault()
}

pub fn simulate_fault(fault: &CliFault) -> Result<(), CliError> {
    fault.simulate_fault()
}

#[derive(Debug, Clone, Default)]
pub struct CliFaultPolicy {
    fault: Option<CliFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl CliFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: CliFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: CliFault, interval: u32) -> Self {
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

    pub fn should_inject(&mut self) -> Option<CliFault> {
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

    pub fn fault(&self) -> Option<&CliFault> {
        self.fault.as_ref()
    }
}

pub fn is_retryable_cli_error_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("broken pipe")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("reset by peer")
        || lower.contains("transient")
        || lower.contains("temporarily unavailable")
        || lower.contains("resource temporarily unavailable")
        || lower.contains("os error 32") // EPIPE
        || lower.contains("os error 11") // EAGAIN
        || lower.contains("os error 104") // ECONNRESET
        || lower.contains("interrupted system call")
        || lower.contains("i/o error")
        || lower.contains("io error")
}

pub fn is_retryable_engine_error(err: &EngineError) -> bool {
    match err {
        EngineError::Timeout => true,
        EngineError::ProcessFailed(msg) => is_retryable_cli_error_message(msg),
        _ => false,
    }
}

/// Wrapper tự phục hồi cho ComputeEngine CLI với cơ chế retry và fault injection testing
#[derive(Clone)]
pub struct ResilientCliEngine<E> {
    inner: E,
    fault_policy: Arc<Mutex<CliFaultPolicy>>,
    max_retries: usize,
    retry_delay_ms: u64,
    resolved_count: Arc<AtomicUsize>,
    recovered_count: Arc<AtomicUsize>,
    rejected_count: Arc<AtomicUsize>,
}

pub type ResilientCliManager<E> = ResilientCliEngine<E>;

impl<E> ResilientCliEngine<E> {
    pub fn new(inner: E, fault_policy: CliFaultPolicy, max_retries: usize) -> Self {
        Self {
            inner,
            fault_policy: Arc::new(Mutex::new(fault_policy)),
            max_retries,
            retry_delay_ms: 0,
            resolved_count: Arc::new(AtomicUsize::new(0)),
            recovered_count: Arc::new(AtomicUsize::new(0)),
            rejected_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_engine(inner: E) -> Self {
        Self::new(inner, CliFaultPolicy::none(), 3)
    }

    pub fn with_fault_policy(inner: E, fault_policy: CliFaultPolicy) -> Self {
        Self::new(inner, fault_policy, 3)
    }

    pub fn with_retry_delay_ms(mut self, delay_ms: u64) -> Self {
        self.retry_delay_ms = delay_ms;
        self
    }

    pub fn resolved_count(&self) -> usize {
        self.resolved_count.load(Ordering::SeqCst)
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count.load(Ordering::SeqCst)
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count.load(Ordering::SeqCst)
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn inner(&self) -> &E {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut E {
        &mut self.inner
    }

    pub fn fault_policy(&self) -> Arc<Mutex<CliFaultPolicy>> {
        Arc::clone(&self.fault_policy)
    }

    pub fn set_fault_policy(&self, policy: CliFaultPolicy) {
        if let Ok(mut lock) = self.fault_policy.lock() {
            *lock = policy;
        }
    }

    pub fn reset_metrics(&self) {
        self.resolved_count.store(0, Ordering::SeqCst);
        self.recovered_count.store(0, Ordering::SeqCst);
        self.rejected_count.store(0, Ordering::SeqCst);
    }
}

#[async_trait]
impl<E: ComputeEngine> ComputeEngine for ResilientCliEngine<E> {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        let mut attempt = 0;
        loop {
            attempt += 1;

            let fault = {
                let mut policy = self.fault_policy.lock().map_err(|e| {
                    EngineError::ProcessFailed(format!("Fault policy lock poisoned: {}", e))
                })?;
                policy.should_inject()
            };

            if let Some(fault) = fault {
                match fault {
                    CliFault::SocketTimeout
                    | CliFault::ProcessTimeout
                    | CliFault::BrokenPipe
                    | CliFault::TransientIo
                    | CliFault::ProcessFailed => {
                        if attempt <= self.max_retries {
                            self.recovered_count.fetch_add(1, Ordering::SeqCst);
                            if self.retry_delay_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(self.retry_delay_ms))
                                    .await;
                            }
                            continue;
                        } else {
                            self.rejected_count.fetch_add(1, Ordering::SeqCst);
                            return Err(fault.to_engine_error());
                        }
                    }
                    CliFault::CorruptedOutput | CliFault::CliNotFound => {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        return Err(fault.to_engine_error());
                    }
                }
            }

            match self.inner.execute(input).await {
                Ok(output) => {
                    self.resolved_count.fetch_add(1, Ordering::SeqCst);
                    return Ok(output);
                }
                Err(err) => {
                    if is_retryable_engine_error(&err) && attempt <= self.max_retries {
                        self.recovered_count.fetch_add(1, Ordering::SeqCst);
                        if self.retry_delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                        }
                        continue;
                    } else {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        return Err(err);
                    }
                }
            }
        }
    }

    fn engine_kind(&self) -> EngineKind {
        self.inner.engine_kind()
    }
}

impl<E: ComputeEngine> ResilientCliEngine<E> {
    pub async fn execute_and_record(
        &self,
        ledger: &mut AuditLedger,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        let mut attempt = 0;
        loop {
            attempt += 1;

            let fault = {
                let mut policy = self.fault_policy.lock().map_err(|e| {
                    EngineError::ProcessFailed(format!("Fault policy lock poisoned: {}", e))
                })?;
                policy.should_inject()
            };

            if let Some(fault) = fault {
                match fault {
                    CliFault::SocketTimeout
                    | CliFault::ProcessTimeout
                    | CliFault::BrokenPipe
                    | CliFault::TransientIo
                    | CliFault::ProcessFailed => {
                        if attempt <= self.max_retries {
                            self.recovered_count.fetch_add(1, Ordering::SeqCst);
                            ledger.record_event(
                                AuditEventType::CommandAllowed,
                                ActionSeverity::Routine,
                                format!(
                                    "CLI transient fault {:?} intercepted, retrying attempt {}/{} for engine '{}'",
                                    fault, attempt, self.max_retries, self.engine_kind()
                                ),
                            );
                            if self.retry_delay_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(self.retry_delay_ms))
                                    .await;
                            }
                            continue;
                        } else {
                            self.rejected_count.fetch_add(1, Ordering::SeqCst);
                            let err = fault.to_engine_error();
                            ledger.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "CLI max retries exceeded for engine '{}': {}",
                                    self.engine_kind(),
                                    err
                                ),
                            );
                            return Err(err);
                        }
                    }
                    CliFault::CorruptedOutput | CliFault::CliNotFound => {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        let err = fault.to_engine_error();
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Fatal CLI fault {:?} rejected for engine '{}': {}",
                                fault,
                                self.engine_kind(),
                                err
                            ),
                        );
                        return Err(err);
                    }
                }
            }

            match self.inner.execute(input).await {
                Ok(output) => {
                    self.resolved_count.fetch_add(1, Ordering::SeqCst);
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!(
                            "CLI execution succeeded with engine '{}'",
                            self.engine_kind()
                        ),
                    );
                    return Ok(output);
                }
                Err(err) => {
                    if is_retryable_engine_error(&err) && attempt <= self.max_retries {
                        self.recovered_count.fetch_add(1, Ordering::SeqCst);
                        ledger.record_event(
                            AuditEventType::CommandAllowed,
                            ActionSeverity::Routine,
                            format!(
                                "CLI error {:?} intercepted, retrying attempt {}/{} for engine '{}'",
                                err, attempt, self.max_retries, self.engine_kind()
                            ),
                        );
                        if self.retry_delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                        }
                        continue;
                    } else {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "CLI execution failed for engine '{}': {}",
                                self.engine_kind(),
                                err
                            ),
                        );
                        return Err(err);
                    }
                }
            }
        }
    }
}

pub trait ResilientCliWrapper: ComputeEngine + Sized {
    fn with_resilience(self, max_retries: usize) -> ResilientCliEngine<Self> {
        ResilientCliEngine::new(self, CliFaultPolicy::none(), max_retries)
    }

    fn with_chaos(
        self,
        fault_policy: CliFaultPolicy,
        max_retries: usize,
    ) -> ResilientCliEngine<Self> {
        ResilientCliEngine::new(self, fault_policy, max_retries)
    }
}

impl<T: ComputeEngine + Sized> ResilientCliWrapper for T {}
