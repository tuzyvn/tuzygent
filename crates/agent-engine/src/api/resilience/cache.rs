use super::ResilientApiEngine;
use crate::traits::EngineOutput;

impl<E> ResilientApiEngine<E> {
    pub fn with_capacity(self, capacity: usize) -> Self {
        if let Ok(mut cache) = self.response_cache.lock() {
            cache.reserve(capacity);
        }
        if let Ok(mut queue) = self.cache_queue.lock() {
            queue.reserve(capacity);
        }
        self
    }

    pub fn with_max_capacity(mut self, max_capacity: usize) -> Self {
        self.max_capacity = Some(max_capacity);
        self.evict_if_needed();
        self
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let (cache_overhead, heap_content) = if let Ok(cache) = self.response_cache.lock() {
            let overhead = cache.capacity()
                * (std::mem::size_of::<String>() + std::mem::size_of::<EngineOutput>());
            let content: usize = cache
                .iter()
                .map(|(key, value)| {
                    key.len() + value.stdout_content.len() + std::mem::size_of::<EngineOutput>()
                })
                .sum();
            (overhead, content)
        } else {
            (0, 0)
        };
        let queue_overhead = if let Ok(queue) = self.cache_queue.lock() {
            queue.capacity() * std::mem::size_of::<String>()
        } else {
            0
        };
        base_size + cache_overhead + queue_overhead + heap_content
    }

    fn evict_if_needed(&self) {
        if let Some(max) = self.max_capacity {
            if let (Ok(mut cache), Ok(mut queue)) =
                (self.response_cache.lock(), self.cache_queue.lock())
            {
                while queue.len() >= max && max > 0 {
                    if let Some(old_key) = queue.pop_front() {
                        cache.remove(&old_key);
                    }
                }
            }
        }
    }

    pub fn cache_response(&self, key: &str, output: &EngineOutput) {
        if let Some(max) = self.max_capacity {
            if max == 0 {
                return;
            }
            if let (Ok(mut cache), Ok(mut queue)) =
                (self.response_cache.lock(), self.cache_queue.lock())
            {
                while queue.len() >= max {
                    if let Some(old_key) = queue.pop_front() {
                        cache.remove(&old_key);
                    }
                }
                if !cache.contains_key(key) {
                    queue.push_back(key.to_string());
                }
                cache.insert(key.to_string(), output.clone());
            }
        }
    }

    pub fn get_cached(&self, key: &str) -> Option<EngineOutput> {
        self.response_cache.lock().ok()?.get(key).cloned()
    }
}
