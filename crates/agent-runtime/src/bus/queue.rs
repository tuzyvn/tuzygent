use crate::bus::channel::InterAgentBus;
use crate::bus::event::{calculate_event_shard, validate_bus_event};
use crate::bus::fault::BusError;
use crate::bus::subscriber::ResilientSubscriber;
use agent_domain::InterAgentEvent;
use tokio::sync::broadcast;

pub const DEFAULT_BUS_SHARDS: usize = 16;

/// High-throughput sharded bus queue that distributes event traffic across
/// multiple independent channels to eliminate lock contention under heavy concurrency.
#[derive(Clone)]
pub struct ShardedBusQueue {
    shards: Vec<InterAgentBus>,
    num_shards: usize,
}

impl Default for ShardedBusQueue {
    fn default() -> Self {
        Self::new(1024, DEFAULT_BUS_SHARDS)
    }
}

impl ShardedBusQueue {
    /// Creates a new `ShardedBusQueue` with the specified capacity per shard and shard count.
    pub fn new(per_shard_capacity: usize, num_shards: usize) -> Self {
        let count = num_shards.max(1);
        let mut shards = Vec::with_capacity(count);
        for _ in 0..count {
            shards.push(InterAgentBus::new(per_shard_capacity));
        }
        Self {
            shards,
            num_shards: count,
        }
    }

    /// Returns the number of shards in this bus queue.
    pub fn shard_count(&self) -> usize {
        self.num_shards
    }

    /// Returns a reference to a specific shard.
    pub fn shard(&self, index: usize) -> Option<&InterAgentBus> {
        self.shards.get(index)
    }

    /// Determines which shard index an event should route to.
    pub fn shard_for_event(&self, event: &InterAgentEvent) -> usize {
        calculate_event_shard(event, self.num_shards)
    }

    /// Publishes an event to its designated shard based on consistent hashing.
    pub fn publish(&self, event: InterAgentEvent) -> Result<usize, String> {
        let shard_idx = self.shard_for_event(&event);
        self.shards[shard_idx].publish(event)
    }

    /// Publishes a batch of events by partitioning them across shards.
    pub fn publish_batch(&self, events: Vec<InterAgentEvent>) -> Result<usize, String> {
        if events.is_empty() {
            return Ok(0);
        }
        let mut total_published = 0;
        for event in events {
            let shard_idx = self.shard_for_event(&event);
            let recipients = self.shards[shard_idx].publish(event)?;
            total_published += recipients;
        }
        Ok(total_published)
    }

    /// Publishes an event safely with validation and structured `BusError` mapping.
    pub fn safe_publish(&self, event: InterAgentEvent) -> Result<usize, BusError> {
        validate_bus_event(&event)?;
        let shard_idx = self.shard_for_event(&event);
        self.shards[shard_idx].safe_publish(event)
    }

    /// Subscribes to events on a specific shard.
    pub fn subscribe_shard(&self, shard_idx: usize) -> Option<broadcast::Receiver<InterAgentEvent>> {
        self.shards.get(shard_idx).map(|s| s.subscribe())
    }

    /// Subscribes resiliently to events on a specific shard.
    pub fn subscribe_resilient_shard(&self, shard_idx: usize) -> Option<ResilientSubscriber> {
        self.shards.get(shard_idx).map(|s| s.subscribe_resilient())
    }

    /// Total number of receivers across all shards.
    pub fn receiver_count(&self) -> usize {
        self.shards.iter().map(|s| s.receiver_count()).sum()
    }

    /// Total capacity across all shards.
    pub fn total_capacity(&self) -> usize {
        self.shards.iter().map(|s| s.capacity()).sum()
    }

    /// Closes all shard broadcast channels.
    pub fn close_all(&self) {
        for s in &self.shards {
            s.close();
        }
    }

    /// Returns true if all shard broadcast channels are closed.
    pub fn is_closed(&self) -> bool {
        self.shards.iter().all(|s| s.is_closed())
    }
}
