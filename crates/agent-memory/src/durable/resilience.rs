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

    pub(crate) fn check_fault_injection(
        &mut self,
        attempt: usize,
    ) -> Result<Option<()>, DurableMemoryError> {
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

#[cfg(test)]
mod tests {
    use super::ResilientDurableMemoryStore;
    use crate::durable::fault::{DurableMemoryError, DurableMemoryFault};
    use crate::durable::policy::DurableMemoryFaultPolicy;
    use crate::durable::storage::{DurableMemoryStore, StructuredLesson};
    use anyhow::Context;

    #[test]
    fn resilience_tests_contain_no_unwrap_calls() {
        let pattern = format!(".{}()", "unwrap");
        let file_content = include_str!("resilience.rs");
        let unwrap_calls: Vec<(usize, &str)> = file_content
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim();
                !trimmed.starts_with("//") && trimmed.contains(&pattern)
            })
            .collect();

        assert!(
            unwrap_calls.is_empty(),
            "crates/agent-memory/src/durable/resilience.rs must contain zero unwrap calls, found: {:?}",
            unwrap_calls
        );
    }

    fn store(name: &str) -> DurableMemoryStore {
        DurableMemoryStore::new(
            std::env::temp_dir().join(format!("resilience_{name}_{}", uuid::Uuid::new_v4())),
        )
    }

    fn lesson(id: &str) -> StructuredLesson {
        StructuredLesson {
            id: id.to_string(),
            timestamp: "2026-11-07T00:00:00Z".to_string(),
            task_title: format!("Lesson {id}"),
            trigger_error: None,
            remediation_applied: None,
            general_insight: format!("Insight for {id}"),
            success: true,
        }
    }

    async fn make_structured_lessons_path_a_directory(name: &str) -> std::path::PathBuf {
        let workspace =
            std::env::temp_dir().join(format!("resilience_{name}_{}", uuid::Uuid::new_v4()));
        let store = DurableMemoryStore::new(&workspace);
        let lessons_path = store.lessons_json_file().to_path_buf();
        tokio::fs::create_dir_all(lessons_path.parent().expect("lessons path has a parent"))
            .await
            .expect("create lessons parent directory");
        tokio::fs::create_dir(&lessons_path)
            .await
            .expect("create directory at lessons file path");
        workspace
    }

    #[tokio::test]
    async fn save_structured_lesson_returns_io_error_without_panicking() {
        let workspace = make_structured_lessons_path_a_directory("save_io_error").await;
        let mut resilient =
            ResilientDurableMemoryStore::with_store(DurableMemoryStore::new(&workspace));

        let result = resilient
            .save_structured_lesson(&lesson("save-error"))
            .await;

        assert!(matches!(result, Err(DurableMemoryError::Io(_))));
        assert_eq!(resilient.saved_count(), 0);
        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[tokio::test]
    async fn load_structured_lessons_returns_io_error_without_panicking() {
        let workspace = make_structured_lessons_path_a_directory("load_io_error").await;
        let mut resilient =
            ResilientDurableMemoryStore::with_store(DurableMemoryStore::new(&workspace));

        let result = resilient.load_structured_lessons().await;

        assert!(matches!(result, Err(DurableMemoryError::Io(_))));
        assert_eq!(resilient.loaded_count(), 0);
        let _ = tokio::fs::remove_dir_all(workspace).await;
    }

    #[test]
    fn with_policy_uses_the_given_policy_and_default_retry_count() {
        let policy = DurableMemoryFaultPolicy::always(DurableMemoryFault::CorruptedState);
        let mut resilient = ResilientDurableMemoryStore::with_policy(store("policy"), policy);

        assert_eq!(resilient.max_retries(), 3);
        assert_eq!(resilient.fault_policy().injected_count(), 0);
        assert!(resilient
            .save_structured_lesson_cached(&lesson("policy"))
            .is_err());
        assert_eq!(resilient.rejected_count(), 1);
    }

    #[test]
    fn with_capacity_reserves_without_limiting_cached_ids() {
        let mut resilient =
            ResilientDurableMemoryStore::with_store(store("capacity")).with_capacity(2);

        resilient.record_lesson_cached(&lesson("first"));
        resilient.record_lesson_cached(&lesson("second"));
        resilient.record_lesson_cached(&lesson("third"));

        assert!(resilient.has_lesson("first"));
        assert!(resilient.has_lesson("second"));
        assert!(resilient.has_lesson("third"));
        assert!(resilient.cached_lessons().is_empty());
    }

    #[test]
    fn store_getters_return_the_store_and_allow_replacement() {
        let mut resilient = ResilientDurableMemoryStore::with_store(store("store_getter"));
        let original_path = resilient.store().memory_file().to_path_buf();
        assert!(original_path.ends_with("memory/MEMORY.md"));

        *resilient.store_mut() = store("store_mut");
        let replacement_path = resilient.store().memory_file();
        assert_ne!(replacement_path, original_path);
        assert!(replacement_path
            .to_string_lossy()
            .contains("resilience_store_mut_"));
    }

    #[test]
    fn policy_getters_and_mutable_aliases_access_the_same_policy() {
        let mut resilient = ResilientDurableMemoryStore::with_policy(
            store("policy_accessors"),
            DurableMemoryFaultPolicy::always(DurableMemoryFault::TransientIo),
        );

        assert_eq!(resilient.fault_policy().injected_count(), 0);
        resilient.fault_policy_mut().should_inject();
        assert_eq!(resilient.policy().injected_count(), 1);

        resilient.policy_mut().reset();
        assert_eq!(resilient.fault_policy().invocation_count(), 0);
        assert_eq!(resilient.policy().injected_count(), 0);
    }

    #[test]
    fn set_policy_and_set_max_retries_change_fault_recovery_behavior() {
        let mut resilient = ResilientDurableMemoryStore::with_store(store("setters"));
        resilient.set_policy(DurableMemoryFaultPolicy::intermittent(
            DurableMemoryFault::TransientIo,
            2,
        ));
        resilient.set_max_retries(1);

        assert_eq!(resilient.max_retries(), 1);
        assert!(resilient
            .save_structured_lesson_cached(&lesson("recovered"))
            .is_ok());
        assert_eq!(resilient.recovered_count(), 1);
        assert_eq!(resilient.saved_count(), 1);
        assert!(resilient.has_lesson("recovered"));
    }

    #[tokio::test]
    async fn reset_counts_clears_counters_and_lesson_indexes() -> anyhow::Result<()> {
        let mut resilient = ResilientDurableMemoryStore::with_store(store("reset"));
        resilient.record_lesson_cached(&lesson("cached"));
        resilient.set_policy(DurableMemoryFaultPolicy::always(
            DurableMemoryFault::CorruptedState,
        ));
        assert!(resilient
            .save_structured_lesson_cached(&lesson("rejected"))
            .is_err());
        resilient.set_policy(DurableMemoryFaultPolicy::none());
        assert!(resilient
            .save_structured_lesson_cached(&lesson("saved"))
            .is_ok());
        resilient
            .save_structured_lesson(&lesson("persisted"))
            .await
            .context("failed to persist lesson before resetting counts")?;
        resilient
            .load_structured_lessons()
            .await
            .context("failed to load persisted lesson before resetting counts")?;
        assert_eq!(resilient.saved_count(), 2);
        assert_eq!(resilient.loaded_count(), 1);
        assert_eq!(resilient.rejected_count(), 1);
        assert!(resilient.has_lesson("cached"));

        resilient.reset_counts();

        assert_eq!(resilient.saved_count(), 0);
        assert_eq!(resilient.loaded_count(), 0);
        assert_eq!(resilient.recovered_count(), 0);
        assert_eq!(resilient.rejected_count(), 0);
        assert!(resilient.cached_lessons().is_empty());
        assert!(!resilient.has_lesson("cached"));
        assert!(!resilient.has_lesson("saved"));
        Ok(())
    }

    #[test]
    fn recording_lessons_tracks_ids_and_evicts_oldest_at_capacity() {
        let mut resilient =
            ResilientDurableMemoryStore::with_store(store("fifo")).with_max_capacity(2);

        resilient.record_lesson_cached(&lesson("first"));
        resilient.record_lesson_cached(&lesson("second"));
        assert_eq!(resilient.cached_lessons().len(), 2);
        assert!(resilient.has_lesson("first"));
        assert!(resilient.has_lesson("second"));

        resilient.record_lesson_cached(&lesson("third"));

        assert_eq!(resilient.cached_lessons().len(), 2);
        assert_eq!(resilient.cached_lessons()[0].id, "second");
        assert_eq!(resilient.cached_lessons()[1].id, "third");
        assert!(!resilient.has_lesson("first"));
        assert!(resilient.has_lesson("second"));
        assert!(resilient.has_lesson("third"));
    }
}
