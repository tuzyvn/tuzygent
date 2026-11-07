use crate::durable::policy::DurableMemoryFaultPolicy;
use crate::durable::resilience::ResilientDurableMemoryStore;
use crate::durable::storage::{DurableMemoryStore, StructuredLesson};
use std::collections::{HashSet, VecDeque};
use std::path::Path;

impl ResilientDurableMemoryStore {
    pub fn new(
        store: DurableMemoryStore,
        fault_policy: DurableMemoryFaultPolicy,
        max_retries: usize,
    ) -> Self {
        Self {
            store,
            fault_policy,
            max_retries,
            saved_count: 0,
            loaded_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            seen_lesson_ids: HashSet::new(),
            lesson_queue: VecDeque::new(),
            cached_lessons: VecDeque::new(),
            max_capacity: None,
        }
    }

    pub fn with_store(store: DurableMemoryStore) -> Self {
        Self::new(store, DurableMemoryFaultPolicy::none(), 3)
    }

    pub fn with_policy(store: DurableMemoryStore, fault_policy: DurableMemoryFaultPolicy) -> Self {
        Self::new(store, fault_policy, 3)
    }

    pub fn open<P: AsRef<Path>>(workspace_dir: P) -> Self {
        Self::with_store(DurableMemoryStore::new(workspace_dir))
    }

    pub fn from_workspace<P: AsRef<Path>>(workspace_dir: P) -> Self {
        Self::with_store(DurableMemoryStore::new(workspace_dir))
    }

    pub fn with_fault_policy(mut self, fault_policy: DurableMemoryFaultPolicy) -> Self {
        self.fault_policy = fault_policy;
        self
    }

    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.seen_lesson_ids.reserve(capacity);
        self.lesson_queue.reserve(capacity);
        self.cached_lessons.reserve(capacity);
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.seen_lesson_ids.reserve(max_capacity);
        self.lesson_queue.reserve(max_capacity);
        self.cached_lessons.reserve(max_capacity);
        self
    }

    pub fn store(&self) -> &DurableMemoryStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut DurableMemoryStore {
        &mut self.store
    }

    pub fn fault_policy(&self) -> &DurableMemoryFaultPolicy {
        &self.fault_policy
    }

    pub fn fault_policy_mut(&mut self) -> &mut DurableMemoryFaultPolicy {
        &mut self.fault_policy
    }

    pub fn policy(&self) -> &DurableMemoryFaultPolicy {
        &self.fault_policy
    }

    pub fn policy_mut(&mut self) -> &mut DurableMemoryFaultPolicy {
        &mut self.fault_policy
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn set_policy(&mut self, policy: DurableMemoryFaultPolicy) {
        self.fault_policy = policy;
    }

    pub fn set_max_retries(&mut self, max_retries: usize) {
        self.max_retries = max_retries;
    }

    pub fn saved_count(&self) -> usize {
        self.saved_count
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn reset_counts(&mut self) {
        self.saved_count = 0;
        self.loaded_count = 0;
        self.recovered_count = 0;
        self.rejected_count = 0;
        self.seen_lesson_ids.clear();
        self.lesson_queue.clear();
        self.cached_lessons.clear();
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let set_mem = self.seen_lesson_ids.capacity() * (std::mem::size_of::<String>() + 40);
        let queue_mem = self.lesson_queue.capacity() * (std::mem::size_of::<String>() + 40);
        let lesson_mem = self.cached_lessons.capacity() * std::mem::size_of::<StructuredLesson>();
        std::mem::size_of::<Self>() + set_mem + queue_mem + lesson_mem
    }

    pub fn cached_lessons(&self) -> &VecDeque<StructuredLesson> {
        &self.cached_lessons
    }
}
