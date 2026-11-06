use crate::durable::fault::DurableMemoryError;
use crate::durable::resilience::ResilientDurableMemoryStore;
use crate::durable::storage::StructuredLesson;
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};

impl ResilientDurableMemoryStore {
    pub async fn save_lesson_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        lesson: &str,
    ) -> Result<(), DurableMemoryError> {
        if lesson.trim().is_empty() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                "Durable store rejected empty lesson entry".to_string(),
            );
            return Err(DurableMemoryError::EmptyContent);
        }

        match self.save_lesson(lesson).await {
            Ok(()) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Durable lesson successfully saved: '{}'", lesson.trim()),
                );
                Ok(())
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Durable save_lesson failure: {}", err),
                );
                Err(err)
            }
        }
    }

    pub async fn save_structured_lesson_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        lesson: &StructuredLesson,
    ) -> Result<(), DurableMemoryError> {
        if lesson.id.trim().is_empty() || lesson.task_title.trim().is_empty() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                "Durable store rejected invalid structured lesson".to_string(),
            );
            return Err(DurableMemoryError::EmptyContent);
        }

        match self.save_structured_lesson(lesson).await {
            Ok(()) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Structured lesson successfully saved: '{}'", lesson.id),
                );
                Ok(())
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Durable save_structured_lesson failure: {}", err),
                );
                Err(err)
            }
        }
    }

    pub async fn load_relevant_context_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        goal: &str,
    ) -> Result<String, DurableMemoryError> {
        match self.load_relevant_context(goal).await {
            Ok(ctx) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Relevant durable context loaded for goal: '{}'", goal),
                );
                Ok(ctx)
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Durable load_relevant_context failure: {}", err),
                );
                Err(err)
            }
        }
    }

    pub async fn save_user_directive_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        directive: &str,
    ) -> Result<(), DurableMemoryError> {
        if directive.trim().is_empty() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                "Durable store rejected empty user directive".to_string(),
            );
            return Err(DurableMemoryError::EmptyContent);
        }

        match self.save_user_directive(directive).await {
            Ok(()) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("User directive successfully saved: '{}'", directive.trim()),
                );
                Ok(())
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Durable save_user_directive failure: {}", err),
                );
                Err(err)
            }
        }
    }
}
