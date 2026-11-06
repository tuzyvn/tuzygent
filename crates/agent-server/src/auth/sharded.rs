use axum::http::HeaderValue;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::cache::{AuthCacheStats, ShardedAuthCache};
use super::fault::AuthFaultPolicy;
use super::types::AuthError;
use super::validator::ResilientAuthValidator;

/// 16-way sharded resilient validator eliminating lock contention under concurrent load.
#[derive(Clone)]
pub struct ShardedResilientAuthValidator {
    shards: Arc<Vec<Mutex<ResilientAuthValidator>>>,
    num_shards: usize,
    total_capacity: Option<usize>,
    cache: Option<Arc<ShardedAuthCache>>,
}

impl ShardedResilientAuthValidator {
    pub fn new(
        token: Arc<String>,
        allowed_origins: Vec<HeaderValue>,
        policy: AuthFaultPolicy,
        max_retries: usize,
        num_shards: usize,
    ) -> Self {
        let shards_count = num_shards.max(1);
        let mut shards = Vec::with_capacity(shards_count);
        for _ in 0..shards_count {
            shards.push(Mutex::new(ResilientAuthValidator::new(
                Arc::clone(&token),
                allowed_origins.clone(),
                policy.clone(),
                max_retries,
            )));
        }
        Self {
            shards: Arc::new(shards),
            num_shards: shards_count,
            total_capacity: None,
            cache: None,
        }
    }

    pub fn with_cache(mut self, capacity: usize) -> Self {
        self.cache = Some(Arc::new(ShardedAuthCache::new(self.num_shards, capacity)));
        self
    }

    pub fn with_shared_cache(mut self, cache: Arc<ShardedAuthCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn cache(&self) -> Option<&Arc<ShardedAuthCache>> {
        self.cache.as_ref()
    }

    pub fn cache_stats(&self) -> Option<AuthCacheStats> {
        self.cache.as_ref().map(|c| c.stats())
    }

    pub fn with_max_capacity(mut self, total_cap: usize) -> Self {
        self.total_capacity = Some(total_cap);
        let per_shard = (total_cap / self.num_shards).max(1);
        if let Some(shards) = Arc::get_mut(&mut self.shards) {
            for shard in shards.iter_mut() {
                let validator = shard.get_mut();
                *validator = validator.clone().with_max_capacity(per_shard);
            }
        }
        self
    }

    pub fn shard_idx(&self, key: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn shard(&self, idx: usize) -> &Mutex<ResilientAuthValidator> {
        &self.shards[idx % self.num_shards]
    }

    pub fn shard_for(&self, key: &str) -> &Mutex<ResilientAuthValidator> {
        self.shard(self.shard_idx(key))
    }

    pub fn num_shards(&self) -> usize {
        self.num_shards
    }

    pub fn total_capacity(&self) -> Option<usize> {
        self.total_capacity
    }

    pub async fn validate_token_resilient(&self, token: &str) -> Result<bool, AuthError> {
        let shard = self.shard_for(token);
        let mut guard = shard.lock().await;

        if let Some(ref cache) = self.cache {
            if !guard.fault_policy().has_active_fault() {
                if let Some(valid) = cache.get(token) {
                    if valid {
                        guard.increment_validated();
                    } else {
                        guard.increment_rejected();
                    }
                    return Ok(valid);
                }
            }
        }

        let res = guard.validate_token_resilient(token);
        if let (Ok(valid), Some(ref cache)) = (&res, &self.cache) {
            cache.insert(token, *valid);
        }
        res
    }

    pub async fn validate_batch_tokens(&self, tokens: &[&str]) -> Vec<Result<bool, AuthError>> {
        let mut results = Vec::with_capacity(tokens.len());
        for &token in tokens {
            results.push(self.validate_token_resilient(token).await);
        }
        results
    }

    pub async fn validated_count(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.validated_count();
        }
        total
    }

    pub async fn recovered_count(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.recovered_count();
        }
        total
    }

    pub async fn rejected_count(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.rejected_count();
        }
        total
    }

    pub async fn memory_usage_bytes(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for shard in self.shards.iter() {
            total += shard.lock().await.memory_usage_bytes();
        }
        if let Some(ref cache) = self.cache {
            total += cache.memory_usage_bytes();
        }
        total
    }

    pub async fn clear_cache(&self) {
        if let Some(ref cache) = self.cache {
            cache.clear();
        }
    }

    pub async fn reset_counts(&self) {
        for shard in self.shards.iter() {
            shard.lock().await.reset_counts();
        }
        if let Some(ref cache) = self.cache {
            cache.clear();
        }
    }
}
