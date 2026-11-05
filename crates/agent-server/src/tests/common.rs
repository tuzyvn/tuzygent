use std::sync::Arc;

use agent_core::prelude::*;
use axum::Router;

use crate::{build_app, AppState, AuthState};

pub fn app() -> Result<Router, Box<dyn std::error::Error>> {
    let bus = InterAgentBus::new(16);
    let temp_vault = std::env::temp_dir().join(format!("test_vault_{}", uuid::Uuid::new_v4().simple()));
    let task_store = Arc::new(agent_runtime::SqliteTaskStore::memory()?);
    let task_service = Arc::new(agent_runtime::TaskService::new(task_store, "/tmp", 2, None, None));
    Ok(build_app(
        AppState {
            supervisor: Arc::new(AgentSupervisor::new(bus.clone())),
            bus,
            workspace: "/tmp".into(),
            vault: Arc::new(agent_engine::AccountVault::new_with_dir(temp_vault)),
            task_service,
        },
        AuthState {
            token: Arc::new("test-secret".into()),
            allowed_origins: vec!["https://dashboard.example".parse()?],
        },
    ))
}
