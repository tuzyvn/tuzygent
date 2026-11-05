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
