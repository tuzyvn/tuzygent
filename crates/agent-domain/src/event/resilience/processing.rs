use super::ResilientEventProcessor;
use crate::event::fault::{EventError, EventFault};
use crate::event::InterAgentEvent;

impl ResilientEventProcessor {
    pub fn process_event(
        &mut self,
        event: InterAgentEvent,
    ) -> Result<Option<InterAgentEvent>, EventError> {
        if let Err(err) = event.validate() {
            self.rejected_count += 1;
            return Err(err);
        }
        if self.seen_ids.contains(&event.id) {
            return Ok(None);
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.policy.should_inject() {
                Some(EventFault::Dropped) => {
                    self.dropped_count += 1;
                    return Ok(None);
                }
                Some(EventFault::Duplicate) => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    return Ok(Some(event));
                }
                Some(EventFault::CorruptedPayload) => {
                    self.rejected_count += 1;
                    return Err(EventError::CorruptedPayload(
                        "Packet payload checksum failure or corrupted JSON".to_string(),
                    ));
                }
                Some(fault) => {
                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                    } else {
                        return Err(fault.to_error());
                    }
                }
                None => {
                    self.record_seen(event.id.clone());
                    self.processed_count += 1;
                    return Ok(Some(event));
                }
            }
        }
    }

    pub fn process_batch(
        &mut self,
        events: Vec<InterAgentEvent>,
    ) -> Result<Vec<InterAgentEvent>, EventError> {
        let mut processed = Vec::with_capacity(events.len());
        for event in events {
            if let Some(ev) = self.process_event(event)? {
                processed.push(ev);
            }
        }
        Ok(processed)
    }
}
