use agent_engine::AccountEngineKind;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use super::buffer::{GatewayBuffer, GatewayRingBuffer, ShardedGatewayRingBuffer};
use super::cache::{GatewayCacheStats, ShardedGatewayCache};
use super::fault::{GatewayError, GatewayFaultPolicy};
use super::resilience::ResilientGatewayService;

/// 16-way sharded resilient gateway service eliminating lock contention under concurrent load.
#[derive(Clone)]
pub struct ShardedResilientGatewayService {
    shards: Arc<Vec<ResilientGatewayService>>,
    num_shards: usize,
    total_capacity: Option<usize>,
    history_buffer: Option<GatewayBuffer>,
    cache: Option<Arc<ShardedGatewayCache>>,
}

impl ShardedResilientGatewayService {
    pub fn new(num_shards: usize) -> Self {
        let shards_count = num_shards.max(1);
        let mut shards = Vec::with_capacity(shards_count);
        for _ in 0..shards_count {
            shards.push(ResilientGatewayService::new());
        }
        Self {
            shards: Arc::new(shards),
            num_shards: shards_count,
            total_capacity: None,
            history_buffer: None,
            cache: None,
        }
    }

    pub fn with_default_shards() -> Self {
        Self::new(16)
    }

    pub fn with_fault_policy(mut self, policy: GatewayFaultPolicy) -> Self {
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_fault_policy(policy.clone());
        }
        self
    }

    pub fn with_max_retries(mut self, max: usize) -> Self {
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_max_retries(max);
        }
        self
    }

    pub fn with_backoff(mut self, backoff: Duration) -> Self {
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_backoff(backoff);
        }
        self
    }

    pub fn with_fallback_model(mut self, fallback: impl Into<String> + Clone) -> Self {
        let fallback_str: String = fallback.into();
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_fallback_model(fallback_str.clone());
        }
        self
    }

    pub fn with_cache(mut self, capacity: usize) -> Self {
        let cache = Arc::new(ShardedGatewayCache::new(self.num_shards, capacity));
        self.cache = Some(cache.clone());
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_shared_cache(cache.clone());
        }
        self
    }

    pub fn with_shared_cache(mut self, cache: Arc<ShardedGatewayCache>) -> Self {
        self.cache = Some(cache.clone());
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_shared_cache(cache.clone());
        }
        self
    }

    pub fn with_max_capacity(mut self, total_cap: usize) -> Self {
        self.total_capacity = Some(total_cap);
        let per_shard = (total_cap / self.num_shards).max(1);
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_max_capacity(per_shard);
        }
        self
    }

    pub fn with_history_buffer(mut self, buffer: Arc<GatewayRingBuffer>) -> Self {
        self.total_capacity = Some(buffer.total_capacity());
        self.history_buffer = Some(GatewayBuffer::Single(buffer.clone()));
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_history_buffer(buffer.clone());
        }
        self
    }

    pub fn with_sharded_history_buffer(mut self, buffer: Arc<ShardedGatewayRingBuffer>) -> Self {
        self.total_capacity = Some(buffer.total_capacity());
        self.history_buffer = Some(GatewayBuffer::Sharded(buffer.clone()));
        let shards = Arc::make_mut(&mut self.shards);
        for shard in shards.iter_mut() {
            *shard = shard.clone().with_sharded_history_buffer(buffer.clone());
        }
        self
    }

    pub fn shard_idx(&self, key: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn shard(&self, idx: usize) -> &ResilientGatewayService {
        &self.shards[idx % self.num_shards]
    }

    pub fn shard_for(&self, key: &str) -> &ResilientGatewayService {
        self.shard(self.shard_idx(key))
    }

    pub fn num_shards(&self) -> usize {
        self.num_shards
    }

    pub fn total_capacity(&self) -> Option<usize> {
        self.total_capacity
    }

    pub fn history_buffer(&self) -> Option<Arc<GatewayRingBuffer>> {
        self.history_buffer
            .as_ref()
            .and_then(|h| h.as_single().cloned())
    }

    pub fn sharded_history_buffer(&self) -> Option<Arc<ShardedGatewayRingBuffer>> {
        self.history_buffer
            .as_ref()
            .and_then(|h| h.as_sharded().cloned())
    }

    pub fn buffer(&self) -> Option<&GatewayBuffer> {
        self.history_buffer.as_ref()
    }

    pub fn cache(&self) -> Option<&Arc<ShardedGatewayCache>> {
        self.cache.as_ref()
    }

    pub fn cache_stats(&self) -> Option<GatewayCacheStats> {
        self.cache.as_ref().map(|c| c.stats())
    }

    pub async fn memory_usage_bytes(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for shard in self.shards.iter() {
            total += shard.memory_usage_bytes();
        }
        if let Some(c) = &self.cache {
            total += c.memory_usage_bytes();
        }
        if let Some(h) = &self.history_buffer {
            total += h.memory_usage_bytes();
        }
        total
    }

    pub async fn resolve_model_resilient(
        &self,
        model_str: &str,
    ) -> Result<(AccountEngineKind, Option<String>), GatewayError> {
        self.shard_for(model_str).resolve_model_resilient(model_str)
    }

    pub async fn resolve_model_cached(
        &self,
        model_str: &str,
    ) -> Result<(AccountEngineKind, Option<String>), GatewayError> {
        if let Some(cache) = &self.cache {
            if let Some(cached) = cache.get(model_str) {
                return Ok(cached);
            }
        }
        let resolved = self
            .shard_for(model_str)
            .resolve_model_resilient(model_str)?;
        if let Some(cache) = &self.cache {
            cache.insert(model_str.to_string(), resolved.clone());
        }
        Ok(resolved)
    }

    pub async fn batch_resolve_models(
        &self,
        models: &[&str],
    ) -> Vec<Result<(AccountEngineKind, Option<String>), GatewayError>> {
        let mut results = Vec::with_capacity(models.len());
        for &model in models {
            results.push(self.resolve_model_cached(model).await);
        }
        results
    }

    pub async fn execute_async_resilient<F, Fut, T>(
        &self,
        key: &str,
        op: F,
    ) -> Result<T, GatewayError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, GatewayError>>,
    {
        self.shard_for(key).execute_async_resilient(op).await
    }
}
