use agent_domain::{AgentId, AgentIdentity, AutonomyLevel};
use agent_runtime::{AgentSupervisor, InterAgentBus};
use std::sync::Arc;

pub async fn create(id: String, name: String, autonomy: String) -> anyhow::Result<()> {
    let bus = InterAgentBus::new(100);
    let supervisor = Arc::new(AgentSupervisor::new(bus));

    let level = match autonomy.to_lowercase().as_str() {
        "supervised" => AutonomyLevel::Supervised,
        "shadow" => AutonomyLevel::ShadowMode,
        _ => AutonomyLevel::Full,
    };

    let identity = AgentIdentity {
        id: AgentId::from_str(&id),
        name,
        description: "Generic Autonomous Digital Employee".to_string(),
        autonomy_level: level,
        created_at: chrono::Utc::now(),
    };

    supervisor.register_agent(identity.clone()).await;
    println!("✨ Created Agent: {} ({})", identity.name, identity.id);
    Ok(())
}

pub async fn list() -> anyhow::Result<()> {
    let bus = InterAgentBus::new(100);
    let supervisor = Arc::new(AgentSupervisor::new(bus));

    println!("👥 Registered Autonomous Agents:");
    let agents = supervisor.list_agents().await;
    if agents.is_empty() {
        println!("   (No agents found. Use `agent-ctl create` or try `agent-ctl multi-agent`)");
    } else {
        for (id, state) in agents {
            println!(
                "   • {} [{:?}] - State: {:?}",
                id.name, id.autonomy_level, state
            );
        }
    }
    Ok(())
}
