use crate::durable::fault::DurableMemoryError;
use crate::durable::resilience::ResilientDurableMemoryStore;
use crate::durable::storage::StructuredLesson;
use tokio::fs;

impl ResilientDurableMemoryStore {
    pub async fn save_lesson(&mut self, lesson: &str) -> Result<(), DurableMemoryError> {
        if lesson.trim().is_empty() {
            self.rejected_count += 1;
            return Err(DurableMemoryError::EmptyContent);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.save_lesson(lesson).await {
                Ok(()) => {
                    self.saved_count += 1;
                    return Ok(());
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn save_structured_lesson(
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
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.save_structured_lesson(lesson).await {
                Ok(()) => {
                    self.saved_count += 1;
                    self.record_lesson_cached(lesson);
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                    let _ = fs::remove_file(&self.store.lessons_json_file()).await;
                    self.recovered_count += 1;
                    match self.store.save_structured_lesson(lesson).await {
                        Ok(()) => {
                            self.saved_count += 1;
                            self.record_lesson_cached(lesson);
                            return Ok(());
                        }
                        Err(e2) => {
                            self.rejected_count += 1;
                            return Err(DurableMemoryError::from(e2));
                        }
                    }
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn load_relevant_context(&mut self, goal: &str) -> Result<String, DurableMemoryError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.load_relevant_context(goal).await {
                Ok(ctx) => {
                    self.loaded_count += 1;
                    return Ok(ctx);
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn load_structured_lessons(&mut self) -> Result<Vec<StructuredLesson>, DurableMemoryError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.load_structured_lessons().await {
                Ok(lessons) => {
                    self.loaded_count += 1;
                    return Ok(lessons);
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }

    pub async fn save_user_directive(&mut self, directive: &str) -> Result<(), DurableMemoryError> {
        if directive.trim().is_empty() {
            self.rejected_count += 1;
            return Err(DurableMemoryError::EmptyContent);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.check_fault_injection(attempt)?.is_some() {
                continue;
            }

            match self.store.save_user_directive(directive).await {
                Ok(()) => {
                    self.saved_count += 1;
                    return Ok(());
                }
                Err(e) => {
                    self.handle_transient_error(attempt, e)?;
                }
            }
        }
    }
}
