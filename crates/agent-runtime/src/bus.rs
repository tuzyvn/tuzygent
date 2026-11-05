use agent_domain::{ActionSeverity, AuditEventType, AuditLedger, InterAgentEvent};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BusError {
    #[error("Socket timeout while publishing event to inter-agent bus")]
    SocketTimeout,
    #[error("Connection reset by peer during bus communication")]
    ConnectionReset,
    #[error("Transient I/O failure on inter-agent bus: {0}")]
    TransientIo(String),
    #[error("Bus broadcast channel is closed")]
    ChannelClosed,
    #[error("Bus broadcast channel is full / lagging: {0}")]
    ChannelFull(String),
    #[error("Corrupted event detected on bus: {0}")]
    CorruptedEvent(String),
    #[error("Inter-agent bus service temporarily unavailable: {0}")]
    ServiceUnavailable(String),
    #[error("Max retry attempts ({0}) exceeded during bus event dispatch")]
    MaxRetriesExceeded(usize),
    #[error("Bus capacity overload: {0}")]
    CapacityOverload(String),
    #[error("Bus operation panicked: {0}")]
    Panic(String),
    #[error("Bus receiver lagged and missed {0} messages")]
    ReceiverLagged(u64),
    #[error("Inter-agent bus error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusFault {
    SocketTimeout,
    ConnectionReset,
    TransientIo,
    ChannelClosed,
    ChannelFull,
    CapacityOverload,
    CorruptedEvent,
    ServiceUnavailable,
    Panic,
}

impl BusFault {
    pub fn to_error(&self) -> BusError {
        match self {
            Self::SocketTimeout => BusError::SocketTimeout,
            Self::ConnectionReset => BusError::ConnectionReset,
            Self::TransientIo => BusError::TransientIo(
                "Simulated transient I/O failure on inter-agent event bus".to_string(),
            ),
            Self::ChannelClosed => BusError::ChannelClosed,
            Self::ChannelFull => BusError::ChannelFull(
                "Simulated broadcast channel capacity saturation".to_string(),
            ),
            Self::CapacityOverload => BusError::CapacityOverload(
                "Simulated capacity overload threshold exceeded on bus".to_string(),
            ),
            Self::CorruptedEvent => BusError::CorruptedEvent(
                "Simulated malformed or unparseable event packet".to_string(),
            ),
            Self::ServiceUnavailable => BusError::ServiceUnavailable(
                "Simulated temporary bus backend downtime".to_string(),
            ),
            Self::Panic => BusError::Panic(
                "Simulated unexpected panic during bus dispatch".to_string(),
            ),
        }
    }

    pub fn simulate_fault(&self) -> Result<(), BusError> {
        Err(self.to_error())
    }
}

pub fn simulate_bus_fault(fault: &BusFault) -> Result<(), BusError> {
    fault.simulate_fault()
}

#[derive(Debug, Clone, Default)]
pub struct BusFaultPolicy {
    fault: Option<BusFault>,
    fail_interval: Option<u32>,
    max_injections: Option<usize>,
    invocation_count: u32,
    injected_count: usize,
}

impl BusFaultPolicy {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn always(fault: BusFault) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: None,
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn intermittent(fault: BusFault, interval: u32) -> Self {
        Self {
            fault: Some(fault),
            fail_interval: Some(interval.max(1)),
            max_injections: None,
            invocation_count: 0,
            injected_count: 0,
        }
    }

    pub fn with_max_injections(mut self, max: usize) -> Self {
        self.max_injections = Some(max);
        self
    }

    pub fn should_inject(&mut self) -> Option<BusFault> {
        let fault = self.fault.as_ref()?;

        if let Some(max) = self.max_injections {
            if self.injected_count >= max {
                return None;
            }
        }

        self.invocation_count = self.invocation_count.wrapping_add(1);

        let inject = match self.fail_interval {
            Some(interval) if interval > 0 => (self.invocation_count - 1).is_multiple_of(interval),
            Some(_) => true,
            None => true,
        };

        if inject {
            self.injected_count += 1;
            Some(fault.clone())
        } else {
            None
        }
    }

    pub fn injected_count(&self) -> usize {
        self.injected_count
    }

    pub fn reset(&mut self) {
        self.invocation_count = 0;
        self.injected_count = 0;
    }
}

#[derive(Clone)]
pub struct InterAgentBus {
    sender: Arc<RwLock<broadcast::Sender<InterAgentEvent>>>,
    closed: Arc<AtomicBool>,
    capacity: Arc<AtomicUsize>,
    max_payload_bytes: Arc<AtomicUsize>,
    overload_watermark: Arc<AtomicUsize>,
}

impl InterAgentBus {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        let (sender, _) = broadcast::channel(cap);
        Self {
            sender: Arc::new(RwLock::new(sender)),
            closed: Arc::new(AtomicBool::new(false)),
            capacity: Arc::new(AtomicUsize::new(cap)),
            max_payload_bytes: Arc::new(AtomicUsize::new(0)),
            overload_watermark: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity.load(Ordering::SeqCst)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    pub fn reconstitute(&self, new_capacity: Option<usize>) -> usize {
        let cap = new_capacity.unwrap_or_else(|| self.capacity()).max(1);
        let (new_sender, _) = broadcast::channel(cap);
        {
            let mut guard = self.sender.write().unwrap_or_else(|p| p.into_inner());
            *guard = new_sender;
        }
        self.capacity.store(cap, Ordering::SeqCst);
        self.closed.store(false, Ordering::SeqCst);
        cap
    }

    pub fn set_max_payload_bytes(&self, max: Option<usize>) {
        self.max_payload_bytes.store(max.unwrap_or(0), Ordering::SeqCst);
    }

    pub fn set_overload_watermark(&self, watermark: Option<usize>) {
        self.overload_watermark.store(watermark.unwrap_or(0), Ordering::SeqCst);
    }

    pub fn publish(&self, event: InterAgentEvent) -> Result<usize, String> {
        let catch_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if self.is_closed() {
                return Err("Bus broadcast channel is closed".to_string());
            }

            let max_bytes = self.max_payload_bytes.load(Ordering::SeqCst);
            if max_bytes > 0 {
                let size = serde_json::to_string(&event.payload).map(|s| s.len()).unwrap_or(0);
                if size > max_bytes {
                    return Err(format!(
                        "Bus payload overload: payload size {} exceeds limit of {} bytes",
                        size, max_bytes
                    ));
                }
            }

            let watermark = self.overload_watermark.load(Ordering::SeqCst);
            let sender = self.sender.read().unwrap_or_else(|p| p.into_inner());
            if watermark > 0 && sender.receiver_count() > watermark {
                return Err(format!(
                    "Bus capacity overload: receiver count {} exceeds watermark {}",
                    sender.receiver_count(),
                    watermark
                ));
            }

            if sender.receiver_count() == 0 {
                return Ok(0);
            }

            match sender.send(event) {
                Ok(recipients) => Ok(recipients),
                Err(_) => {
                    if sender.receiver_count() == 0 {
                        Ok(0)
                    } else {
                        Err("Failed to publish inter-agent event: broadcast channel error".to_string())
                    }
                }
            }
        }));

        match catch_res {
            Ok(res) => res,
            Err(panic_payload) => {
                let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic during bus publication".to_string()
                };
                Err(format!("Bus publish panicked: {}", msg))
            }
        }
    }

    pub fn safe_publish(&self, event: InterAgentEvent) -> Result<usize, BusError> {
        self.publish(event).map_err(|e| {
            if e.contains("closed") {
                BusError::ChannelClosed
            } else if e.contains("overload") || e.contains("capacity") {
                BusError::CapacityOverload(e)
            } else if e.contains("panicked") {
                BusError::Panic(e)
            } else {
                BusError::Other(e)
            }
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<InterAgentEvent> {
        let sender = self.sender.read().unwrap_or_else(|p| p.into_inner());
        sender.subscribe()
    }

    pub fn subscribe_resilient(&self) -> ResilientSubscriber {
        ResilientSubscriber::new(self.subscribe())
    }

    pub fn receiver_count(&self) -> usize {
        let sender = self.sender.read().unwrap_or_else(|p| p.into_inner());
        sender.receiver_count()
    }
}

#[derive(Debug)]
pub struct ResilientSubscriber {
    receiver: broadcast::Receiver<InterAgentEvent>,
    total_lagged_messages: u64,
    auto_catchup: bool,
}

impl ResilientSubscriber {
    pub fn new(receiver: broadcast::Receiver<InterAgentEvent>) -> Self {
        Self {
            receiver,
            total_lagged_messages: 0,
            auto_catchup: true,
        }
    }

    pub fn with_auto_catchup(mut self, auto_catchup: bool) -> Self {
        self.auto_catchup = auto_catchup;
        self
    }

    pub fn total_lagged(&self) -> u64 {
        self.total_lagged_messages
    }

    pub fn inner_mut(&mut self) -> &mut broadcast::Receiver<InterAgentEvent> {
        &mut self.receiver
    }

    pub async fn recv_resilient(&mut self) -> Result<InterAgentEvent, BusError> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Ok(event),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    self.total_lagged_messages += missed;
                    tracing::warn!(
                        missed_messages = missed,
                        total_missed = self.total_lagged_messages,
                        "Bus subscriber lagged due to capacity overload"
                    );
                    if self.auto_catchup {
                        continue;
                    } else {
                        return Err(BusError::ReceiverLagged(missed));
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(BusError::ChannelClosed);
                }
            }
        }
    }

    pub fn try_recv_resilient(&mut self) -> Result<Option<InterAgentEvent>, BusError> {
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Lagged(missed)) => {
                self.total_lagged_messages += missed;
                if self.auto_catchup {
                    match self.receiver.try_recv() {
                        Ok(event) => Ok(Some(event)),
                        Err(broadcast::error::TryRecvError::Empty) => Ok(None),
                        Err(broadcast::error::TryRecvError::Lagged(m2)) => {
                            self.total_lagged_messages += m2;
                            Err(BusError::ReceiverLagged(missed + m2))
                        }
                        Err(broadcast::error::TryRecvError::Closed) => Err(BusError::ChannelClosed),
                    }
                } else {
                    Err(BusError::ReceiverLagged(missed))
                }
            }
            Err(broadcast::error::TryRecvError::Closed) => Err(BusError::ChannelClosed),
        }
    }
}

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
            return Err(BusError::CorruptedEvent("Event ID cannot be empty".to_string()));
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
