use crate::durable::policy::DurableMemoryFaultPolicy;
use crate::durable::storage::{DurableMemoryStore, StructuredLesson};
use std::collections::{HashSet, VecDeque};

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
    pub(super) seen_lesson_ids: HashSet<String>,
    pub(super) lesson_queue: VecDeque<String>,
    pub(super) cached_lessons: VecDeque<StructuredLesson>,
    pub(super) max_capacity: Option<usize>,
}
