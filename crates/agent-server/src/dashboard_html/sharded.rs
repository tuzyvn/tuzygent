use axum::http::StatusCode;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use super::cache::{DashboardCacheStats, ShardedDashboardCache};
use super::service::ResilientDashboardService;
use super::types::{DashboardContext, DashboardError, DashboardFaultPolicy};

/// 16-way sharded resilient dashboard service eliminating lock contention under concurrent load.
#[derive(Clone)]
pub struct ShardedResilientDashboardService {
    shards: Arc<Vec<RwLock<ResilientDashboardService>>>,
    num_shards: usize,
    total_capacity: Option<usize>,
    cache: Option<Arc<ShardedDashboardCache>>,
}

impl ShardedResilientDashboardService {
    pub fn new(num_shards: usize) -> Self {
        let shards_count = num_shards.max(1);
        let mut shards = Vec::with_capacity(shards_count);
        for _ in 0..shards_count {
            shards.push(RwLock::new(ResilientDashboardService::new()));
        }
        Self {
            shards: Arc::new(shards),
            num_shards: shards_count,
            total_capacity: None,
            cache: None,
        }
    }

    pub fn with_default_shards() -> Self {
        Self::new(16)
    }

    pub fn with_fault_policy(self, policy: DashboardFaultPolicy) -> Self {
        for shard in self.shards.iter() {
            if let Ok(mut svc) = shard.try_write() {
                *svc = svc.clone().with_fault_policy(policy.clone());
            }
        }
        self
    }

    pub fn with_max_retries(self, max: usize) -> Self {
        for shard in self.shards.iter() {
            if let Ok(mut svc) = shard.try_write() {
                *svc = svc.clone().with_max_retries(max);
            }
        }
        self
    }

    pub fn with_backoff(self, backoff: Duration) -> Self {
        for shard in self.shards.iter() {
            if let Ok(mut svc) = shard.try_write() {
                *svc = svc.clone().with_backoff(backoff);
            }
        }
        self
    }

    pub fn with_custom_asset_dir(self, dir: impl Into<PathBuf> + Clone) -> Self {
        for shard in self.shards.iter() {
            if let Ok(mut svc) = shard.try_write() {
                *svc = svc.clone().with_custom_asset_dir(dir.clone());
            }
        }
        self
    }

    pub fn with_cache(mut self, capacity: usize) -> Self {
        let cache = Arc::new(ShardedDashboardCache::new(self.num_shards, capacity));
        self.cache = Some(cache.clone());
        for shard in self.shards.iter() {
            if let Ok(mut svc) = shard.try_write() {
                *svc = svc.clone().with_shared_cache(cache.clone());
            }
        }
        self
    }

    pub fn with_shared_cache(mut self, cache: Arc<ShardedDashboardCache>) -> Self {
        self.cache = Some(cache.clone());
        for shard in self.shards.iter() {
            if let Ok(mut svc) = shard.try_write() {
                *svc = svc.clone().with_shared_cache(cache.clone());
            }
        }
        self
    }

    pub fn with_max_capacity(mut self, total_cap: usize) -> Self {
        self.total_capacity = Some(total_cap);
        let per_shard = (total_cap / self.num_shards).max(1);
        for shard in self.shards.iter() {
            if let Ok(mut svc) = shard.try_write() {
                *svc = svc.clone().with_max_capacity(per_shard);
            }
        }
        self
    }

    pub fn shard_idx(&self, key: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn shard(&self, idx: usize) -> &RwLock<ResilientDashboardService> {
        &self.shards[idx % self.num_shards]
    }

    pub fn shard_for(&self, key: &str) -> &RwLock<ResilientDashboardService> {
        self.shard(self.shard_idx(key))
    }

    pub fn num_shards(&self) -> usize {
        self.num_shards
    }

    pub fn total_capacity(&self) -> Option<usize> {
        self.total_capacity
    }

    pub fn cache(&self) -> Option<&Arc<ShardedDashboardCache>> {
        self.cache.as_ref()
    }

    pub fn cache_stats(&self) -> Option<DashboardCacheStats> {
        self.cache.as_ref().map(|c| c.stats())
    }

    pub async fn memory_usage_bytes(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for shard in self.shards.iter() {
            let guard = shard.read().await;
            total += guard.memory_usage_bytes();
        }
        if let Some(c) = &self.cache {
            total += c.memory_usage_bytes();
        }
        total
    }

    pub async fn render_dashboard(
        &self,
        key: &str,
        ctx: &DashboardContext,
    ) -> Result<String, DashboardError> {
        let shard = self.shard_for(key);
        let guard = shard.read().await;
        guard.render_dashboard(ctx).await
    }

    pub async fn render_dashboard_cached(
        &self,
        key: &str,
        ctx: &DashboardContext,
    ) -> Result<String, DashboardError> {
        if let Some(cache) = &self.cache {
            if let Some(cached) = cache.get(key) {
                return Ok(cached);
            }
        }
        let shard = self.shard_for(key);
        let guard = shard.read().await;
        let rendered = guard.render_dashboard(ctx).await?;
        if let Some(cache) = &self.cache {
            cache.insert(key.to_string(), rendered.clone());
        }
        Ok(rendered)
    }

    pub async fn render_dashboard_with_fallback(
        &self,
        key: &str,
        ctx: &DashboardContext,
    ) -> (String, StatusCode) {
        let shard = self.shard_for(key);
        let guard = shard.read().await;
        guard.render_dashboard_with_fallback(ctx).await
    }

    pub async fn get_asset(&self, name: &str) -> (Vec<u8>, &'static str, StatusCode) {
        let shard = self.shard_for(name);
        let guard = shard.read().await;
        guard.get_asset(name).await
    }
}

#[cfg(test)]
#[path = "sharded_tests.rs"]
mod tests;
