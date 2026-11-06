use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

/// Thread-safe bounded FIFO ring buffer protected by `RwLock`.
///
/// Reuses pre-allocated string buffers on eviction to eliminate heap reallocation
/// and maintain strictly bounded memory footprint under continuous bursts.
pub struct GatewayRingBuffer {
    inner: RwLock<VecDeque<String>>,
    capacity: usize,
    total_pushed: AtomicUsize,
    current_len: AtomicUsize,
}

impl GatewayRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: RwLock::new(VecDeque::with_capacity(capacity.min(2048))),
            capacity,
            total_pushed: AtomicUsize::new(0),
            current_len: AtomicUsize::new(0),
        }
    }

    pub fn with_default_capacity() -> Self {
        Self::new(1024)
    }

    pub fn push(&self, item: &str) {
        self.push_batch(&[item]);
    }

    pub fn push_batch(&self, items: &[&str]) {
        if self.capacity == 0 || items.is_empty() {
            return;
        }
        let mut guard = match self.inner.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        for &item in items {
            if guard.len() >= self.capacity {
                if let Some(mut old) = guard.pop_front() {
                    old.clear();
                    old.push_str(item);
                    guard.push_back(old);
                } else {
                    guard.push_back(item.to_string());
                }
            } else {
                guard.push_back(item.to_string());
            }
        }
        self.current_len.store(guard.len(), Ordering::Relaxed);
        self.total_pushed.fetch_add(items.len(), Ordering::Relaxed);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.current_len.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        let mut guard = match self.inner.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.clear();
        self.current_len.store(0, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Vec<String> {
        let guard = match self.inner.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.iter().cloned().collect()
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let guard = match self.inner.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let queue_mem = guard.capacity() * std::mem::size_of::<String>()
            + guard.iter().map(|s| s.capacity()).sum::<usize>();
        std::mem::size_of::<Self>() + queue_mem
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn total_capacity(&self) -> usize {
        self.capacity
    }

    pub fn total_pushed(&self) -> usize {
        self.total_pushed.load(Ordering::Relaxed)
    }
}

/// Sharded concurrent ring buffer partitioning requests across multiple `RwLock` ring buffers.
/// Disperses lock contention evenly via atomic round-robin or deterministic hash partitioning.
pub struct ShardedGatewayRingBuffer {
    shards: Vec<GatewayRingBuffer>,
    num_shards: usize,
    total_capacity: usize,
    round_robin_counter: AtomicUsize,
}

impl ShardedGatewayRingBuffer {
    pub fn new(num_shards: usize, total_capacity: usize) -> Self {
        let num_shards = num_shards.max(1);
        let capacity_per_shard = (total_capacity / num_shards).max(1);
        let mut shards = Vec::with_capacity(num_shards);
        for _ in 0..num_shards {
            shards.push(GatewayRingBuffer::new(capacity_per_shard));
        }
        Self {
            shards,
            num_shards,
            total_capacity,
            round_robin_counter: AtomicUsize::new(0),
        }
    }

    pub fn with_default_shards(total_capacity: usize) -> Self {
        Self::new(16, total_capacity)
    }

    fn shard_idx(&self, key: &str) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    pub fn push(&self, item: &str) {
        let idx = self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % self.num_shards;
        self.shards[idx].push(item);
    }

    pub fn push_keyed(&self, key: &str, item: &str) {
        self.shards[self.shard_idx(key)].push(item);
    }

    pub fn push_batch(&self, items: &[&str]) {
        if items.is_empty() {
            return;
        }
        if self.num_shards == 1 {
            self.shards[0].push_batch(items);
            return;
        }
        let mut shard_batches: Vec<Vec<&str>> = vec![Vec::new(); self.num_shards];
        for &item in items {
            let idx = self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % self.num_shards;
            shard_batches[idx].push(item);
        }
        for (idx, batch) in shard_batches.into_iter().enumerate() {
            if !batch.is_empty() {
                self.shards[idx].push_batch(&batch);
            }
        }
    }

    pub fn push_keyed_batch(&self, items: &[(&str, &str)]) {
        if items.is_empty() {
            return;
        }
        if self.num_shards == 1 {
            let values: Vec<&str> = items.iter().map(|&(_, v)| v).collect();
            self.shards[0].push_batch(&values);
            return;
        }
        let mut shard_batches: Vec<Vec<&str>> = vec![Vec::new(); self.num_shards];
        for &(key, val) in items {
            let idx = self.shard_idx(key);
            shard_batches[idx].push(val);
        }
        for (idx, batch) in shard_batches.into_iter().enumerate() {
            if !batch.is_empty() {
                self.shards[idx].push_batch(&batch);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        for shard in &self.shards {
            shard.clear();
        }
    }

    pub fn snapshot(&self) -> Vec<String> {
        let mut all = Vec::with_capacity(self.len());
        for shard in &self.shards {
            all.extend(shard.snapshot());
        }
        all
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let shards_mem: usize = self.shards.iter().map(|s| s.memory_usage_bytes()).sum();
        std::mem::size_of::<Self>() + shards_mem
    }

    pub fn total_capacity(&self) -> usize {
        self.total_capacity
    }

    pub fn total_pushed(&self) -> usize {
        self.shards.iter().map(|s| s.total_pushed()).sum()
    }

    pub fn num_shards(&self) -> usize {
        self.num_shards
    }
}

/// Unified gateway buffer abstraction supporting either single bounded or sharded ring buffers.
#[derive(Clone)]
pub enum GatewayBuffer {
    Single(Arc<GatewayRingBuffer>),
    Sharded(Arc<ShardedGatewayRingBuffer>),
}

impl GatewayBuffer {
    pub fn single(capacity: usize) -> Self {
        Self::Single(Arc::new(GatewayRingBuffer::new(capacity)))
    }

    pub fn sharded(num_shards: usize, capacity: usize) -> Self {
        Self::Sharded(Arc::new(ShardedGatewayRingBuffer::new(num_shards, capacity)))
    }

    pub fn push(&self, item: &str) {
        match self {
            Self::Single(b) => b.push(item),
            Self::Sharded(b) => b.push(item),
        }
    }

    pub fn push_batch(&self, items: &[&str]) {
        match self {
            Self::Single(b) => b.push_batch(items),
            Self::Sharded(b) => b.push_batch(items),
        }
    }

    pub fn push_keyed(&self, key: &str, item: &str) {
        match self {
            Self::Single(b) => b.push(item),
            Self::Sharded(b) => b.push_keyed(key, item),
        }
    }

    pub fn push_keyed_batch(&self, items: &[(&str, &str)]) {
        match self {
            Self::Single(b) => {
                let values: Vec<&str> = items.iter().map(|&(_, v)| v).collect();
                b.push_batch(&values);
            }
            Self::Sharded(b) => b.push_keyed_batch(items),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Single(b) => b.len(),
            Self::Sharded(b) => b.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        match self {
            Self::Single(b) => b.clear(),
            Self::Sharded(b) => b.clear(),
        }
    }

    pub fn memory_usage_bytes(&self) -> usize {
        match self {
            Self::Single(b) => b.memory_usage_bytes(),
            Self::Sharded(b) => b.memory_usage_bytes(),
        }
    }

    pub fn total_capacity(&self) -> usize {
        match self {
            Self::Single(b) => b.total_capacity(),
            Self::Sharded(b) => b.total_capacity(),
        }
    }

    pub fn total_pushed(&self) -> usize {
        match self {
            Self::Single(b) => b.total_pushed(),
            Self::Sharded(b) => b.total_pushed(),
        }
    }

    pub fn as_single(&self) -> Option<&Arc<GatewayRingBuffer>> {
        match self {
            Self::Single(b) => Some(b),
            Self::Sharded(_) => None,
        }
    }

    pub fn as_sharded(&self) -> Option<&Arc<ShardedGatewayRingBuffer>> {
        match self {
            Self::Single(_) => None,
            Self::Sharded(b) => Some(b),
        }
    }
}


