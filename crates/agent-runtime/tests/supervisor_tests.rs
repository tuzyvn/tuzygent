use agent_domain::{AgentId, AgentIdentity, AutonomyLevel};
use agent_runtime::{AgentState, AgentSupervisor, InterAgentBus};
use chrono::Utc;

fn identity(id: AgentId, name: &str) -> AgentIdentity {
    AgentIdentity {
        id,
        name: name.to_string(),
        description: format!("{name} agent"),
        autonomy_level: AutonomyLevel::Supervised,
        created_at: Utc::now(),
    }
}

fn supervisor() -> AgentSupervisor {
    AgentSupervisor::new(InterAgentBus::new(16))
}

#[tokio::test]
async fn register_agent_starts_idle_and_re_registration_replaces_identity() {
    let supervisor = supervisor();
    let id = AgentId::from_str("agent-1");
    let first_identity = identity(id.clone(), "first");

    supervisor.register_agent(first_identity.clone()).await;

    let agents = supervisor.list_agents().await;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].0, first_identity);
    assert!(matches!(agents[0].1, AgentState::Idle));
    assert!(matches!(
        supervisor.get_agent_state(&id).await,
        Some(AgentState::Idle)
    ));

    supervisor
        .update_state(
            &id,
            AgentState::Working {
                task_id: "task-before-reregister".to_string(),
            },
        )
        .await;
    let replacement_identity = identity(id.clone(), "replacement");
    supervisor
        .register_agent(replacement_identity.clone())
        .await;

    let agents = supervisor.list_agents().await;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].0, replacement_identity);
    assert!(matches!(agents[0].1, AgentState::Idle));
}

#[tokio::test]
async fn get_agent_state_returns_none_for_an_unregistered_agent() {
    let supervisor = supervisor();
    assert!(supervisor
        .get_agent_state(&AgentId::from_str("missing-agent"))
        .await
        .is_none());
}

#[tokio::test]
async fn update_state_updates_registered_agents_and_ignores_unknown_ids() {
    let supervisor = supervisor();
    let id = AgentId::from_str("agent-1");
    let registered_identity = identity(id.clone(), "worker");
    supervisor.register_agent(registered_identity.clone()).await;

    supervisor
        .update_state(
            &id,
            AgentState::Working {
                task_id: "task-42".to_string(),
            },
        )
        .await;
    assert!(matches!(
        supervisor.get_agent_state(&id).await,
        Some(AgentState::Working { task_id }) if task_id == "task-42"
    ));

    supervisor
        .update_state(
            &id,
            AgentState::WaitingApproval {
                reason: "needs review".to_string(),
            },
        )
        .await;
    assert!(matches!(
        supervisor.get_agent_state(&id).await,
        Some(AgentState::WaitingApproval { reason }) if reason == "needs review"
    ));

    supervisor.update_state(&id, AgentState::Stopped).await;
    assert!(matches!(
        supervisor.get_agent_state(&id).await,
        Some(AgentState::Stopped)
    ));

    let before_unknown_update = supervisor.list_agents().await;
    supervisor
        .update_state(
            &AgentId::from_str("missing-agent"),
            AgentState::Working {
                task_id: "task-ignored".to_string(),
            },
        )
        .await;
    let after_unknown_update = supervisor.list_agents().await;

    assert_eq!(after_unknown_update.len(), before_unknown_update.len());
    assert_eq!(after_unknown_update[0].0, registered_identity);
    assert!(matches!(after_unknown_update[0].1, AgentState::Stopped));
}
