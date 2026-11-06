use crate::fault::{CoreExecutionError, CoreFaultPolicy};
use crate::resilience::{CoreExecutionResult, ResilientCoreAgent};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

pub const SHARD_COUNT: usize = 16;

/// 16-way concurrency sharded core agent runner for ultra-high throughput
/// and zero-lock-contention across embedded tasks.
pub struct ShardedCoreAgentRunner {
    shards: Vec<Arc<Mutex<ResilientCoreAgent>>>,
}

impl Default for ShardedCoreAgentRunner {
    fn default() -> Self {
        Self::new(5000)
    }
}

impl ShardedCoreAgentRunner {
    pub fn new(capacity_per_shard: usize) -> Self {
        let mut shards = Vec::with_capacity(SHARD_COUNT);
        for _ in 0..SHARD_COUNT {
            let agent = ResilientCoreAgent::new().with_max_capacity(capacity_per_shard);
            shards.push(Arc::new(Mutex::new(agent)));
        }
        Self { shards }
    }

    pub fn with_shared_fault_policy(policy: Arc<CoreFaultPolicy>, capacity_per_shard: usize) -> Self {
        let mut shards = Vec::with_capacity(SHARD_COUNT);
        for _ in 0..SHARD_COUNT {
            let agent = ResilientCoreAgent::new()
                .with_max_capacity(capacity_per_shard)
                .with_fault_policy(Arc::clone(&policy));
            shards.push(Arc::new(Mutex::new(agent)));
        }
        Self { shards }
    }

    fn select_shard(&self, key: &str) -> &Arc<Mutex<ResilientCoreAgent>> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let idx = (hasher.finish() as usize) % SHARD_COUNT;
        &self.shards[idx]
    }

    /// Executes an operation on the dedicated partition shard.
    pub fn execute(
        &self,
        name: &str,
        payload: &str,
    ) -> Result<CoreExecutionResult, CoreExecutionError> {
        let shard = self.select_shard(name);
        let lock = match shard.lock() {
            Ok(guard) => guard,
            Err(p) => p.into_inner(),
        };
        lock.execute_operation(name, payload)
    }

    /// Executes an operation with automatic retry on its partition shard.
    pub fn execute_with_retry(
        &self,
        name: &str,
        payload: &str,
        max_retries: usize,
    ) -> Result<CoreExecutionResult, CoreExecutionError> {
        let shard = self.select_shard(name);
        let lock = match shard.lock() {
            Ok(guard) => guard,
            Err(p) => p.into_inner(),
        };
        lock.execute_with_retry(name, payload, max_retries)
    }

    /// Executes a batch across shards in parallel/sequentially.
    pub fn execute_batch(
        &self,
        ops: Vec<(&str, &str)>,
    ) -> Vec<Result<CoreExecutionResult, CoreExecutionError>> {
        ops.into_iter()
            .map(|(name, payload)| self.execute(name, payload))
            .collect()
    }

    pub fn total_memory_usage(&self) -> usize {
        self.shards
            .iter()
            .map(|s| {
                let lock = match s.lock() {
                    Ok(guard) => guard,
                    Err(p) => p.into_inner(),
                };
                lock.memory_usage_bytes()
            })
            .sum()
    }

    pub fn total_record_count(&self) -> usize {
        self.shards
            .iter()
            .map(|s| {
                let lock = match s.lock() {
                    Ok(guard) => guard,
                    Err(p) => p.into_inner(),
                };
                lock.record_count()
            })
            .sum()
    }

    pub fn clear_all(&self) {
        for s in &self.shards {
            let lock = match s.lock() {
                Ok(guard) => guard,
                Err(p) => p.into_inner(),
            };
            lock.clear();
        }
    }
}
