use super::ResilientCliEngine;
use crate::cli::fault::CliFaultPolicy;
use std::sync::{Arc, Mutex};

impl<E> ResilientCliEngine<E> {
    pub fn new(inner: E, fault_policy: CliFaultPolicy, max_retries: usize) -> Self {
        Self {
            inner,
            fault_policy: Arc::new(Mutex::new(fault_policy)),
            max_retries,
            retry_delay_ms: 0,
            resolved_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            recovered_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            rejected_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            response_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
            cache_queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            max_capacity: None,
            backoff_policy: None,
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

    pub fn with_backoff_policy(mut self, policy: crate::backoff::BackoffPolicy) -> Self {
        self.backoff_policy = Some(policy);
        self
    }

    pub fn backoff_policy(&self) -> Option<&crate::backoff::BackoffPolicy> {
        self.backoff_policy.as_ref()
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
}
