use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;
use std::time::Instant;

/// Individual cache entry holding validation outcome and access statistics.
#[derive(Debug)]
pub struct AuthCacheEntry {
    pub is_valid: bool,
    pub cached_at: Instant,
    pub hit_count: AtomicUsize,
}

impl Clone for AuthCacheEntry {
    fn clone(&self) -> Self {
        Self {
            is_valid: self.is_valid,
            cached_at: self.cached_at,
            hit_count: AtomicUsize::new(self.hit_count.load(Ordering::Relaxed)),
        }
    }
}

/// Snapshot of auth cache metrics across all shards.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AuthCacheStats {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub total_entries: usize,
}

struct CacheShard {
    entries: HashMap<String, AuthCacheEntry>,
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

    fn get(&self, token: &str) -> Option<bool> {
        self.entries.get(token).map(|entry| {
            entry.hit_count.fetch_add(1, Ordering::Relaxed);
            entry.is_valid
        })
    }

    fn insert(&mut self, token: String, is_valid: bool) -> bool {
        if self.capacity == 0 {
            return false;
        }
        if let Some(entry) = self.entries.get_mut(&token) {
            entry.is_valid = is_valid;
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
        self.order.push_back(token.clone());
        self.entries.insert(
            token,
            AuthCacheEntry {
                is_valid,
                cached_at: Instant::now(),
                hit_count: AtomicUsize::new(0),
            },
        );
        evicted
    }

    fn invalidate(&mut self, token: &str) -> bool {
        if self.entries.remove(token).is_some() {
            self.order.retain(|k| k != token);
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
            * (std::mem::size_of::<String>() + std::mem::size_of::<AuthCacheEntry>());
        let queue_overhead = self.order.capacity() * std::mem::size_of::<String>();
        let heap: usize = self.entries.keys().map(|k| k.len()).sum();
        base + map_overhead + queue_overhead + heap
    }
}

/// Sharded concurrent authentication cache designed for ultra-low lock contention.
pub struct ShardedAuthCache {
    shards: Vec<RwLock<CacheShard>>,
    num_shards: usize,
    capacity_per_shard: usize,
    hits: AtomicUsize,
    misses: AtomicUsize,
    evictions: AtomicUsize,
}

pub type ConcurrentAuthCache = ShardedAuthCache;

impl ShardedAuthCache {
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

    fn shard_idx(&self, token: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn get(&self, token: &str) -> Option<bool> {
        let idx = self.shard_idx(token);
        let res = match self.shards[idx].read() {
            Ok(shard) => shard.get(token),
            Err(poisoned) => poisoned.into_inner().get(token),
        };
        if res.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        res
    }

    pub fn insert(&self, token: &str, is_valid: bool) {
        let idx = self.shard_idx(token);
        let evicted = match self.shards[idx].write() {
            Ok(mut shard) => shard.insert(token.to_string(), is_valid),
            Err(poisoned) => poisoned.into_inner().insert(token.to_string(), is_valid),
        };
        if evicted {
            self.evictions.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn batch_get(&self, tokens: &[&str]) -> Vec<Option<bool>> {
        tokens.iter().map(|&tok| self.get(tok)).collect()
    }

    pub fn batch_insert(&self, entries: &[(&str, bool)]) {
        for &(tok, valid) in entries {
            self.insert(tok, valid);
        }
    }

    pub fn invalidate(&self, token: &str) -> bool {
        let idx = self.shard_idx(token);
        match self.shards[idx].write() {
            Ok(mut shard) => shard.invalidate(token),
            Err(poisoned) => poisoned.into_inner().invalidate(token),
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

    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| match shard.read() {
                Ok(s) => s.len(),
                Err(p) => p.into_inner().len(),
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn total_capacity(&self) -> usize {
        self.num_shards * self.capacity_per_shard
    }

    pub fn stats(&self) -> AuthCacheStats {
        AuthCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            total_entries: self.len(),
        }
    }

    pub fn memory_usage_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .shards
                .iter()
                .map(|shard| match shard.read() {
                    Ok(s) => s.memory_usage_bytes(),
                    Err(p) => p.into_inner().memory_usage_bytes(),
                })
                .sum::<usize>()
    }
}

impl Default for ShardedAuthCache {
    fn default() -> Self {
        Self::with_default_shards(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_get_evict_clear() {
        let cache = ShardedAuthCache::new(1, 2);
        assert_eq!(cache.get("tok1"), None);
        cache.insert("tok1", true);
        cache.insert("tok2", false);
        assert_eq!(cache.get("tok1"), Some(true));
        assert_eq!(cache.get("tok2"), Some(false));

        // Eviction when cap=2 exceeded
        cache.insert("tok3", true);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get("tok1"), None);
        assert!(cache.stats().evictions >= 1);

        // Invalidate and clear
        assert!(cache.invalidate("tok2"));
        assert_eq!(cache.get("tok2"), None);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_cache_batch_operations() {
        let cache = ShardedAuthCache::new(4, 16);
        cache.batch_insert(&[("t1", true), ("t2", false), ("t3", true)]);
        let results = cache.batch_get(&["t1", "t2", "t4"]);
        assert_eq!(results, vec![Some(true), Some(false), None]);
    }
}
