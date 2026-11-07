use crate::bus::channel::InterAgentBus;
use crate::bus::fault::BusError;
use crate::bus::policy::BusFaultPolicy;
use crate::bus::subscriber::ResilientSubscriber;
use agent_domain::{ActionSeverity, AuditEventType, AuditLedger, InterAgentEvent};
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct ResilientInterAgentBus {
    bus: InterAgentBus,
    policy: BusFaultPolicy,
    max_retries: usize,
    auto_reconstitute_on_failure: bool,
    published_count: usize,
    recovered_count: usize,
    rejected_count: usize,
    failed_count: usize,
    reconstituted_count: usize,
    panics_caught_count: usize,
}

impl ResilientInterAgentBus {
    pub fn new(capacity: usize, policy: BusFaultPolicy, max_retries: usize) -> Self {
        Self {
            bus: InterAgentBus::new(capacity),
            policy,
            max_retries,
            auto_reconstitute_on_failure: true,
            published_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            failed_count: 0,
            reconstituted_count: 0,
            panics_caught_count: 0,
        }
    }

    pub fn from_bus(bus: InterAgentBus, policy: BusFaultPolicy, max_retries: usize) -> Self {
        Self {
            bus,
            policy,
            max_retries,
            auto_reconstitute_on_failure: true,
            published_count: 0,
            recovered_count: 0,
            rejected_count: 0,
            failed_count: 0,
            reconstituted_count: 0,
            panics_caught_count: 0,
        }
    }

    pub fn with_auto_reconstitute(mut self, auto: bool) -> Self {
        self.auto_reconstitute_on_failure = auto;
        self
    }

    pub fn with_max_capacity(self, cap: usize) -> Self {
        self.bus.reconstitute(Some(cap));
        self
    }

    pub fn bus(&self) -> &InterAgentBus {
        &self.bus
    }

    pub fn bus_mut(&mut self) -> &mut InterAgentBus {
        &mut self.bus
    }

    pub fn policy(&self) -> &BusFaultPolicy {
        &self.policy
    }

    pub fn policy_mut(&mut self) -> &mut BusFaultPolicy {
        &mut self.policy
    }

    pub fn subscribe(&self) -> broadcast::Receiver<InterAgentEvent> {
        self.bus.subscribe()
    }

    pub fn subscribe_resilient(&self) -> ResilientSubscriber {
        self.bus.subscribe_resilient()
    }

    pub fn receiver_count(&self) -> usize {
        self.bus.receiver_count()
    }

    pub fn published_count(&self) -> usize {
        self.published_count
    }

    pub fn recovered_count(&self) -> usize {
        self.recovered_count
    }

    pub fn rejected_count(&self) -> usize {
        self.rejected_count
    }

    pub fn failed_count(&self) -> usize {
        self.failed_count
    }

    pub fn reconstituted_count(&self) -> usize {
        self.reconstituted_count
    }

    pub fn panics_caught_count(&self) -> usize {
        self.panics_caught_count
    }

    pub fn reconstitute_bus(&mut self, new_capacity: Option<usize>) -> usize {
        self.reconstituted_count += 1;
        self.bus.reconstitute(new_capacity)
    }

    pub fn memory_usage_bytes(&self) -> usize {
        let bus_mem = self.bus.capacity() * std::mem::size_of::<InterAgentEvent>();
        std::mem::size_of::<Self>() + bus_mem
    }

    fn is_transient_error(err: &BusError) -> bool {
        matches!(
            err,
            BusError::SocketTimeout
                | BusError::ConnectionReset
                | BusError::TransientIo(_)
                | BusError::ServiceUnavailable(_)
                | BusError::ChannelFull(_)
                | BusError::CapacityOverload(_)
                | BusError::Panic(_)
        )
    }

    pub fn publish_resilient(&mut self, event: InterAgentEvent) -> Result<usize, BusError> {
        if event.id.is_empty() {
            self.rejected_count += 1;
            return Err(BusError::CorruptedEvent(
                "Event ID cannot be empty".to_string(),
            ));
        }

        let mut attempt = 0;
        loop {
            attempt += 1;

            if let Some(fault) = self.policy.should_inject() {
                let err = fault.to_error();
                if !Self::is_transient_error(&err) {
                    self.rejected_count += 1;
                    return Err(err);
                }

                if attempt <= self.max_retries {
                    self.recovered_count += 1;
                    if self.auto_reconstitute_on_failure && matches!(err, BusError::ChannelClosed) {
                        self.bus.reconstitute(None);
                        self.reconstituted_count += 1;
                    }
                    continue;
                } else {
                    self.failed_count += 1;
                    return Err(BusError::MaxRetriesExceeded(self.max_retries));
                }
            }

            let send_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.bus.publish(event.clone())
            }));

            let pub_res = match send_result {
                Ok(res) => res,
                Err(panic_payload) => {
                    self.panics_caught_count += 1;
                    let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "Panic caught during bus publication".to_string()
                    };
                    Err(format!("Bus publish panicked: {}", msg))
                }
            };

            match pub_res {
                Ok(recipients) => {
                    self.published_count += 1;
                    return Ok(recipients);
                }
                Err(err_msg) => {
                    let is_closed = self.bus.is_closed() || err_msg.contains("closed");
                    let is_panic = err_msg.contains("panicked");

                    if is_panic {
                        self.panics_caught_count += 1;
                    }

                    if self.auto_reconstitute_on_failure && is_closed {
                        self.bus.reconstitute(None);
                        self.reconstituted_count += 1;
                    }

                    if attempt <= self.max_retries {
                        self.recovered_count += 1;
                        continue;
                    }

                    self.failed_count += 1;
                    if is_panic {
                        return Err(BusError::Panic(err_msg));
                    } else if is_closed {
                        return Err(BusError::ChannelClosed);
                    } else if err_msg.contains("overload") || err_msg.contains("capacity") {
                        return Err(BusError::CapacityOverload(err_msg));
                    } else {
                        return Err(BusError::Other(err_msg));
                    }
                }
            }
        }
    }

    pub fn publish_resilient_batch(
        &mut self,
        events: Vec<InterAgentEvent>,
    ) -> Result<usize, BusError> {
        for event in &events {
            if event.id.is_empty() {
                self.rejected_count += 1;
                return Err(BusError::CorruptedEvent(
                    "Event ID cannot be empty".to_string(),
                ));
            }
        }

        let mut successful = 0;
        for event in events {
            match self.publish_resilient(event) {
                Ok(_) => successful += 1,
                Err(e) => return Err(e),
            }
        }
        Ok(successful)
    }

    pub fn publish_and_record(
        &mut self,
        ledger: &mut AuditLedger,
        event: InterAgentEvent,
    ) -> Result<usize, BusError> {
        let event_id = event.id.clone();
        let topic = event.event_type.clone();

        let catch_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.publish_resilient(event)
        }));

        let pub_res = match catch_res {
            Ok(res) => res,
            Err(panic_payload) => {
                self.panics_caught_count += 1;
                let panic_msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Panic caught during publish_and_record".to_string()
                };
                Err(BusError::Panic(panic_msg))
            }
        };

        match pub_res {
            Ok(recipients) => {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ledger.record_event(
                        AuditEventType::CommandAllowed,
                        ActionSeverity::Routine,
                        format!(
                            "InterAgentBus event '{}' [topic: {}] published successfully to {} receivers",
                            event_id, topic, recipients
                        ),
                    );
                }));
                Ok(recipients)
            }
            Err(err) => {
                let severity = match &err {
                    BusError::CorruptedEvent(_) | BusError::ChannelClosed | BusError::Panic(_) => {
                        ActionSeverity::Destructive
                    }
                    _ => ActionSeverity::HighImpact,
                };
                let event_type = match &err {
                    BusError::CorruptedEvent(_) => AuditEventType::CommandBlocked,
                    _ => AuditEventType::CommandException,
                };
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ledger.record_event(
                        event_type,
                        severity,
                        format!(
                            "InterAgentBus publish failure for event '{}' [topic: {}]: {}",
                            event_id, topic, err
                        ),
                    );
                }));
                Err(err)
            }
        }
    }
}
