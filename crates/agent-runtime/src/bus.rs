use tokio::sync::broadcast;
use agent_domain::InterAgentEvent;

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
        self.sender
            .send(event)
            .map_err(|e| format!("Failed to publish inter-agent event: {}", e))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<InterAgentEvent> {
        self.sender.subscribe()
    }
}
