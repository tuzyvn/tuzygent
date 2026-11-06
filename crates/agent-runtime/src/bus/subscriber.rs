use crate::bus::fault::BusError;
use agent_domain::InterAgentEvent;
use tokio::sync::broadcast;

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
