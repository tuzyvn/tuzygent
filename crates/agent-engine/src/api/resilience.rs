use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use async_trait::async_trait;

use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};
use crate::api::fault::{is_retryable_engine_error, ApiFault, ApiFaultPolicy};
use crate::traits::{ComputeEngine, EngineError, EngineInput, EngineKind, EngineOutput};

/// Wrapper tự phục hồi cho ComputeEngine với cơ chế retry có kiểm soát, bounded memory cache và injection fault testing
#[derive(Clone)]
pub struct ResilientApiEngine<E> {
    inner: E,
    fault_policy: Arc<Mutex<ApiFaultPolicy>>,
    max_retries: usize,
    retry_delay_ms: u64,
    resolved_count: Arc<AtomicUsize>,
    recovered_count: Arc<AtomicUsize>,
    rejected_count: Arc<AtomicUsize>,
    response_cache: Arc<Mutex<HashMap<String, EngineOutput>>>,
    cache_queue: Arc<Mutex<VecDeque<String>>>,
    max_capacity: Option<usize>,
    backoff_policy: Option<crate::backoff::BackoffPolicy>,
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
            response_cache: Arc::new(Mutex::new(HashMap::new())),
            cache_queue: Arc::new(Mutex::new(VecDeque::new())),
            max_capacity: None,
            backoff_policy: None,
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

    pub fn with_backoff_policy(mut self, policy: crate::backoff::BackoffPolicy) -> Self {
        self.backoff_policy = Some(policy);
        self
    }

    pub fn backoff_policy(&self) -> Option<&crate::backoff::BackoffPolicy> {
        self.backoff_policy.as_ref()
    }

    pub fn with_capacity(self, capacity: usize) -> Self {
        if let Ok(mut cache) = self.response_cache.lock() {
            cache.reserve(capacity);
        }
        if let Ok(mut queue) = self.cache_queue.lock() {
            queue.reserve(capacity);
        }
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.evict_if_needed();
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

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let (cache_overhead, heap_content) = if let Ok(cache) = self.response_cache.lock() {
            let overhead = cache.capacity()
                * (std::mem::size_of::<String>() + std::mem::size_of::<EngineOutput>());
            let content: usize = cache
                .iter()
                .map(|(k, v)| k.len() + v.stdout_content.len() + std::mem::size_of::<EngineOutput>())
                .sum();
            (overhead, content)
        } else {
            (0, 0)
        };
        let queue_overhead = if let Ok(queue) = self.cache_queue.lock() {
            queue.capacity() * std::mem::size_of::<String>()
        } else {
            0
        };
        base_size + cache_overhead + queue_overhead + heap_content
    }

    fn evict_if_needed(&self) {
        if let Some(max) = self.max_capacity {
            if let (Ok(mut cache), Ok(mut queue)) =
                (self.response_cache.lock(), self.cache_queue.lock())
            {
                while queue.len() >= max && max > 0 {
                    if let Some(old_key) = queue.pop_front() {
                        cache.remove(&old_key);
                    }
                }
            }
        }
    }

    pub fn cache_response(&self, key: &str, output: &EngineOutput) {
        if let Some(max) = self.max_capacity {
            if max == 0 {
                return;
            }
            if let (Ok(mut cache), Ok(mut queue)) =
                (self.response_cache.lock(), self.cache_queue.lock())
            {
                while queue.len() >= max {
                    if let Some(old_key) = queue.pop_front() {
                        cache.remove(&old_key);
                    }
                }
                if !cache.contains_key(key) {
                    queue.push_back(key.to_string());
                }
                cache.insert(key.to_string(), output.clone());
            }
        }
    }

    pub fn get_cached(&self, key: &str) -> Option<EngineOutput> {
        self.response_cache.lock().ok()?.get(key).cloned()
    }
}

impl<E: ComputeEngine> ResilientApiEngine<E> {
    async fn execute_with_optional_ledger(
        &self,
        input: &EngineInput,
        mut ledger: Option<&mut AuditLedger>,
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
                            if let Some(ref mut l) = ledger {
                                l.record_event(
                                    AuditEventType::CommandAllowed,
                                    ActionSeverity::Routine,
                                    format!(
                                        "API transient fault {:?} intercepted, retrying attempt {}/{} for engine '{}'",
                                        fault, attempt, self.max_retries, self.engine_kind()
                                    ),
                                );
                            }
                            if let Some(ref policy) = self.backoff_policy {
                                policy.sleep_for_attempt(attempt as u32).await;
                            } else if self.retry_delay_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                            }
                            continue;
                        } else {
                            self.rejected_count.fetch_add(1, Ordering::SeqCst);
                            let err = fault.to_engine_error();
                            if let Some(ref mut l) = ledger {
                                l.record_event(
                                    AuditEventType::CommandException,
                                    ActionSeverity::HighImpact,
                                    format!(
                                        "API max retries exceeded for engine '{}': {}",
                                        self.engine_kind(),
                                        err
                                    ),
                                );
                            }
                            return Err(err);
                        }
                    }
                    ApiFault::CorruptedResponse => {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        let err = fault.to_engine_error();
                        if let Some(ref mut l) = ledger {
                            l.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "Corrupted API response rejected for engine '{}': {}",
                                    self.engine_kind(),
                                    err
                                ),
                            );
                        }
                        return Err(err);
                    }
                }
            }

            match self.inner.execute(input).await {
                Ok(output) => {
                    self.resolved_count.fetch_add(1, Ordering::SeqCst);
                    self.cache_response(&input.prompt_or_command, &output);
                    if let Some(ref mut l) = ledger {
                        l.record_event(
                            AuditEventType::CommandAllowed,
                            ActionSeverity::Routine,
                            format!("API execution succeeded with engine '{}'", self.engine_kind()),
                        );
                    }
                    return Ok(output);
                }
                Err(err) => {
                    if is_retryable_engine_error(&err) && attempt <= self.max_retries {
                        self.recovered_count.fetch_add(1, Ordering::SeqCst);
                        if let Some(ref mut l) = ledger {
                            l.record_event(
                                AuditEventType::CommandAllowed,
                                ActionSeverity::Routine,
                                format!(
                                    "API error {:?} intercepted, retrying attempt {}/{} for engine '{}'",
                                    err, attempt, self.max_retries, self.engine_kind()
                                ),
                            );
                        }
                        if let Some(ref policy) = self.backoff_policy {
                            policy.sleep_for_attempt(attempt as u32).await;
                        } else if self.retry_delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                        }
                        continue;
                    } else {
                        self.rejected_count.fetch_add(1, Ordering::SeqCst);
                        if let Some(ref mut l) = ledger {
                            l.record_event(
                                AuditEventType::CommandException,
                                ActionSeverity::HighImpact,
                                format!(
                                    "API execution failed for engine '{}': {}",
                                    self.engine_kind(),
                                    err
                                ),
                            );
                        }
                        return Err(err);
                    }
                }
            }
        }
    }

    pub async fn execute_batch(
        &self,
        inputs: &[EngineInput],
    ) -> Vec<Result<EngineOutput, EngineError>> {
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            results.push(self.execute(input).await);
        }
        results
    }

    pub async fn execute_and_record(
        &self,
        ledger: &mut AuditLedger,
        input: &EngineInput,
    ) -> Result<EngineOutput, EngineError> {
        self.execute_with_optional_ledger(input, Some(ledger)).await
    }
}

#[async_trait]
impl<E: ComputeEngine> ComputeEngine for ResilientApiEngine<E> {
    async fn execute(&self, input: &EngineInput) -> Result<EngineOutput, EngineError> {
        self.execute_with_optional_ledger(input, None).await
    }

    fn engine_kind(&self) -> EngineKind {
        self.inner.engine_kind()
    }
}
