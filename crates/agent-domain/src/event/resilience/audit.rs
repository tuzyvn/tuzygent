use super::ResilientEventProcessor;
use crate::audit_ledger::{AuditEventType, AuditLedger};
use crate::event::fault::{EventError, EventFault};
use crate::event::InterAgentEvent;
use crate::guardrail::ActionSeverity;

impl ResilientEventProcessor {
    pub fn process_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        event: InterAgentEvent,
    ) -> Result<Option<InterAgentEvent>, EventError> {
        if let Err(err) = event.validate() {
            self.rejected_count += 1;
            ledger.record_event(
                AuditEventType::CommandBlocked,
                ActionSeverity::Destructive,
                format!(
                    "Event '{}' rejected due to invalid structure: {}",
                    event.id, err
                ),
            );
            return Err(err);
        }
        if self.seen_ids.contains(&event.id) {
            ledger.record_event(
                AuditEventType::CommandAllowed,
                ActionSeverity::Routine,
                format!("Event '{}' deduplicated and discarded", event.id),
            );
            return Ok(None);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.policy.should_inject() {
                Some(EventFault::Dropped) => {
                    self.dropped_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandBlocked,
                        ActionSeverity::Destructive,
                        format!("Event '{}' dropped in transit during chaos test", event.id),
                    );
                    return Ok(None);
                }
                Some(EventFault::Duplicate) => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!(
                            "Event '{}' processed under duplicate fault injection",
                            event.id
                        ),
                    );
                    return Ok(Some(event));
                }
                Some(EventFault::CorruptedPayload) => {
                    self.rejected_count += 1;
                    let err = EventError::CorruptedPayload(
                        "Packet payload checksum failure or corrupted JSON".to_string(),
                    );
                    ledger.record_event(
                        AuditEventType::CommandException,
                        ActionSeverity::HighImpact,
                        format!(
                            "Event '{}' rejected due to corrupted payload: {}",
                            event.id, err
                        ),
                    );
                    return Err(err);
                }
                Some(fault) => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                    } else {
                        let err = fault.to_error();
                        ledger.record_event(
                            AuditEventType::CommandException,
                            ActionSeverity::HighImpact,
                            format!(
                                "Event '{}' failed after {} retries: {}",
                                event.id, self.max_retries, err
                            ),
                        );
                        return Err(err);
                    }
                }
                None => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!("Event '{}' processed successfully", event.id),
                    );
                    return Ok(Some(event));
                }
            }
        }
    }
}
