use agent_domain::InterAgentEvent;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct InterAgentBus {
    sender: broadcast::Sender<InterAgentEvent>,
}

impl InterAgentBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn publish(&self, event: InterAgentEvent) -> Result<usize, String> {
        if self.sender.receiver_count() == 0 {
            return Ok(0);
        }
        self.sender
            .send(event)
            .map_err(|e| format!("Failed to publish inter-agent event: {}", e))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<InterAgentEvent> {
        self.sender.subscribe()
    }
}
