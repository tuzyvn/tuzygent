use super::fault::HarnessError;
use super::resilience::ResilientHarnessMemoryStore;
use super::HarnessLesson;
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger};

impl ResilientHarnessMemoryStore {
    pub async fn record_lesson_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        node_type: &str,
        identifier: &str,
        error_pattern: &str,
        remediation_strategy: &str,
    ) -> Result<HarnessLesson, HarnessError> {
        if node_type.trim().is_empty() || identifier.trim().is_empty() {
            self.failed_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                "Harness store rejected empty node_type or identifier".to_string(),
            );
            return Err(HarnessError::InvalidParameter(
                "node_type and identifier cannot be blank".to_string(),
            ));
        }

        match self
            .record_lesson_resilient(node_type, identifier, error_pattern, remediation_strategy)
            .await
        {
            Ok(lesson) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!(
                        "Harness lesson recorded: '{}' for node {}",
                        lesson.id, node_type
                    ),
                );
                Ok(lesson)
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Harness record_lesson failure: {}", err),
                );
                Err(err)
            }
        }
    }

    pub async fn load_lessons_and_record(
        &mut self,
        ledger: &mut AuditLedger,
    ) -> Result<Vec<HarnessLesson>, HarnessError> {
        match self.load_lessons_resilient().await {
            Ok(lessons) => {
                ledger.record_event(
                    AuditEventType::CommandAllowed,
                    ActionSeverity::Routine,
                    format!("Loaded {} harness lessons successfully", lessons.len()),
                );
                Ok(lessons)
            }
            Err(err) => {
                ledger.record_event(
                    AuditEventType::CommandException,
                    ActionSeverity::HighImpact,
                    format!("Harness load_lessons failure: {}", err),
                );
                Err(err)
            }
        }
    }
}
