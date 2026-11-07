use crate::durable::fault::{DurableMemoryError, DurableMemoryFault};
use crate::durable::resilience::ResilientDurableMemoryStore;
use crate::durable::storage::StructuredLesson;

impl ResilientDurableMemoryStore {
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
}
