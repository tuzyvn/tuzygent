use crate::bus::fault::BusError;
use crate::bus::subscriber::ResilientSubscriber;
use agent_domain::InterAgentEvent;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

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

    pub fn publish_batch(&self, events: Vec<InterAgentEvent>) -> Result<usize, String> {
        if events.is_empty() {
            return Ok(0);
        }

        let catch_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if self.is_closed() {
                return Err("Bus broadcast channel is closed".to_string());
            }

            let max_bytes = self.max_payload_bytes.load(Ordering::SeqCst);
            let watermark = self.overload_watermark.load(Ordering::SeqCst);
            let sender = self.sender.read().unwrap_or_else(|p| p.into_inner());

            if watermark > 0 && sender.receiver_count() > watermark {
                return Err(format!(
                    "Bus capacity overload: receiver count {} exceeds watermark {}",
                    sender.receiver_count(),
                    watermark
                ));
            }

            let mut count = 0;
            for event in events {
                if max_bytes > 0 {
                    let size = serde_json::to_string(&event.payload).map(|s| s.len()).unwrap_or(0);
                    if size > max_bytes {
                        return Err(format!(
                            "Bus payload overload: payload size {} exceeds limit of {} bytes",
                            size, max_bytes
                        ));
                    }
                }

                if sender.receiver_count() > 0 {
                    let _ = sender.send(event);
                }
                count += 1;
            }

            Ok(count)
        }));

        match catch_res {
            Ok(res) => res,
            Err(panic_payload) => {
                let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic during bus batch publication".to_string()
                };
                Err(format!("Bus publish batch panicked: {}", msg))
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
