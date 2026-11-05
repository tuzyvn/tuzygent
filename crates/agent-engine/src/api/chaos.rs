use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};

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

/// Wrapper tự phục hồi cho ComputeEngine với cơ chế retry có kiểm soát và injection fault testing
#[derive(Clone)]
pub struct ResilientApiEngine<E> {
    inner: E,
    fault_policy: Arc<Mutex<ApiFaultPolicy>>,
    max_retries: usize,
    retry_delay_ms: u64,
    resolved_count: Arc<AtomicUsize>,
    recovered_count: Arc<AtomicUsize>,
    rejected_count: Arc<AtomicUsize>,
}

pub type ResilientApiWrapper<E> = ResilientApiEngine<E>;
pub type ResilientApiManager<E> = ResilientApiEngine<E>;

impl<E> ResilientApiEngine<E> {
    pub fn new(inner: E, fault_policy: ApiFaultPolicy, max_retries: usize) -> Self {
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
        Self::new(inner, ApiFaultPolicy::none(), 3)
    }

    pub fn with_fault_policy(inner: E, fault_policy: ApiFaultPolicy) -> Self {
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

    pub fn fault_policy(&self) -> Arc<Mutex<ApiFaultPolicy>> {
        Arc::clone(&self.fault_policy)
    }

    pub fn set_fault_policy(&self, policy: ApiFaultPolicy) {
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
impl<E: ComputeEngine> ComputeEngine for ResilientApiEngine<E> {
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
                    ApiFault::SocketTimeout
                    | ApiFault::ConnectionReset
                    | ApiFault::TransientIo
                    | ApiFault::ServiceUnavailable
                    | ApiFault::RateLimitExceeded => {
                        if attempt <= self.max_retries {
                            self.recovered_count.fetch_add(1, Ordering::SeqCst);
                            if self.retry_delay_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                            }
                            continue;
                        } else {
                            self.rejected_count.fetch_add(1, Ordering::SeqCst);
                            return Err(fault.to_engine_error());
                        }
                    }
                    ApiFault::CorruptedResponse => {
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

impl<E: ComputeEngine> ResilientApiEngine<E> {
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
                    ApiFault::SocketTimeout
                    | ApiFault::ConnectionReset
                    | ApiFault::TransientIo
                    | ApiFault::ServiceUnavailable
                    | ApiFault::RateLimitExceeded => {
                        if attempt <= self.max_retries {
                            self.recovered_count.fetch_add(1, Ordering::SeqCst);
                            ledger.record_event(
                                AuditEventType::CommandAllowed,
                                ActionSeverity::Routine,
                                format!(
                                    "API transient fault {:?} intercepted, retrying attempt {}/{} for engine '{}'",
                                    fault, attempt, self.max_retries, self.engine_kind()
                                ),
                            );
                            if self.retry_delay_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                            }
                            continue;
                        } else {
                            self.rejected_count.fetch_add(1, Ordering::SeqCst);
                            let err = fault.to_engine_error();
                            ledger.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "API max retries exceeded for engine '{}': {}",
                                    self.engine_kind(),
                                    err
                                ),
                            );
                            return Err(err);
                        }
                    }
                    ApiFault::CorruptedResponse => {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        let err = fault.to_engine_error();
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Corrupted API response rejected for engine '{}': {}",
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
                        format!("API execution succeeded with engine '{}'", self.engine_kind()),
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
                                "API error {:?} intercepted, retrying attempt {}/{} for engine '{}'",
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
                                "API execution failed for engine '{}': {}",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct MockEngineWithFailures {
        failures_remaining: AtomicUsize,
        fail_error: EngineError,
    }

    #[async_trait]
    impl ComputeEngine for MockEngineWithFailures {
        async fn execute(&self, _input: &EngineInput) -> Result<EngineOutput, EngineError> {
            let rem = self.failures_remaining.load(Ordering::SeqCst);
            if rem > 0 {
                self.failures_remaining.fetch_sub(1, Ordering::SeqCst);
                Err(match &self.fail_error {
                    EngineError::ApiRequestFailed(s) => EngineError::ApiRequestFailed(s.clone()),
                    EngineError::Timeout => EngineError::Timeout,
                    other => EngineError::ProcessFailed(other.to_string()),
                })
            } else {
                Ok(EngineOutput {
                    stdout_content: "success".to_string(),
                    exit_code: 0,
                    execution_duration_ms: 5,
                    engine_used: EngineKind::CustomAgentApi,
                    session_id: None,
                })
            }
        }

        fn engine_kind(&self) -> EngineKind {
            EngineKind::CustomAgentApi
        }
    }

    fn dummy_input() -> EngineInput {
        EngineInput {
            on_event: None,
            command_sandbox: None,
            prompt_or_command: "test".to_string(),
            system_context: "ctx".to_string(),
            working_dir: ".".to_string(),
            timeout_seconds: 5,
            on_stream_chunk: None,
            temperature: None,
            max_tokens: None,
            max_output_bytes: None,
        }
    }

    #[tokio::test]
    async fn test_fault_simulation_and_taxonomy() {
        assert_eq!(simulate_fault(&ApiFault::SocketTimeout), Err(ApiError::SocketTimeout));
        assert_eq!(simulate_fault(&ApiFault::ConnectionReset), Err(ApiError::ConnectionReset));
        assert!(matches!(
            simulate_fault(&ApiFault::RateLimitExceeded),
            Err(ApiError::RateLimitExceeded(_))
        ));
        assert!(matches!(
            simulate_fault(&ApiFault::TransientIo),
            Err(ApiError::TransientIo(_))
        ));
        assert!(matches!(
            simulate_fault(&ApiFault::CorruptedResponse),
            Err(ApiError::CorruptedResponse(_))
        ));
        assert!(matches!(
            simulate_fault(&ApiFault::ServiceUnavailable),
            Err(ApiError::ServiceUnavailable(_))
        ));
    }

    #[tokio::test]
    async fn test_policy_intermittent_and_max_injections() {
        let mut policy = ApiFaultPolicy::intermittent(ApiFault::SocketTimeout, 2)
            .with_max_injections(2);

        assert_eq!(policy.should_inject(), Some(ApiFault::SocketTimeout)); // inv 1
        assert_eq!(policy.should_inject(), None);                          // inv 2
        assert_eq!(policy.should_inject(), Some(ApiFault::SocketTimeout)); // inv 3
        assert_eq!(policy.should_inject(), None);                          // max reached
        assert_eq!(policy.injected_count(), 2);
    }

    #[tokio::test]
    async fn test_resilient_engine_auto_recovers_transient_failures() {
        let inner = MockEngineWithFailures {
            failures_remaining: AtomicUsize::new(2),
            fail_error: EngineError::ApiRequestFailed("HTTP error: connection reset by peer".to_string()),
        };

        let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 3);
        let res = resilient.execute(&dummy_input()).await;

        assert!(res.is_ok());
        assert_eq!(resilient.recovered_count(), 2);
        assert_eq!(resilient.resolved_count(), 1);
        assert_eq!(resilient.rejected_count(), 0);
    }

    #[tokio::test]
    async fn test_resilient_engine_fails_when_exceeding_max_retries() {
        let inner = MockEngineWithFailures {
            failures_remaining: AtomicUsize::new(5),
            fail_error: EngineError::ApiRequestFailed("503 Service Unavailable".to_string()),
        };

        let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 2);
        let res = resilient.execute(&dummy_input()).await;

        assert!(res.is_err());
        assert_eq!(resilient.recovered_count(), 2);
        assert_eq!(resilient.rejected_count(), 1);
        assert_eq!(resilient.resolved_count(), 0);
    }

    #[tokio::test]
    async fn test_resilient_engine_chaos_injection_recovery() {
        let inner = MockEngineWithFailures {
            failures_remaining: AtomicUsize::new(0),
            fail_error: EngineError::Timeout,
        };

        // Injects 2 transient faults via policy, wrapper retries and recovers
        let policy = ApiFaultPolicy::always(ApiFault::SocketTimeout).with_max_injections(2);
        let resilient = ResilientApiEngine::new(inner, policy, 3);
        let res = resilient.execute(&dummy_input()).await;

        assert!(res.is_ok());
        assert_eq!(resilient.recovered_count(), 2);
        assert_eq!(resilient.resolved_count(), 1);
    }

    #[tokio::test]
    async fn test_resilient_engine_corrupted_response_not_retried() {
        let inner = MockEngineWithFailures {
            failures_remaining: AtomicUsize::new(0),
            fail_error: EngineError::Timeout,
        };

        // Corrupted response is fatal and rejected immediately
        let policy = ApiFaultPolicy::always(ApiFault::CorruptedResponse);
        let resilient = ResilientApiEngine::new(inner, policy, 3);
        let res = resilient.execute(&dummy_input()).await;

        assert!(res.is_err());
        assert_eq!(resilient.rejected_count(), 1);
        assert_eq!(resilient.recovered_count(), 0);
    }

    #[tokio::test]
    async fn test_execute_and_record_with_audit_ledger() {
        let inner = MockEngineWithFailures {
            failures_remaining: AtomicUsize::new(1),
            fail_error: EngineError::ApiRequestFailed("429 Too Many Requests".to_string()),
        };

        let resilient = ResilientApiEngine::new(inner, ApiFaultPolicy::none(), 3);
        let mut ledger = AuditLedger::new();
        let res = resilient.execute_and_record(&mut ledger, &dummy_input()).await;

        assert!(res.is_ok());
        assert_eq!(resilient.recovered_count(), 1);
        assert_eq!(resilient.resolved_count(), 1);
        assert_eq!(ledger.len(), 2); // 1 retry attempt event + 1 success event
    }
}
