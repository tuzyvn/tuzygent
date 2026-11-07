use agent_core::prelude::*;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAgentReq {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub autonomy: Option<String>,
}

pub async fn list_agents_handler(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.supervisor.list_agents().await;
    let list: Vec<serde_json::Value> = agents
        .into_iter()
        .map(|(identity, agent_state)| {
            serde_json::json!({
                "id": identity.id.to_string(),
                "name": identity.name,
                "description": identity.description,
                "autonomy_level": format!("{:?}", identity.autonomy_level),
                "state": format!("{:?}", agent_state),
                "created_at": identity.created_at,
            })
        })
        .collect();

    Json(serde_json::json!({ "agents": list }))
}

pub async fn create_agent_handler(
    State(state): State<AppState>,
    Json(payload): Json<CreateAgentReq>,
) -> impl IntoResponse {
    let level = match payload.autonomy.as_deref().unwrap_or("full") {
        "supervised" => AutonomyLevel::Supervised,
        "shadow" => AutonomyLevel::ShadowMode,
        _ => AutonomyLevel::Full,
    };

    let identity = AgentIdentity {
        id: AgentId::from_str(&payload.id),
        name: payload.name,
        description: payload
            .description
            .unwrap_or_else(|| "Autonomous Digital Employee".to_string()),
        autonomy_level: level,
        created_at: chrono::Utc::now(),
    };

    state.supervisor.register_agent(identity.clone()).await;
    info!("✨ Registered agent via REST: {}", identity.id);

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "success": true,
            "agent_id": identity.id.to_string(),
            "name": identity.name,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::CreateAgentReq;

    #[test]
    fn create_agent_request_serialization_preserves_json_contract() {
        let request = CreateAgentReq {
            id: "agent-42".to_string(),
            name: "Research Agent".to_string(),
            description: Some("Reviews source material".to_string()),
            autonomy: None,
        };

        let serialized = serde_json::to_value(&request).expect("request serializes");
        assert_eq!(
            serialized,
            serde_json::json!({
                "id": "agent-42",
                "name": "Research Agent",
                "description": "Reviews source material",
                "autonomy": null,
            })
        );

        let round_trip: CreateAgentReq =
            serde_json::from_value(serialized).expect("serialized request deserializes");
        assert_eq!(round_trip.id, request.id);
        assert_eq!(round_trip.name, request.name);
        assert_eq!(round_trip.description, request.description);
        assert_eq!(round_trip.autonomy, request.autonomy);
    }

    #[test]
    fn create_agent_request_defaults_missing_optional_fields_and_ignores_unknown_fields() {
        let request: CreateAgentReq = serde_json::from_value(serde_json::json!({
            "id": "agent-7",
            "name": "Default Agent",
            "future_field": true,
        }))
        .expect("optional fields default and unknown fields are ignored");

        assert_eq!(request.id, "agent-7");
        assert_eq!(request.name, "Default Agent");
        assert_eq!(request.description, None);
        assert_eq!(request.autonomy, None);
    }
}
