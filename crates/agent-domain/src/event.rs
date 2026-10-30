use crate::identity::AgentId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterAgentEvent {
    pub specversion: String,
    pub id: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub time: chrono::DateTime<chrono::Utc>,
    pub from_agent: AgentId,
    pub to_agent: Option<AgentId>,
    pub payload: serde_json::Value,
}

impl InterAgentEvent {
    pub fn new(
        from: AgentId,
        to: Option<AgentId>,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            specversion: "1.0".to_string(),
            id: format!("EVT-{}", Uuid::new_v4().simple()),
            source: format!("urn:agent:{}", from),
            event_type: event_type.to_string(),
            time: chrono::Utc::now(),
            from_agent: from,
            to_agent: to,
            payload,
        }
    }
}
