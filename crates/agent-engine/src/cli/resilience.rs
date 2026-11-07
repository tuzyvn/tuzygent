use crate::cli::fault::CliFaultPolicy;
use crate::traits::EngineOutput;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Wrapper tự phục hồi cho ComputeEngine CLI với cơ chế retry có kiểm soát, bounded memory cache và injection fault testing
#[derive(Clone)]
pub struct ResilientCliEngine<E> {
    pub(super) inner: E,
    pub(super) fault_policy: Arc<Mutex<CliFaultPolicy>>,
    pub(super) max_retries: usize,
    pub(super) retry_delay_ms: u64,
    pub(super) resolved_count: Arc<AtomicUsize>,
    pub(super) recovered_count: Arc<AtomicUsize>,
    pub(super) rejected_count: Arc<AtomicUsize>,
    pub(super) response_cache: Arc<Mutex<HashMap<String, EngineOutput>>>,
    pub(super) cache_queue: Arc<Mutex<VecDeque<String>>>,
    pub(super) max_capacity: Option<usize>,
    pub(super) backoff_policy: Option<crate::backoff::BackoffPolicy>,
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
            response_cache: Arc::new(Mutex::new(HashMap::new())),
            cache_queue: Arc::new(Mutex::new(VecDeque::new())),
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

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let (cache_overhead, heap_content) = if let Ok(cache) = self.response_cache.lock() {
            let overhead = cache.capacity()
                * (std::mem::size_of::<String>() + std::mem::size_of::<EngineOutput>());
            let content: usize = cache
                .iter()
                .map(|(k, v)| {
                    k.len() + v.stdout_content.len() + std::mem::size_of::<EngineOutput>()
                })
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
