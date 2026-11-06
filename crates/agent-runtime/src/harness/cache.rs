use super::HarnessLesson;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Default)]
pub struct HarnessLessonCache {
    cache: HashMap<String, HarnessLesson>,
    order: VecDeque<String>,
    max_capacity: Option<usize>,
}

impl HarnessLessonCache {
    pub fn new(max_capacity: Option<usize>) -> Self {
        Self {
            cache: HashMap::new(),
            order: VecDeque::new(),
            max_capacity,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            max_capacity: None,
        }
    }

    pub fn set_max_capacity(&mut self, max: usize) {
        self.max_capacity = Some(max);
        self.evict_if_needed();
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.max_capacity
    }

    pub fn evict_if_needed(&mut self) {
        if let Some(max) = self.max_capacity {
            while self.order.len() >= max && max > 0 {
                if let Some(old_id) = self.order.pop_front() {
                    self.cache.remove(&old_id);
                }
            }
        }
    }

    pub fn insert(&mut self, lesson: &HarnessLesson) {
        if let Some(max) = self.max_capacity {
            if max == 0 {
                return;
            }
            while self.order.len() >= max {
                if let Some(old_id) = self.order.pop_front() {
                    self.cache.remove(&old_id);
                }
            }
            if !self.cache.contains_key(&lesson.id) {
                self.order.push_back(lesson.id.clone());
            }
            self.cache.insert(lesson.id.clone(), lesson.clone());
        }
    }

    pub fn get(&self, id: &str) -> Option<&HarnessLesson> {
        self.cache.get(id)
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let cache_overhead = self.cache.capacity()
            * (std::mem::size_of::<String>() + std::mem::size_of::<HarnessLesson>());
        let heap_content: usize = self
            .cache
            .iter()
            .map(|(k, v)| {
                k.len()
                    + v.id.len()
                    + v.node_type.len()
                    + v.identifier.len()
                    + v.error_pattern.len()
                    + v.remediation_strategy.len()
                    + v.created_at.len()
                    + v.last_applied_at.as_ref().map(|s| s.len()).unwrap_or(0)
                    + std::mem::size_of::<HarnessLesson>()
            })
            .sum();
        let queue_overhead = self.order.capacity() * std::mem::size_of::<String>();
        base_size + cache_overhead + queue_overhead + heap_content
    }
}
