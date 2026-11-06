use crate::durable::fault::{DurableMemoryError, DurableMemoryFault};
use crate::durable::policy::DurableMemoryFaultPolicy;
use crate::durable::storage::{DurableMemoryStore, StructuredLesson};
use std::collections::{HashSet, VecDeque};
use std::path::Path;

pub type ResilientDurableMemory = ResilientDurableMemoryStore;

#[derive(Clone)]
pub struct ResilientDurableMemoryStore {
    pub(crate) store: DurableMemoryStore,
    pub(crate) fault_policy: DurableMemoryFaultPolicy,
    pub(crate) max_retries: usize,
    pub(crate) saved_count: usize,
    pub(crate) loaded_count: usize,
    pub(crate) recovered_count: usize,
    pub(crate) rejected_count: usize,
    seen_lesson_ids: HashSet<String>,
    lesson_queue: VecDeque<String>,
    cached_lessons: VecDeque<StructuredLesson>,
    max_capacity: Option<usize>,
}

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

    pub fn has_lesson(&self, id: &str) -> bool {
        self.seen_lesson_ids.contains(id)
    }

    pub fn record_lesson_cached(&mut self, lesson: &StructuredLesson) {
        if let Some(max_cap) = self.max_capacity {
            while self.lesson_queue.len() >= max_cap {
                if let Some(oldest_id) = self.lesson_queue.pop_front() {
                    self.seen_lesson_ids.remove(&oldest_id);
                }
                self.cached_lessons.pop_front();
            }
            self.lesson_queue.push_back(lesson.id.clone());
            self.cached_lessons.push_back(lesson.clone());
        }
        self.seen_lesson_ids.insert(lesson.id.clone());
    }

    pub fn save_structured_lesson_cached(
        &mut self,
        lesson: &StructuredLesson,
    ) -> Result<(), DurableMemoryError> {
        if lesson.id.trim().is_empty() || lesson.task_title.trim().is_empty() {
            self.rejected_count += 1;
            return Err(DurableMemoryError::EmptyContent);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.fault_policy.should_inject() {
                match fault {
                    DurableMemoryFault::SocketTimeout
                    | DurableMemoryFault::ConnectionReset
                    | DurableMemoryFault::TransientIo
                    | DurableMemoryFault::LockFailure => {
                        if attempt <= self.max_retries {
                            self.recovered_count += 1;
                            continue;
                        } else {
                            return Err(fault.to_error());
                        }
                    }
                    DurableMemoryFault::CorruptedState => {
                        self.rejected_count += 1;
                        return Err(fault.to_error());
                    }
                }
            } else {
                self.saved_count += 1;
                self.record_lesson_cached(lesson);
                return Ok(());
            }
        }
    }

    pub fn save_batch(
        &mut self,
        lessons: &[StructuredLesson],
    ) -> Vec<Result<(), DurableMemoryError>> {
        let mut results = Vec::with_capacity(lessons.len());
        for l in lessons {
            results.push(self.save_structured_lesson_cached(l));
        }
        results
    }

    pub(crate) fn check_fault_injection(&mut self, attempt: usize) -> Result<Option<()>, DurableMemoryError> {
        if let Some(fault) = self.fault_policy.should_inject() {
            match fault {
                DurableMemoryFault::SocketTimeout
                | DurableMemoryFault::ConnectionReset
                | DurableMemoryFault::TransientIo
                | DurableMemoryFault::LockFailure => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        Ok(Some(()))
                    } else {
                        Err(fault.to_error())
                    }
                }
                DurableMemoryFault::CorruptedState => {
                    self.rejected_count += 1;
                    Err(fault.to_error())
                }
            }
        } else {
            Ok(None)
        }
    }

    pub(crate) fn handle_transient_error(
        &mut self,
        attempt: usize,
        err: std::io::Error,
    ) -> Result<(), DurableMemoryError> {
        if attempt <= self.max_retries {
            self.recovered_count += 1;
            Ok(())
        } else {
            Err(DurableMemoryError::from(err))
        }
    }
}
