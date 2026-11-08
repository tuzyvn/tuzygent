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
    use super::{create_agent_handler, list_agents_handler, CreateAgentReq};
    use crate::state::AppState;
    use agent_core::prelude::*;
    use axum::{extract::State, response::IntoResponse, Json};
    use std::sync::Arc;

    fn test_state() -> AppState {
        let bus = InterAgentBus::new(16);
        let task_store = Arc::new(agent_runtime::SqliteTaskStore::memory().unwrap());
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(
                std::env::temp_dir().join(format!("test-vault-{}", uuid::Uuid::new_v4())),
            )),
            task_service: Arc::new(agent_runtime::TaskService::new(
                task_store, "/tmp", 2, None, None,
            )),
            metrics: Arc::new(crate::metrics::ResilientMetricsService::default()),
        }
    }

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

    #[tokio::test]
    async fn list_agents_handler_returns_registered_agent_details() {
        let state = test_state();
        state
            .supervisor
            .register_agent(AgentIdentity {
                id: AgentId::from_str("agent-list-test"),
                name: "Listed Agent".into(),
                description: "An agent for the list response".into(),
                autonomy_level: AutonomyLevel::Supervised,
                created_at: chrono::Utc::now(),
            })
            .await;

        let response = list_agents_handler(State(state)).await.into_response();
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .expect("response body is readable");
        let value: serde_json::Value =
            serde_json::from_slice(&body).expect("response is valid JSON");

        assert_eq!(value["agents"].as_array().unwrap().len(), 1);
        assert_eq!(value["agents"][0]["id"], "agent-list-test");
        assert_eq!(value["agents"][0]["name"], "Listed Agent");
        assert_eq!(value["agents"][0]["description"], "An agent for the list response");
        assert_eq!(value["agents"][0]["autonomy_level"], "Supervised");
    }

    #[tokio::test]
    async fn create_agent_handler_returns_created_and_registers_agent() {
        let state = test_state();
        let response = create_agent_handler(
            State(state.clone()),
            Json(CreateAgentReq {
                id: "agent-create-test".into(),
                name: "Created Agent".into(),
                description: Some("Created through the handler".into()),
                autonomy: Some("shadow".into()),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), axum::http::StatusCode::CREATED);
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .expect("response body is readable");
        let value: serde_json::Value =
            serde_json::from_slice(&body).expect("response is valid JSON");
        assert_eq!(
            value,
            serde_json::json!({
                "success": true,
                "agent_id": "agent-create-test",
                "name": "Created Agent",
            })
        );

        let agents = state.supervisor.list_agents().await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].0.id.to_string(), "agent-create-test");
        assert_eq!(agents[0].0.name, "Created Agent");
        assert_eq!(agents[0].0.description, "Created through the handler");
        assert_eq!(agents[0].0.autonomy_level, AutonomyLevel::ShadowMode);
    }
}
