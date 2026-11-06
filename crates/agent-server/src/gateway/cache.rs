use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;
use std::time::Instant;

use agent_engine::AccountEngineKind;

/// Individual cache entry holding resolved model engine and override.
#[derive(Debug)]
pub struct GatewayCacheEntry {
    pub resolved: (AccountEngineKind, Option<String>),
    pub cached_at: Instant,
    pub hit_count: AtomicUsize,
}

impl Clone for GatewayCacheEntry {
    fn clone(&self) -> Self {
        Self {
            resolved: self.resolved.clone(),
            cached_at: self.cached_at,
            hit_count: AtomicUsize::new(self.hit_count.load(Ordering::Relaxed)),
        }
    }
}

/// Snapshot of gateway cache metrics across all shards.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GatewayCacheStats {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub total_entries: usize,
}

struct CacheShard {
    entries: HashMap<String, GatewayCacheEntry>,
    order: VecDeque<String>,
    capacity: usize,
}

impl CacheShard {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity.min(512)),
            order: VecDeque::with_capacity(capacity.min(512)),
            capacity,
        }
    }

    fn get(&self, key: &str) -> Option<(AccountEngineKind, Option<String>)> {
        self.entries.get(key).map(|entry| {
            entry.hit_count.fetch_add(1, Ordering::Relaxed);
            entry.resolved.clone()
        })
    }

    fn insert(&mut self, key: String, resolved: (AccountEngineKind, Option<String>)) -> bool {
        if self.capacity == 0 {
            return false;
        }
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.resolved = resolved;
            return false;
        }
        let mut evicted = false;
        while self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
                evicted = true;
            } else {
                break;
            }
        }
        self.order.push_back(key.clone());
        self.entries.insert(
            key,
            GatewayCacheEntry {
                resolved,
                cached_at: Instant::now(),
                hit_count: AtomicUsize::new(0),
            },
        );
        evicted
    }

    fn invalidate(&mut self, key: &str) -> bool {
        if self.entries.remove(key).is_some() {
            self.order.retain(|k| k != key);
            true
        } else {
            false
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn memory_usage_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let map_overhead = self.entries.capacity()
            * (std::mem::size_of::<String>() + std::mem::size_of::<GatewayCacheEntry>());
        let queue_overhead = self.order.capacity() * std::mem::size_of::<String>();
        let heap: usize = self
            .entries
            .iter()
            .map(|(k, v)| k.len() + v.resolved.1.as_ref().map(|s| s.len()).unwrap_or(0))
            .sum();
        base + map_overhead + queue_overhead + heap
    }
}

/// Sharded concurrent gateway model cache designed for ultra-low lock contention under 10k+ requests.
pub struct ShardedGatewayCache {
    shards: Vec<RwLock<CacheShard>>,
    num_shards: usize,
    capacity_per_shard: usize,
    hits: AtomicUsize,
    misses: AtomicUsize,
    evictions: AtomicUsize,
}

pub type ConcurrentGatewayCache = ShardedGatewayCache;

impl ShardedGatewayCache {
    pub fn new(num_shards: usize, total_capacity: usize) -> Self {
        let num_shards = num_shards.max(1);
        let capacity_per_shard = (total_capacity / num_shards).max(1);
        let mut shards = Vec::with_capacity(num_shards);
        for _ in 0..num_shards {
            shards.push(RwLock::new(CacheShard::new(capacity_per_shard)));
        }
        Self {
            shards,
            num_shards,
            capacity_per_shard,
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            evictions: AtomicUsize::new(0),
        }
    }

    pub fn with_default_shards(total_capacity: usize) -> Self {
        Self::new(16, total_capacity)
    }

    fn shard_idx(&self, key: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn get(&self, key: &str) -> Option<(AccountEngineKind, Option<String>)> {
        let idx = self.shard_idx(key);
        let res = match self.shards[idx].read() {
            Ok(shard) => shard.get(key),
            Err(poisoned) => poisoned.into_inner().get(key),
        };
        if res.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        res
    }

    pub fn insert(&self, key: String, resolved: (AccountEngineKind, Option<String>)) {
        let idx = self.shard_idx(&key);
        let evicted = match self.shards[idx].write() {
            Ok(mut shard) => shard.insert(key, resolved),
            Err(poisoned) => poisoned.into_inner().insert(key, resolved),
        };
        if evicted {
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn invalidate(&self, key: &str) -> bool {
        let idx = self.shard_idx(key);
        match self.shards[idx].write() {
            Ok(mut shard) => shard.invalidate(key),
            Err(poisoned) => poisoned.into_inner().invalidate(key),
        }
    }

    pub fn clear(&self) {
        for shard in &self.shards {
            match shard.write() {
                Ok(mut s) => s.clear(),
                Err(poisoned) => poisoned.into_inner().clear(),
            }
        }
    }

    pub fn stats(&self) -> GatewayCacheStats {
        let total_entries = self.total_entries();
        GatewayCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            total_entries,
        }
    }

    pub fn total_entries(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| match shard.read() {
                Ok(s) => s.len(),
                Err(poisoned) => poisoned.into_inner().len(),
            })
            .sum()
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let shards_mem: usize = self
            .shards
            .iter()
            .map(|s| match s.read() {
                Ok(shard) => shard.memory_usage_bytes(),
                Err(p) => p.into_inner().memory_usage_bytes(),
            })
            .sum();
        base + shards_mem
    }

    pub fn num_shards(&self) -> usize {
        self.num_shards
    }

    pub fn capacity_per_shard(&self) -> usize {
        self.capacity_per_shard
    }
}
