use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_engine::AccountEngineKind;

use super::buffer::{GatewayBuffer, GatewayRingBuffer, ShardedGatewayRingBuffer};
use super::cache::{GatewayCacheStats, ShardedGatewayCache};
use super::fault::{GatewayError, GatewayFaultPolicy};
use super::types::parse_model_spec;

#[derive(Clone)]
pub struct ResilientGatewayService {
    max_retries: usize,
    backoff: Duration,
    fault_policy: Arc<Mutex<GatewayFaultPolicy>>,
    has_active_fault: Arc<AtomicBool>,
    fallback_model: Option<String>,
    recovered_count: Arc<AtomicUsize>,
    retries_attempted: Arc<AtomicUsize>,
    resolved_count: Arc<AtomicUsize>,
    rejected_count: Arc<AtomicUsize>,
    max_capacity: Option<usize>,
    history: Option<GatewayBuffer>,
    cache: Option<Arc<ShardedGatewayCache>>,
}

impl Default for ResilientGatewayService {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff: Duration::from_millis(5),
            fault_policy: Arc::new(Mutex::new(GatewayFaultPolicy::none())),
            has_active_fault: Arc::new(AtomicBool::new(false)),
            fallback_model: None,
            recovered_count: Arc::new(AtomicUsize::new(0)),
            retries_attempted: Arc::new(AtomicUsize::new(0)),
            resolved_count: Arc::new(AtomicUsize::new(0)),
            rejected_count: Arc::new(AtomicUsize::new(0)),
            max_capacity: None,
            history: None,
            cache: None,
        }
    }
}

impl ResilientGatewayService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_retries(mut self, max: usize) -> Self {
        self.max_retries = max;
        self
    }

    pub fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    pub fn with_fault_policy(mut self, policy: GatewayFaultPolicy) -> Self {
        let active = policy.has_active_fault();
        self.has_active_fault = Arc::new(AtomicBool::new(active));
        self.fault_policy = Arc::new(Mutex::new(policy));
        self
    }

    pub fn with_fallback_model(mut self, fallback: impl Into<String>) -> Self {
        self.fallback_model = Some(fallback.into());
        self
    }

    pub fn with_max_capacity(mut self, max: usize) -> Self {
        self.max_capacity = Some(max);
        self.history = Some(GatewayBuffer::single(max));
        self
    }

    pub fn with_history_buffer(mut self, buffer: Arc<GatewayRingBuffer>) -> Self {
        self.max_capacity = Some(buffer.total_capacity());
        self.history = Some(GatewayBuffer::Single(buffer));
        self
    }

    pub fn with_sharded_history_buffer(mut self, buffer: Arc<ShardedGatewayRingBuffer>) -> Self {
        self.max_capacity = Some(buffer.total_capacity());
        self.history = Some(GatewayBuffer::Sharded(buffer));
        self
    }

    pub fn with_buffer(mut self, buffer: GatewayBuffer) -> Self {
        self.max_capacity = Some(buffer.total_capacity());
        self.history = Some(buffer);
        self
    }

    pub fn with_cache(mut self, capacity: usize) -> Self {
        self.cache = Some(Arc::new(ShardedGatewayCache::with_default_shards(capacity)));
        self
    }

    pub fn with_shared_cache(mut self, cache: Arc<ShardedGatewayCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.max_capacity
    }

    pub fn history_buffer(&self) -> Option<Arc<GatewayRingBuffer>> {
        self.history.as_ref().and_then(|h| h.as_single().cloned())
    }

    pub fn sharded_history_buffer(&self) -> Option<Arc<ShardedGatewayRingBuffer>> {
        self.history.as_ref().and_then(|h| h.as_sharded().cloned())
    }

    pub fn buffer(&self) -> Option<&GatewayBuffer> {
        self.history.as_ref()
    }

    pub fn cache(&self) -> Option<&Arc<ShardedGatewayCache>> {
        self.cache.as_ref()
    }

    pub fn cache_stats(&self) -> Option<GatewayCacheStats> {
        self.cache.as_ref().map(|c| c.stats())
    }

    pub fn fault_policy(&self) -> Arc<Mutex<GatewayFaultPolicy>> {
        self.fault_policy.clone()
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count.load(Ordering::SeqCst)
    }

    pub fn retries_attempted(&self) -> usize {
        self.retries_attempted.load(Ordering::SeqCst)
    }

    pub fn resolved_count(&self) -> usize {
        self.resolved_count.load(Ordering::SeqCst)
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count.load(Ordering::SeqCst)
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let history_mem = self.history.as_ref().map_or(0, |h| h.memory_usage_bytes());
        let cache_mem = self.cache.as_ref().map_or(0, |c| c.memory_usage_bytes());
        base + history_mem + cache_mem
    }

    pub fn check_fault_injection(&self) -> Result<(), GatewayError> {
        if !self.has_active_fault.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut policy = self.fault_policy.lock().map_err(|_| {
            GatewayError::IoError("Failed to acquire gateway fault policy lock".to_string())
        })?;
        if let Some(fault) = policy.should_inject() {
            Err(fault.to_gateway_error())
        } else {
            Ok(())
        }
    }

    pub fn set_fault_policy(&self, policy: GatewayFaultPolicy) {
        let active = policy.has_active_fault();
        if let Ok(mut guard) = self.fault_policy.lock() {
            *guard = policy;
            self.has_active_fault.store(active, Ordering::SeqCst);
        }
    }

    pub fn resolve_model_resilient(
        &self,
        model_str: &str,
    ) -> Result<(AccountEngineKind, Option<String>), GatewayError> {
        self.resolve_model_resilient_internal(model_str, true)
    }

    fn resolve_model_resilient_internal(
        &self,
        model_str: &str,
        record_history: bool,
    ) -> Result<(AccountEngineKind, Option<String>), GatewayError> {
        if record_history {
            if let Some(ref hist) = self.history {
                hist.push_keyed(model_str, model_str);
            }
        }

        if let Err(err) = self.check_fault_injection() {
            self.rejected_count.fetch_add(1, Ordering::SeqCst);
            return Err(err);
        }

        match parse_model_spec(model_str) {
            Ok(resolved) => {
                self.resolved_count.fetch_add(1, Ordering::SeqCst);
                Ok(resolved)
            }
            Err(primary_err) => {
                if let Some(ref fallback) = self.fallback_model {
                    if let Ok(resolved_fallback) = parse_model_spec(fallback) {
                        self.recovered_count.fetch_add(1, Ordering::SeqCst);
                        self.resolved_count.fetch_add(1, Ordering::SeqCst);
                        return Ok(resolved_fallback);
                    }
                }
                self.rejected_count.fetch_add(1, Ordering::SeqCst);
                Err(GatewayError::InvalidModel(primary_err))
            }
        }
    }

    pub fn resolve_model_cached(
        &self,
        model_str: &str,
    ) -> Result<(AccountEngineKind, Option<String>), GatewayError> {
        self.resolve_model_cached_internal(model_str, true)
    }

    fn resolve_model_cached_internal(
        &self,
        model_str: &str,
        record_history: bool,
    ) -> Result<(AccountEngineKind, Option<String>), GatewayError> {
        if let Some(ref cache) = self.cache {
            if let Some(cached) = cache.get(model_str) {
                self.resolved_count.fetch_add(1, Ordering::SeqCst);
                if record_history {
                    if let Some(ref hist) = self.history {
                        hist.push_keyed(model_str, model_str);
                    }
                }
                return Ok(cached);
            }
        }
        let resolved = self.resolve_model_resilient_internal(model_str, record_history)?;
        if let Some(ref cache) = self.cache {
            cache.insert(model_str.to_string(), resolved.clone());
        }
        Ok(resolved)
    }

    pub fn batch_resolve_models(
        &self,
        models: &[&str],
    ) -> Vec<Result<(AccountEngineKind, Option<String>), GatewayError>> {
        if let Some(ref hist) = self.history {
            hist.push_batch(models);
        }
        models
            .iter()
            .map(|&m| self.resolve_model_cached_internal(m, false))
            .collect()
    }

    pub async fn execute_async_resilient<F, Fut, T>(&self, mut op: F) -> Result<T, GatewayError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, GatewayError>>,
    {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let result = match self.check_fault_injection() {
                Err(err) => Err(err),
                Ok(()) => op().await,
            };
            match result {
                Ok(val) => {
                    if attempts > 1 {
                        self.recovered_count.fetch_add(1, Ordering::SeqCst);
                    }
                    return Ok(val);
                }
                Err(err) => {
                    if !err.is_retryable() {
                        return Err(err);
                    }
                    if attempts > self.max_retries {
                        return Err(GatewayError::MaxRetriesExceeded(self.max_retries));
                    }
                    self.retries_attempted.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(self.backoff * (attempts as u32)).await;
                }
            }
        }
    }

    pub fn execute_sync_resilient<F, T>(&self, mut op: F) -> Result<T, GatewayError>
    where
        F: FnMut() -> Result<T, GatewayError>,
    {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let result = match self.check_fault_injection() {
                Err(err) => Err(err),
                Ok(()) => op(),
            };
            match result {
                Ok(val) => {
                    if attempts > 1 {
                        self.recovered_count.fetch_add(1, Ordering::SeqCst);
                    }
                    return Ok(val);
                }
                Err(err) => {
                    if !err.is_retryable() {
                        return Err(err);
                    }
                    if attempts > self.max_retries {
                        return Err(GatewayError::MaxRetriesExceeded(self.max_retries));
                    }
                    self.retries_attempted.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(self.backoff * (attempts as u32));
                }
            }
        }
    }
}
