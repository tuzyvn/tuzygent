use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use crate::fault::{CliExecutionError, CliFaultPolicy};

use super::resilience::ResilientUpdater;

#[derive(Clone)]
pub struct ShardedResilientUpdater {
    shards: Arc<Vec<tokio::sync::Mutex<ResilientUpdater>>>,
    num_shards: usize,
    total_capacity: Option<usize>,
}

impl ShardedResilientUpdater {
    pub fn new(policy: CliFaultPolicy, max_retries: usize, num_shards: usize) -> Self {
        let shards_count = num_shards.max(1);
        let mut shards = Vec::with_capacity(shards_count);
        for _ in 0..shards_count {
            shards.push(tokio::sync::Mutex::new(ResilientUpdater::with_policy(
                policy.clone(),
                max_retries,
            )));
        }
        Self {
            shards: Arc::new(shards),
            num_shards: shards_count,
            total_capacity: None,
        }
    }

    pub fn with_max_capacity(mut self, total_cap: usize) -> Self {
        self.total_capacity = Some(total_cap);
        let per_shard = (total_cap / self.num_shards).max(1);
        if let Some(shards) = Arc::get_mut(&mut self.shards) {
            for shard in shards.iter_mut() {
                let updater = shard.get_mut();
                *updater = updater.clone().with_max_capacity(per_shard);
            }
        }
        self
    }

    pub fn with_audit_workspace(mut self, path: impl AsRef<Path>) -> Self {
        let p = path.as_ref();
        if let Some(shards) = Arc::get_mut(&mut self.shards) {
            for shard in shards.iter_mut() {
                let updater = shard.get_mut();
                *updater = updater.clone().with_audit_workspace(p);
            }
        }
        self
    }

    pub fn shard_idx(&self, key: &str) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn shard(&self, idx: usize) -> &tokio::sync::Mutex<ResilientUpdater> {
        &self.shards[idx % self.num_shards]
    }

    pub fn shard_for(&self, key: &str) -> &tokio::sync::Mutex<ResilientUpdater> {
        self.shard(self.shard_idx(key))
    }

    pub fn num_shards(&self) -> usize {
        self.num_shards
    }

    pub fn total_capacity(&self) -> Option<usize> {
        self.total_capacity
    }

    pub async fn execute_operation<F, Fut, T>(
        &self,
        action_name: &str,
        op: F,
    ) -> Result<T, CliExecutionError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, CliExecutionError>>,
    {
        let shard = self.shard_for(action_name);
        let mut guard = shard.lock().await;
        guard.execute_operation(action_name, op).await
    }

    pub async fn total_executed(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.total_executed();
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

    pub async fn failed_count(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.failed_count();
        }
        total
    }

    pub async fn panics_caught_count(&self) -> usize {
        let mut total = 0;
        for shard in self.shards.iter() {
            total += shard.lock().await.panics_caught_count();
        }
        total
    }

    pub async fn memory_usage_bytes(&self) -> usize {
        let mut total = std::mem::size_of::<Self>();
        for shard in self.shards.iter() {
            total += shard.lock().await.memory_usage_bytes();
        }
        total
    }

    pub async fn reset_counts(&self) {
        for shard in self.shards.iter() {
            shard.lock().await.reset_counts();
        }
    }
}
