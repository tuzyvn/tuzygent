use super::cache::HarnessLessonCache;
use super::fault::{is_retryable_harness_error, HarnessError, HarnessFault, HarnessFaultPolicy};
use super::{HarnessLesson, HarnessMemoryStore};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type ResilientHarnessMemory = ResilientHarnessMemoryStore;

#[derive(Clone)]
pub struct ResilientHarnessMemoryStore {
    pub(crate) store: HarnessMemoryStore,
    pub(crate) policy: HarnessFaultPolicy,
    pub(crate) max_retries: usize,
    pub(crate) recorded_count: usize,
    pub(crate) loaded_count: usize,
    pub(crate) recovered_count: usize,
    pub(crate) panics_caught_count: usize,
    pub(crate) failed_count: usize,
    pub(crate) cache: Arc<Mutex<HarnessLessonCache>>,
}

impl ResilientHarnessMemoryStore {
    pub fn new(store: HarnessMemoryStore, policy: HarnessFaultPolicy, max_retries: usize) -> Self {
        Self {
            store,
            policy,
            max_retries,
            recorded_count: 0,
            loaded_count: 0,
            recovered_count: 0,
            panics_caught_count: 0,
            failed_count: 0,
            cache: Arc::new(Mutex::new(HarnessLessonCache::default())),
        }
    }

    pub fn with_store(store: HarnessMemoryStore) -> Self {
        Self::new(store, HarnessFaultPolicy::none(), 3)
    }

    pub fn memory() -> Self {
        Self::with_store(HarnessMemoryStore::memory())
    }

    pub fn open<P: AsRef<Path>>(workspace_dir: P) -> Self {
        let path_str = workspace_dir.as_ref().to_string_lossy();
        Self::with_store(HarnessMemoryStore::new(&path_str))
    }

    pub fn from_workspace<P: AsRef<Path>>(workspace_dir: P) -> Self {
        Self::open(workspace_dir)
    }

    pub fn with_policy(mut self, policy: HarnessFaultPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_max_retries(mut self, max: usize) -> Self {
        self.max_retries = max;
        self
    }

    pub fn with_max_capacity(self, max: usize) -> Self {
        if let Ok(mut cache) = self.cache.lock() {
            cache.set_max_capacity(max);
        }
        self
    }

    pub fn with_capacity(self, capacity: usize) -> Self {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = HarnessLessonCache::with_capacity(capacity);
        }
        self
    }

    pub fn max_capacity(&self) -> Option<usize> {
        self.cache.lock().ok().and_then(|c| c.max_capacity())
    }

    pub fn get_cached(&self, id: &str) -> Option<HarnessLesson> {
        self.cache.lock().ok().and_then(|c| c.get(id).cloned())
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let base_size = std::mem::size_of::<Self>();
        let cache_size = self
            .cache
            .lock()
            .map(|c| c.memory_usage_bytes())
            .unwrap_or(0);
        base_size + cache_size
    }

    pub fn store(&self) -> &HarnessMemoryStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut HarnessMemoryStore {
        &mut self.store
    }

    pub fn policy(&self) -> &HarnessFaultPolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut HarnessFaultPolicy {
        &mut self.policy
    }

    pub fn set_policy(&mut self, policy: HarnessFaultPolicy) {
        self.policy = policy;
    }

    pub fn max_retries(&self) -> usize {
        self.max_retries
    }

    pub fn set_max_retries(&mut self, max: usize) {
        self.max_retries = max;
    }

    pub fn recorded_count(&self) -> usize {
        self.recorded_count
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn panics_caught_count(&self) -> usize {
        self.panics_caught_count
    }

    pub fn failed_count(&self) -> usize {
        self.failed_count
    }

    pub fn reset_counts(&mut self) {
        self.recorded_count = 0;
        self.loaded_count = 0;
        self.recovered_count = 0;
        self.panics_caught_count = 0;
        self.failed_count = 0;
    }

    pub async fn record_batch(
        &mut self,
        items: &[(&str, &str, &str, &str)],
    ) -> Vec<Result<HarnessLesson, HarnessError>> {
        let mut results = Vec::with_capacity(items.len());
        for (node_type, identifier, error_pattern, remediation) in items {
            results.push(
                self.record_lesson_resilient(node_type, identifier, error_pattern, remediation)
                    .await,
            );
        }
        results
    }

    pub async fn record_lesson_resilient(
        &mut self,
        node_type: &str,
        identifier: &str,
        error_pattern: &str,
        remediation_strategy: &str,
    ) -> Result<HarnessLesson, HarnessError> {
        if node_type.trim().is_empty() || identifier.trim().is_empty() {
            self.failed_count += 1;
            return Err(HarnessError::InvalidParameter(
                "node_type and identifier cannot be blank".to_string(),
            ));
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.policy.should_inject() {
                if fault == HarnessFault::Panic {
                    self.panics_caught_count += 1;
                }
                if fault.is_transient() && attempt <= self.max_retries {
                    self.recovered_count += 1;
                    continue;
                } else {
                    self.failed_count += 1;
                    return Err(fault.to_error());
                }
            }

            match self
                .store
                .record_lesson(node_type, identifier, error_pattern, remediation_strategy)
                .await
            {
                Ok(lesson) => {
                    self.recorded_count += 1;
                    if let Ok(mut c) = self.cache.lock() {
                        c.insert(&lesson);
                    }
                    return Ok(lesson);
                }
                Err(e) => {
                    let err = HarnessError::from(e);
                    if matches!(err, HarnessError::CorruptedState(_)) {
                        let _ = tokio::fs::remove_file(self.store.memory_path()).await;
                        self.recovered_count += 1;
                        if let Ok(lesson) = self
                            .store
                            .record_lesson(
                                node_type,
                                identifier,
                                error_pattern,
                                remediation_strategy,
                            )
                            .await
                        {
                            self.recorded_count += 1;
                            if let Ok(mut c) = self.cache.lock() {
                                c.insert(&lesson);
                            }
                            return Ok(lesson);
                        }
                    }

                    if is_retryable_harness_error(&err) && attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    } else {
                        self.failed_count += 1;
                        return Err(err);
                    }
                }
            }
        }
    }

    pub async fn load_lessons_resilient(&mut self) -> Result<Vec<HarnessLesson>, HarnessError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if let Some(fault) = self.policy.should_inject() {
                if fault == HarnessFault::Panic {
                    self.panics_caught_count += 1;
                }
                if fault.is_transient() && attempt <= self.max_retries {
                    self.recovered_count += 1;
                    continue;
                } else {
                    self.failed_count += 1;
                    return Err(fault.to_error());
                }
            }

            let lessons = self.store.load_lessons().await;
            self.loaded_count += 1;
            return Ok(lessons);
        }
    }

    pub async fn find_relevant_lessons_resilient(
        &mut self,
        node_type: &str,
        identifier: &str,
    ) -> Result<Vec<HarnessLesson>, HarnessError> {
        let lessons = self.load_lessons_resilient().await?;
        let filtered = lessons
            .into_iter()
            .filter(|l| {
                l.node_type == node_type && (l.identifier == identifier || l.identifier == "*")
            })
            .collect();
        Ok(filtered)
    }
}
