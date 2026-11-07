use super::ResilientEventProcessor;
use crate::event::fault::EventFaultPolicy;
use std::collections::HashSet;

impl ResilientEventProcessor {
    pub fn new(policy: EventFaultPolicy, max_retries: usize) -> Self {
        Self {
            policy,
            max_retries,
            seen_ids: HashSet::new(),
            id_queue: Default::default(),
            max_capacity: None,
            processed_count: 0,
            recovered_count: 0,
            dropped_count: 0,
            rejected_count: 0,
        }
    }

    pub fn with_policy(policy: EventFaultPolicy) -> Self {
        Self::new(policy, 3)
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.seen_ids.reserve(capacity);
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.seen_ids.reserve(max_capacity);
        self.id_queue.reserve(max_capacity);
        self
    }

    pub fn policy(&self) -> &EventFaultPolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut EventFaultPolicy {
        &mut self.policy
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn set_max_retries(&mut self, max_retries: usize) {
        self.max_retries = max_retries;
    }

    pub fn seen_ids(&self) -> &HashSet<String> {
        &self.seen_ids
    }

    pub fn clear_seen_ids(&mut self) {
        self.seen_ids.clear();
        self.id_queue.clear();
    }

    pub fn reset_counts(&mut self) {
        self.processed_count = 0;
        self.recovered_count = 0;
        self.dropped_count = 0;
        self.rejected_count = 0;
    }

    pub fn processed_count(&self) -> usize {
        self.processed_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn dropped_count(&self) -> usize {
        self.dropped_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let set_mem = self.seen_ids.capacity() * (std::mem::size_of::<String>() + 36);
        let queue_mem = self.id_queue.capacity() * (std::mem::size_of::<String>() + 36);
        std::mem::size_of::<Self>() + set_mem + queue_mem
    }
}
