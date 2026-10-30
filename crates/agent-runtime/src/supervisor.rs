use crate::bus::InterAgentBus;
use agent_domain::{AgentId, AgentIdentity};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub enum AgentState {
    Idle,
    Working { task_id: String },
    WaitingApproval { reason: String },
    Stopped,
}

pub struct AgentSupervisor {
    agents: Arc<RwLock<HashMap<AgentId, (AgentIdentity, AgentState)>>>,
    #[allow(dead_code)]
    bus: InterAgentBus,
}

impl AgentSupervisor {
    pub fn new(bus: InterAgentBus) -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            bus,
        }
    }

    pub async fn register_agent(&self, identity: AgentIdentity) {
        let mut map = self.agents.write().await;
        map.insert(identity.id.clone(), (identity, AgentState::Idle));
    }

    pub async fn get_agent_state(&self, id: &AgentId) -> Option<AgentState> {
        let map = self.agents.read().await;
        map.get(id).map(|(_, state)| state.clone())
    }

    pub async fn list_agents(&self) -> Vec<(AgentIdentity, AgentState)> {
        let map = self.agents.read().await;
        map.values().cloned().collect()
    }

    pub async fn update_state(&self, id: &AgentId, new_state: AgentState) {
        let mut map = self.agents.write().await;
        if let Some((_, state)) = map.get_mut(id) {
            *state = new_state;
        }
    }
}
